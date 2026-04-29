//! Per-instance network policy: profiles, validation, DNS resolution,
//! and translation into the CubeAPI wire shape.
//!
//! The CubeAPI accepts a `network` block with `allowOut` and `denyOut`
//! IPv4 CIDR lists plus an `allow_internet_access` toggle (see
//! `CubeSandbox/CubeAPI/src/handlers/sandboxes.rs::build_cubevs_context`).
//! The eBPF egress filter applies them in `allow > deny > default-allow`
//! order on every outbound packet.  No DNS-aware filtering — the cube
//! maps are pure IPv4 LPM tries.
//!
//! Four user-facing profiles, mapping to that surface:
//!
//! | Profile     | allow_internet_access | allowOut                            | denyOut                                |
//! |-------------|-----------------------|-------------------------------------|----------------------------------------|
//! | `open`      | true                  | `["0.0.0.0/0", "<llm-cidr>"]`       | DEFAULT_DENY_OUT                       |
//! | `airgap`    | false                 | `["<llm-cidr>"]`                    | DEFAULT_DENY_OUT                       |
//! | `allowlist` | false                 | `["<llm-cidr>", ...resolved-user]`  | DEFAULT_DENY_OUT                       |
//! | `denylist`  | true                  | `["0.0.0.0/0", "<llm-cidr>"]`       | `[...DEFAULT_DENY_OUT, ...user]`       |
//!
//! `<llm-cidr>` is derived from `cfg.cube_facing_addr` (the swarm proxy
//! the dyson talks to for `/llm` traffic).  Hostnames in user-supplied
//! entries are DNS-resolved at hire-time via the `HostResolver` trait;
//! the resolved IPv4 set is what lands in the cube's allowOut/denyOut
//! map AND in the row's `network_policy_cidrs` column.  The original
//! user-typed entries are preserved separately on the row so the SPA
//! can show "you typed github.com" alongside "the cube enforces these
//! /32s".
//!
//! `Open`'s wire shape is byte-identical to the pre-feature hardcoded
//! body (see `cube_client.rs::DEFAULT_ALLOW_OUT`/`DEFAULT_DENY_OUT`)
//! so existing instances and SPA flows aren't perturbed.

use std::net::IpAddr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// `0.0.0.0/0` keeps the cube's eBPF policy in blacklist mode (allow
/// everything not in `denyOut`).  `192.168.0.1/32` punches through
/// the always-denied `192.168.0.0/16` for the cube → host path —
/// the cube image ships an `/etc/hosts` entry mapping the swarm
/// hostname to that IP to dodge the host's NAT-hairpin failure.
/// The eBPF rule is "allow > deny > default-allow", so the /32 wins.
pub const DEFAULT_OPEN_ALLOW_OUT: &[&str] = &["0.0.0.0/0", "192.168.0.1/32"];

/// Default outbound deny — same as CubeNet's hardcoded
/// `alwaysDeniedSandboxCIDRs`.  Mirrored here so the swarm's HTTP
/// payload makes the policy explicit at the CubeAPI boundary.
pub const DEFAULT_DENY_OUT: &[&str] = &[
    "10.0.0.0/8",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.168.0.0/16",
];

/// Profile chosen by the operator at hire time.  Persisted on the
/// instance row.  Snapshot/restore + binary-rotation carry it
/// through to successors verbatim.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum NetworkPolicy {
    /// Full internet, default deny only on RFC1918+linklocal.  Same as
    /// the pre-feature hardcoded behaviour.  Default for any row that
    /// doesn't explicitly opt in.
    #[default]
    Open,
    /// No egress except to the swarm /llm proxy.  Cube can't reach
    /// anything else — no DNS, no upstream APIs, nothing.  Useful for
    /// chat-only agents that should never touch the public internet.
    Airgap,
    /// LLM proxy + a curated allow list.  Each entry is either a
    /// CIDR/bare-IPv4 (used verbatim) or a hostname (DNS-resolved at
    /// hire time; resolved A-records become /32 CIDRs in the cube
    /// map).
    Allowlist {
        #[serde(default)]
        entries: Vec<String>,
    },
    /// Full internet minus the swarm defaults plus a curated deny
    /// list.  Same hostname/CIDR resolution rules as Allowlist.
    Denylist {
        #[serde(default)]
        entries: Vec<String>,
    },
}

