use crate::cli::prelude::*;

#[derive(Args)]
pub struct VitCliCmdInstall {
    #[usage(flatten)]
    manifest_args: VitCliArgsManifest,

    /// Never hit network, use only local cache.
    #[usage(short, long, default = "false")]
    offline: bool,
}

impl RunAsync for VitCliCmdInstall {
    type Output = Result<()>;

    async fn run_async(self) -> Self::Output {
        VitVendor::install(self.manifest_args.manifest.as_deref(), self.offline).await
    }
}
