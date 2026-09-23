use crate::prelude::*;

pub struct VdmPaths {
    pub root: PathBuf,
    pub manifest: PathBuf,
    pub lock: PathBuf,
}

impl VdmPaths {
    pub async fn resolve(path: Option<&Path>) -> Result<Self> {
        let manifest = Self::resolve_manifest_path(path)
            .await
            .with_context(|| "Failed to resolve Vdm paths")?;

        Ok(Self::from_manifest(manifest))
    }

    fn from_manifest(manifest: PathBuf) -> Self {
        let root = manifest
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_owned();

        Self {
            lock: root.join("vendor.lock.toml"),
            root,
            manifest,
        }
    }

    async fn resolve_manifest_path(path: Option<&Path>) -> Result<PathBuf> {
        let path_buf = match path {
            None => {
                let current_dir = std::env::current_dir()
                    .with_context(|| "Failed to resolve current directory")?;
                return Self::discover_manifest_path(&current_dir).await;
            }

            Some(path) => {
                match tokio::fs::metadata(path).await {
                    Ok(metadata) if metadata.is_dir() => return Ok(path.join("vendor.toml")),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("Failed to inspect {}", path.display()));
                    }
                }
                ensure!(
                    path.file_name().is_some_and(|name| name == "vendor.toml"),
                    "Manifest file name must be vendor.toml, got {path:?}",
                );
                path.to_owned()
            }
        };
        Ok(path_buf)
    }

    async fn discover_manifest_path(start: &Path) -> Result<PathBuf> {
        for directory in start.ancestors() {
            let candidate = directory.join("vendor.toml");
            match tokio::fs::metadata(&candidate).await {
                Ok(metadata) if metadata.is_file() => return Ok(candidate),
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to inspect {}", candidate.display()));
                }
            }
        }

        Ok(start.join("vendor.toml"))
    }

    pub fn target(&self, target: &dyn VdmTarget) -> PathBuf {
        self.root.join("vendor").join(target.vendor_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn discovers_manifest_in_nearest_ancestor() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("parent");
        let nested = parent.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(directory.path().join("vendor.toml"), "").unwrap();
        fs::write(parent.join("vendor.toml"), "").unwrap();

        let manifest = VdmPaths::discover_manifest_path(&nested).await.unwrap();
        let paths = VdmPaths::from_manifest(manifest);
        let target = VdmSourceInput::parse_target("gh:js-fns/js-fns:src/file.ts@main").unwrap();

        assert_eq!(paths.manifest, parent.join("vendor.toml"));
        assert_eq!(paths.root, parent);
        assert_eq!(paths.lock, paths.root.join("vendor.lock.toml"));
        assert_eq!(
            paths.target(target.as_ref()),
            paths.root.join("vendor/@gh/js-fns/js-fns/src/file.ts")
        );
    }

    #[tokio::test]
    async fn falls_back_to_manifest_in_starting_directory() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested");
        fs::create_dir(&nested).unwrap();

        let manifest = VdmPaths::discover_manifest_path(&nested).await.unwrap();

        assert_eq!(manifest, nested.join("vendor.toml"));
    }

    #[tokio::test]
    async fn explicit_manifest_directory_bypasses_discovery() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(directory.path().join("vendor.toml"), "").unwrap();

        let paths = VdmPaths::resolve(Some(&nested)).await.unwrap();

        assert_eq!(paths.manifest, nested.join("vendor.toml"));
        assert_eq!(paths.root, nested);
        assert_eq!(paths.lock, paths.root.join("vendor.lock.toml"));
    }
}
