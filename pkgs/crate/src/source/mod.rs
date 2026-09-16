use crate::prelude::*;

mod input;
pub use input::*;

mod file;
pub use file::*;

#[async_trait]
pub trait VdmSource: Send + Sync {
    fn parse(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>>;

    async fn download(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile>;
}
