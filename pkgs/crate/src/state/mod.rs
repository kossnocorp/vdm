use crate::prelude::*;

mod initializing;
pub use initializing::*;

mod errored;
pub use errored::*;

mod initialized;
pub use initialized::*;

mod locked;
pub use locked::*;

pub enum VdmState {
    Initializing(VdmStateInitializing),
    Errored(VdmStateErrored),
    Initialized(VdmStateInitialized),
    Locked(VdmStateLocked),
}

impl VdmState {
    pub fn create() -> Self {
        VdmStateInitializing::create_state()
    }

    pub async fn initialize(self, path: Option<&Path>) -> Result<VdmState> {
        match self {
            VdmState::Errored(_) => {
                // Nothing to do, already in errored state
                Ok(self)
            }

            VdmState::Initializing(state) => state.as_initialized_state(path).await,

            VdmState::Initialized(_) | VdmState::Locked(_) => {
                bail!("Already initialized")
            }
        }
    }

    pub async fn initialize_lock(self) -> Result<VdmState> {
        match self {
            VdmState::Errored(_) => {
                // Nothing to do, already in errored state
                Ok(self)
            }

            VdmState::Initializing(_) => {
                bail!("Cannot load lock while initializing")
            }

            VdmState::Initialized(state) => Ok(state.as_locked().await?.into()),

            VdmState::Locked(_) => {
                bail!("Already in locked state")
            }
        }
    }
}
