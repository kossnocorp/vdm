use crate::prelude::*;

pub struct VdmStateInitializing {
    pub dirs: VdmDirs,
}

impl VdmStateInitializing {
    pub fn create_state() -> VdmState {
        match VdmDirs::resolve() {
            Ok(dirs) => VdmStateInitializing { dirs }.into(),

            Err(error) => VdmStateErrored::create_error(error),
        }
    }

    pub async fn as_initialized_state(self, path: Option<&Path>) -> Result<VdmState> {
        match VdmPaths::resolve(path).await {
            Ok(paths) => match VdmManifest::read_toml(&paths.manifest).await {
                Ok(manifest) => Ok(VdmStateInitialized {
                    dirs: self.dirs,
                    paths,
                    manifest,
                }
                .into()),

                Err(err) => Ok(VdmStateErrored::initialize_error(
                    self.dirs,
                    Some(paths),
                    err,
                )),
            },

            Err(err) => Ok(VdmStateErrored::initialize_error(self.dirs, None, err)),
        }
    }
}

impl From<VdmStateInitializing> for VdmState {
    fn from(state: VdmStateInitializing) -> VdmState {
        VdmState::Initializing(state)
    }
}
