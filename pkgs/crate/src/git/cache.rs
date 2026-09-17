use crate::prelude::*;

use fs2::FileExt;
use git2::{Oid, Repository};
use std::fs::{self, OpenOptions};
use std::process::Command;
use tokio::sync::Semaphore;

const MAX_CONCURRENT_FETCHES: usize = 4;
static FETCH_PERMITS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_FETCHES);

#[derive(Clone)]
pub struct VdmGitCache {
    root: PathBuf,
}

impl VdmGitCache {
    pub fn try_new() -> Result<Self> {
        let dirs = ProjectDirs::from("fyi", "vdm", "vdm")
            .context("Failed to locate the local data directory")?;
        Ok(Self {
            root: dirs.data_local_dir().join("git/db"),
        })
    }

    pub async fn fetch(&self, target: VdmGitTarget) -> Result<VdmSourceFile> {
        let mut files = self.fetch_files(target).await?;
        ensure!(files.len() == 1, "Expected a single Git file");
        Ok(files.remove(0).1)
    }

    pub(crate) async fn fetch_files(
        &self,
        target: VdmGitTarget,
    ) -> Result<Vec<(VdmGitTarget, VdmSourceFile)>> {
        let url = target.repository_url();
        self.fetch_url(target, url).await
    }

    pub(crate) fn repository(&self, target: &VdmGitTarget) -> PathBuf {
        self.root.join(target.cache_path())
    }

    pub(crate) async fn fetch_revision(
        &self,
        target: &VdmGitTarget,
        revision: &str,
    ) -> Result<VdmSourceFile> {
        self.fetch(target.with_version(revision)).await
    }

    async fn fetch_url(
        &self,
        target: VdmGitTarget,
        url: String,
    ) -> Result<Vec<(VdmGitTarget, VdmSourceFile)>> {
        let _permit = FETCH_PERMITS
            .acquire()
            .await
            .context("Git fetch concurrency limiter closed")?;
        let cache = self.clone();
        tokio::task::spawn_blocking(move || cache.fetch_url_blocking(&target, &url))
            .await
            .context("Git cache task failed")?
    }

    fn fetch_url_blocking(
        &self,
        target: &VdmGitTarget,
        url: &str,
    ) -> Result<Vec<(VdmGitTarget, VdmSourceFile)>> {
        let repo_path = self.repository(target);
        let repository_dir = repo_path.parent().context("Git cache has no parent")?;
        fs::create_dir_all(repository_dir)
            .with_context(|| format!("Failed to create {}", repository_dir.display()))?;

        let lock_path = repo_path.with_extension("lock");
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("Failed to open {}", lock_path.display()))?;
        lock.lock_exclusive()
            .with_context(|| format!("Failed to lock {}", lock_path.display()))?;

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
                "Git glob {} matched no files at commit {}",
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
            git(&repo_path, &args).context("Failed to fetch Git file contents")?;
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

    let remote = repo.find_remote("origin")?;
    let output = git_remote_refs(remote.url().context("Git origin has no URL")?, false)?;
    let names = output
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, name)| name.to_owned()))
        .collect::<Vec<_>>();

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

pub(crate) fn git_remote_refs(url: &str, symbolic: bool) -> Result<String> {
    let mut command = Command::new("git");
    command.arg("ls-remote");
    if symbolic {
        command.arg("--symref");
    }
    command.arg("--").arg(url);
    if symbolic {
        command.arg("HEAD");
    }
    let output = command.output().context("Failed to run git ls-remote")?;
    ensure!(
        output.status.success(),
        "Failed to list Git remote refs: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8(output.stdout).context("Git remote refs are not UTF-8")
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

        let cache = VdmGitCache {
            root: temp.path().join("cache"),
        };
        let parsed = VDM_GITHUB_SOURCE
            .parse("gh:owner/repo/file.txt@main")
            .unwrap()
            .unwrap();
        let target = parsed.as_any().downcast_ref::<VdmGitTarget>().unwrap();
        let download = cache
            .fetch_url(target.clone(), format!("file://{}", source_path.display()))
            .await
            .unwrap();

        assert_eq!(download[0].1.bytes, b"first\n");
        assert!(cache.repository(target).is_dir());
        assert!(!cache.repository(target).join("file.txt").exists());

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
