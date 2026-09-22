use crate::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct VdmGitTarget {
    key: VdmManifestTargetUrl,
    repository: GitRepository,
    path: VdmManifestTargetPath,
    version: VdmManifestSourceVersion,
    source_url: String,
}

#[derive(Clone, Debug, PartialEq)]
enum GitRepository {
    GitHub { owner: String, repo: String },
    Url(String),
}

impl VdmTarget for VdmGitTarget {
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
        self.vendor_root().join(self.path.as_str())
    }

    fn source(&self) -> &'static dyn VdmSource {
        match self.repository {
            GitRepository::GitHub { .. } => &VDM_GITHUB_SOURCE,
            GitRepository::Url(_) => &VDM_GIT_SOURCE,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl VdmGitTarget {
    pub(crate) fn new_github(
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
            repository: GitRepository::GitHub { owner, repo },
            path,
            version,
            source_url,
        }
    }

    pub(crate) fn new_git(
        url: String,
        path: VdmManifestTargetPath,
        version: VdmManifestSourceVersion,
    ) -> Self {
        Self {
            key: VdmManifestTargetUrl::new(format!("git:{url}//{path}")),
            source_url: format!("git:{url}//{path}@{version}"),
            repository: GitRepository::Url(url),
            path,
            version,
        }
    }

    pub(crate) fn repository_url(&self) -> String {
        match &self.repository {
            GitRepository::GitHub { owner, repo } => {
                format!("https://github.com/{owner}/{repo}.git")
            }
            GitRepository::Url(url) => url.clone(),
        }
    }

    pub(crate) fn cache_path(&self) -> PathBuf {
        match &self.repository {
            GitRepository::GitHub { owner, repo } => PathBuf::from("github.com")
                .join(owner)
                .join(format!("{repo}.git")),
            GitRepository::Url(url) => {
                PathBuf::from("remotes").join(format!("{:x}.git", Sha256::digest(url.as_bytes())))
            }
        }
    }

    pub(crate) fn vendor_root(&self) -> PathBuf {
        match &self.repository {
            GitRepository::GitHub { owner, repo } => PathBuf::from(format!("@{owner}")).join(repo),
            GitRepository::Url(url) => {
                PathBuf::from("@git").join(format!("{:x}", Sha256::digest(url.as_bytes())))
            }
        }
    }

    pub(crate) fn path(&self) -> &str {
        self.path.as_str()
    }

    pub(crate) fn glob(&self) -> Result<Option<globset::GlobMatcher>> {
        if !self.path().contains(['*', '?', '[', '{']) {
            return Ok(None);
        }

        let pattern = if self.path().ends_with('/') {
            format!("{}**", self.path())
        } else {
            self.path().to_owned()
        };
        Ok(Some(
            globset::GlobBuilder::new(&pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .with_context(|| format!("Invalid Git glob {:?}", self.path()))?
                .compile_matcher(),
        ))
    }

    pub(crate) fn matches_member(&self, file: &Self) -> Result<bool> {
        if self.repository != file.repository {
            return Ok(false);
        }
        Ok(if let Some(matcher) = self.glob()? {
            matcher.is_match(file.path())
        } else {
            file.path()
                .strip_prefix(self.path().trim_end_matches('/'))
                .is_some_and(|suffix| suffix.starts_with('/'))
        })
    }

    pub(crate) async fn resolve_version(self) -> Result<Self> {
        if self.version.as_str() != "HEAD" {
            return Ok(self);
        }
        tokio::task::spawn_blocking(move || {
            let url = self.repository_url();
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
            "Resolved Git path {} is outside the repository",
            path.display()
        );
        let path = path.to_string_lossy().into_owned();
        Ok(self.rebuild(VdmManifestTargetPath::new(path), self.version.clone()))
    }

    pub(crate) fn with_version(&self, version: &str) -> Self {
        self.rebuild(self.path.clone(), VdmManifestSourceVersion::new(version))
    }

    fn rebuild(&self, path: VdmManifestTargetPath, version: VdmManifestSourceVersion) -> Self {
        match &self.repository {
            GitRepository::GitHub { owner, repo } => {
                Self::new_github(owner.clone(), repo.clone(), path, version)
            }
            GitRepository::Url(url) => Self::new_git(url.clone(), path, version),
        }
    }
}

fn default_branch(url: &str) -> Result<String> {
    let output = git_remote_refs(url, true)?;
    output
        .lines()
        .find_map(|line| {
            line.strip_prefix("ref: refs/heads/")?
                .strip_suffix("\tHEAD")
                .map(str::to_owned)
        })
        .context("Git remote did not advertise a default branch")
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
