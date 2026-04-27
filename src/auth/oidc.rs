//! OIDC `Authenticator` — populated in phase 4.
//!
//! The shape is fixed: validate an inbound `Bearer <jwt>` against the IdP's
//! JWKS, check `iss` and `aud`, and project the `sub`/`email`/`name` claims
//! onto a [`UserIdentity`]. JWKS is fetched once on first use and refreshed
//! when an unknown `kid` shows up.

// Re-exports land in phase 4. Keeping the file present so `mod oidc;` in
// `auth/mod.rs` compiles.
