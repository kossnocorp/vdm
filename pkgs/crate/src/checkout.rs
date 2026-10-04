use crate::prelude::*;

/// A local snapshot of a remote source. Keep this handle alive while reading
/// `path()`. Files are removed on drop; the shared Git cache is retained.
pub struct VdmCheckout {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl VdmCheckout {
    /// Download an HTTP file or check out a Git/GitHub file, directory, or glob.
    /// Uses the same source syntax and Git cache as the CLI, without creating a
    /// vendor directory, manifest, or lockfile in the caller's project.
    /// Requires a Tokio runtime with IO and time enabled.
    pub async fn fetch(input: &str) -> Result<Self> {
        let target = VdmSourceInput::parse_target(input)?;
        let dirs = ProjectDirs::from("fyi", "vdm", "vdm")
            .context("Failed to locate the local cache directory")?;
        let root = dirs.cache_dir().join("checkouts");
        tokio::fs::create_dir_all(&root).await?;
        let directory = tempfile::Builder::new()
            .prefix("checkout-")
            .tempdir_in(root)?;
        let path;
        if let Some(git) = target.as_any().downcast_ref::<VdmGitTarget>() {
            let git = git.clone().resolve_version().await?;
            let files = VdmGitCache::try_new()?.fetch_files(git.clone()).await?;
            let single =
                git.glob()?.is_none() && files.len() == 1 && files[0].0.path() == git.path();
            for (file_target, file) in files {
                let relative = Path::new(file_target.path());
                ensure!(
                    relative
                        .components()
                        .all(|c| matches!(c, Component::Normal(_))),
                    "Invalid checkout path: {}",
                    relative.display()
                );
                file.write(&directory.path().join(relative)).await?;
            }
            path = if git.glob()?.is_some() {
                directory.path().to_path_buf()
            } else {
                directory.path().join(git.path())
            };
            ensure!(
                if single {
                    path.is_file()
                } else {
                    path.is_dir()
                },
                "Invalid checkout selection"
            );
        } else {
            let file = target.source().download(target.as_ref()).await?;
            let vendor_path = target.vendor_path();
            let name = vendor_path
                .file_name()
                .context("HTTP source has no filename")?;
            path = directory.path().join(name);
            file.write(&path).await?;
        }
        Ok(Self {
            _directory: directory,
            path,
        })
    }

    /// The selected file or directory. Treat its contents as read-only.
    pub fn path(&self) -> &Path {
        &self.path
    }
}