impl NetworkPolicy {
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Airgap => "airgap",
            Self::Allowlist { .. } => "allowlist",
            Self::Denylist { .. } => "denylist",
        }
    }

    pub fn entries(&self) -> &[String] {
        match self {
            Self::Open | Self::Airgap => &[],
            Self::Allowlist { entries } | Self::Denylist { entries } => entries,
        }
    }
}

/// Wire shape handed to the CubeAPI's `POST /sandboxes` body.
/// Built once per `create` / `restore` from a `NetworkPolicy` +
/// the swarm's resolved llm CIDR + a `HostResolver`.  All hostname
/// resolution happens before this struct is constructed; from this
/// point on the policy is pure IPv4 CIDRs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPolicy {
    pub allow_internet_access: bool,
    pub allow_out: Vec<String>,
    pub deny_out: Vec<String>,
    /// The `<llm-cidr>` (or fallback) used in this resolution.
    /// Captured for logging/tests.
    pub llm_cidr_used: Option<String>,
}

impl Default for ResolvedPolicy {
    /// Default = the legacy `Open` wire shape.  Used by tests that
    /// don't care about the policy and by code paths that haven't
    /// been migrated yet.
    fn default() -> Self {
        Self {
            allow_internet_access: true,
            allow_out: DEFAULT_OPEN_ALLOW_OUT.iter().map(|s| (*s).to_owned()).collect(),
            deny_out: DEFAULT_DENY_OUT.iter().map(|s| (*s).to_owned()).collect(),
            llm_cidr_used: Some("192.168.0.1/32".to_owned()),
        }
    }
}

/// Errors surfaced from the network-policy translation layer.  All
/// map to `SwarmError::BadRequest` at the HTTP boundary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("invalid network entry {0:?}: must be an IPv4 CIDR (a.b.c.d/N), a bare IPv4, or a hostname")]
    InvalidEntry(String),
    #[error("hostname {0:?} resolved to no IPv4 addresses")]
    HostUnresolvable(String),
    #[error("Allowlist requires at least one entry — empty allowlist is the same as Airgap; pick Airgap directly")]
    EmptyAllowlist,
    #[error("Airgap and Allowlist require an LLM-proxy CIDR — set `cube_facing_addr` to an IPv4 in swarm.toml")]
    LlmCidrRequired,
}

/// Either an IPv4 CIDR (passed through verbatim) or a hostname (to
/// be DNS-resolved).  Output of `validate_entry`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryShape {
    /// Already a CIDR — `a.b.c.d/N` or bare `a.b.c.d` (treated as `/32`).
    Cidr(String),
    /// Hostname — needs DNS resolution before it can hit the cube.
    Host(String),
}

/// Trait for resolving hostnames to IPv4 addresses at hire-time.
/// The production impl wraps `tokio::net::lookup_host`; tests use a
/// `BTreeMap`-backed mock.
#[async_trait]
pub trait HostResolver: Send + Sync {
    async fn resolve_ipv4(&self, host: &str) -> Result<Vec<String>, PolicyError>;
}

/// Production resolver.  Uses `tokio::net::lookup_host(host:0)` and
/// keeps only the IPv4 addresses (CubeNet's eBPF maps are IPv4-only).
/// No new dependency — `tokio` is already in the tree.
pub struct DnsHostResolver;

