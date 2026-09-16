mod prelude;
use prelude::*;

mod cmd;
use cmd::*;

mod args;
use args::*;

#[derive(Cli)]
#[usage(run_async, bin = "vdm", about = "Vendored dependencies manager")]
pub struct VdmCli {
    #[usage(subcommand)]
    pub command: VdmCliCmd,
}
