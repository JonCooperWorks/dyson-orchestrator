use async_trait::async_trait;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::db::pg::map_sqlx;
use crate::db::{INGEST_PROVIDER, state_sync_provider, token_lookup_key};
use crate::envelope::{
    CipherDirectory, EnvelopeCipher, EnvelopeError, KmsContext, KmsScope, SecretAccessOperation,
    SecretAccessReason, open_context, rewrap_context_as_string, seal_context_as_string,
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

    fn token_context(instance_id: &str, owner_id: Option<&str>, provider: &str) -> KmsContext {
        KmsContext {
            scope: KmsScope::RuntimeToken,
            owner_id: owner_id
                .filter(|id| !id.is_empty())
                .map(std::borrow::ToOwned::to_owned),
            instance_id: Some(instance_id.to_owned()),
            name: Some(format!("proxy_token:{provider}")),
        }
    }

    async fn instance_owner_id(&self, instance_id: &str) -> Result<Option<String>, StoreError> {
        let row = sqlx::query("SELECT owner_id FROM instances WHERE id = $1")
            .bind(instance_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(row.map(|r| r.get("owner_id")))
    }

    async fn seal_token(
        &self,
        instance_id: &str,
        owner_id: Option<&str>,
        provider: &str,
        token: &str,
    ) -> Result<String, StoreError> {
        if let Some(ciphers) = self.ciphers.as_deref() {
            let context = Self::token_context(instance_id, owner_id, provider);
            let reason = token_reason(provider);
            let actor = KmsAuditActor::runtime(instance_id);
            let sealed = seal_context_as_string(ciphers, &context, token.as_bytes(), reason);
            match sealed {
                Ok(sealed) => {
                    kms_audit::best_effort_record(
                        self.audit.as_ref(),
                        kms_audit::success_entry(
                            &actor,
                            reason,
                            SecretAccessOperation::Encrypt,
                            &context,
                            None,
                        ),
                    )
                    .await;
                    return Ok(sealed);
                }
                Err(err) => {
                    kms_audit::best_effort_record(
                        self.audit.as_ref(),
                        kms_audit::failure_entry(
                            &actor,
                            reason,
                            SecretAccessOperation::Encrypt,
                            &context,
                            "EnvelopeError",
                            &err.to_string(),
                        ),
                    )
                    .await;
                    return Err(StoreError::Io(format!("seal proxy token: {err}")));
                }
            }
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
            let owner_id = self.instance_owner_id(instance_id).await?;
            let context = Self::token_context(instance_id, owner_id.as_deref(), provider);
            let legacy_context = Self::token_context(instance_id, None, provider);
            let reason = token_reason(provider);
            let actor = KmsAuditActor::runtime(instance_id);
            let (opened, needs_context_rewrap) =
                match open_context(ciphers, &context, stored.as_bytes(), reason) {
                    Ok(opened) => {
                        kms_audit::best_effort_record(
                            self.audit.as_ref(),
                            kms_audit::success_entry(
                                &actor,
                                reason,
                                SecretAccessOperation::Decrypt,
                                &context,
                                Some(&opened),
                            ),
                        )
                        .await;
                        (opened, false)
                    }
                    Err(err) => {
                        if owner_id.is_some() && matches!(err, EnvelopeError::ContextMismatch) {
                            match open_context(ciphers, &legacy_context, stored.as_bytes(), reason)
                            {
                                Ok(opened) => {
                                    kms_audit::best_effort_record(
                                        self.audit.as_ref(),
                                        kms_audit::success_entry(
                                            &actor,
                                            reason,
                                            SecretAccessOperation::Decrypt,
                                            &context,
                                            Some(&opened),
                                        ),
                                    )
                                    .await;
                                    (opened, true)
                                }
                                Err(fallback_err) => {
                                    kms_audit::best_effort_record(
                                        self.audit.as_ref(),
                                        kms_audit::failure_entry(
                                            &actor,
                                            reason,
                                            SecretAccessOperation::Decrypt,
                                            &context,
                                            "EnvelopeError",
                                            &fallback_err.to_string(),
                                        ),
                                    )
                                    .await;
                                    return Err(StoreError::Malformed(format!(
                                        "open proxy token: {fallback_err}"
                                    )));
                                }
                            }
                        } else {
                            kms_audit::best_effort_record(
                                self.audit.as_ref(),
                                kms_audit::failure_entry(
                                    &actor,
                                    reason,
                                    SecretAccessOperation::Decrypt,
                                    &context,
                                    "EnvelopeError",
                                    &err.to_string(),
                                ),
                            )
                            .await;
                            return Err(StoreError::Malformed(format!("open proxy token: {err}")));
                        }
                    }
                };
            let plaintext = String::from_utf8(opened.plaintext.clone())
                .map_err(|_| StoreError::Malformed("proxy token plaintext was not utf-8".into()))?;
            return Ok(OpenedToken {
                plaintext,
                opened: Some(opened),
                needs_context_rewrap,
                owner_id,
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
            needs_context_rewrap: false,
            owner_id: None,
        })
    }

    async fn rewrap_token_if_needed(
        &self,
        instance_id: &str,
        provider: &str,
        stored: &str,
        opened: &OpenedToken,
    ) -> Result<(), StoreError> {
        let Some(opened_meta) = opened.opened.as_ref() else {
            return Ok(());
        };
        if !opened_meta.needs_rewrap && !opened.needs_context_rewrap {
            return Ok(());
        }
        let Some(ciphers) = self.ciphers.as_deref() else {
            return Ok(());
        };
        let context = Self::token_context(instance_id, opened.owner_id.as_deref(), provider);
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
                        SecretAccessOperation::Rewrap,
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
                    SecretAccessOperation::Rewrap,
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
    needs_context_rewrap: bool,
    owner_id: Option<String>,
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
        let owner_id = self.instance_owner_id(instance_id).await?;
        let stored_token = self
            .seal_token(instance_id, owner_id.as_deref(), provider, &token)
            .await?;
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

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;
    use crate::envelope::AgeCipherDirectory;
    use crate::network_policy::NetworkPolicy;
    use crate::traits::{InstanceRow, InstanceStatus, InstanceStore};

    fn unique(prefix: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{prefix}-{nanos}")
    }

    async fn fixture(name: &str) -> Option<(PgPool, String, String)> {
        let url = match std::env::var("PG_TEST_URL") {
            Ok(url) if !url.trim().is_empty() => url,
            _ => return None,
        };
        let pool = crate::db::pg::open(&url).await.unwrap();
        let owner = unique(&format!("owner-{name}"));
        let instance = unique(&format!("inst-{name}"));
        sqlx::query(
            "INSERT INTO users (id, subject, status, created_at, activated_at) \
             VALUES ($1, $2, 'active', 0, 0)",
        )
        .bind(&owner)
        .bind(format!("subject-{owner}"))
        .execute(&pool)
        .await
        .unwrap();
        let instances = crate::db::pg::instances::PgInstanceStore::new(
            pool.clone(),
            crate::db::sqlite::test_system_cipher(),
        );
        instances
            .create(InstanceRow {
                id: instance.clone(),
                owner_id: owner.clone(),
                name: "agent".into(),
                task: "task".into(),
                cube_sandbox_id: Some("cube-1".into()),
                state_generation: "gen-1".into(),
                template_id: "tmpl".into(),
                status: InstanceStatus::Live,
                bearer_token: "bearer".into(),
                pinned: false,
                expires_at: None,
                last_active_at: 0,
                last_probe_at: None,
                last_probe_status: None,
                created_at: 0,
                destroyed_at: None,
                rotated_to: None,
                network_policy: NetworkPolicy::NoLocalNet,
                network_policy_cidrs: Vec::new(),
                models: Vec::new(),
                tools: Vec::new(),
            })
            .await
            .unwrap();
        Some((pool, owner, instance))
    }

    #[tokio::test]
    async fn pg_kms_runtime_audit_decrypt_records_instance_owner() {
        let Some((pool, owner, instance)) = fixture("kms-owner").await else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let ciphers: Arc<dyn CipherDirectory> =
            Arc::new(AgeCipherDirectory::new(tmp.path()).unwrap());
        let system_cipher = ciphers.system().unwrap();
        let audit = Arc::new(crate::db::pg::audit::PgSecretAccessAuditStore::new(
            pool.clone(),
        ));
        let store = PgTokenStore::new_with_kms(pool.clone(), system_cipher, ciphers, audit);

        let token = store.mint(&instance, "openai").await.unwrap();
        assert!(store.resolve(&token).await.unwrap().is_some());

        let rows: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT owner_id FROM secret_access_audit \
             WHERE scope = 'runtime_token' AND operation = 'decrypt' AND result = 'success' AND instance_id = $1",
        )
        .bind(&instance)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(
            rows.iter()
                .any(|row| row.as_deref() == Some(owner.as_str()))
        );
    }
}
