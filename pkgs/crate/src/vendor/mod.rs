use crate::prelude::*;

mod add;

mod install;

mod update;

pub struct VitVendor;

impl VitVendor {
    async fn write_graph(
        state: &mut VitStateLocked,
        graph: BTreeMap<VitManifestTargetUrl, VitGraphFile>,
        direct: &BTreeSet<VitManifestTargetUrl>,
    ) -> Result<usize> {
        let mut written = 0;
        for (key, file) in graph {
            let destination = state.paths.target(file.target.as_ref());
            let next = VitLockFile::new(
                file.target.as_ref(),
                &file.download,
                &state.paths,
                direct.contains(&key),
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
        lock: &VitLock,
        roots: impl IntoIterator<Item = VitManifestTargetUrl>,
    ) -> BTreeSet<VitManifestTargetUrl> {
        let mut reachable = BTreeSet::new();
        let mut pending = roots.into_iter().collect::<Vec<_>>();
        while let Some(key) = pending.pop() {
            if !reachable.insert(key.clone()) {
                continue;
            }
            if let Some(file) = lock.files.get(&key) {
                pending.extend(file.dependencies.iter().cloned());
            }
        }
        reachable
    }

    async fn prune_unreachable(
        state: &mut VitStateLocked,
        roots: impl IntoIterator<Item = VitManifestTargetUrl>,
    ) -> Result<usize> {
        let reachable = Self::reachable(&state.lock, roots);
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
    async fn install_removes_files_missing_from_manifest_and_updates_lock() {
        let directory = tempfile::tempdir().unwrap();
        let paths = VitPaths::resolve(Some(directory.path())).await.unwrap();
        VitManifest::new()
            .write_toml(&paths.manifest)
            .await
            .unwrap();

        let stale_path = directory.path().join("vendor/@owner/repo/stale.txt");
        fs::create_dir_all(stale_path.parent().unwrap()).unwrap();
        fs::write(&stale_path, "stale").unwrap();
        let unrelated_path = directory.path().join("vendor/unrelated.txt");
        fs::write(&unrelated_path, "keep").unwrap();

        let mut lock = VitLock::default();
        lock.files.insert(
            VitManifestTargetUrl::new("gh:owner/repo/stale.txt"),
            VitLockFile {
                direct: true,
                version: VitManifestSourceVersion::new("main"),
                revision: "revision".to_owned(),
                hash: "sha256:stale".to_owned(),
                source: "https://example.com/stale.txt".to_owned(),
                path: "vendor/@owner/repo/stale.txt".to_owned(),
                dependencies: Vec::new(),
            },
        );
        lock.write_toml(&paths.lock).await.unwrap();

        VitVendor::install(Some(directory.path()), false)
            .await
            .unwrap();

        assert!(!stale_path.exists());
        assert!(!directory.path().join("vendor/@owner").exists());
        assert_eq!(fs::read_to_string(unrelated_path).unwrap(), "keep");
        assert!(
            VitLock::read_toml(&paths.lock)
                .await
                .unwrap()
                .files
                .is_empty()
        );
    }

    #[tokio::test]
    async fn resolves_manifest_and_target_paths() {
        let directory = tempfile::tempdir().unwrap();
        let paths = VitPaths::resolve(Some(directory.path())).await.unwrap();
        let target = VitSourceInput::parse_target("gh:js-fns/js-fns/src/file.ts@main").unwrap();
        assert_eq!(paths.manifest, directory.path().join("vendor.toml"));
        assert_eq!(
            paths.target(target.as_ref()),
            directory.path().join("vendor/@js-fns/js-fns/src/file.ts")
        );
        assert!(
            VitPaths::resolve(Some(Path::new("other.toml")))
                .await
                .is_err()
        );
    }
}
