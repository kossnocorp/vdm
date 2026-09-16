use crate::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct VdmHttpTarget {
    url: Url,
    key: VdmManifestTargetUrl,
    version: VdmManifestSourceVersion,
    vendor_path: PathBuf,
}

impl VdmHttpTarget {
    pub(crate) fn new(
        url: Url,
        key: VdmManifestTargetUrl,
        version: VdmManifestSourceVersion,
        vendor_path: PathBuf,
    ) -> Self {
        Self {
            url,
            key,
            version,
            vendor_path,
        }
    }

    pub(crate) fn with_content_version(mut self, bytes: &[u8]) -> Self {
        self.version = VdmManifestSourceVersion::new(format!("sha256:{:x}", Sha256::digest(bytes)));
        self
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }
}

impl VdmTarget for VdmHttpTarget {
    fn key(&self) -> &VdmManifestTargetUrl {
        &self.key
    }

    fn version(&self) -> &VdmManifestSourceVersion {
        &self.version
    }

    fn source_url(&self) -> &str {
        self.key.as_str()
    }

    fn vendor_path(&self) -> PathBuf {
        self.vendor_path.clone()
    }

    fn source(&self) -> &'static dyn VdmSource {
        &VDM_HTTP_SOURCE
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
