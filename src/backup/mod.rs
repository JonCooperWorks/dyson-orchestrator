//! Backup sinks. Step 8 lands [`local::LocalDiskBackupSink`]; step 9 adds
//! the S3 sink. Both implement the [`crate::traits::BackupSink`] trait so
//! the snapshot module can switch sinks without per-call branching.

pub mod local;
