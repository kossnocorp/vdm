use super::prelude::*;

mod install;
use install::*;

mod add;
use add::*;

mod update;
use update::*;

#[derive(Subcommands)]
#[usage(run_async)]
pub enum VitCliCmd {
    /// Install dependencies
    Install(VitCliCmdInstall),

    /// Add a dependency
    Add(VitCliCmdAdd),

    /// Update a dependency
    Update(VitCliCmdUpdate),
}
