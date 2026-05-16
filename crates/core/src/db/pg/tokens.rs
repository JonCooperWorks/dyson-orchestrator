use async_trait::async_trait;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::db::pg::map_sqlx;
use crate::db::{INGEST_PROVIDER, state_sync_provider, token_lookup_key};
use crate::envelope::{
    CipherDirectory, EnvelopeCipher, KmsContext, KmsScope, SecretAccessReason, open_context,
    rewrap_context_as_string, seal_context_as_string,
};
use crate::error::StoreError;
use crate::kms_audit::{self, KmsAuditActor, NoopSecretAccessAuditStore};
use crate::now_secs;
use crate::traits::{SecretAccessAuditStore, TokenRecord, TokenStore};

#[derive(Clone)]
pub struct PgTokenStore {
    pool: PgPool,
    cipher: Arc<dyn EnvelopeCipher>,
    ciphers: Option<Arc<dyn CipherDirectory>>,
    audit: Arc<dyn SecretAccessAuditStore>,
}

impl PgTokenStore {
    pub fn new(pool: PgPool, cipher: Arc<dyn EnvelopeCipher>) -> Self {
        Self {
            pool,
            cipher,
            ciphers: None,
            audit: Arc::new(NoopSecretAccessAuditStore),
        }
    }

    pub fn new_with_kms(
        pool: PgPool,
        cipher: Arc<dyn EnvelopeCipher>,
        ciphers: Arc<dyn CipherDirectory>,
        audit: Arc<dyn SecretAccessAuditStore>,
    ) -> Self {
        Self {
            pool,
            cipher,
            ciphers: Some(ciphers),
            audit,
        }
    }

    fn token_context(instance_id: &str, provider: &str) -> KmsContext {
        KmsContext {
            scope: KmsScope::RuntimeToken,
            owner_id: None,
            instance_id: Some(instance_id.to_owned()),
            name: Some(format!("proxy_token:{provider}")),
        }
    }

    fn seal_token(
        &self,
        instance_id: &str,
        provider: &str,
        token: &str,
    ) -> Result<String, StoreError> {
        if let Some(ciphers) = self.ciphers.as_deref() {
            let context = Self::token_context(instance_id, provider);
            return seal_context_as_string(
                ciphers,
                &context,
                token.as_bytes(),
                token_reason(provider),
            )
            .map_err(|e| StoreError::Io(format!("seal proxy token: {e}")));
        }
        let sealed = self
            .cipher
            .seal(token.as_bytes())
            .map_err(|e| StoreError::Io(format!("seal proxy token: {e}")))?;
        String::from_utf8(sealed)
            .map_err(|_| StoreError::Malformed("sealed proxy token was not utf-8".into()))
    }

    async fn open_token(
        &self,
        instance_id: &str,
        provider: &str,
        stored: &str,
    ) -> Result<OpenedToken, StoreError> {
        if let Some(ciphers) = self.ciphers.as_deref() {
            let context = Self::token_context(instance_id, provider);
            let reason = token_reason(provider);
            let actor = KmsAuditActor::runtime(instance_id);
            let opened = match open_context(ciphers, &context, stored.as_bytes(), reason) {
                Ok(opened) => {
                    kms_audit::best_effort_record(
                        self.audit.as_ref(),
                        kms_audit::success_entry(
                            &actor,
                            reason,
                            crate::envelope::SecretAccessOperation::Decrypt,
                            &context,
                            Some(&opened),
                        ),
                    )
                    .await;
                    opened
                }
                Err(err) => {
                    kms_audit::best_effort_record(
                        self.audit.as_ref(),
                        kms_audit::failure_entry(
                            &actor,
                            reason,
                            crate::envelope::SecretAccessOperation::Decrypt,
                            &context,
                            "EnvelopeError",
                            &err.to_string(),
                        ),
                    )
                    .await;
                    return Err(StoreError::Malformed(format!("open proxy token: {err}")));
                }
            };
            let plaintext = String::from_utf8(opened.plaintext.clone())
                .map_err(|_| StoreError::Malformed("proxy token plaintext was not utf-8".into()))?;
            return Ok(OpenedToken {
                plaintext,
                opened: Some(opened),
            });
        }
        let plain = self
            .cipher
            .open(stored.as_bytes())
            .map_err(|e| StoreError::Malformed(format!("open proxy token: {e}")))?;
        let plaintext = String::from_utf8(plain)
            .map_err(|_| StoreError::Malformed("proxy token plaintext was not utf-8".into()))?;
        Ok(OpenedToken {
            plaintext,
            opened: None,
        })
    }

