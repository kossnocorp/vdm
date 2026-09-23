use crate::prelude::*;

impl VdmGitSource {
    pub(super) async fn download_file(&self, target: &dyn VdmTarget) -> Result<VdmSourceFile> {
        let target = target
            .as_any()
            .downcast_ref::<VdmGitTarget>()
            .context("Git source received a target from another source")?
            .clone();
        VdmGitCache::try_new()?
            .fetch(target.resolve_version().await?)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn git_globs_resolve_dependencies_update_and_restore_locked_revisions() {
        git_group_resolves_dependencies_update_and_restore("src/*.ts").await;
    }

    #[tokio::test]
    async fn git_folders_resolve_dependencies_update_and_restore_locked_revisions() {
        for path in ["src", "src/"] {
            git_group_resolves_dependencies_update_and_restore(path).await;
        }
    }

    async fn git_group_resolves_dependencies_update_and_restore(selection: &str) {
        use std::fs;
        let source_dir = tempfile::tempdir().unwrap();
        let source = git2::Repository::init(source_dir.path()).unwrap();
        source.set_head("refs/heads/custom/default").unwrap();
        fs::create_dir(source_dir.path().join("src")).unwrap();
        let commit = |name: &str, dependency: &str| {
            fs::write(
                source_dir.path().join("src/root.ts"),
                format!("import '../{dependency}';\n"),
            )
            .unwrap();
            fs::write(source_dir.path().join(format!("{dependency}.ts")), name).unwrap();
            let mut index = source.index().unwrap();
            index
                .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            let tree = source.find_tree(index.write_tree().unwrap()).unwrap();
            let signature = git2::Signature::now("Test", "test@example.com").unwrap();
            let parent = source
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok());
            let parents = parent.iter().collect::<Vec<_>>();
            source
                .commit(Some("HEAD"), &signature, &signature, name, &tree, &parents)
                .unwrap()
        };
        let first = commit("first", "dep");
        source
            .tag_lightweight("v1", &source.find_object(first, None).unwrap(), false)
            .unwrap();
        let url = Url::from_file_path(source_dir.path()).unwrap();
        let input = format!("{url}:{selection}");
        let key = VdmSourceInput::parse_target(&input).unwrap().key().clone();
        let directory = tempfile::tempdir().unwrap();
        let paths = VdmPaths::resolve(Some(directory.path())).await.unwrap();
        VdmVendor::add(Some(directory.path()), &input)
            .await
            .unwrap();
        let manifest = VdmManifest::read_toml(&paths.manifest).await.unwrap();
        assert_eq!(
            manifest.targets().unwrap()[&key].version().as_str(),
            "custom/default"
        );
        let lock = VdmLock::read_toml(&paths.lock).await.unwrap();
        if !selection.contains('*') {
            let error = VdmVendor::add(
                Some(directory.path()),
                &format!("git:{url}:/src/nested/.././"),
            )
            .await
            .unwrap_err();
            assert!(error.to_string().contains("already present"));
        }
        assert_eq!(lock.files.len(), 2);
        assert_eq!(lock.globs[&key].len(), 1);
        assert!(
            lock.files
                .values()
                .all(|file| file.revision == first.to_string())
        );
        let root_key = VdmManifestTargetUrl::new(format!("git:{url}:src/root.ts"));
        let dep_key = VdmManifestTargetUrl::new(format!("git:{url}:dep.ts"));
        assert_eq!(lock.files[&root_key].dependencies, vec![dep_key.clone()]);
        let second = commit("second", "other");
        let root_path = paths.root.join(&lock.files[&root_key].path);
        fs::remove_file(&root_path).unwrap();
        assert!(
            VdmVendor::install(Some(directory.path()), true)
                .await
                .is_err()
        );
        VdmVendor::install(Some(directory.path()), false)
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(&root_path).unwrap(),
            "import '../dep';\n"
        );
        let update = if selection.contains('*') {
            key.to_string()
        } else {
            format!("git:{url}:src/nested/..")
        };
        VdmVendor::update(Some(directory.path()), &update, false)
            .await
            .unwrap();
        let updated = VdmLock::read_toml(&paths.lock).await.unwrap();
        assert_eq!(updated.files.len(), 2);
        assert!(
            updated
                .files
                .values()
                .all(|file| file.revision == second.to_string())
        );
        assert!(!paths.root.join(&lock.files[&dep_key].path).exists());
        VdmVendor::install(Some(directory.path()), true)
            .await
            .unwrap();
        for version in ["v1".to_owned(), first.to_string()] {
            let target = VdmSourceInput::parse_target(&format!("{url}:dep.ts@{version}")).unwrap();
            let file = target.source().download(target.as_ref()).await.unwrap();
            assert_eq!(file.bytes, b"first");
            assert_eq!(file.revision, first.to_string());
        }
        let target = manifest.targets().unwrap().remove(&key).unwrap();
        let target = target.as_any().downcast_ref::<VdmGitTarget>().unwrap();
        let repository = VdmGitCache::try_new().unwrap().repository(target);
        fs::remove_dir_all(&repository).unwrap();
        fs::remove_file(repository.with_extension("lock")).unwrap();
    }
}
