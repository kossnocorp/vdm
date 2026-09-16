use crate::prelude::*;

mod cache;
pub use cache::*;

pub static VDM_SOURCE_GITHUB: VdmSourceGitHub = VdmSourceGitHub;

pub struct VdmSourceGitHub;

#[derive(Clone, Debug, PartialEq)]
pub struct VdmSourceGitHubTarget {
    key: VdmManifestTargetUrl,
    owner: String,
    repo: String,
    path: VdmManifestTargetPath,
    version: VdmManifestSourceVersion,
    source_url: String,
}

#[async_trait]
impl VdmSource for VdmSourceGitHub {
    fn parse(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        let Some(input) = input.strip_prefix("gh:") else {
            return Ok(None);
        };
        let (source, version) = input.rsplit_once('@').with_context(|| {
            format!("Invalid GitHub target {input:?}; expected gh:owner/repo/path@version")
        })?;
        ensure!(!version.is_empty(), "GitHub version must not be empty");

        let mut parts = source.splitn(3, '/');
        let owner = parts.next().unwrap_or_default();
        let repo = parts.next().unwrap_or_default();
        let path = parts.next().unwrap_or_default();

        ensure!(!owner.is_empty(), "Repository owner must not be empty");
        ensure!(!repo.is_empty(), "Repository name must not be empty");
        ensure!(
            [owner, repo].iter().all(|part| matches!(
                Path::new(part).components().next(),
                Some(Component::Normal(_))
            )),
            "Repository owner and name must be safe path components"
        );
        ensure!(!path.is_empty(), "Repository file path must not be empty");
        ensure!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "Repository file path must not contain absolute or traversal components"
        );

        let valid_ref = version.len() == 40 && version.bytes().all(|byte| byte.is_ascii_hexdigit())
            || git2::Reference::is_valid_name(&format!("refs/heads/{version}"));
        ensure!(valid_ref, "Version must be a valid Git ref or commit SHA");

        Ok(Some(Box::new(VdmSourceGitHubTarget {
            key: VdmManifestTargetUrl::new(format!("gh:{source}")),
            owner: owner.to_owned(),
            repo: repo.to_owned(),
            path: VdmManifestTargetPath::new(path),
            version: VdmManifestSourceVersion::new(version),
            source_url: format!("https://github.com/{owner}/{repo}/blob/{version}/{path}"),
        })))
    }

    async fn download(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        let target = target
            .as_any()
            .downcast_ref::<VdmSourceGitHubTarget>()
            .context("GitHub source received a target from another source")?
            .clone();
        VdmGitHubCache::try_new()?.fetch(target).await
    }
}

impl VdmTarget for VdmSourceGitHubTarget {
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
        &VDM_SOURCE_GITHUB
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl VdmSourceGitHubTarget {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixed_github_target() {
        let target = VDM_SOURCE_GITHUB
            .parse("gh:js-fns/js-fns/vitest.config.ts@main")
            .unwrap()
            .unwrap();
        assert_eq!(
            target.key(),
            &VdmManifestTargetUrl::new("gh:js-fns/js-fns/vitest.config.ts")
        );
        assert_eq!(target.version(), &VdmManifestSourceVersion::new("main"));
        assert_eq!(
            target.vendor_path(),
            Path::new("@js-fns/js-fns/vitest.config.ts")
        );
        assert!(
            VDM_SOURCE_GITHUB
                .parse("js-fns/js-fns/file@main")
                .unwrap()
                .is_none()
        );
        assert!(
            VDM_SOURCE_GITHUB
                .parse("gh:js-fns/js-fns/../secret@main")
                .is_err()
        );
    }
}
