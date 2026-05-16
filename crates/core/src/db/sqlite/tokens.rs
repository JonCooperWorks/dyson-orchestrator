use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::db::sqlite::map_sqlx;
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
pub struct SqlxTokenStore {
    pool: SqlitePool,
    cipher: Arc<dyn EnvelopeCipher>,
    ciphers: Option<Arc<dyn CipherDirectory>>,
    audit: Arc<dyn SecretAccessAuditStore>,
}

impl SqlxTokenStore {
    pub fn new(pool: SqlitePool, cipher: Arc<dyn EnvelopeCipher>) -> Self {
        Self {
            pool,
            cipher,
            ciphers: None,
            audit: Arc::new(NoopSecretAccessAuditStore),
        }
    }

    pub fn new_with_ciphers(
        pool: SqlitePool,
        cipher: Arc<dyn EnvelopeCipher>,
        ciphers: Arc<dyn CipherDirectory>,
    ) -> Self {
        Self {
            pool,
            cipher,
            ciphers: Some(ciphers),
            audit: Arc::new(NoopSecretAccessAuditStore),
        }
    }

    pub fn new_with_kms(
        pool: SqlitePool,
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
             SET token = ? \
             WHERE instance_id = ? AND provider = ? AND token = ?",
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

impl SqlxTokenStore {
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
             VALUES (?, ?, ?, ?, ?, NULL, NULL)",
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
impl TokenStore for SqlxTokenStore {
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
            "UPDATE proxy_tokens SET expected_src_ip = ? \
             WHERE instance_id = ? AND revoked_at IS NULL",
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
             WHERE token_lookup = ? AND revoked_at IS NULL \
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
            "UPDATE proxy_tokens SET revoked_at = ? WHERE instance_id = ? AND revoked_at IS NULL",
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
            "SELECT token, instance_id, provider FROM proxy_tokens \
             WHERE token_lookup = ? AND revoked_at IS NULL LIMIT 1",
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
            "UPDATE proxy_tokens SET revoked_at = ? \
             WHERE token_lookup = ? AND token = ? AND revoked_at IS NULL",
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
             WHERE instance_id = ? AND provider = ? AND revoked_at IS NULL \
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sqlite::instances::SqlxInstanceStore;
    use crate::db::sqlite::open_in_memory;
    use crate::envelope::{AgeCipherDirectory, EnvelopeError};
    use crate::traits::{InstanceRow, InstanceStatus, InstanceStore};

    #[derive(Debug)]
    struct TestCipher;

    impl EnvelopeCipher for TestCipher {
        fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
            let mut out = b"sealed:".to_vec();
            out.extend_from_slice(plaintext);
            Ok(out)
        }

        fn open(&self, ciphertext: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
            ciphertext
                .strip_prefix(b"sealed:")
                .map(|s| s.to_vec())
                .ok_or(EnvelopeError::Corrupt)
        }
    }

    async fn seed(pool: &SqlitePool, id: &str) {
        let store = SqlxInstanceStore::new(pool.clone(), Arc::new(TestCipher));
        store
            .create(InstanceRow {
                id: id.into(),
                owner_id: "legacy".into(),
                name: String::new(),
                task: String::new(),
                cube_sandbox_id: None,
                state_generation: String::new(),
                template_id: "t".into(),
                status: InstanceStatus::Live,
                bearer_token: "b".into(),
                pinned: false,
                expires_at: None,
                last_active_at: 0,
                last_probe_at: None,
                last_probe_status: None,
                created_at: 0,
                destroyed_at: None,
                rotated_to: None,
                network_policy: crate::network_policy::NetworkPolicy::Open,
                network_policy_cidrs: Vec::new(),
                models: Vec::new(),
                tools: Vec::new(),
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mint_resolve_revoke() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let tok = store.mint("i1", "anthropic").await.unwrap();
        assert!(tok.starts_with("pt_"));
        assert_eq!(tok.len(), 35);

        let resolved = store.resolve(&tok).await.unwrap().expect("present");
        assert_eq!(resolved.instance_id, "i1");
        assert_eq!(resolved.provider, "anthropic");
        assert!(resolved.revoked_at.is_none());

        store.revoke_for_instance("i1").await.unwrap();
        assert!(store.resolve(&tok).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_token_resolves_none() {
        let pool = open_in_memory().await.unwrap();
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        assert!(store.resolve("not-a-token").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn revoke_only_targets_one_instance() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        seed(&pool, "i2").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let t1 = store.mint("i1", "openai").await.unwrap();
        let t2 = store.mint("i2", "openai").await.unwrap();
        store.revoke_for_instance("i1").await.unwrap();
        assert!(store.resolve(&t1).await.unwrap().is_none());
        assert!(store.resolve(&t2).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn revoke_token_targets_named_row_only() {
        // B1 regression: revoking a single leaked proxy_token must
        // NOT cascade to sibling tokens on the same instance.  We
        // mint two rows for one instance (a contrived shape — mint
        // is normally called once per create — but the SPA could
        // hand-issue), revoke one by value, and assert the other
        // remains live.
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let t1 = store.mint("i1", "openai").await.unwrap();
        let t2 = store.mint("i1", "anthropic").await.unwrap();

        let revoked = store.revoke_token(&t1).await.unwrap();
        assert!(revoked, "revoke_token returns true on first call");
        assert!(store.resolve(&t1).await.unwrap().is_none());
        // Sibling token on the same instance survives.
        let r2 = store.resolve(&t2).await.unwrap().expect("t2 still live");
        assert_eq!(r2.instance_id, "i1");
        assert!(r2.revoked_at.is_none());

        // Idempotent: revoking again returns false (already revoked).
        let again = store.revoke_token(&t1).await.unwrap();
        assert!(!again);

        // Unknown token: false, no error.
        let unknown = store.revoke_token("not-a-real-token").await.unwrap();
        assert!(!unknown);
    }

    #[tokio::test]
    async fn mint_ingest_uses_it_prefix_and_ingest_provider() {
        // Ingest tokens live in the same `proxy_tokens` table but the
        // wire-side route filters by prefix and the operator grep path
        // filters by provider.  Both must be set correctly on mint.
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let tok = store.mint_ingest("i1").await.unwrap();
        assert!(
            tok.starts_with("it_"),
            "ingest token must start with `it_`, got {tok:?}"
        );
        assert_eq!(tok.len(), 35, "it_ + 32 hex chars");

        let resolved = store.resolve(&tok).await.unwrap().expect("present");
        assert_eq!(resolved.instance_id, "i1");
        assert_eq!(resolved.provider, INGEST_PROVIDER);
        assert!(resolved.revoked_at.is_none());
    }

    #[tokio::test]
    async fn revoke_for_instance_cleans_up_ingest_alongside_chat_token() {
        // Instance destroy must take the ingest token down with it —
        // we don't want a destroyed instance's ingest URL accepting
        // pushes from a still-running cube the destroy didn't catch.
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let chat = store.mint("i1", "openai").await.unwrap();
        let ingest = store.mint_ingest("i1").await.unwrap();
        let state = store
            .mint_state_sync_for_generation("i1", "gen-a")
            .await
            .unwrap();

        store.revoke_for_instance("i1").await.unwrap();
        assert!(
            store.resolve(&chat).await.unwrap().is_none(),
            "chat token revoked"
        );
        assert!(
            store.resolve(&ingest).await.unwrap().is_none(),
            "ingest token revoked"
        );
        assert!(
            store.resolve(&state).await.unwrap().is_none(),
            "state token revoked"
        );
    }

    #[tokio::test]
    async fn ingest_and_chat_tokens_are_distinguishable_after_mint() {
        // Token-prefix discrimination at the route layer relies on the
        // `pt_`, `it_`, and `st_` prefixes never colliding.  Belt-and-
        // braces assertion that token kinds produce disjoint shapes.
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let pt = store.mint("i1", "openai").await.unwrap();
        let it = store.mint_ingest("i1").await.unwrap();
        let st = store
            .mint_state_sync_for_generation("i1", "gen-a")
            .await
            .unwrap();
        assert!(pt.starts_with("pt_"));
        assert!(it.starts_with("it_"));
        assert!(st.starts_with("st_"));
        assert_ne!(pt, it);
        assert_ne!(pt, st);
        assert_ne!(it, st);
        let resolved = store.resolve(&st).await.unwrap().expect("present");
        assert_eq!(resolved.provider, "state_sync:gen-a");
    }

    #[tokio::test]
    async fn state_sync_generation_is_encoded_in_provider() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let tok = store
            .mint_state_sync_for_generation("i1", "gen-a")
            .await
            .unwrap();
        let resolved = store.resolve(&tok).await.unwrap().expect("present");

        assert_eq!(resolved.provider, "state_sync:gen-a");
        assert!(crate::db::state_sync_provider_matches(
            &resolved.provider,
            "gen-a"
        ));
        assert!(!crate::db::state_sync_provider_matches(
            &resolved.provider,
            "gen-b"
        ));
        assert!(!crate::db::state_sync_provider_matches(
            &resolved.provider,
            ""
        ));
    }

    #[tokio::test]
    async fn store_does_not_persist_plaintext_tokens() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let store = SqlxTokenStore::new(pool.clone(), Arc::new(TestCipher));
        let tok = store
            .mint("i1", crate::instance::SHARED_PROVIDER)
            .await
            .unwrap();

        let row =
            sqlx::query("SELECT token, token_lookup FROM proxy_tokens WHERE instance_id = 'i1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let stored: String = row.get("token");
        let lookup: String = row.get("token_lookup");
        assert_ne!(stored, tok);
        assert_eq!(stored, format!("sealed:{tok}"));
        assert_eq!(lookup, token_lookup_key(&tok));

        let resolved = store
            .resolve(&tok)
            .await
            .unwrap()
            .expect("sealed token resolves");
        assert_eq!(resolved.token, tok);
        assert_eq!(
            store.lookup_by_instance("i1").await.unwrap(),
            Some(tok.clone())
        );

        assert!(store.revoke_token(&tok).await.unwrap());
        assert!(store.resolve(&tok).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn store_rejects_unmigrated_plaintext_tokens() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let lookup = token_lookup_key("pt_unmigrated");
        sqlx::query(
            "INSERT INTO proxy_tokens (token, token_lookup, instance_id, provider, created_at, revoked_at)
             VALUES ('pt_unmigrated', ?, 'i1', 'openrouter', 0, NULL)",
        )
        .bind(lookup)
        .execute(&pool)
        .await
        .unwrap();

        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        let err = store.resolve("pt_unmigrated").await.unwrap_err();
        assert!(matches!(err, StoreError::Malformed(_)));
    }

    #[tokio::test]
    async fn resolve_skips_unrelated_ciphertext_by_lookup() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        sqlx::query(
            "INSERT INTO proxy_tokens (token, token_lookup, instance_id, provider, created_at, revoked_at)
             VALUES ('not-sealed', ?, 'i1', 'openrouter', 0, NULL)",
        )
        .bind(token_lookup_key("pt_some_other_token"))
        .execute(&pool)
        .await
        .unwrap();

        let store = SqlxTokenStore::new(pool, Arc::new(TestCipher));
        assert!(store.resolve("pt_missing").await.unwrap().is_none());
        assert!(!store.revoke_token("pt_missing").await.unwrap());
    }

    #[tokio::test]
    async fn kms_runtime_audit_records_success_and_failure_without_plaintext() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let tmp = tempfile::tempdir().unwrap();
        let ciphers: Arc<dyn CipherDirectory> =
            Arc::new(AgeCipherDirectory::new(tmp.path()).unwrap());
        let system_cipher = ciphers.system().unwrap();
        let audit = crate::db::sqlite::secret_access_audit_store(pool.clone());
        let store =
            SqlxTokenStore::new_with_kms(pool.clone(), system_cipher, ciphers.clone(), audit);

        let tok = store.mint("i1", "openai").await.unwrap();
        let resolved = store.resolve(&tok).await.unwrap().expect("present");
        assert_eq!(resolved.token, tok);

        sqlx::query("UPDATE proxy_tokens SET token = 'not-age' WHERE instance_id = 'i1'")
            .execute(&pool)
            .await
            .unwrap();
        let _ = store.resolve(&tok).await;

        let rows = sqlx::query(
            "SELECT result, error_message FROM secret_access_audit \
             WHERE scope = 'runtime_token' ORDER BY timestamp ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(rows.len() >= 2);
        assert!(
            rows.iter()
                .any(|r| r.get::<String, _>("result") == "success")
        );
        assert!(
            rows.iter()
                .any(|r| r.get::<String, _>("result") == "failure")
        );
        for row in rows {
            let message: Option<String> = row.get("error_message");
            assert!(
                !message.unwrap_or_default().contains(&tok),
                "audit error must not contain plaintext token"
            );
        }
    }

    #[tokio::test]
    async fn resolves_legacy_proxy_token_and_lazy_rewraps_to_v2() {
        let pool = open_in_memory().await.unwrap();
        seed(&pool, "i1").await;
        let tmp = tempfile::tempdir().unwrap();
        let ciphers: Arc<dyn CipherDirectory> =
            Arc::new(AgeCipherDirectory::new(tmp.path()).unwrap());
        let system_cipher = ciphers.system().unwrap();
        let token = "pt_0123456789abcdef0123456789abcdef";
        let legacy = String::from_utf8(system_cipher.seal(token.as_bytes()).unwrap()).unwrap();
        sqlx::query(
            "INSERT INTO proxy_tokens \
             (token, token_lookup, instance_id, provider, created_at, revoked_at, expected_src_ip) \
             VALUES (?, ?, 'i1', 'openai', 0, NULL, NULL)",
        )
        .bind(&legacy)
        .bind(token_lookup_key(token))
        .execute(&pool)
        .await
        .unwrap();

        let audit = crate::db::sqlite::secret_access_audit_store(pool.clone());
        let store = SqlxTokenStore::new_with_kms(pool.clone(), system_cipher, ciphers, audit);
        assert!(store.resolve(token).await.unwrap().is_some());

        let stored: String =
            sqlx::query_scalar("SELECT token FROM proxy_tokens WHERE instance_id = 'i1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_ne!(stored, legacy);
        assert!(crate::envelope::is_v2_envelope(stored.as_bytes()));
    }
}