    async fn rewrap_token_if_needed(
        &self,
        instance_id: &str,
        provider: &str,
        stored: &str,
        opened: &OpenedToken,
    ) -> Result<(), StoreError> {
        let Some(opened_meta) = opened.opened.as_ref().filter(|o| o.needs_rewrap) else {
            return Ok(());
        };
        let Some(ciphers) = self.ciphers.as_deref() else {
            return Ok(());
        };
        let context = Self::token_context(instance_id, provider);
        let reason = token_reason(provider);
        let actor = KmsAuditActor::runtime(instance_id);
        let next = match rewrap_context_as_string(
            ciphers,
            &context,
            opened.plaintext.as_bytes(),
            reason,
        ) {
            Ok(next) => next,
            Err(err) => {
                kms_audit::best_effort_record(
                    self.audit.as_ref(),
                    kms_audit::failure_entry(
                        &actor,
                        reason,
                        crate::envelope::SecretAccessOperation::Rewrap,
                        &context,
                        "EnvelopeError",
                        &err.to_string(),
                    ),
                )
                .await;
                return Err(StoreError::Io(format!("rewrap proxy token: {err}")));
            }
        };
        let result = sqlx::query(
            "UPDATE proxy_tokens \
             SET token = $1 \
             WHERE instance_id = $2 AND provider = $3 AND token = $4",
        )
        .bind(&next)
        .bind(instance_id)
        .bind(provider)
        .bind(stored)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        if result.rows_affected() > 0 {
            kms_audit::best_effort_record(
                self.audit.as_ref(),
                kms_audit::success_entry(
                    &actor,
                    reason,
                    crate::envelope::SecretAccessOperation::Rewrap,
                    &context,
                    Some(opened_meta),
                ),
            )
            .await;
        }
        Ok(())
    }
}

struct OpenedToken {
    plaintext: String,
    opened: Option<crate::envelope::OpenEnvelopeResult>,
}

fn token_reason(provider: &str) -> SecretAccessReason {
    if provider == INGEST_PROVIDER {
        SecretAccessReason::ArtefactRead
    } else if provider.starts_with("state_sync:") {
        SecretAccessReason::StateReplay
    } else {
        SecretAccessReason::LlmProviderProxy
    }
}

impl PgTokenStore {
    /// Common mint path — the prefix and provider are the only knobs
    /// the public surfaces (`mint` / `mint_ingest`) flex.  Both paths
    /// share the same row layout in `proxy_tokens` so revoke and
    /// resolve work uniformly across token kinds.
    async fn mint_with_prefix(
        &self,
        prefix: &str,
        instance_id: &str,
        provider: &str,
    ) -> Result<String, StoreError> {
        let token = format!("{prefix}{}", Uuid::new_v4().simple());
        let stored_token = self.seal_token(instance_id, provider, &token)?;
        let lookup = token_lookup_key(&token);
        sqlx::query(
            "INSERT INTO proxy_tokens \
             (token, token_lookup, instance_id, provider, created_at, revoked_at, expected_src_ip) \
             VALUES ($1, $2, $3, $4, $5, NULL, NULL)",
        )
        .bind(&stored_token)
        .bind(&lookup)
        .bind(instance_id)
        .bind(provider)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(token)
    }
}

#[async_trait]
impl TokenStore for PgTokenStore {
    async fn mint(&self, instance_id: &str, provider: &str) -> Result<String, StoreError> {
        // `pt_` prefix lets operators grep proxy tokens out of access
        // logs without false matches against bare 32-hex strings (UUIDs,
        // OR key ids, etc).  128 bits of entropy still come from the
        // UUID body — the prefix is purely for log distinguishability.
        self.mint_with_prefix("pt_", instance_id, provider).await
    }

    async fn mint_ingest(&self, instance_id: &str) -> Result<String, StoreError> {
        // `it_` prefix marks ingest tokens (artefact push from dyson →
        // swarm).  Same row layout as `pt_` proxy tokens; the prefix +
        // `provider = "ingest"` let the internal-ingest route reject
        // chat-provider tokens at the door and let operators grep the
        // table apart.
        self.mint_with_prefix("it_", instance_id, INGEST_PROVIDER)
            .await
    }

    async fn mint_state_sync_for_generation(
        &self,
        instance_id: &str,
        generation: &str,
    ) -> Result<String, StoreError> {
        self.mint_with_prefix("st_", instance_id, &state_sync_provider(generation))
            .await
    }

