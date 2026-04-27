//! Admin-bearer middleware for `/v1/*`.
//!
//! Two modes:
//! - `Some(token)`: require `Authorization: Bearer <token>`. Mismatch → 401.
//! - `None` (i.e. `--dangerous-no-auth`): pass-through, but every response
//!   carries an `X-Warden-Insecure: 1` header so callers cannot mistake an
//!   unauthenticated environment for an authenticated one.

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

#[derive(Clone, Debug)]
pub struct AuthState {
    pub admin_token: Option<String>,
}

impl AuthState {
    pub fn enforced(token: impl Into<String>) -> Self {
        Self { admin_token: Some(token.into()) }
    }

    pub fn dangerous_no_auth() -> Self {
        Self { admin_token: None }
    }
}

pub async fn admin_bearer(
    State(auth): State<AuthState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(expected) = auth.admin_token.as_deref() else {
        let mut resp = next.run(req).await;
        resp.headers_mut()
            .insert("X-Warden-Insecure", HeaderValue::from_static("1"));
        return resp;
    };

    let header = req.headers().get("authorization").and_then(|h| h.to_str().ok());
    let supplied = header
        .and_then(|h| h.strip_prefix("Bearer "))
        .or_else(|| header.and_then(|h| h.strip_prefix("bearer ")));
    if supplied == Some(expected) {
        next.run(req).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;

    async fn ok() -> &'static str {
        "ok"
    }

    fn app(state: AuthState) -> Router {
        Router::new()
            .route("/v1/x", get(ok))
            .layer(axum::middleware::from_fn_with_state(state, admin_bearer))
    }

    async fn spawn(state: AuthState) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let r = app(state);
        tokio::spawn(async move {
            axum::serve(listener, r).await.unwrap();
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn missing_bearer_is_401() {
        let base = spawn(AuthState::enforced("s3cr3t")).await;
        let r = reqwest::get(format!("{base}/v1/x")).await.unwrap();
        assert_eq!(r.status(), 401);
    }

    #[tokio::test]
    async fn bad_bearer_is_401() {
        let base = spawn(AuthState::enforced("s3cr3t")).await;
        let r = reqwest::Client::new()
            .get(format!("{base}/v1/x"))
            .bearer_auth("nope")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401);
    }

    #[tokio::test]
    async fn good_bearer_passes() {
        let base = spawn(AuthState::enforced("s3cr3t")).await;
        let r = reqwest::Client::new()
            .get(format!("{base}/v1/x"))
            .bearer_auth("s3cr3t")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        assert!(r.headers().get("x-warden-insecure").is_none());
    }

    #[tokio::test]
    async fn dangerous_no_auth_passes_with_marker_header() {
        let base = spawn(AuthState::dangerous_no_auth()).await;
        let r = reqwest::get(format!("{base}/v1/x")).await.unwrap();
        assert_eq!(r.status(), 200);
        assert_eq!(
            r.headers().get("x-warden-insecure").map(|v| v.to_str().unwrap()),
            Some("1")
        );
    }
}
