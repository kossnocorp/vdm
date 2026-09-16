use crate::prelude::*;

pub trait VdmTarget: Any + Send + Sync {
    fn key(&self) -> &VdmManifestTargetUrl;

    fn version(&self) -> &VdmManifestSourceVersion;

    fn source_url(&self) -> &str;

    fn vendor_path(&self) -> PathBuf;

    fn source(&self) -> &'static dyn VdmSource;

    fn as_any(&self) -> &dyn Any;
}