#[async_trait]
impl HostResolver for DnsHostResolver {
    async fn resolve_ipv4(&self, host: &str) -> Result<Vec<String>, PolicyError> {
        // `lookup_host` requires a port; we use `:0` purely as a
        // syntactic anchor — the port is dropped before the CIDR is
        // emitted.  This calls the host's resolver (systemd-resolved /
        // /etc/resolv.conf), which is what we want: operators
        // control which DNS the swarm trusts at the host level.
        let target = format!("{host}:0");
        let addrs = tokio::net::lookup_host(&target)
            .await
            .map_err(|_| PolicyError::HostUnresolvable(host.to_owned()))?;
        let mut out: Vec<String> = addrs
            .filter_map(|sa| match sa.ip() {
                IpAddr::V4(v4) => Some(format!("{v4}/32")),
                IpAddr::V6(_) => None,
            })
            .collect();
        // Deduplicate so a hostname returning the same A-record under
        // multiple DNS round-trips doesn't bloat the cube map.
        out.sort();
        out.dedup();
        if out.is_empty() {
            return Err(PolicyError::HostUnresolvable(host.to_owned()));
        }
        Ok(out)
    }
}

/// Validate a user-supplied entry as either an IPv4 CIDR or a
/// hostname.  Hand-rolled — no `cidr` / `ipnet` crate added.  IPv6
/// is intentionally rejected because the cube's eBPF maps don't
/// support it.
pub fn validate_entry(s: &str) -> Result<EntryShape, PolicyError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(PolicyError::InvalidEntry(s.to_owned()));
    }
    if let Some(shape) = parse_cidr_or_ip(s) {
        return Ok(shape);
    }
    if is_valid_hostname(s) {
        return Ok(EntryShape::Host(s.to_ascii_lowercase()));
    }
    Err(PolicyError::InvalidEntry(s.to_owned()))
}

/// Parse `a.b.c.d/N` or bare `a.b.c.d`.  Returns `Some(EntryShape::Cidr)`
/// on success with the canonical form (bare IP gets `/32` appended);
/// returns `None` if the shape isn't a CIDR (caller falls through to
/// hostname check).
fn parse_cidr_or_ip(s: &str) -> Option<EntryShape> {
    let (ip_part, prefix_part) = match s.split_once('/') {
        Some((ip, prefix)) => (ip, Some(prefix)),
        None => (s, None),
    };
    let octets: Vec<&str> = ip_part.split('.').collect();
    if octets.len() != 4 {
        return None;
    }
    for o in &octets {
        let n: u16 = o.parse().ok()?;
        if n > 255 {
            return None;
        }
    }
    let prefix: u8 = match prefix_part {
        Some(p) => p.parse().ok()?,
        None => 32,
    };
    if prefix > 32 {
        return None;
    }
    Some(EntryShape::Cidr(format!("{ip_part}/{prefix}")))
}

/// Cheap hostname validator — labels of 1-63 ASCII alnum/`-`,
/// at least one dot.  Doesn't pretend to be RFC-perfect; the
/// authoritative check is the DNS resolution itself.
fn is_valid_hostname(s: &str) -> bool {
    if s.len() > 253 {
        return false;
    }
    if !s.contains('.') {
        return false;
    }
    // All-digits-and-dots strings are malformed IPs (`1.2.3`,
    // `999.999`, etc.) — reject before the loop so they don't slip
    // through as "hostnames" that DNS will then refuse.
    if s.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return false;
    }
    for label in s.split('.') {
        if label.is_empty() || label.len() > 63 {
            return false;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return false;
        }
        if !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return false;
        }
    }
    true
}

/// Resolve every user entry into IPv4 CIDRs (CIDR pass-through;
/// hostname → DNS lookup).  Returns the flattened, deduped CIDR list
/// alongside the original raw entries (caller persists both — the
/// cube only ever sees `cidrs`).  Empty input is OK; the caller
/// gates on emptiness for Allowlist.
pub async fn resolve_entries(
    raw: &[String],
    resolver: &dyn HostResolver,
) -> Result<Vec<String>, PolicyError> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        match validate_entry(entry)? {
            EntryShape::Cidr(c) => out.push(c),
            EntryShape::Host(h) => {
                let resolved = resolver.resolve_ipv4(&h).await?;
                out.extend(resolved);
            }
        }
    }
    // Stable order so wire-shape tests are deterministic; dedupe so a
    // user typing `github.com, 140.82.121.4` doesn't double-insert.
    out.sort();
    out.dedup();
    Ok(out)
}

