use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::process::Command;

use crate::error::SwarmError;

#[async_trait]
pub trait EgressPolicySync: Send + Sync {
    async fn refresh(&self) -> Result<(), SwarmError>;
}

#[derive(Clone, Default)]
pub struct SystemdEgressPolicySync;

impl SystemdEgressPolicySync {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EgressPolicySync for SystemdEgressPolicySync {
    async fn refresh(&self) -> Result<(), SwarmError> {
        systemctl(
            &["start", "dyson-egress-policy.service"],
            "egress policy rebuild",
        )
        .await;
        systemctl(
            &["kill", "--signal=SIGHUP", "dyson-egress-proxy"],
            "egress proxy reload",
        )
        .await;
        Ok(())
    }
}

async fn systemctl(args: &[&str], action: &'static str) {
    match Command::new("sudo")
        .args(["-n", "/usr/bin/systemctl"])
        .args(args)
        .status()
        .await
    {
        Ok(status) if status.success() => {
            tracing::debug!(action, "egress-policy-sync: systemctl succeeded");
        }
        Ok(status) => {
            tracing::warn!(
                action,
                status = %status,
                "egress-policy-sync: systemctl failed; timer remains safety net"
            );
        }
        Err(err) => {
            tracing::warn!(
                action,
                error = %err,
                "egress-policy-sync: systemctl spawn failed; timer remains safety net"
            );
        }
    }
}

#[derive(Clone, Default)]
pub struct NoopEgressPolicySync {
    calls: Arc<Mutex<Vec<()>>>,
}

impl NoopEgressPolicySync {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("egress sync calls poisoned").len()
    }

    pub fn clear(&self) {
        self.calls
            .lock()
            .expect("egress sync calls poisoned")
            .clear();
    }
}

#[async_trait]
impl EgressPolicySync for NoopEgressPolicySync {
    async fn refresh(&self) -> Result<(), SwarmError> {
        self.calls
            .lock()
            .expect("egress sync calls poisoned")
            .push(());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn noop_records_refresh_calls() {
        let sync = NoopEgressPolicySync::new();
        assert_eq!(sync.call_count(), 0);

        sync.refresh().await.unwrap();
        sync.refresh().await.unwrap();

        assert_eq!(sync.call_count(), 2);
    }

    #[tokio::test]
    async fn trait_object_refresh_contract_is_callable() {
        let sync = NoopEgressPolicySync::new();
        let dyn_sync: Arc<dyn EgressPolicySync> = Arc::new(sync.clone());

        dyn_sync.refresh().await.unwrap();

        assert_eq!(sync.call_count(), 1);
    }
}
