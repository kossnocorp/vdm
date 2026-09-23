use crate::prelude::*;

mod add;

mod install;

mod update;

mod review;

pub struct VdmVendor;

impl VdmVendor {
    fn graph_version<'a>(
        graph: &'a BTreeMap<VdmManifestTargetUrl, VdmGraphFile>,
        key: &VdmManifestTargetUrl,
    ) -> Result<&'a VdmManifestSourceVersion> {
        Ok(graph
            .get(key)
            .or_else(|| graph.values().next())
            .context("Resolved graph is empty")?
            .target
            .version())
    }

    fn record_glob(
        lock: &mut VdmLock,
        target: Option<&VdmGitTarget>,
        graph: &BTreeMap<VdmManifestTargetUrl, VdmGraphFile>,
    ) -> Result<()> {
        if let Some(target) = target
            && (target.glob()?.is_some() || !graph.contains_key(target.key()))
        {
            let members = graph
                .iter()
                .filter_map(|(key, file)| {
                    let file = file.target.as_any().downcast_ref::<VdmGitTarget>()?;
                    match target.matches_member(file) {
                        Ok(true) => Some(Ok(key.clone())),
                        Ok(false) => None,
                        Err(error) => Some(Err(error)),
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            lock.globs.insert(target.key().clone(), members);
        }
        Ok(())
    }

    async fn write_graph(
        state: &mut VdmStateLocked,
        graph: BTreeMap<VdmManifestTargetUrl, VdmGraphFile>,
        direct: &BTreeSet<VdmManifestTargetUrl>,
    ) -> Result<usize> {
        let mut written = 0;
        for (key, file) in graph {
            let destination = state.paths.target(file.target.as_ref());
            let next = VdmLockFile::new(
                file.target.as_ref(),
                &file.download,
                &state.paths,
                direct.contains(&key)
                    || state
                        .lock
                        .globs
                        .values()
                        .any(|members| members.contains(&key)),
                file.dependencies,
            );
            let changed = state.lock.files.get(&key) != Some(&next)
                || !Self::file_matches(&destination, &next.hash).await?;
            if changed {
                file.download.write(&destination).await?;
                written += 1;
            }
            state.lock.files.insert(key, next);
        }
        Ok(written)
    }

    fn reachable(
        lock: &VdmLock,
        roots: impl IntoIterator<Item = VdmManifestTargetUrl>,
    ) -> BTreeSet<VdmManifestTargetUrl> {
        let mut reachable = BTreeSet::new();
        let mut pending = roots.into_iter().collect::<Vec<_>>();
        while let Some(key) = pending.pop() {
            if !reachable.insert(key.clone()) {
                continue;
            }
            if let Some(file) = lock.files.get(&key) {
                pending.extend(file.dependencies.iter().cloned());
            }
            if let Some(members) = lock.globs.get(&key) {
                pending.extend(members.iter().cloned());
            }
        }
        reachable
    }

    async fn prune_unreachable(
        state: &mut VdmStateLocked,
        roots: impl IntoIterator<Item = VdmManifestTargetUrl>,
    ) -> Result<usize> {
        let reachable = Self::reachable(&state.lock, roots);
        state.lock.globs.retain(|key, _| reachable.contains(key));
        let stale = state
            .lock
            .files
            .keys()
            .filter(|key| !reachable.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in &stale {
            let entry = &state.lock.files[key];
            Self::remove_locked_file(
                &Self::lock_destination(&state.paths, &entry.path)?,
                &state.paths.root.join("vendor"),
            )
            .await?;
            state.lock.files.remove(key);
        }
        Ok(stale.len())
    }
}
#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use std::fs;

    #[tokio::test]
    async fn github_globs_add_restore_overlap_and_update() {
        github_group_add_restore_overlap_and_update("**/*.sh", "nested/*.sh", 2).await;
    }

    #[tokio::test]
    async fn github_folders_add_restore_overlap_and_update() {
        github_group_add_restore_overlap_and_update("nested", "nested/*.sh", 1).await;
    }

    #[tokio::test]
    async fn github_repositories_add_restore_overlap_and_update() {
        github_group_add_restore_overlap_and_update("", "nested", 3).await;
    }

    async fn github_group_add_restore_overlap_and_update(
        selection: &str,
        overlap: &str,
        count: usize,
    ) {
        // Seed the Git cache with local commits so the full vendor workflow is
        // exercised without depending on GitHub or mutating process environment.
        let cache = VdmGitCache::try_new().unwrap();
        let placeholder = VdmSourceInput::parse_target("gh:fixture/repo:**/*.sh").unwrap();
        let repository =
            cache.repository(placeholder.as_any().downcast_ref::<VdmGitTarget>().unwrap());
        let cache_root = repository.parent().unwrap().parent().unwrap();
        fs::create_dir_all(cache_root).unwrap();
        let owner_dir = tempfile::Builder::new()
            .prefix("vdm-test-")
            .tempdir_in(cache_root)
            .unwrap();
        let owner = owner_dir.path().file_name().unwrap().to_str().unwrap();
        let repo = git2::Repository::init_bare(owner_dir.path().join("repo.git")).unwrap();
        let commit = |name: &str, bytes: &[u8]| {
            let mut nested = repo.treebuilder(None).unwrap();
            nested
                .insert(name, repo.blob(bytes).unwrap(), 0o100644)
                .unwrap();
            let mut root = repo.treebuilder(None).unwrap();
            root.insert("nested", nested.write().unwrap(), 0o040000)
                .unwrap();
            root.insert("root.sh", repo.blob(b"root").unwrap(), 0o100644)
                .unwrap();
            root.insert("ignore.txt", repo.blob(b"ignored").unwrap(), 0o100644)
                .unwrap();
            let tree = repo.find_tree(root.write().unwrap()).unwrap();
            let signature = git2::Signature::now("Test", "test@example.com").unwrap();
            repo.commit(None, &signature, &signature, name, &tree, &[])
                .unwrap()
                .to_string()
        };
        let first = commit("a.sh", b"first");
        let second = commit("b.sh", b"second");
        let directory = tempfile::tempdir().unwrap();
        let paths = VdmPaths::resolve(Some(directory.path())).await.unwrap();
        let key = VdmSourceInput::parse_target(&format!("gh:{owner}/repo:{selection}"))
            .unwrap()
            .key()
            .clone();
        VdmVendor::add(Some(directory.path()), &format!("{key}@{first}"))
            .await
            .unwrap();
        let manifest = VdmManifest::read_toml(&paths.manifest).await.unwrap();
        assert_eq!(manifest.targets().unwrap()[&key].version().as_str(), first);
        let lock = VdmLock::read_toml(&paths.lock).await.unwrap();
        assert_eq!(lock.globs[&key].len(), count);
        let nested = directory
            .path()
            .join(format!("vendor/@gh/{owner}/repo/nested"));
        fs::remove_file(nested.join("a.sh")).unwrap();
        assert!(
            VdmVendor::install(Some(directory.path()), true)
                .await
                .is_err()
        );
        VdmVendor::install(Some(directory.path()), false)
            .await
            .unwrap();
        assert_eq!(fs::read(nested.join("a.sh")).unwrap(), b"first");
        assert_eq!(
            directory
                .path()
                .join(format!("vendor/@gh/{owner}/repo/ignore.txt"))
                .exists(),
            selection.is_empty()
        );

        let overlapping = VdmManifestTargetUrl::new(format!("gh:{owner}/repo:{overlap}"));
        VdmVendor::add(Some(directory.path()), &format!("{overlapping}@{first}"))
            .await
            .unwrap();
        let mut manifest = VdmManifest::new();
        manifest.add(&overlapping, &VdmManifestSourceVersion::new(&first));
        manifest.write_toml(&paths.manifest).await.unwrap();
        VdmVendor::install(Some(directory.path()), true)
            .await
            .unwrap();
        assert!(nested.join("a.sh").is_file());
        assert!(!nested.parent().unwrap().join("root.sh").exists());

        VdmVendor::update(
            Some(directory.path()),
            &format!("{overlapping}@{second}"),
            false,
        )
        .await
        .unwrap();
        assert!(!nested.join("a.sh").exists());
        assert_eq!(fs::read(nested.join("b.sh")).unwrap(), b"second");
        let lock = VdmLock::read_toml(&paths.lock).await.unwrap();
        assert_eq!(lock.globs.len(), 1);
        assert_eq!(lock.globs[&overlapping].len(), 1);
        assert_eq!(lock.files.len(), 1);
        assert_eq!(lock.files.values().next().unwrap().revision, second);
        VdmVendor::install(Some(directory.path()), true)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn http_versions_follow_content_across_add_install_and_update() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = std::sync::Arc::new(std::sync::Mutex::new("first"));
        let server_body = body.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _read = stream.read(&mut request).await.unwrap();
                let body = *server_body.lock().unwrap();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let paths = VdmPaths::resolve(Some(directory.path())).await.unwrap();
        let url = format!("http://{address}/package@latest/file.txt");
        let key = VdmManifestTargetUrl::new(format!("http:{url}"));
        VdmVendor::add(Some(directory.path()), &url).await.unwrap();
        let manifest = VdmManifest::read_toml(&paths.manifest).await.unwrap();
        let targets = manifest.targets().unwrap();
        let hash = format!("sha256:{:x}", Sha256::digest(b"first"));
        assert_eq!(targets[&key].version().as_str(), hash);
        let lock = VdmLock::read_toml(&paths.lock).await.unwrap();
        assert_eq!(lock.files[&key].version.as_str(), hash);
        let destination = paths.target(targets[&key].as_ref());
        fs::remove_file(&destination).unwrap();
        VdmVendor::install(Some(directory.path()), false)
            .await
            .unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"first");

        *body.lock().unwrap() = "second";
        fs::remove_file(&destination).unwrap();
        assert!(
            VdmVendor::install(Some(directory.path()), false)
                .await
                .is_err()
        );
        VdmVendor::update(Some(directory.path()), &url, false)
            .await
            .unwrap();
        let manifest = VdmManifest::read_toml(&paths.manifest).await.unwrap();
        assert_eq!(
            manifest.targets().unwrap()[&key].version().as_str(),
            format!("sha256:{:x}", Sha256::digest(b"second"))
        );
        assert_eq!(fs::read(&destination).unwrap(), b"second");
        server.abort();
    }

    #[tokio::test]
    async fn install_removes_files_missing_from_manifest_and_updates_lock() {
        let directory = tempfile::tempdir().unwrap();
        let paths = VdmPaths::resolve(Some(directory.path())).await.unwrap();
        VdmManifest::new()
            .write_toml(&paths.manifest)
            .await
            .unwrap();

        let stale_path = directory.path().join("vendor/@gh/owner/repo/stale.txt");
        fs::create_dir_all(stale_path.parent().unwrap()).unwrap();
        fs::write(&stale_path, "stale").unwrap();
        let unrelated_path = directory.path().join("vendor/unrelated.txt");
        fs::write(&unrelated_path, "keep").unwrap();

        let mut lock = VdmLock::default();
        lock.files.insert(
            VdmManifestTargetUrl::new("gh:owner/repo:stale.txt"),
            VdmLockFile {
                direct: true,
                version: VdmManifestSourceVersion::new("main"),
                revision: "revision".to_owned(),
                hash: "sha256:stale".to_owned(),
                source: "https://example.com/stale.txt".to_owned(),
                path: "vendor/@gh/owner/repo/stale.txt".to_owned(),
                dependencies: Vec::new(),
            },
        );
        lock.write_toml(&paths.lock).await.unwrap();

        VdmVendor::install(Some(directory.path()), false)
            .await
            .unwrap();

        assert!(!stale_path.exists());
        assert!(!directory.path().join("vendor/@gh").exists());
        assert_eq!(fs::read_to_string(unrelated_path).unwrap(), "keep");
        assert!(
            VdmLock::read_toml(&paths.lock)
                .await
                .unwrap()
                .files
                .is_empty()
        );
    }

    #[tokio::test]
    async fn resolves_manifest_and_target_paths() {
        let directory = tempfile::tempdir().unwrap();
        let paths = VdmPaths::resolve(Some(directory.path())).await.unwrap();
        let target = VdmSourceInput::parse_target("gh:js-fns/js-fns:src/file.ts@main").unwrap();
        assert_eq!(paths.manifest, directory.path().join("vendor.toml"));
        assert_eq!(
            paths.target(target.as_ref()),
            directory
                .path()
                .join("vendor/@gh/js-fns/js-fns/src/file.ts")
        );
        assert!(
            VdmPaths::resolve(Some(Path::new("other.toml")))
                .await
                .is_err()
        );
    }
}
