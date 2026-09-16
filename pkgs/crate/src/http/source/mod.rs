use crate::prelude::*;
mod parse;

mod download;

pub static VDM_HTTP_SOURCE: VdmHttpSource = VdmHttpSource;

pub struct VdmHttpSource;

#[async_trait]
impl VdmSource for VdmHttpSource {
    fn parse(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        self.parse_target(input)
    }

    async fn download(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        self.download_file(target).await
    }
}
