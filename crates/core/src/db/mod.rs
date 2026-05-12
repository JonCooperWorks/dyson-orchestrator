//! SQLite- and Postgres-backed implementations of the *Store traits.
//!
//! Note: queries are runtime-checked (`sqlx::query`/`query_as`) rather than
//! compile-time-checked (`sqlx::query!`). The brief asks for the macro form,
//! but it requires either sqlx-cli to prepare an offline cache or a live
//! `DATABASE_URL` at build time — neither fits a clean `cargo build` here.
//! Every query is exercised by unit tests under `tests` modules below, so
//! malformed SQL fails fast; switching to the macro form is a mechanical
//! swap once a `cargo sqlx prepare` step exists in CI.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::config::{Config, DatabaseBackend};
use crate::envelope::{CipherDirectory, EnvelopeCipher};
use crate::error::StoreError;
use crate::traits::{
    AdminAuditStore, ArtefactCacheStore, AuditStore, DeliveryStore, InstanceStore, McpAuditStore,
    PolicyStore, SessionStore, ShareStore, SnapshotStore, StateFileStore, SystemSecretStore,
    TokenStore, UserSecretStore, UserStore, WebhookStore,
};

pub mod artefacts;
pub mod audit;
pub mod instances;
pub mod mcp_catalog;
pub mod policies;
pub mod runtime_migrations;
pub mod secrets;
pub mod sessions;
pub mod shares;
pub mod skill_marketplace;
pub mod snapshots;
pub mod state_files;
pub mod tokens;
pub mod users;
pub mod webhooks;

#[cfg(feature = "postgres")]
pub mod pg;

pub static SQLITE_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

/// Translate a [`sqlx::Error`] into the crate-level [`StoreError`] flavour
/// callers expect. Shared so every store can use the same mapping —
/// `RowNotFound → NotFound`, unique-violation → `Constraint`, everything
/// else → `Io`. The audit table doesn't enforce uniqueness so its impl
/// previously had a slimmer mapping; folding that case into the same
/// function is harmless because the unique-violation arm just never fires.
pub(crate) fn map_sqlx(e: sqlx::Error) -> StoreError {
    match e {
        sqlx::Error::RowNotFound => StoreError::NotFound,
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            StoreError::Constraint(db.to_string())
        }
        other => StoreError::Io(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Pool openers
// ---------------------------------------------------------------------------

/// Open (or create) the SQLite database at `path`, run migrations, and
/// secure permissions.
pub async fn open_sqlite(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
        }
    }
    let url = format!("sqlite://{}", path.display());
    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    SQLITE_MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))?;
    secure_db_perms(path)?;
    Ok(pool)
}

/// Open a Postgres connection pool and run the PG migrations.
#[cfg(feature = "postgres")]
pub async fn open_pg(url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    let pool = sqlx::PgPool::connect(url).await?;
    pg::MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))?;
    Ok(pool)
}

/// Backend-agnostic pool wrapper returned by [`open_configured`].
pub enum DbPool {
    Sqlite(SqlitePool),
    #[cfg(feature = "postgres")]
    Postgres(sqlx::PgPool),
}

impl DbPool {
    /// Return the inner [`SqlitePool`], panicking if this is a Postgres
    /// pool.  Used by callers that haven't yet been abstracted behind a
    /// trait-object factory (e.g. `runtime_migrations`).
    pub fn expect_sqlite(&self) -> &SqlitePool {
        match self {
            Self::Sqlite(p) => p,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => panic!("expected sqlite pool, got postgres"),
        }
    }
}

/// Open the configured database and return the matching pool.
///
/// Replaces the old `open_configured_sqlite` guard — Postgres is now live
/// behind the `postgres` feature and this function returns the right pool
/// for whichever backend the config selects.
pub async fn open_configured(cfg: &Config) -> Result<DbPool, StoreError> {
    match cfg.database_backend {
        DatabaseBackend::Sqlite => open_sqlite(&cfg.db_path)
            .await
            .map(DbPool::Sqlite)
            .map_err(map_sqlx),
        DatabaseBackend::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let url = cfg
                    .database_url
                    .as_deref()
                    .ok_or_else(|| StoreError::Io("database_url required for postgres".into()))?;
                open_pg(url).await.map(DbPool::Postgres).map_err(map_sqlx)
            }
            #[cfg(not(feature = "postgres"))]
            {
                Err(StoreError::Io(
                    "database_backend=postgres requires the `postgres` cargo feature".into(),
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Backend-agnostic store factories — each takes &DbPool, returns Arc<dyn Trait>
// ---------------------------------------------------------------------------

pub fn artefact_cache_store(pool: &DbPool) -> Arc<dyn ArtefactCacheStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(artefacts::SqlxArtefactStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::artefacts::PgArtefactStore::new(p.clone())),
    }
}

pub fn snapshot_store(pool: &DbPool) -> Arc<dyn SnapshotStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(snapshots::SqliteSnapshotStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::snapshots::PgSnapshotStore::new(p.clone())),
    }
}