/// Translate a `NetworkPolicy` into the wire shape the CubeAPI
/// expects.  Hostname entries are resolved through `resolver` here;
/// the returned `ResolvedPolicy` is pure IPv4 CIDRs.
///
/// `llm_cidr` is the swarm-proxy CIDR (`<a.b.c.d>/32` derived from
/// `cfg.cube_facing_addr`).  `Airgap` and `Allowlist` require it
/// (the dyson can't reach the LLM otherwise); `Open` and `Denylist`
/// fall back to the legacy `192.168.0.1/32` literal so deployments
/// without `cube_facing_addr` keep their pre-feature wire shape.
pub async fn resolve(
    policy: &NetworkPolicy,
    llm_cidr: Option<&str>,
    resolver: &dyn HostResolver,
) -> Result<ResolvedPolicy, PolicyError> {
    let llm_owned = llm_cidr.map(str::to_owned);
    let llm_or_default = llm_owned.clone().unwrap_or_else(|| "192.168.0.1/32".to_owned());
    let default_deny: Vec<String> = DEFAULT_DENY_OUT.iter().map(|s| (*s).to_owned()).collect();
    match policy {
        NetworkPolicy::Open => Ok(ResolvedPolicy {
            allow_internet_access: true,
            allow_out: vec!["0.0.0.0/0".to_owned(), llm_or_default.clone()],
            deny_out: default_deny,
            llm_cidr_used: Some(llm_or_default),
        }),
        NetworkPolicy::Airgap => {
            let llm = llm_owned.ok_or(PolicyError::LlmCidrRequired)?;
            Ok(ResolvedPolicy {
                allow_internet_access: false,
                allow_out: vec![llm.clone()],
                deny_out: default_deny,
                llm_cidr_used: Some(llm),
            })
        }
        NetworkPolicy::Allowlist { entries } => {
            if entries.is_empty() {
                return Err(PolicyError::EmptyAllowlist);
            }
            let llm = llm_owned.ok_or(PolicyError::LlmCidrRequired)?;
            let user_resolved = resolve_entries(entries, resolver).await?;
            let mut allow_out = vec![llm.clone()];
            allow_out.extend(user_resolved);
            // Dedup AFTER prepending the LLM hop so a user who also
            // typed the LLM hop's CIDR doesn't get a doubled entry.
            allow_out.dedup();
            Ok(ResolvedPolicy {
                allow_internet_access: false,
                allow_out,
                deny_out: default_deny,
                llm_cidr_used: Some(llm),
            })
        }
        NetworkPolicy::Denylist { entries } => {
            let user_resolved = resolve_entries(entries, resolver).await?;
            let mut deny_out = default_deny;
            deny_out.extend(user_resolved);
            deny_out.dedup();
            Ok(ResolvedPolicy {
                allow_internet_access: true,
                allow_out: vec!["0.0.0.0/0".to_owned(), llm_or_default.clone()],
                deny_out,
                llm_cidr_used: Some(llm_or_default),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// `BTreeMap`-backed mock resolver.  Hostname → IPv4 list.  An
    /// unmapped hostname returns `HostUnresolvable`.
    #[derive(Default)]
    struct MockResolver {
        map: Mutex<BTreeMap<String, Vec<String>>>,
    }

    impl MockResolver {
        fn with(map: &[(&str, &[&str])]) -> Self {
            let mut m = BTreeMap::new();
            for (host, ips) in map {
                m.insert(
                    (*host).to_owned(),
                    ips.iter().map(|ip| format!("{ip}/32")).collect(),
                );
            }
            Self { map: Mutex::new(m) }
        }
    }

    #[async_trait]
    impl HostResolver for MockResolver {
        async fn resolve_ipv4(&self, host: &str) -> Result<Vec<String>, PolicyError> {
            self.map
                .lock()
                .unwrap()
                .get(host)
                .cloned()
                .ok_or_else(|| PolicyError::HostUnresolvable(host.to_owned()))
        }
    }

    #[tokio::test]
    async fn resolve_open_matches_legacy_default_allow_out() {
        // Wire-shape regression guard.  Pre-feature `cube_client.rs`
        // hardcoded these bytes; a refactor must keep them byte-
        // identical for `Open` (the default for every existing row).
        let r = MockResolver::default();
        let resolved = resolve(&NetworkPolicy::Open, Some("192.168.0.1/32"), &r)
            .await
            .unwrap();
        assert!(resolved.allow_internet_access);
        assert_eq!(resolved.allow_out, vec!["0.0.0.0/0", "192.168.0.1/32"]);
        assert_eq!(
            resolved.deny_out,
            vec![
                "10.0.0.0/8",
                "127.0.0.0/8",
                "169.254.0.0/16",
                "172.16.0.0/12",
                "192.168.0.0/16",
            ],
        );
    }

    #[tokio::test]
    async fn resolve_open_falls_back_when_llm_cidr_missing() {
        // `Open` doesn't structurally need the LLM CIDR (it includes
        // `0.0.0.0/0`).  Fall back to the literal `192.168.0.1/32` so
        // deployments without `cube_facing_addr` configured keep their
        // pre-feature wire shape.
        let r = MockResolver::default();
        let resolved = resolve(&NetworkPolicy::Open, None, &r).await.unwrap();
        assert_eq!(resolved.allow_out, vec!["0.0.0.0/0", "192.168.0.1/32"]);
    }

    #[tokio::test]
    async fn resolve_airgap_emits_only_llm_cidr() {
        let r = MockResolver::default();
        let resolved = resolve(&NetworkPolicy::Airgap, Some("10.20.30.40/32"), &r)
            .await
            .unwrap();
        assert!(!resolved.allow_internet_access);
        assert_eq!(resolved.allow_out, vec!["10.20.30.40/32"]);
    }

    #[tokio::test]
    async fn resolve_airgap_without_llm_cidr_errors() {
        // `Airgap` needs the LLM CIDR (the dyson can't reach the
        // LLM otherwise).  Operators without `cube_facing_addr` set
        // see this error at hire time, not silent broken instances.
        let r = MockResolver::default();
        let err = resolve(&NetworkPolicy::Airgap, None, &r).await.unwrap_err();
        assert_eq!(err, PolicyError::LlmCidrRequired);
    }

    #[tokio::test]
    async fn resolve_allowlist_prepends_llm_cidr() {
        let r = MockResolver::default();
        let p = NetworkPolicy::Allowlist {
            entries: vec!["8.8.8.8/32".to_owned()],
        };
        let resolved = resolve(&p, Some("10.0.0.1/32"), &r).await.unwrap();
        assert!(!resolved.allow_internet_access);
        assert_eq!(resolved.allow_out[0], "10.0.0.1/32");
        assert!(resolved.allow_out.contains(&"8.8.8.8/32".to_owned()));
    }

    #[tokio::test]
    async fn resolve_allowlist_resolves_hostnames_to_cidrs() {
        let r = MockResolver::with(&[("github.com", &["140.82.121.4"])]);
        let p = NetworkPolicy::Allowlist {
            entries: vec!["github.com".to_owned()],
        };
        let resolved = resolve(&p, Some("10.0.0.1/32"), &r).await.unwrap();
        assert!(resolved.allow_out.contains(&"140.82.121.4/32".to_owned()));
    }

    #[tokio::test]
    async fn resolve_allowlist_with_multiple_a_records_includes_all() {
        let r = MockResolver::with(&[(
            "example.com",
            &["93.184.216.34", "93.184.216.35"],
        )]);
        let p = NetworkPolicy::Allowlist {
            entries: vec!["example.com".to_owned()],
        };
        let resolved = resolve(&p, Some("10.0.0.1/32"), &r).await.unwrap();
        assert!(resolved.allow_out.contains(&"93.184.216.34/32".to_owned()));
        assert!(resolved.allow_out.contains(&"93.184.216.35/32".to_owned()));
    }

    #[tokio::test]
    async fn resolve_allowlist_empty_entries_errors() {
        // SPA prevents this case (auto-flips the radio to Airgap when
        // chip count → 0); the API rejects it as belt-and-braces so a
        // direct curl can't bypass.
        let r = MockResolver::default();
        let p = NetworkPolicy::Allowlist { entries: vec![] };
        let err = resolve(&p, Some("10.0.0.1/32"), &r).await.unwrap_err();
        assert_eq!(err, PolicyError::EmptyAllowlist);
    }

    #[tokio::test]
    async fn resolve_denylist_appends_resolved_to_default_deny() {
        let r = MockResolver::with(&[("evil.example", &["1.2.3.4"])]);
        let p = NetworkPolicy::Denylist {
            entries: vec!["evil.example".to_owned(), "5.6.7.0/24".to_owned()],
        };
        let resolved = resolve(&p, Some("10.0.0.1/32"), &r).await.unwrap();
        assert!(resolved.allow_internet_access);
        // Default deny still present.
        assert!(resolved.deny_out.iter().any(|c| c == "10.0.0.0/8"));
        // User entries appended (post-DNS).
        assert!(resolved.deny_out.iter().any(|c| c == "1.2.3.4/32"));
        assert!(resolved.deny_out.iter().any(|c| c == "5.6.7.0/24"));
    }

    #[tokio::test]
    async fn resolve_unresolvable_host_errors() {
        let r = MockResolver::default();
        let p = NetworkPolicy::Allowlist {
            entries: vec!["nope.example".to_owned()],
        };
        let err = resolve(&p, Some("10.0.0.1/32"), &r).await.unwrap_err();
        assert!(matches!(err, PolicyError::HostUnresolvable(ref h) if h == "nope.example"));
    }

    #[test]
    fn validate_entry_accepts_cidr_and_host() {
        assert_eq!(
            validate_entry("10.0.0.0/8").unwrap(),
            EntryShape::Cidr("10.0.0.0/8".into())
        );
        assert_eq!(
            validate_entry("8.8.8.8").unwrap(),
            EntryShape::Cidr("8.8.8.8/32".into())
        );
        assert_eq!(
            validate_entry("github.com").unwrap(),
            EntryShape::Host("github.com".into())
        );
        assert_eq!(
            validate_entry("API.GitHub.com").unwrap(),
            EntryShape::Host("api.github.com".into()),
            "hostnames are lowercased",
        );
    }

    #[test]
    fn validate_entry_accepts_zero_zero_zero_zero_slash_zero() {
        // Edge case: `0.0.0.0/0` is the canonical "everywhere"
        // — the cube's allow > deny rule means an operator who
        // wants a permissive Denylist can rely on this round-tripping.
        assert_eq!(
            validate_entry("0.0.0.0/0").unwrap(),
            EntryShape::Cidr("0.0.0.0/0".into())
        );
    }

    #[test]
    fn validate_entry_rejects_garbage() {
        assert!(matches!(
            validate_entry(""),
            Err(PolicyError::InvalidEntry(_))
        ));
        assert!(matches!(
            validate_entry("nope"),
            // Single-label "nope" has no dot → not a hostname
            // → InvalidEntry.
            Err(PolicyError::InvalidEntry(_))
        ));
        assert!(matches!(
            validate_entry("1.2.3.4/99"),
            Err(PolicyError::InvalidEntry(_))
        ));
        assert!(matches!(
            validate_entry("1.2.3"),
            Err(PolicyError::InvalidEntry(_))
        ));
        assert!(matches!(
            validate_entry("256.0.0.0/8"),
            Err(PolicyError::InvalidEntry(_))
        ));
        assert!(matches!(
            validate_entry("-bad.example"),
            Err(PolicyError::InvalidEntry(_))
        ));
    }
}
