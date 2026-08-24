use crate::cli::prelude::*;

#[derive(Args)]
pub struct VitCliCmdUpdate {
    #[usage(flatten)]
    manifest_args: VitCliArgsManifest,

    #[usage(value_name = "FILE")]
    file: String,
}

impl RunAsync for VitCliCmdUpdate {
    type Output = Result<()>;

    async fn run_async(self) -> Self::Output {
        VitVendor::update(self.manifest_args.manifest.as_deref(), &self.file).await
    }
}
