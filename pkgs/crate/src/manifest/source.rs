use crate::prelude::*;

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VdmManifestSource {
    Files(VdmManifestSourceFiles),
    File(VdmManifestSourceFile),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VdmManifestSourceFile {
    Version(VdmManifestSourceVersion),
    Config(VdmManifestSourceFileConfig),
}

impl VdmManifestSourceFile {
    pub fn version(&self) -> &VdmManifestSourceVersion {
        match self {
            Self::Version(version) => version,
            Self::Config(config) => &config.version,
        }
    }

    pub(super) fn set_version(&mut self, version: &VdmManifestSourceVersion) {
        match self {
            Self::Version(current) => *current = version.clone(),
            Self::Config(config) => config.version = version.clone(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VdmManifestSourceFiles {
    version: VdmManifestSourceVersion,
    files: Vec<VdmManifestSourceMapFile>,
}

impl VdmManifestSourceFiles {
    pub fn iter_versions(
        &self,
    ) -> impl Iterator<Item = (&VdmManifestTargetPath, &VdmManifestSourceVersion)> {
        self.files.iter().map(move |file| match file {
            VdmManifestSourceMapFile::Path(path) => (path, &self.version),
            VdmManifestSourceMapFile::Config(config) => (&config.path, &config.common.version),
        })
    }

    pub(super) fn set_version(
        &mut self,
        base: &VdmManifestTargetUrl,
        target: &VdmManifestTargetUrl,
        version: &VdmManifestSourceVersion,
    ) -> Result<bool> {
        for file in &mut self.files {
            let path = match file {
                VdmManifestSourceMapFile::Path(path) => path,
                VdmManifestSourceMapFile::Config(config) => &config.path,
            };
            if &base.join(path)? != target {
                continue;
            }
            match file {
                VdmManifestSourceMapFile::Path(path) => {
                    *file = VdmManifestSourceMapFile::Config(VdmManifestSourceMapFileConfig {
                        path: path.clone(),
                        common: VdmManifestSourceFileConfig {
                            version: version.clone(),
                        },
                    });
                }
                VdmManifestSourceMapFile::Config(config) => {
                    config.common.version = version.clone();
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VdmManifestSourceMapFile {
    Path(VdmManifestTargetPath),
    Config(VdmManifestSourceMapFileConfig),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VdmManifestSourceMapFileConfig {
    path: VdmManifestTargetPath,
    #[serde(flatten)]
    common: VdmManifestSourceFileConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VdmManifestSourceFileConfig {
    version: VdmManifestSourceVersion,
    // NOTE: We will add more fields here allowing more granular control over the source definition
}

/// Version of the target, e.g., "main", "v2" or "cc99017271f65dc5e223344c6963df8c3fc429b8".
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct VdmManifestSourceVersion(String);

impl VdmManifestSourceVersion {
    pub fn new(version: impl AsRef<str>) -> Self {
        Self(version.as_ref().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VdmManifestSourceVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
