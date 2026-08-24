mod prelude;
use prelude::*;

mod cmd;
use cmd::*;

mod args;
use args::*;

#[derive(Cli)]
#[usage(run_async, bin = "vit", about = "Vendored dependencies manager")]
pub struct VitCli {
    #[usage(subcommand)]
    pub command: VitCliCmd,
}
