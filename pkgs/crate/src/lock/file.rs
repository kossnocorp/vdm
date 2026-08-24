use crate::prelude::*;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct VitLockFile {
    pub direct: bool,
    pub version: VitManifestSourceVersion,
    pub revision: String,
    pub hash: String,
    pub source: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<VitManifestTargetUrl>,
}

impl VitLockFile {
    pub fn new(
        target: &dyn VitTarget,
        download: &VitSourceFile,
        paths: &VitPaths,
        direct: bool,
        dependencies: Vec<VitManifestTargetUrl>,
    ) -> VitLockFile {
        let hash = Sha256::digest(&download.bytes);
        let path = paths.target(target);
        VitLockFile {
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
