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
pub struct VdmGitHubCache {
    root: PathBuf,
}

impl VdmGitHubCache {
    pub fn try_new() -> Result<Self> {
        let dirs = ProjectDirs::from("fyi", "vdm", "vdm")
            .context("Failed to locate the local data directory")?;
        Ok(Self {
            root: dirs.data_local_dir().join("git/db/github.com"),
        })
    }

    pub async fn fetch(&self, target: VdmGitHubTarget) -> Result<VdmSourceFile> {
        let mut files = self.fetch_files(target).await?;
        ensure!(files.len() == 1, "Expected a single GitHub file");
        Ok(files.remove(0).1)
    }

    pub(crate) async fn fetch_files(
        &self,
        target: VdmGitHubTarget,
    ) -> Result<Vec<(VdmGitHubTarget, VdmSourceFile)>> {
        let url = format!(
            "https://github.com/{}/{}.git",
            target.owner(),
            target.repo()
        );
        self.fetch_url(target, url).await
    }

    pub(crate) fn repository(&self, target: &VdmGitHubTarget) -> PathBuf {
        self.root
            .join(target.owner())
            .join(format!("{}.git", target.repo()))
    }

    pub(crate) async fn fetch_revision(
        &self,
        target: &VdmGitHubTarget,
        revision: &str,
    ) -> Result<VdmSourceFile> {
        self.fetch(target.with_version(revision)).await
    }

