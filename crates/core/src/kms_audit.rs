//! Shared writer helpers for KMS secret access audit rows.
//!
//! Call sites should not hand-roll SQL for `secret_access_audit`; the store
//! trait keeps SQLite/Postgres parity and this module centralizes redaction.

use async_trait::async_trait;

use crate::envelope::{
    KmsContext, OpenEnvelopeResult, SecretAccessOperation, SecretAccessReason, SecretAccessResult,
};
use crate::error::StoreError;
use crate::traits::{SecretAccessAuditEntry, SecretAccessAuditPage, SecretAccessAuditStore};

const MAX_ERROR_MESSAGE: usize = 240;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KmsAuditActor {
    pub kind: String,
    pub id: Option<String>,
}

impl KmsAuditActor {
    pub fn runtime(id: impl Into<String>) -> Self {
        Self {
            kind: "runtime".into(),
            id: Some(id.into()),
        }
    }

    pub fn operator_cli() -> Self {
        Self {
            kind: "operator_cli".into(),
            id: None,
        }
    }

    pub fn system(service: impl Into<String>) -> Self {
        Self {
            kind: "system".into(),
            id: Some(service.into()),
        }
    }

    pub fn test() -> Self {
        Self {
            kind: "test".into(),
            id: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopSecretAccessAuditStore;

#[async_trait]
impl SecretAccessAuditStore for NoopSecretAccessAuditStore {
    async fn insert(&self, _entry: &SecretAccessAuditEntry) -> Result<(), StoreError> {
        Ok(())
    }

    async fn list(
        &self,
        _filter: crate::traits::SecretAccessAuditFilter,
    ) -> Result<SecretAccessAuditPage, StoreError> {
        Ok(SecretAccessAuditPage {
            items: Vec::new(),
            next_offset: None,
        })
    }
}

pub fn redact_error_message(message: impl AsRef<str>) -> String {
    let mut redacted = String::new();
    for ch in message.as_ref().chars() {
        if redacted.len() >= MAX_ERROR_MESSAGE {
            redacted.push_str("...");
            break;
        }
        if ch.is_control() {
            redacted.push(' ');
        } else {
            redacted.push(ch);
        }
    }
    redacted
}

pub fn entry(
    actor: &KmsAuditActor,
    reason: SecretAccessReason,
    operation: SecretAccessOperation,
    context: &KmsContext,
    opened: Option<&OpenEnvelopeResult>,
    result: SecretAccessResult,
    error_class: Option<&str>,
    error_message: Option<&str>,
) -> SecretAccessAuditEntry {
    SecretAccessAuditEntry {
        timestamp: crate::now_secs(),
        actor_kind: actor.kind.clone(),
        actor_id: actor.id.clone(),
        reason,
        operation,
        scope: context.scope,
        owner_id: context.owner_id.clone(),
        instance_id: context.instance_id.clone(),
        secret_name: context.name.clone(),
        key_id: opened.map(|o| o.key_id.clone()),
        key_version: opened.map(|o| o.key_version),
        result,
        error_class: error_class.map(str::to_owned),
        error_message: error_message.map(redact_error_message),
    }
}

pub async fn record(
    store: &dyn SecretAccessAuditStore,
    entry: SecretAccessAuditEntry,
) -> Result<(), StoreError> {
    store.insert(&entry).await
}

pub async fn best_effort_record(store: &dyn SecretAccessAuditStore, entry: SecretAccessAuditEntry) {
    if let Err(err) = store.insert(&entry).await {
        tracing::warn!(error = %err, "failed to write KMS secret access audit row");
    }
}

pub fn success_entry(
    actor: &KmsAuditActor,
    reason: SecretAccessReason,
    operation: SecretAccessOperation,
    context: &KmsContext,
    opened: Option<&OpenEnvelopeResult>,
) -> SecretAccessAuditEntry {
    entry(
        actor,
        reason,
        operation,
        context,
        opened,
        SecretAccessResult::Success,
        None,
        None,
    )
}

pub fn failure_entry(
    actor: &KmsAuditActor,
    reason: SecretAccessReason,
    operation: SecretAccessOperation,
    context: &KmsContext,
    error_class: &str,
    error_message: &str,
) -> SecretAccessAuditEntry {
    entry(
        actor,
        reason,
        operation,
        context,
        None,
        SecretAccessResult::Failure,
        Some(error_class),
        Some(error_message),
    )
}
