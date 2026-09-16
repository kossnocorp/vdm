use crate::prelude::*;

pub struct VdmStateErrored {
    pub dirs: Option<VdmDirs>,
    pub paths: Option<VdmPaths>,
    pub error: Error,
}

impl VdmStateErrored {
    pub fn create_error(err: Error) -> VdmState {
        Self {
            dirs: None,
            paths: None,
            error: err.context("Failed to create Vdm state"),
        }
        .into()
    }

    pub fn initialize_error(dirs: VdmDirs, paths: Option<VdmPaths>, err: Error) -> VdmState {
        Self {
            dirs: Some(dirs),
            paths,
            error: err.context("Failed to initialize Vdm state"),
        }
        .into()
    }
}

impl From<VdmStateErrored> for VdmState {
    fn from(val: VdmStateErrored) -> Self {
        VdmState::Errored(val)
    }
}
