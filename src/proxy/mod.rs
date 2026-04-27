//! LLM proxy.
//!
//! Step 10 lands the policy enforcement entrypoint
//! ([`policy_check::enforce`]); steps 14-15 add the streaming proxy router
//! and per-provider adapters. The proxy is intentionally a separate module
//! tree from `/v1/*` so it can carry its own per-instance-bearer middleware
//! and not touch the admin auth path.

pub mod policy_check;
