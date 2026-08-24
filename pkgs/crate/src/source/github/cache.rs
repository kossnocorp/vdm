use crate::prelude::*;

use fs2::FileExt;
use git2::{Oid, Repository};
// This module runs as one repository transaction inside spawn_blocking. Keeping
// git2, fs2 locking, and Git subprocesses together preserves cache consistency.
use std::fs::{self, OpenOptions};
use std::process::Command;
use tokio::sync::Semaphore;

const MAX_CONCURRENT_FETCHES: usize = 4;
static FETCH_PERMITS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_FETCHES);

#[derive(Clone)]
pub struct VitGitHubCache {
    root: PathBuf,
}

impl VitGitHubCache {
    pub fn try_new() -> Result<Self> {
        let dirs = ProjectDirs::from("org", "vendorit", "vit")
            .context("Failed to locate the local data directory")?;
        Ok(Self {
            root: dirs.data_local_dir().join("git/db/github.com"),
        })
    }

    pub async fn fetch(&self, target: VitSourceGitHubTarget) -> Result<VitSourceFile> {
        let url = format!("https://github.com/{}/{}.git", target.owner, target.repo);
        self.fetch_url(target, url).await
    }

    pub(crate) fn repository(&self, target: &VitSourceGitHubTarget) -> PathBuf {
        self.root
            .join(&target.owner)
            .join(format!("{}.git", target.repo))
    }

    pub(crate) async fn fetch_revision(
        &self,
        target: &VitSourceGitHubTarget,
        revision: &str,
    ) -> Result<VitSourceFile> {
        self.fetch(target.with_version(revision)).await
    }

    async fn fetch_url(&self, target: VitSourceGitHubTarget, url: String) -> Result<VitSourceFile> {
        let _permit = FETCH_PERMITS
            .acquire()
            .await
            .context("GitHub fetch concurrency limiter closed")?;
        let cache = self.clone();
        tokio::task::spawn_blocking(move || cache.fetch_url_blocking(&target, &url))
            .await
            .context("Git cache task failed")?
    }

    fn fetch_url_blocking(
        &self,
        target: &VitSourceGitHubTarget,
        url: &str,
    ) -> Result<VitSourceFile> {
        let owner_dir = self.root.join(&target.owner);
        fs::create_dir_all(&owner_dir)
            .with_context(|| format!("Failed to create {}", owner_dir.display()))?;

        let lock_path = owner_dir.join(format!("{}.lock", target.repo));
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("Failed to open {}", lock_path.display()))?;
        lock.lock_exclusive()
            .with_context(|| format!("Failed to lock {}", lock_path.display()))?;

        let repo_path = self.repository(target);
        let repo = if repo_path.exists() {
            Repository::open_bare(&repo_path)
                .with_context(|| format!("Failed to open Git cache {}", repo_path.display()))?
        } else {
            Repository::init_bare(&repo_path).with_context(|| {
                format!("Failed to initialize Git cache {}", repo_path.display())
            })?
        };
        configure_origin(&repo, url)?;

        let cached_oid = Oid::from_str(target.version.as_str())
            .ok()
            .filter(|oid| repo.find_commit(*oid).is_ok());
        let commit = if let Some(oid) = cached_oid {
            repo.find_commit(oid)?
        } else {
            let source = resolve_remote_ref(&repo, &target.version)?;
            let refspec = format!("+{source}:refs/vit/fetch");
            git(
                &repo_path,
                &[
                    "fetch",
                    "--depth=1",
                    "--filter=blob:none",
                    "--no-tags",
                    "origin",
                    &refspec,
                ],
            )
            .with_context(|| format!("Failed to fetch {} from {url}", target.version))?;

            repo.revparse_single("refs/vit/fetch")?
                .peel_to_commit()
                .with_context(|| format!("{} does not resolve to a commit", target.version))?
        };
        repo.reference(
            &format!("refs/vit/revisions/{}", commit.id()),
            commit.id(),
            true,
            "retain revision for vendorit cache",
        )?;
        let entry = commit
            .tree()?
            .get_path(Path::new(target.path.as_str()))
            .with_context(|| format!("{} is not present at commit {}", target.path, commit.id()))?;
        let blob_id = entry.id();
        if repo.find_blob(blob_id).is_err() {
            git(&repo_path, &["cat-file", "-e", &blob_id.to_string()])
                .with_context(|| format!("Failed to fetch contents of {}", target.path))?;
        }
        let blob = repo
            .find_blob(blob_id)
            .with_context(|| format!("{} is not a file at commit {}", target.path, commit.id()))?;

        Ok(VitSourceFile {
            revision: commit.id().to_string(),
            bytes: blob.content().to_vec(),
        })
    }
}

fn configure_origin(repo: &Repository, url: &str) -> Result<()> {
    match repo.find_remote("origin") {
        Ok(_) => repo
            .remote_set_url("origin", url)
            .context("Failed to update Git cache origin")?,
        Err(error) if error.code() == git2::ErrorCode::NotFound => {
            repo.remote("origin", url)
                .context("Failed to configure Git cache origin")?;
        }
        Err(error) => return Err(error).context("Failed to inspect Git cache origin"),
    }
    let mut config = repo.config()?;
    config.set_bool("remote.origin.promisor", true)?;
    config.set_str("remote.origin.partialclonefilter", "blob:none")?;
    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .output()
        .context("Failed to run git; ensure it is installed and available in PATH")?;
    ensure!(
        output.status.success(),
        "Git exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn resolve_remote_ref(repo: &Repository, version: &VitManifestSourceVersion) -> Result<String> {
    if version.as_str().len() == 40 && Oid::from_str(version.as_str()).is_ok() {
        return Ok(version.as_str().to_owned());
    }

    let mut remote = repo.find_remote("origin")?;
    remote
        .connect(git2::Direction::Fetch)
        .context("Failed to connect to Git origin")?;
    let names = remote
        .list()
        .context("Failed to list Git origin refs")?
        .iter()
        .map(|head| head.name())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    remote.disconnect()?;

    let branch = format!("refs/heads/{version}");
    let tag = format!("refs/tags/{version}");
    if names.contains(&branch) {
        Ok(branch)
    } else if names.contains(&tag) {
        Ok(tag)
    } else {
        bail!("Git origin does not contain ref {version:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::Signature;

    #[tokio::test]
    async fn caches_a_bare_repo_and_reads_a_file_from_its_tree() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = Repository::init(&source_path).unwrap();
        source
            .config()
            .unwrap()
            .set_bool("uploadpack.allowFilter", true)
            .unwrap();
        fs::write(source_path.join("file.txt"), "first\n").unwrap();
        let mut index = source.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = source.find_tree(tree_id).unwrap();
        let signature = Signature::now("Vit Test", "vit@example.com").unwrap();
        source
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "first",
                &tree,
                &[],
            )
            .unwrap();

        let cache = VitGitHubCache {
            root: temp.path().join("cache"),
        };
        let parsed = VIT_SOURCE_GITHUB
            .parse("gh:owner/repo/file.txt@main")
            .unwrap()
            .unwrap();
        let target = parsed
            .as_any()
            .downcast_ref::<VitSourceGitHubTarget>()
            .unwrap();
        let download = cache
            .fetch_url(target.clone(), format!("file://{}", source_path.display()))
            .await
            .unwrap();

        assert_eq!(download.bytes, b"first\n");
        assert!(cache.root.join("owner/repo.git").is_dir());
        assert!(!cache.root.join("owner/repo.git/file.txt").exists());
    }
}
