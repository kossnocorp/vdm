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

    /// Update a dependency
    Update(VdmCliCmdUpdate),
}
