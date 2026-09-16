use crate::prelude::*;

pub struct VdmStateLocked {
    pub dirs: VdmDirs,
    pub paths: VdmPaths,
    pub manifest: VdmManifest,
    pub lock: VdmLock,
}

impl From<VdmStateLocked> for VdmState {
    fn from(state: VdmStateLocked) -> VdmState {
        VdmState::Locked(state)
    }
}