    async fn fetch_url(
        &self,
        target: VdmGitHubTarget,
        url: String,
    ) -> Result<Vec<(VdmGitHubTarget, VdmSourceFile)>> {
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
        target: &VdmGitHubTarget,
        url: &str,
    ) -> Result<Vec<(VdmGitHubTarget, VdmSourceFile)>> {
        let owner_dir = self.root.join(target.owner());
        fs::create_dir_all(&owner_dir)
            .with_context(|| format!("Failed to create {}", owner_dir.display()))?;

        let lock_path = owner_dir.join(format!("{}.lock", target.repo()));
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

        let cached_oid = Oid::from_str(target.version().as_str())
            .ok()
            .filter(|oid| repo.find_commit(*oid).is_ok());
        let commit = if let Some(oid) = cached_oid {
            repo.find_commit(oid)?
        } else {
            let source = resolve_remote_ref(&repo, target.version())?;
            let refspec = format!("+{source}:refs/vdm/fetch");
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
            .with_context(|| format!("Failed to fetch {} from {url}", target.version()))?;

            repo.revparse_single("refs/vdm/fetch")?
                .peel_to_commit()
                .with_context(|| format!("{} does not resolve to a commit", target.version()))?
        };
        repo.reference(
            &format!("refs/vdm/revisions/{}", commit.id()),
            commit.id(),
            true,
            "retain revision for vdm cache",
        )?;
        let tree = commit.tree()?;
        let mut paths = Vec::new();
        if let Some(matcher) = target.glob()? {
            tree.walk(git2::TreeWalkMode::PreOrder, |directory, entry| {
                if entry.kind() == Some(git2::ObjectType::Blob)
                    && let Some(name) = entry.name()
                {
                    let path = format!("{directory}{name}");
                    if matcher.is_match(&path) {
                        paths.push(path);
                    }
                }
                git2::TreeWalkResult::Ok
            })?;
            ensure!(
                !paths.is_empty(),
                "GitHub glob {} matched no files at commit {}",
                target.path(),
                commit.id()
            );
            paths.sort();
        } else {
            paths.push(target.path().to_owned());
        }
        // Partial clones initially contain only trees. Fetch missing blobs in
        // batches rather than paying for a network round trip for every match.
        let mut missing = BTreeSet::new();
        for path in &paths {
            let entry = tree
                .get_path(Path::new(path))
                .with_context(|| format!("{path} is not present at commit {}", commit.id()))?;
            ensure!(
                entry.kind() == Some(git2::ObjectType::Blob),
                "{path} is not a file at commit {}",
                commit.id()
            );
            if repo.find_blob(entry.id()).is_err() {
                missing.insert(entry.id().to_string());
            }
        }
        let missing = missing.into_iter().collect::<Vec<_>>();
        for batch in missing.chunks(128) {
            let mut args = vec![
                "-c",
                "fetch.negotiationAlgorithm=noop",
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--recurse-submodules=no",
                "--filter=blob:none",
                "origin",
            ];
            args.extend(batch.iter().map(String::as_str));
            git(&repo_path, &args).context("Failed to fetch GitHub file contents")?;
        }
        paths
            .into_iter()
            .map(|path| {
                let target = target.with_path(Path::new(&path))?;
                let entry = tree.get_path(Path::new(target.path())).with_context(|| {
                    format!("{} is not present at commit {}", target.path(), commit.id())
                })?;
                let blob_id = entry.id();
                let blob = repo.find_blob(blob_id).with_context(|| {
                    format!("{} is not a file at commit {}", target.path(), commit.id())
                })?;

                Ok((
                    target,
                    VdmSourceFile {
                        revision: commit.id().to_string(),
                        bytes: blob.content().to_vec(),
                    },
                ))
            })
            .collect()
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

fn resolve_remote_ref(repo: &Repository, version: &VdmManifestSourceVersion) -> Result<String> {
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
        fs::create_dir_all(source_path.join("nested/deep")).unwrap();
        for path in ["root.sh", "nested/a.sh", "nested/deep/b.sh"] {
            fs::write(source_path.join(path), path).unwrap();
        }
        let mut index = source.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = source.find_tree(tree_id).unwrap();
        let signature = Signature::now("Vdm Test", "vdm@example.com").unwrap();
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

        let cache = VdmGitHubCache {
            root: temp.path().join("cache"),
        };
        let parsed = VDM_GITHUB_SOURCE
            .parse("gh:owner/repo/file.txt@main")
            .unwrap()
            .unwrap();
        let target = parsed.as_any().downcast_ref::<VdmGitHubTarget>().unwrap();
        let download = cache
            .fetch_url(target.clone(), format!("file://{}", source_path.display()))
            .await
            .unwrap();

        assert_eq!(download[0].1.bytes, b"first\n");
        assert!(cache.root.join("owner/repo.git").is_dir());
        assert!(!cache.root.join("owner/repo.git/file.txt").exists());

        let glob = target.with_path(Path::new("**/*.sh")).unwrap();
        let matches = cache
            .fetch_url(glob.clone(), format!("file://{}", source_path.display()))
            .await
            .unwrap();
        assert_eq!(
            matches
                .iter()
                .map(|(target, _)| target.path())
                .collect::<Vec<_>>(),
            ["nested/a.sh", "nested/deep/b.sh", "root.sh"]
        );
        for (target, file) in &matches {
            assert_eq!(file.bytes, target.path().as_bytes());
            assert_eq!(file.revision, download[0].1.revision);
        }
        let shallow = cache
            .fetch_url(
                target.with_path(Path::new("*.sh")).unwrap(),
                format!("file://{}", source_path.display()),
            )
            .await
            .unwrap();
        assert_eq!(shallow.len(), 1);
        assert_eq!(shallow[0].0.path(), "root.sh");
        assert!(
            cache
                .fetch_url(
                    target.with_path(Path::new("**/*.missing")).unwrap(),
                    format!("file://{}", source_path.display())
                )
                .await
                .is_err()
        );

        let paths = VdmPaths::resolve(Some(temp.path())).await.unwrap();
        let mut manifest = VdmManifest::new();
        manifest.add(glob.key(), glob.version());
        manifest.write_toml(&paths.manifest).await.unwrap();
        let mut lock = VdmLock::default();
        let mut members = Vec::new();
        for (target, download) in matches {
            download.write(&paths.target(&target)).await.unwrap();
            members.push(target.key().clone());
            lock.files.insert(
                target.key().clone(),
                VdmLockFile::new(&target, &download, &paths, true, Vec::new()),
            );
        }
        lock.globs.insert(glob.key().clone(), members);
        lock.write_toml(&paths.lock).await.unwrap();
        let serialized = fs::read_to_string(&paths.lock).unwrap();
        let document: toml::Value = toml::from_str(&serialized).unwrap();
        assert_eq!(
            document["files"][glob.key().as_str()]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(document["files"].as_table().unwrap().len(), 1);
        let restored = VdmLock::read_toml(&paths.lock).await.unwrap();
        assert_eq!(restored.files, lock.files);
        assert_eq!(restored.globs, lock.globs);
        VdmVendor::install(Some(temp.path()), true).await.unwrap();
        assert!(
            temp.path()
                .join("vendor/@owner/repo/nested/deep/b.sh")
                .is_file()
        );
        VdmManifest::new()
            .write_toml(&paths.manifest)
            .await
            .unwrap();
        VdmVendor::install(Some(temp.path()), true).await.unwrap();
        assert!(!temp.path().join("vendor/@owner").exists());
    }
}
