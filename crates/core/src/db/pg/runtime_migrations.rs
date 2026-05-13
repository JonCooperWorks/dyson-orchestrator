//! Postgres runtime data migrations.
//!
//! Schema migrations live under `migrations/postgres/`. Runtime data
//! migrations that need application secrets land here, behind the same
//! [`RuntimeMigrator`] trait the SQLite path implements. Empty for now:
//! fresh Pg deployments have no legacy SQLite plaintext rows to rewrite.

use async_trait::async_trait;

use crate::db::{RuntimeMigrationReport, RuntimeMigrator};
use crate::envelope::EnvelopeCipher;
use crate::error::StoreError;

#[derive(Debug, Clone, Default)]
pub struct PgRuntimeMigrator;

impl PgRuntimeMigrator {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RuntimeMigrator for PgRuntimeMigrator {
    async fn migrate(
        &self,
        _cipher: &dyn EnvelopeCipher,
    ) -> Result<RuntimeMigrationReport, StoreError> {
        Ok(RuntimeMigrationReport::default())
    }
}
