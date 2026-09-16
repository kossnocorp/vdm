use crate::cli::prelude::*;

#[derive(Args)]
pub struct VdmCliCmdInstall {
    #[usage(flatten)]
    manifest_args: VdmCliArgsManifest,

    /// Never hit network, use only local cache.
    #[usage(short, long, default = "false")]
    offline: bool,
}

impl RunAsync for VdmCliCmdInstall {
    type Output = Result<()>;

    async fn run_async(self) -> Self::Output {
        VdmVendor::install(self.manifest_args.manifest.as_deref(), self.offline).await
    }
}
