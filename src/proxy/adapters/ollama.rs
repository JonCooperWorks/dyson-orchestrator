//! Ollama — local-first, no auth swap needed. The adapter exists so the
//! proxy router doesn't need a special-case branch; `rewrite_auth` simply
//! drops any `Authorization` header so the proxy bearer never leaks
//! upstream.

use axum::http::{HeaderMap, Uri};

use crate::config::ProviderConfig;
use crate::traits::ProviderAdapter;

pub struct OllamaAdapter;

impl ProviderAdapter for OllamaAdapter {
    fn name(&self) -> &'static str {
        "ollama"
    }

    fn upstream_base_url<'a>(&self, config: &'a ProviderConfig) -> &'a str {
        &config.upstream
    }

    fn rewrite_auth(&self, headers: &mut HeaderMap, _url: &mut Uri, _real_key: &str) {
        headers.remove(axum::http::header::AUTHORIZATION);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_authorization_header_no_other_changes() {
        let a = OllamaAdapter;
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer client"),
        );
        headers.insert("x-other", axum::http::HeaderValue::from_static("kept"));
        let mut url: Uri = "/api/generate".parse().unwrap();
        a.rewrite_auth(&mut headers, &mut url, "");
        assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
        assert_eq!(headers.get("x-other").unwrap(), "kept");
        assert_eq!(url.path(), "/api/generate");
    }
}
