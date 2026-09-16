use crate::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct VdmGitHubTarget {
    key: VdmManifestTargetUrl,
    owner: String,
    repo: String,
    path: VdmManifestTargetPath,
    version: VdmManifestSourceVersion,
    source_url: String,
}

impl VdmTarget for VdmGitHubTarget {
    fn key(&self) -> &VdmManifestTargetUrl {
        &self.key
    }

    fn version(&self) -> &VdmManifestSourceVersion {
        &self.version
    }

    fn source_url(&self) -> &str {
        &self.source_url
    }

    fn vendor_path(&self) -> PathBuf {
        PathBuf::from(format!("@{}", self.owner))
            .join(&self.repo)
            .join(self.path.as_str())
    }

    fn source(&self) -> &'static dyn VdmSource {
        &VDM_GITHUB_SOURCE
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl VdmGitHubTarget {
    pub(crate) fn new(
        owner: String,
        repo: String,
        path: VdmManifestTargetPath,
        version: VdmManifestSourceVersion,
    ) -> Self {
        let source = format!("{owner}/{repo}/{path}");
        let key = VdmManifestTargetUrl::new(format!("gh:{source}"));
        let source_url = format!("https://github.com/{owner}/{repo}/blob/{version}/{path}");

        Self {
            key,
            owner,
            repo,
            path,
            version,
            source_url,
        }
    }

    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }

    pub(crate) fn repo(&self) -> &str {
        &self.repo
    }

    pub(crate) fn path(&self) -> &str {
        self.path.as_str()
    }

    pub(crate) fn glob(&self) -> Result<Option<globset::GlobMatcher>> {
        if !self.path().contains(['*', '?', '[', '{']) {
            return Ok(None);
        }

        Ok(Some(
            globset::GlobBuilder::new(self.path())
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .with_context(|| format!("Invalid GitHub glob {:?}", self.path()))?
                .compile_matcher(),
        ))
    }

    pub(crate) async fn resolve_version(self) -> Result<Self> {
        if self.version.as_str() != "HEAD" {
            return Ok(self);
        }
        tokio::task::spawn_blocking(move || {
            let url = format!("https://github.com/{}/{}.git", self.owner, self.repo);
            Ok(self.with_version(&default_branch(&url)?))
        })
        .await
        .context("Default branch resolution task failed")?
    }

    pub(crate) fn repository_path(&self) -> &str {
        self.path.as_str()
    }

    pub(crate) fn with_path(&self, path: &Path) -> Result<Self> {
        ensure!(
            path.components()
                .all(|part| matches!(part, Component::Normal(_))),
            "Resolved GitHub path {} is outside the repository",
            path.display()
        );
        let path = path.to_string_lossy().into_owned();
        Ok(Self {
            key: VdmManifestTargetUrl::new(format!("gh:{}/{}/{path}", self.owner, self.repo)),
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            path: VdmManifestTargetPath::new(&path),
            version: self.version.clone(),
            source_url: format!(
                "https://github.com/{}/{}/blob/{}/{path}",
                self.owner, self.repo, self.version
            ),
        })
    }

    pub(crate) fn with_version(&self, version: &str) -> Self {
        let mut target = self.clone();
        target.version = VdmManifestSourceVersion::new(version);
        target.source_url = format!(
            "https://github.com/{}/{}/blob/{}/{}",
            target.owner, target.repo, version, target.path
        );
        target
    }
}

fn default_branch(url: &str) -> Result<String> {
    let mut remote = git2::Remote::create_detached(url)?;
    remote
        .connect(git2::Direction::Fetch)
        .with_context(|| format!("Failed to connect to {url}"))?;
    let branch = remote.default_branch()?;
    Ok(branch
        .as_str()
        .context("Default branch is not UTF-8")?
        .strip_prefix("refs/heads/")
        .context("Invalid default branch ref")?
        .to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_remote_default_branch() {
        let temp = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(temp.path()).unwrap();
        let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("Test", "test@example.com").unwrap();
        for branch in ["main", "master", "custom/default"] {
            let reference = format!("refs/heads/{branch}");
            repo.commit(
                Some(&reference),
                &signature,
                &signature,
                "initial",
                &tree,
                &[],
            )
            .unwrap();
            repo.set_head(&reference).unwrap();
            assert_eq!(
                default_branch(&format!("file://{}", temp.path().display())).unwrap(),
                branch
            );
        }
        let target = VDM_GITHUB_SOURCE
            .parse("gh:owner/repo/file")
            .unwrap()
            .unwrap();
        assert_eq!(target.version().as_str(), "HEAD");
        assert!(VDM_GITHUB_SOURCE.parse("gh:owner/repo/file@").is_err());
    }
}
