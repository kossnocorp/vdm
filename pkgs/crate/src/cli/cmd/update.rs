use crate::cli::prelude::*;

#[derive(Args)]
pub struct VdmCliCmdUpdate {
    #[usage(flatten)]
    pub(super) manifest_args: VdmCliArgsManifest,

    #[usage(value_name = "FILE")]
    file: Option<String>,

    /// Review each changed file before applying the update.
    #[usage(long, default = "false")]
    review: bool,
}

impl RunAsync for VdmCliCmdUpdate {
    type Output = Result<()>;

    async fn run_async(self) -> Self::Output {
        VdmVendor::update_packages(
            self.manifest_args.manifest.as_deref(),
            self.file.as_deref(),
            self.review,
        )
        .await
    }
}