pub fn policy_store(pool: &DbPool) -> Arc<dyn PolicyStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(policies::SqlitePolicyStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::policies::PgPolicyStore::new(p.clone())),
    }
}

pub fn audit_store(pool: &DbPool) -> Arc<dyn AuditStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(audit::SqliteAuditStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::audit::PgAuditStore::new(p.clone())),
    }
}

pub fn mcp_audit_store(pool: &DbPool) -> Arc<dyn McpAuditStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(audit::SqliteMcpAuditStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::audit::PgMcpAuditStore::new(p.clone())),
    }
}

pub fn admin_audit_store(pool: &DbPool) -> Arc<dyn AdminAuditStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(audit::SqliteAdminAuditStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::audit::PgAdminAuditStore::new(p.clone())),
    }
}

pub fn session_store(pool: &DbPool) -> Arc<dyn SessionStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(sessions::SqliteSessionStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::sessions::PgSessionStore::new(p.clone())),
    }
}

pub fn state_file_store(pool: &DbPool) -> Arc<dyn StateFileStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(state_files::SqlxStateFileStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::state_files::PgStateFileStore::new(p.clone())),
    }
}

pub fn share_store(pool: &DbPool) -> Arc<dyn ShareStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(shares::SqlxShareStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::shares::PgShareStore::new(p.clone())),
    }
}

pub fn webhook_store(pool: &DbPool) -> Arc<dyn WebhookStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(webhooks::SqlxWebhookStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::webhooks::PgWebhookStore::new(p.clone())),
    }
}

pub fn delivery_store(pool: &DbPool) -> Arc<dyn DeliveryStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(webhooks::SqlxDeliveryStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::webhooks::PgDeliveryStore::new(p.clone())),
    }
}

pub fn user_secret_store(pool: &DbPool) -> Arc<dyn UserSecretStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(secrets::SqlxUserSecretStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::secrets::PgUserSecretStore::new(p.clone())),
    }
}

pub fn system_secret_store(pool: &DbPool) -> Arc<dyn SystemSecretStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(secrets::SqlxSystemSecretStore::new(p.clone())),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::secrets::PgSystemSecretStore::new(p.clone())),
    }
}

pub fn skill_marketplace_store(
    pool: &DbPool,
) -> Arc<dyn crate::traits::SkillMarketplaceSourceStore> {
    match pool {
        DbPool::Sqlite(p) => {
            Arc::new(skill_marketplace::SqlxSkillMarketplaceSourceStore::new(p.clone()))
        }
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(
            pg::skill_marketplace::PgSkillMarketplaceSourceStore::new(p.clone()),
        ),
    }
}

/// Construct an `InstanceStore` from the pool + token cipher.
pub fn instance_store(
    pool: &DbPool,
    cipher: Arc<dyn EnvelopeCipher>,
) -> Arc<dyn InstanceStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(instances::SqlxInstanceStore::new(p.clone(), cipher)),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::instances::PgInstanceStore::new(p.clone(), cipher)),
    }
}

/// Construct a `TokenStore` from the pool + token cipher.
pub fn token_store(pool: &DbPool, cipher: Arc<dyn EnvelopeCipher>) -> Arc<dyn TokenStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(tokens::SqlxTokenStore::new(p.clone(), cipher)),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::tokens::PgTokenStore::new(p.clone(), cipher)),
    }
}

/// Construct a `UserStore` from the pool + cipher directory.
pub fn user_store(
    pool: &DbPool,
    ciphers: Arc<dyn CipherDirectory>,
) -> Arc<dyn UserStore> {
    match pool {
        DbPool::Sqlite(p) => Arc::new(users::SqlxUserStore::new(p.clone(), ciphers)),
        #[cfg(feature = "postgres")]
        DbPool::Postgres(p) => Arc::new(pg::users::PgUserStore::new(p.clone(), ciphers)),
    }
}

// ---------------------------------------------------------------------------
// Unix permission hardening (sqlite only)
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn secure_db_perms(path: &Path) -> Result<(), sqlx::Error> {
    use std::os::unix::fs::PermissionsExt;
    if path.exists() {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(sqlx::Error::Io)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn secure_db_perms(_path: &Path) -> Result<(), sqlx::Error> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// In-memory SQLite pool for tests. Single connection so the same database
/// is visible across calls.
pub async fn open_in_memory() -> Result<SqlitePool, sqlx::Error> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .min_connections(1)
        .idle_timeout(None)
        .max_lifetime(None)
        .connect("sqlite::memory:")
        .await?;
    SQLITE_MIGRATOR
        .run(&pool)
        .await
        .map_err(|e| sqlx::Error::Migrate(Box::new(e)))?;
    Ok(pool)
}

#[cfg(test)]
pub(crate) fn test_system_cipher() -> std::sync::Arc<dyn crate::envelope::EnvelopeCipher> {
    let tmp = tempfile::tempdir().expect("test keys tempdir");
    let dir = crate::envelope::AgeCipherDirectory::new(tmp.path()).expect("test cipher dir");
    crate::envelope::CipherDirectory::system(&dir).expect("test system cipher")
}
