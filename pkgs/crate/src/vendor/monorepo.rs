use crate::prelude::*;

impl VdmVendor {
    /// Expand package globs relative to the declaring manifest. Packages without
    /// vendor.toml are ignored until their first dependency is added.
    pub(super) async fn package_manifests(path: Option<&Path>) -> Result<Vec<PathBuf>> {
        let paths = VdmPaths::resolve(path).await?;
        let manifest = VdmManifest::read_toml(&paths.manifest).await?;
        let Some(monorepo) = &manifest.monorepo else {
            return Ok(vec![paths.manifest]);
        };
        let root = tokio::fs::canonicalize(&paths.root).await?;
        let mut packages = BTreeSet::new();
        for pattern in &monorepo.pkgs {
            ensure!(
                !Path::new(pattern).is_absolute()
                    && !Path::new(pattern)
                        .components()
                        .any(|part| matches!(part, Component::ParentDir)),
                "Package pattern must be relative to the monorepo root: {pattern}"
            );
            let pattern = format!(
                "{}/{pattern}",
                glob::Pattern::escape(&root.to_string_lossy())
            );
            for entry in glob::glob(&pattern)
                .with_context(|| format!("Invalid package pattern: {pattern}"))?
            {
                let directory = entry?;
                if !directory.is_dir() {
                    continue;
                }
                let directory = tokio::fs::canonicalize(directory).await?;
                ensure!(
                    directory.starts_with(&root),
                    "Package is outside monorepo: {}",
                    directory.display()
                );
                if directory != root && directory.join("vendor.toml").is_file() {
                    packages.insert(directory.join("vendor.toml"));
                }
            }
        }
        // The root may also vendor its own dependencies.
        if !manifest.targets()?.is_empty() || paths.lock.is_file() {
            packages.insert(paths.manifest);
        }
        Ok(packages.into_iter().collect())
    }

    pub async fn update_packages(
        path: Option<&Path>,
        input: Option<&str>,
        review: bool,
    ) -> Result<()> {
        let requested = input.map(VdmSourceInput::parse_target).transpose()?;
        let mut updates = Vec::new();
        for manifest_path in Self::package_manifests(path).await? {
            let manifest = VdmManifest::read_toml(&manifest_path).await?;
            let targets = manifest.targets()?;
            if let Some(target) = &requested {
                if targets.contains_key(target.key()) {
                    updates.push((manifest_path, input.unwrap().to_owned()));
                }
            } else {
                for key in targets.keys() {
                    updates.push((manifest_path.clone(), key.to_string()));
                }
            }
        }
        ensure!(
            input.is_none() || !updates.is_empty(),
            "{} is not present in any selected manifest",
            input.unwrap_or_default()
        );
        for (manifest, input) in updates {
            println!("Updating {}", manifest.display());
            Self::update(Some(&manifest), &input, review)
                .await
                .with_context(|| format!("Failed to update {}", manifest.display()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn installs_and_updates_matching_packages_from_root() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = std::sync::Arc::new(std::sync::Mutex::new("first"));
        let server_body = body.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).await.unwrap();
                let body = *server_body.lock().unwrap();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let source = format!("http://{address}/shared.txt");
        let other = format!("http://{address}/other.txt");
        let target = VdmSourceInput::parse_target(&source).unwrap();
        fs::write(
            root.join("vendor.toml"),
            "[monorepo]\npkgs = ['pkgs/*', 'pkgs/a']\n",
        )
        .unwrap();
        for (package, source) in [("a", &source), ("b", &source), ("c", &other)] {
            let path = root.join("pkgs").join(package);
            fs::create_dir_all(&path).unwrap();
            let argv = [
                "--pkg".into(),
                path.into_os_string(),
                "add".into(),
                source.into(),
            ];
            VdmCli::parse_from(&argv.iter().map(|arg| arg.as_os_str()).collect::<Vec<_>>())
                .unwrap()
                .run_async()
                .await
                .unwrap();
        }
        fs::create_dir_all(root.join("pkgs/empty")).unwrap();
        let manifests = VdmVendor::package_manifests(Some(root)).await.unwrap();
        assert_eq!(manifests.len(), 3);
        for name in ["a", "b", "c"] {
            fs::remove_dir_all(root.join("pkgs").join(name).join("vendor")).unwrap();
        }
        VdmVendor::install(Some(root), false).await.unwrap();
        *body.lock().unwrap() = "second";
        VdmVendor::update_packages(Some(root), Some(&source), false)
            .await
            .unwrap();
        for name in ["a", "b"] {
            let paths = VdmPaths::resolve(Some(&root.join("pkgs").join(name)))
                .await
                .unwrap();
            assert_eq!(fs::read(paths.target(target.as_ref())).unwrap(), b"second");
        }
        let other_target = VdmSourceInput::parse_target(&other).unwrap();
        let paths = VdmPaths::resolve(Some(&root.join("pkgs/c"))).await.unwrap();
        assert_eq!(
            fs::read(paths.target(other_target.as_ref())).unwrap(),
            b"first"
        );
        let command = VdmCli::parse_from(&[
            std::ffi::OsStr::new("update"),
            std::ffi::OsStr::new("--manifest"),
            root.as_os_str(),
        ])
        .unwrap();
        Box::pin(command.run_async()).await.unwrap();
        assert_eq!(
            fs::read(paths.target(other_target.as_ref())).unwrap(),
            b"second"
        );
        VdmVendor::install(Some(root), true).await.unwrap();
        assert!(!root.join("vendor.lock.toml").exists());
        assert!(
            VdmVendor::update_packages(Some(root), Some("https://example.com/missing.txt"), false)
                .await
                .is_err()
        );
        server.abort();
    }

    #[tokio::test]
    async fn preserves_monorepo_configuration_when_writing_manifest() {
        let directory = tempfile::tempdir().unwrap();
        let mut manifest: VdmManifest = toml::from_str("[monorepo]\npkgs = ['pkgs/*']").unwrap();
        manifest.add(
            &VdmManifestTargetUrl::new("gh:owner/repo:file.txt"),
            &VdmManifestSourceVersion::new("main"),
        );
        let path = directory.path().join("vendor.toml");
        manifest.write_toml(&path).await.unwrap();
        let restored = VdmManifest::read_toml(&path).await.unwrap();
        assert_eq!(restored.monorepo.unwrap().pkgs, ["pkgs/*"]);
        assert_eq!(
            VdmVendor::package_manifests(Some(directory.path()))
                .await
                .unwrap(),
            [path]
        );
    }
}
