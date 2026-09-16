use crate::cli::prelude::*;

#[derive(Args)]
pub struct VdmCliCmdUpdate {
    #[usage(flatten)]
    manifest_args: VdmCliArgsManifest,

    #[usage(value_name = "FILE")]
    file: String,

    /// Review each changed file before applying the update.
    #[usage(long, default = "false")]
    review: bool,
}

impl RunAsync for VdmCliCmdUpdate {
    type Output = Result<()>;

    async fn run_async(self) -> Self::Output {
        VdmVendor::update(
            self.manifest_args.manifest.as_deref(),
            &self.file,
            self.review,
        )
        .await
    }
}
