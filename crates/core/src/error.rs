use thiserror::Error;

#[derive(Debug, Error)]
pub enum CubeError {
    #[error("cube returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("cube transport error: {0}")]
    Transport(String),
    #[error("cube response decode error: {0}")]
    Decode(String),
}

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("store i/o: {0}")]
    Io(String),
    #[error("not found")]
    NotFound,
    #[error("constraint violation: {0}")]
    Constraint(String),
    #[error("malformed row: {0}")]
    Malformed(String),
}

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("backup sink error: {0}")]
    Sink(String),
    #[error("snapshot has no local copy and no remote URI")]
    Missing,
    #[error("backup i/o: {0}")]
    Io(String),
}

#[derive(Debug, Error)]
pub enum SwarmError {
    #[error(transparent)]
    Cube(#[from] CubeError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Backup(#[from] BackupError),
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found")]
    NotFound,
    /// Used for failures we surface but don't have a more specific variant
    /// for — currently only the create-time configure-push retry budget
    /// exhaustion (which would otherwise leave the cube in warmup mode).
    #[error("internal: {0}")]
    Internal(String),
    /// A3: snapshot bytes on disk don't match the SHA-256 we recorded
    /// at snapshot-time — either disk corruption or tampering.  The
    /// http layer maps this to 409 Conflict so the SPA surfaces an
    /// actionable error instead of "internal".
    #[error("snapshot corrupt: {0}")]
    SnapshotCorrupt(String),
    /// A6: per-instance snapshot quota exceeded.  Mapped to 429 by
    /// the http layer with a body that exposes the limit so the SPA
    /// can render "you have N of N snapshots, delete one to take
    /// another".
    #[error("snapshot quota exceeded (limit {limit})")]
    SnapshotQuotaExceeded { limit: u64 },
}

impl From<crate::network_policy::PolicyError> for SwarmError {
    fn from(e: crate::network_policy::PolicyError) -> Self {
        SwarmError::BadRequest(e.to_string())
    }
}
