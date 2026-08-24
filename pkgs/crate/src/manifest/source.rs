use crate::prelude::*;

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VitManifestSource {
    Files(VitManifestSourceFiles),
    File(VitManifestSourceFile),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum VitManifestSourceFile {
    Version(VitManifestSourceVersion),
    Config(VitManifestSourceFileConfig),
}

impl VitManifestSourceFile {
    pub fn version(&self) -> &VitManifestSourceVersion {
        match self {
            Self::Version(version) => version,
            Self::Config(config) => &config.version,
        }
    }

    pub(super) fn set_version(&mut self, version: &VitManifestSourceVersion) {
        match self {
            Self::Version(current) => *current = version.clone(),
            Self::Config(config) => config.version = version.clone(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VitManifestSourceFiles {
    version: VitManifestSourceVersion,
    files: Vec<VitManifestSourceMapFile>,
}

impl VitManifestSourceFiles {
    pub fn iter_versions(
        &self,
    ) -> impl Iterator<Item = (&VitManifestTargetPath, &VitManifestSourceVersion)> {
        self.files.iter().map(move |file| match file {
            VitManifestSourceMapFile::Path(path) => (path, &self.version),
            VitManifestSourceMapFile::Config(config) => (&config.path, &config.common.version),
        })
    }

    pub(super) fn set_version(
        &mut self,
        base: &VitManifestTargetUrl,
        target: &VitManifestTargetUrl,
        version: &VitManifestSourceVersion,
    ) -> Result<bool> {
        for file in &mut self.files {
            let path = match file {
                VitManifestSourceMapFile::Path(path) => path,
                VitManifestSourceMapFile::Config(config) => &config.path,
            };
            if &base.join(path)? != target {
                continue;
            }
            match file {
                VitManifestSourceMapFile::Path(path) => {
                    *file = VitManifestSourceMapFile::Config(VitManifestSourceMapFileConfig {
                        path: path.clone(),
                        common: VitManifestSourceFileConfig {
                            version: version.clone(),
                        },
                    });
                }
                VitManifestSourceMapFile::Config(config) => {
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
pub enum VitManifestSourceMapFile {
    Path(VitManifestTargetPath),
    Config(VitManifestSourceMapFileConfig),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VitManifestSourceMapFileConfig {
    path: VitManifestTargetPath,
    #[serde(flatten)]
    common: VitManifestSourceFileConfig,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VitManifestSourceFileConfig {
    version: VitManifestSourceVersion,
    // NOTE: We will add more fields here allowing more granular control over the source definition
}

/// Version of the target, e.g., "main", "v2" or "cc99017271f65dc5e223344c6963df8c3fc429b8".
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(transparent)]
pub struct VitManifestSourceVersion(String);

impl VitManifestSourceVersion {
    pub fn new(version: impl AsRef<str>) -> Self {
        Self(version.as_ref().to_owned())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for VitManifestSourceVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}
