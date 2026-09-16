use crate::prelude::*;

mod add;

mod install;

mod update;

mod review;

pub struct VdmVendor;

impl VdmVendor {
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
        }
        reachable
    }

    async fn prune_unreachable(
        state: &mut VdmStateLocked,
        roots: impl IntoIterator<Item = VdmManifestTargetUrl>,
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
        let paths = VdmPaths::resolve(Some(directory.path())).await.unwrap();
        VdmManifest::new()
            .write_toml(&paths.manifest)
            .await
            .unwrap();

        let stale_path = directory.path().join("vendor/@owner/repo/stale.txt");
        fs::create_dir_all(stale_path.parent().unwrap()).unwrap();
        fs::write(&stale_path, "stale").unwrap();
        let unrelated_path = directory.path().join("vendor/unrelated.txt");
        fs::write(&unrelated_path, "keep").unwrap();

        let mut lock = VdmLock::default();
        lock.files.insert(
            VdmManifestTargetUrl::new("gh:owner/repo/stale.txt"),
            VdmLockFile {
                direct: true,
                version: VdmManifestSourceVersion::new("main"),
                revision: "revision".to_owned(),
                hash: "sha256:stale".to_owned(),
                source: "https://example.com/stale.txt".to_owned(),
                path: "vendor/@owner/repo/stale.txt".to_owned(),
                dependencies: Vec::new(),
            },
        );
        lock.write_toml(&paths.lock).await.unwrap();

        VdmVendor::install(Some(directory.path()), false)
            .await
            .unwrap();

        assert!(!stale_path.exists());
        assert!(!directory.path().join("vendor/@owner").exists());
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
        let target = VdmSourceInput::parse_target("gh:js-fns/js-fns/src/file.ts@main").unwrap();
        assert_eq!(paths.manifest, directory.path().join("vendor.toml"));
        assert_eq!(
            paths.target(target.as_ref()),
            directory.path().join("vendor/@js-fns/js-fns/src/file.ts")
        );
        assert!(
            VdmPaths::resolve(Some(Path::new("other.toml")))
                .await
                .is_err()
        );
    }
}
