use super::prelude::*;

mod install;
use install::*;

mod add;
use add::*;

mod update;
use update::*;

#[derive(Subcommands)]
#[usage(run_async)]
pub enum VdmCliCmd {
    /// Install dependencies
    Install(VdmCliCmdInstall),

    /// Add a dependency
    Add(VdmCliCmdAdd),

    /// Update one or all dependencies
    Update(VdmCliCmdUpdate),
}

impl VdmCliCmd {
    pub(super) fn manifest_args_mut(&mut self) -> &mut VdmCliArgsManifest {
        match self {
            Self::Install(command) => &mut command.manifest_args,
            Self::Add(command) => &mut command.manifest_args,
            Self::Update(command) => &mut command.manifest_args,
        }
    }
}
