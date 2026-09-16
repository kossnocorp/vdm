use crate::prelude::*;

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct VdmLockFile {
    pub direct: bool,
    pub version: VdmManifestSourceVersion,
    pub revision: String,
    pub hash: String,
    pub source: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<VdmManifestTargetUrl>,
}

impl VdmLockFile {
    pub fn new(
        target: &dyn VdmTarget,
        download: &VdmSourceFile,
        paths: &VdmPaths,
        direct: bool,
        dependencies: Vec<VdmManifestTargetUrl>,
    ) -> VdmLockFile {
        let hash = Sha256::digest(&download.bytes);
        let path = paths.target(target);
        VdmLockFile {
            direct,
            version: target.version().clone(),
            revision: download.revision.clone(),
            hash: format!("sha256:{hash:x}"),
            source: target.source_url().to_owned(),
            path: path
                .strip_prefix(&paths.root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned(),
            dependencies,
        }
    }
}
