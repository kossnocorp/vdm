mod prelude;
use prelude::*;

mod cmd;
use cmd::*;

mod args;
use args::*;

#[derive(Cli)]
#[usage(bin = "vdm", about = "Vendored dependencies manager")]
pub struct VdmCli {
    /// Target a package directory instead of the discovered manifest.
    #[usage(long, value_name = "PATH")]
    pub pkg: Option<PathBuf>,
    #[usage(subcommand)]
    pub command: VdmCliCmd,
}

impl RunAsync for VdmCli {
    type Output = Result<()>;

    async fn run_async(mut self) -> Self::Output {
        if let Some(pkg) = self.pkg {
            ensure!(
                pkg.is_dir(),
                "Package directory does not exist: {}",
                pkg.display()
            );
            let args = self.command.manifest_args_mut();
            ensure!(
                args.manifest.is_none(),
                "--pkg and --manifest cannot be used together"
            );
            args.manifest = Some(pkg.join("vendor.toml"));
        }
        self.command.run_async().await
    }
}
