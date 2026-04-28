//! Try a sequence of [`Authenticator`]s in order.
//!
//! Resolution rules:
//! - First `Ok(identity)` wins.
//! - `Err(Unsupported)` falls through to the next authenticator. This is the
//!   "wrong shape, try someone else" channel — e.g. bearer-style tokens
//!   getting handed to OIDC.
//! - `Err(Missing)` falls through too — no point asking the next link about
//!   a credential nobody supplied.
//! - `Err(Invalid)` short-circuits. A credential that was claimed but failed
//!   validation should not fall through to a different authenticator that
//!   happens to accept the same shape with weaker rules.
//! - `Err(Backend)` short-circuits — the operator needs to see the error,
//!   not have it papered over.

use std::sync::Arc;

use async_trait::async_trait;
use axum::http::HeaderMap;

use crate::auth::{AuthError, Authenticator, UserIdentity};

#[derive(Clone)]
pub struct ChainAuthenticator {
    links: Vec<Arc<dyn Authenticator>>,
}

impl ChainAuthenticator {
    pub fn new(links: Vec<Arc<dyn Authenticator>>) -> Self {
        Self { links }
    }
}

#[async_trait]
impl Authenticator for ChainAuthenticator {
    async fn authenticate(&self, headers: &HeaderMap) -> Result<UserIdentity, AuthError> {
        let mut last_missing_or_unsupported = AuthError::Missing;
        for link in &self.links {
            match link.authenticate(headers).await {
                Ok(id) => return Ok(id),
                Err(AuthError::Missing) => last_missing_or_unsupported = AuthError::Missing,
                Err(AuthError::Unsupported) => {
                    last_missing_or_unsupported = AuthError::Unsupported;
                }
                Err(other) => return Err(other),
            }
        }
        Err(last_missing_or_unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Clone)]
    struct Scripted {
        outcome: Arc<Mutex<Option<Result<UserIdentity, AuthError>>>>,
    }

    impl Scripted {
        fn new(outcome: Result<UserIdentity, AuthError>) -> Self {
            Self {
                outcome: Arc::new(Mutex::new(Some(outcome))),
            }
        }
    }

    #[async_trait]
    impl Authenticator for Scripted {
        async fn authenticate(&self, _: &HeaderMap) -> Result<UserIdentity, AuthError> {
            self.outcome
                .lock()
                .unwrap()
                .take()
                .expect("scripted auth used twice")
        }
    }

    fn id() -> UserIdentity {
        UserIdentity {
            subject: "alice".into(),
            email: None,
            display_name: None,
            source: crate::auth::AuthSource::Bearer,
            claims: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn first_ok_wins() {
        let chain = ChainAuthenticator::new(vec![
            Arc::new(Scripted::new(Ok(id()))),
            Arc::new(Scripted::new(Err(AuthError::Invalid("never run".into())))),
        ]);
        let res = chain.authenticate(&HeaderMap::new()).await.unwrap();
        assert_eq!(res.subject, "alice");
    }

    #[tokio::test]
    async fn unsupported_falls_through_invalid_short_circuits() {
        let chain = ChainAuthenticator::new(vec![
            Arc::new(Scripted::new(Err(AuthError::Unsupported))),
            Arc::new(Scripted::new(Err(AuthError::Invalid("bad".into())))),
            Arc::new(Scripted::new(Ok(id()))),
        ]);
        let err = chain.authenticate(&HeaderMap::new()).await.unwrap_err();
        assert!(matches!(err, AuthError::Invalid(_)));
    }

    #[tokio::test]
    async fn all_missing_returns_missing() {
        let chain = ChainAuthenticator::new(vec![
            Arc::new(Scripted::new(Err(AuthError::Missing))),
            Arc::new(Scripted::new(Err(AuthError::Missing))),
        ]);
        let err = chain.authenticate(&HeaderMap::new()).await.unwrap_err();
        assert!(matches!(err, AuthError::Missing));
    }
}
