use crate::cli::prelude::*;

#[derive(Args)]
pub struct VdmCliCmdAdd {
    #[usage(flatten)]
    manifest_args: VdmCliArgsManifest,

    #[usage(value_name = "FILE")]
    file: String,
}

impl RunAsync for VdmCliCmdAdd {
    type Output = Result<()>;

    async fn run_async(self) -> Self::Output {
        VdmVendor::add(self.manifest_args.manifest.as_deref(), &self.file).await
    }
}
