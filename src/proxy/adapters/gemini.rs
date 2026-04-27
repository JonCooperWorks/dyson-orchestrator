//! Gemini — auth is delivered as `?key=<api_key>` on the URL. There is no
//! `Authorization` header to swap; if one is present we strip it so it
//! doesn't leak the proxy token upstream.

use axum::http::{HeaderMap, Uri};

use crate::config::ProviderConfig;
use crate::traits::ProviderAdapter;

pub struct GeminiAdapter;

impl ProviderAdapter for GeminiAdapter {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn upstream_base_url<'a>(&self, config: &'a ProviderConfig) -> &'a str {
        &config.upstream
    }

    fn rewrite_auth(&self, headers: &mut HeaderMap, url: &mut Uri, real_key: &str) {
        headers.remove(axum::http::header::AUTHORIZATION);

        let path = url.path().to_string();
        let existing = url.query().unwrap_or("");
        let new_query = if existing.is_empty() {
            format!("key={real_key}")
        } else if existing.contains("key=") {
            // Replace the existing key= param so we never leak the client's
            // value upstream. Splice on `&` boundaries.
            let mut out = String::new();
            let mut first = true;
            for part in existing.split('&') {
                if part.starts_with("key=") {
                    continue;
                }
                if !first {
                    out.push('&');
                }
                out.push_str(part);
                first = false;
            }
            if !out.is_empty() {
                out.push('&');
            }
            out.push_str("key=");
            out.push_str(real_key);
            out
        } else {
            format!("{existing}&key={real_key}")
        };

        let pq = format!("{path}?{new_query}");
        if let Ok(rebuilt) = pq.parse::<axum::http::uri::PathAndQuery>() {
            let mut parts = url.clone().into_parts();
            parts.path_and_query = Some(rebuilt);
            if let Ok(rebuilt_uri) = Uri::from_parts(parts) {
                *url = rebuilt_uri;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_key_when_no_query() {
        let a = GeminiAdapter;
        let mut headers = HeaderMap::new();
        let mut url: Uri = "/v1beta/models/gemini-pro:generateContent".parse().unwrap();
        a.rewrite_auth(&mut headers, &mut url, "AIza-real");
        assert_eq!(url.query(), Some("key=AIza-real"));
        assert_eq!(url.path(), "/v1beta/models/gemini-pro:generateContent");
    }

    #[test]
    fn appends_to_existing_query() {
        let a = GeminiAdapter;
        let mut headers = HeaderMap::new();
        let mut url: Uri = "/v1beta/models/gemini-pro:generateContent?alt=sse".parse().unwrap();
        a.rewrite_auth(&mut headers, &mut url, "AIza-real");
        assert_eq!(url.query(), Some("alt=sse&key=AIza-real"));
    }

    #[test]
    fn replaces_existing_client_key() {
        let a = GeminiAdapter;
        let mut headers = HeaderMap::new();
        let mut url: Uri = "/v1beta?key=client-leaked".parse().unwrap();
        a.rewrite_auth(&mut headers, &mut url, "AIza-real");
        assert_eq!(url.query(), Some("key=AIza-real"));
    }

    #[test]
    fn strips_authorization_header() {
        let a = GeminiAdapter;
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer client"),
        );
        let mut url: Uri = "/v1beta".parse().unwrap();
        a.rewrite_auth(&mut headers, &mut url, "AIza-real");
        assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
    }
}
