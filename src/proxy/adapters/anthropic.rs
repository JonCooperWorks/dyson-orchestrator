//! Anthropic — strip `Authorization`, set `x-api-key` and
//! `anthropic-version`. The version comes from the provider config
//! (`providers.anthropic.anthropic_version` in the TOML); a missing version
//! defaults to a stable date so the proxy is usable without explicit config
//! tuning.

use axum::http::{HeaderMap, HeaderName, HeaderValue, Uri};

use crate::config::ProviderConfig;
use crate::traits::ProviderAdapter;

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicAdapter;

impl AnthropicAdapter {
    fn version<'a>(config: &'a ProviderConfig) -> &'a str {
        config
            .anthropic_version
            .as_deref()
            .unwrap_or(DEFAULT_ANTHROPIC_VERSION)
    }
}

impl ProviderAdapter for AnthropicAdapter {
    fn name(&self) -> &'static str {
        "anthropic"
    }

    fn upstream_base_url<'a>(&self, config: &'a ProviderConfig) -> &'a str {
        &config.upstream
    }

    fn rewrite_auth(&self, headers: &mut HeaderMap, _url: &mut Uri, real_key: &str) {
        headers.remove(axum::http::header::AUTHORIZATION);
        let key = HeaderValue::from_str(real_key).expect("api key header");
        headers.insert(HeaderName::from_static("x-api-key"), key);
        // Note: the version is filled in at proxy-handler time because this
        // method only sees the key. The handler will inject it from the
        // provider config alongside this call. Until then, set the default
        // so a configuration-less Anthropic call still works.
        headers
            .entry(HeaderName::from_static("anthropic-version"))
            .or_insert(HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION));
    }
}

/// Helper used by the proxy handler so the version can be sourced from
/// `ProviderConfig` (not visible to `rewrite_auth` per the trait).
pub fn apply_version(headers: &mut HeaderMap, config: &ProviderConfig) {
    let v = AnthropicAdapter::version(config);
    if let Ok(hv) = HeaderValue::from_str(v) {
        headers.insert(HeaderName::from_static("anthropic-version"), hv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;

    fn cfg(version: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            api_key: Some("sk-real".into()),
            upstream: "https://api.anthropic.com".into(),
            anthropic_version: version.map(String::from),
        }
    }

    #[test]
    fn rewrite_strips_authorization_and_sets_x_api_key() {
        let a = AnthropicAdapter;
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer client-token"),
        );
        let mut url: Uri = "/v1/messages".parse().unwrap();
        a.rewrite_auth(&mut headers, &mut url, "sk-real");
        assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-real");
        assert_eq!(headers.get("anthropic-version").unwrap(), DEFAULT_ANTHROPIC_VERSION);
    }

    #[test]
    fn apply_version_uses_configured_value() {
        let mut headers = HeaderMap::new();
        apply_version(&mut headers, &cfg(Some("2024-09-01")));
        assert_eq!(headers.get("anthropic-version").unwrap(), "2024-09-01");
    }

    #[test]
    fn apply_version_default_when_unset() {
        let mut headers = HeaderMap::new();
        apply_version(&mut headers, &cfg(None));
        assert_eq!(headers.get("anthropic-version").unwrap(), DEFAULT_ANTHROPIC_VERSION);
    }
}
