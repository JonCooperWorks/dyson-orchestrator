//! Secret material lives in `instance_secrets` (plaintext at rest, per the
//! brief's trust model). This module is a thin wrapper around `SecretStore`
//! plus the env-map composition function used at create/restore time.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::error::StoreError;
use crate::traits::SecretStore;

#[derive(Clone)]
pub struct SecretsService {
    store: Arc<dyn SecretStore>,
}

impl SecretsService {
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    pub async fn put(&self, instance_id: &str, name: &str, value: &str) -> Result<(), StoreError> {
        self.store.put(instance_id, name, value).await
    }

    pub async fn delete(&self, instance_id: &str, name: &str) -> Result<(), StoreError> {
        self.store.delete(instance_id, name).await
    }

    pub async fn list(&self, instance_id: &str) -> Result<Vec<(String, String)>, StoreError> {
        self.store.list(instance_id).await
    }
}

/// Compose the env map handed to a CubeSandbox at create/restore time.
///
/// Priority order from the brief: **template → managed → caller → pre-existing
/// rows**. Read left-to-right, with the rightmost source winning on key
/// collision. Pre-existing rows ("`instance_secrets`" already in the DB) take
/// the highest priority so an operator-curated secret is never clobbered by a
/// transient caller-supplied value or a managed default.
///
/// - **template**: env baked into the agent template (model defaults, etc.).
/// - **managed**: orchestrator-injected (`WARDEN_PROXY_URL`, `WARDEN_PROXY_TOKEN`).
/// - **caller**: env supplied in the `POST /v1/instances` request body.
/// - **existing**: rows previously written via `PUT /v1/instances/:id/secrets/:name`.
pub fn compose_env(
    template: &BTreeMap<String, String>,
    managed: &BTreeMap<String, String>,
    caller: &BTreeMap<String, String>,
    existing: &[(String, String)],
) -> BTreeMap<String, String> {
    let mut out = template.clone();
    for (k, v) in managed {
        out.insert(k.clone(), v.clone());
    }
    for (k, v) in caller {
        out.insert(k.clone(), v.clone());
    }
    for (k, v) in existing {
        out.insert(k.clone(), v.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn compose_env_priority_order() {
        let template = m([("A", "tpl"), ("B", "tpl"), ("C", "tpl"), ("D", "tpl")]);
        let managed = m([("B", "mgr"), ("C", "mgr"), ("D", "mgr")]);
        let caller = m([("C", "call"), ("D", "call")]);
        let existing = vec![("D".into(), "exist".into())];
        let merged = compose_env(&template, &managed, &caller, &existing);
        assert_eq!(merged["A"], "tpl");
        assert_eq!(merged["B"], "mgr");
        assert_eq!(merged["C"], "call");
        assert_eq!(merged["D"], "exist");
    }

    #[test]
    fn compose_env_empty_inputs_are_identity() {
        let empty = BTreeMap::new();
        let only_template = m([("X", "1")]);
        let merged = compose_env(&only_template, &empty, &empty, &[]);
        assert_eq!(merged, only_template);
    }
}