    async fn bind_expected_src_ip(
        &self,
        instance_id: &str,
        expected_src_ip: &str,
    ) -> Result<(), StoreError> {
        let expected_src_ip = expected_src_ip.trim();
        if expected_src_ip.is_empty() {
            return Ok(());
        }
        sqlx::query(
            "UPDATE proxy_tokens SET expected_src_ip = $1 \
             WHERE instance_id = $2 AND revoked_at IS NULL",
        )
        .bind(expected_src_ip)
        .bind(instance_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn resolve(&self, token: &str) -> Result<Option<TokenRecord>, StoreError> {
        let lookup = token_lookup_key(token);
        let row = sqlx::query(
            "SELECT token, instance_id, provider, created_at, revoked_at, expected_src_ip \
             FROM proxy_tokens \
             WHERE token_lookup = $1 AND revoked_at IS NULL \
             LIMIT 1",
        )
        .bind(&lookup)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let stored: String = row.get("token");
        let instance_id: String = row.get("instance_id");
        let provider: String = row.get("provider");
        let opened = self.open_token(&instance_id, &provider, &stored).await?;
        if !bool::from(opened.plaintext.as_bytes().ct_eq(token.as_bytes())) {
            return Ok(None);
        }
        self.rewrap_token_if_needed(&instance_id, &provider, &stored, &opened)
            .await?;
        Ok(Some(TokenRecord {
            token: opened.plaintext,
            instance_id,
            provider,
            created_at: row.get("created_at"),
            revoked_at: row.get("revoked_at"),
            expected_src_ip: row.get("expected_src_ip"),
        }))
    }

    async fn revoke_for_instance(&self, instance_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE proxy_tokens SET revoked_at = $1 WHERE instance_id = $2 AND revoked_at IS NULL",
        )
        .bind(now_secs())
        .bind(instance_id)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn revoke_token(&self, token: &str) -> Result<bool, StoreError> {
        // Targeted single-row revoke (B1).  Does NOT cascade to other
        // tokens on the same instance — that's `revoke_for_instance`'s
        // job, called by the destroy path.  Already-revoked rows
        // return `false` (no-op) rather than an error so a duplicate
        // revoke is idempotent at the API boundary.
        let lookup = token_lookup_key(token);
        let row = sqlx::query(
            "SELECT token, instance_id, provider FROM proxy_tokens WHERE token_lookup = $1 AND revoked_at IS NULL LIMIT 1",
        )
        .bind(&lookup)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let Some(row) = row else {
            return Ok(false);
        };
        let stored: String = row.get("token");
        let instance_id: String = row.get("instance_id");
        let provider: String = row.get("provider");
        let opened = self.open_token(&instance_id, &provider, &stored).await?;
        if !bool::from(opened.plaintext.as_bytes().ct_eq(token.as_bytes())) {
            return Ok(false);
        }
        let r = sqlx::query(
            "UPDATE proxy_tokens SET revoked_at = $1 \
             WHERE token_lookup = $2 AND token = $3 AND revoked_at IS NULL",
        )
        .bind(now_secs())
        .bind(&lookup)
        .bind(stored)
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(r.rows_affected() > 0)
    }

    async fn lookup_by_instance(&self, instance_id: &str) -> Result<Option<String>, StoreError> {
        // Caller wants the chat-side proxy token specifically — pin
        // the provider filter to `SHARED_PROVIDER` so an ingest
        // token (`provider = "ingest"`) minted after the chat one
        // doesn't shadow it at the rotation paths that were written
        // before the ingest token existed.
        self.lookup_by_instance_for_provider(instance_id, crate::instance::SHARED_PROVIDER)
            .await
    }

    async fn lookup_by_instance_for_provider(
        &self,
        instance_id: &str,
        provider: &str,
    ) -> Result<Option<String>, StoreError> {
        // Multiple non-revoked rows for one instance + provider
        // shouldn't happen (mint is called once per create), but
        // order-by-created_at makes the choice deterministic if it
        // ever does.  LIMIT 1 keeps the query cheap regardless.
        let row = sqlx::query(
            "SELECT token FROM proxy_tokens \
             WHERE instance_id = $1 AND provider = $2 AND revoked_at IS NULL \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(instance_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let stored: String = row.get("token");
        let opened = self.open_token(instance_id, provider, &stored).await?;
        self.rewrap_token_if_needed(instance_id, provider, &stored, &opened)
            .await?;
        Ok(Some(opened.plaintext))
    }
}
