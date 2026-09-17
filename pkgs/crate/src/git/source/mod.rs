use crate::prelude::*;

mod parse;

mod download;

pub static VDM_GIT_SOURCE: VdmGitSource = VdmGitSource;

pub struct VdmGitSource;

#[async_trait]
impl VdmSource for VdmGitSource {
    fn parse(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        self.parse_target(input)
    }

    async fn download(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        self.download_file(target).await
    }
}
