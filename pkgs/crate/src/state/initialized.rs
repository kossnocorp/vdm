use crate::prelude::*;

pub struct VdmStateInitialized {
    pub dirs: VdmDirs,
    pub paths: VdmPaths,
    pub manifest: VdmManifest,
}

impl VdmStateInitialized {
    pub async fn as_locked(self) -> Result<VdmStateLocked> {
        let lock = VdmLock::read_toml(&self.paths.lock).await?;
        Ok(VdmStateLocked {
            dirs: self.dirs,
            paths: self.paths,
            manifest: self.manifest,
            lock,
        })
    }
}

impl From<VdmStateInitialized> for VdmState {
    fn from(val: VdmStateInitialized) -> Self {
        VdmState::Initialized(val)
    }
}
