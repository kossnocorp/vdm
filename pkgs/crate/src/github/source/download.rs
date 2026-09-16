use crate::prelude::*;

impl VdmGithubSource {
    pub(super) async fn download_file(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        let target = target
            .as_any()
            .downcast_ref::<VdmGitHubTarget>()
            .context("GitHub source received a target from another source")?
            .clone();
        VdmGitHubCache::try_new()?
            .fetch(target.resolve_version().await?)
            .await
    }
}
