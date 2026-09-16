use crate::prelude::*;

use std::any::Any;

mod input;
pub use input::*;

mod github;
pub use github::*;

mod http;
pub use http::*;

mod file;
pub use file::*;

#[async_trait]
pub trait VdmSource: Send + Sync {
    fn parse(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>>;

    async fn download(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile>;
}

pub trait VdmTarget: Any + Send + Sync {
    fn key(&self) -> &VdmManifestTargetUrl;

    fn version(&self) -> &VdmManifestSourceVersion;

    fn source_url(&self) -> &str;

    fn vendor_path(&self) -> PathBuf;

    fn source(&self) -> &'static dyn VdmSource;

    fn as_any(&self) -> &dyn Any;
}
