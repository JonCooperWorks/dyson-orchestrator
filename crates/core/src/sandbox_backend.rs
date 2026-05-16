use std::sync::Arc;

use async_trait::async_trait;

use crate::error::CubeError;
use crate::traits::{CreateSandboxArgs, CubeClient, SandboxBackend, SandboxInfo, SnapshotInfo};

#[derive(Clone)]
pub struct CubeSandboxBackend {
    cube: Arc<dyn CubeClient>,
}

impl CubeSandboxBackend {
    pub fn new(cube: Arc<dyn CubeClient>) -> Self {
        Self { cube }
    }
}

#[async_trait]
impl SandboxBackend for CubeSandboxBackend {
    async fn create_sandbox(&self, args: CreateSandboxArgs) -> Result<SandboxInfo, CubeError> {
        self.cube.create_sandbox(args).await
    }

    async fn destroy_sandbox(&self, sandbox_id: &str) -> Result<(), CubeError> {
        self.cube.destroy_sandbox(sandbox_id).await
    }

    async fn snapshot_sandbox(
        &self,
        sandbox_id: &str,
        name: &str,
    ) -> Result<SnapshotInfo, CubeError> {
        self.cube.snapshot_sandbox(sandbox_id, name).await
    }

    async fn delete_snapshot(&self, snapshot_id: &str, host_ip: &str) -> Result<(), CubeError> {
        self.cube.delete_snapshot(snapshot_id, host_ip).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use crate::network_policy::ResolvedPolicy;

    #[derive(Default)]
    struct RecordingCube {
        creates: Mutex<Vec<CreateSandboxArgs>>,
        destroys: Mutex<Vec<String>>,
        snapshots: Mutex<Vec<(String, String)>>,
        deletes: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl CubeClient for RecordingCube {
        async fn create_sandbox(&self, args: CreateSandboxArgs) -> Result<SandboxInfo, CubeError> {
            self.creates.lock().unwrap().push(args);
            Ok(SandboxInfo {
                sandbox_id: "sb-1".into(),
                host_ip: "10.0.0.2".into(),
                url: "https://sb-1.cube.test".into(),
            })
        }

        async fn destroy_sandbox(&self, sandbox_id: &str) -> Result<(), CubeError> {
            self.destroys.lock().unwrap().push(sandbox_id.to_owned());
            Ok(())
        }

        async fn snapshot_sandbox(
            &self,
            sandbox_id: &str,
            name: &str,
        ) -> Result<SnapshotInfo, CubeError> {
            self.snapshots
                .lock()
                .unwrap()
                .push((sandbox_id.to_owned(), name.to_owned()));
            Ok(SnapshotInfo {
                snapshot_id: "snap-1".into(),
                path: "/snap/snap-1".into(),
                host_ip: "10.0.0.2".into(),
            })
        }

        async fn delete_snapshot(&self, snapshot_id: &str, host_ip: &str) -> Result<(), CubeError> {
            self.deletes
                .lock()
                .unwrap()
                .push((snapshot_id.to_owned(), host_ip.to_owned()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn cube_backend_preserves_lifecycle_request_shapes() {
        let cube = Arc::new(RecordingCube::default());
        let backend = CubeSandboxBackend::new(cube.clone());
        let mut env = std::collections::BTreeMap::new();
        env.insert("A".into(), "B".into());
        let policy = ResolvedPolicy {
            allow_internet_access: true,
            allow_out: vec!["0.0.0.0/0".into()],
            deny_out: vec!["192.168.0.0/16".into()],
            llm_cidr_used: None,
        };

        let info = backend
            .create_sandbox(CreateSandboxArgs {
                template_id: "tpl-1".into(),
                env: env.clone(),
                from_snapshot_path: Some(PathBuf::from("/snap/in")),
                resolved_policy: policy.clone(),
            })
            .await
            .unwrap();
        assert_eq!(info.sandbox_id, "sb-1");

        backend.snapshot_sandbox("sb-1", "snap-name").await.unwrap();
        backend.destroy_sandbox("sb-1").await.unwrap();
        backend.delete_snapshot("snap-1", "10.0.0.2").await.unwrap();

        let creates = cube.creates.lock().unwrap();
        assert_eq!(creates.len(), 1);
        assert_eq!(creates[0].template_id, "tpl-1");
        assert_eq!(creates[0].env, env);
        assert_eq!(
            creates[0].from_snapshot_path.as_deref(),
            Some(std::path::Path::new("/snap/in"))
        );
        assert_eq!(creates[0].resolved_policy, policy);
        assert_eq!(
            cube.snapshots.lock().unwrap().as_slice(),
            &[("sb-1".into(), "snap-name".into())]
        );
        assert_eq!(
            cube.destroys.lock().unwrap().as_slice(),
            &["sb-1".to_owned()]
        );
        assert_eq!(
            cube.deletes.lock().unwrap().as_slice(),
            &[("snap-1".into(), "10.0.0.2".into())]
        );
    }
}
