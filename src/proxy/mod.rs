//! LLM proxy.
//!
//! - [`policy_check::enforce`] is the single composed policy gate (step 10).
//! - [`http::router`] mounts `/llm/<provider>/...` with per-instance-bearer
//!   handling baked into the catch-all handler.
//! - [`adapters`] holds the per-provider quirks.
//!
//! The proxy is intentionally a separate module tree from `/v1/*` so it can
//! carry its own auth posture and not touch the admin auth path.

pub mod adapters;
pub mod http;
pub mod policy_check;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;

use crate::config::{ProviderConfig, Providers};
use crate::proxy::policy_check::{InstancePolicy, UsageSnapshot};
use crate::traits::{ProviderAdapter, TokenStore};

/// Wires the proxy together. Cheap to clone — every field is `Arc` or
/// scalar.
pub struct ProxyService {
    pub pool: SqlitePool,
    pub tokens: Arc<dyn TokenStore>,
    pub providers: Providers,
    pub adapters: HashMap<&'static str, Arc<dyn ProviderAdapter>>,
    pub http: reqwest::Client,
    pub default_policy: InstancePolicy,
    rate: Arc<RateWindow>,
}

impl ProxyService {
    pub fn new(
        pool: SqlitePool,
        tokens: Arc<dyn TokenStore>,
        providers: Providers,
        default_policy: InstancePolicy,
    ) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .pool_idle_timeout(Some(Duration::from_secs(90)))
            .build()?;
        Ok(Self {
            pool,
            tokens,
            providers,
            adapters: adapters::registry(),
            http,
            default_policy,
            rate: Arc::new(RateWindow::default()),
        })
    }

    /// Resolve a provider name to its config. Returns an owned clone because
    /// the adapter's `upstream_base_url` borrows from it.
    pub fn provider_config(&self, name: &str) -> Option<ProviderConfig> {
        match name {
            "anthropic" => self.providers.anthropic.clone(),
            "openai" => self.providers.openai.clone(),
            "gemini" => self.providers.gemini.clone(),
            "openrouter" => self.providers.openrouter.clone(),
            "ollama" => self.providers.ollama.clone(),
            _ => None,
        }
    }

    /// Build a [`UsageSnapshot`] for `instance_id`. RPS comes from an
    /// in-memory rolling window; daily tokens come from `llm_audit`. Monthly
    /// USD is currently zero — the audit table doesn't carry per-call USD
    /// and the brief defines the policy primitive without prescribing the
    /// computation.
    pub async fn snapshot(&self, instance_id: &str) -> UsageSnapshot {
        // Side effect: record this request in the rate window so the next
        // call sees it.
        let recent_rps = self.rate.observe(instance_id);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let daily_tokens = crate::db::audit::daily_tokens(&self.pool, instance_id, now)
            .await
            .unwrap_or(0);
        UsageSnapshot {
            recent_rps,
            daily_tokens,
            monthly_usd: 0.0,
        }
    }
}

/// One-second rolling window of request timestamps per instance. `observe`
/// records `now` and returns the current count (including the just-recorded
/// timestamp) so the policy gate sees the RPS the operator's limit cares
/// about.
#[derive(Default)]
struct RateWindow {
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateWindow {
    fn observe(&self, instance_id: &str) -> u32 {
        let mut m = self.buckets.lock().expect("rate window poisoned");
        let q = m.entry(instance_id.to_string()).or_default();
        let now = Instant::now();
        q.push_back(now);
        prune(q, now);
        q.len() as u32
    }
}

fn prune(q: &mut VecDeque<Instant>, now: Instant) {
    let cutoff = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
    while let Some(front) = q.front() {
        if *front < cutoff {
            q.pop_front();
        } else {
            break;
        }
    }
}

#[cfg(test)]
impl ProxyService {
    /// Build a service with no providers configured and an in-memory pool —
    /// used by tests that don't actually run the proxy router.
    pub async fn empty_in_memory() -> Self {
        use crate::db::open_in_memory;
        use crate::db::tokens::SqlxTokenStore;
        let pool = open_in_memory().await.unwrap();
        let tokens: Arc<dyn TokenStore> = Arc::new(SqlxTokenStore::new(pool.clone()));
        Self::new(
            pool,
            tokens,
            Providers::default(),
            InstancePolicy {
                allowed_providers: vec!["*".into()],
                allowed_models: vec!["*".into()],
                daily_token_budget: None,
                monthly_usd_budget: None,
                rps_limit: None,
            },
        )
        .expect("build")
    }
}

