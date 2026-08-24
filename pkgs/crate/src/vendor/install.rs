use crate::prelude::*;

use tokio::io::AsyncReadExt;

impl VitVendor {
    pub async fn install(manifest_path: Option<&Path>, offline: bool) -> Result<()> {
        let state = VitState::create()
            .initialize(manifest_path)
            .await?
            .initialize_lock()
            .await?;

        let VitState::Locked(mut state) = state else {
            bail!("Failed to initialize Vit state, expected locked state");
        };

        let targets = state.manifest.targets()?;
        let roots = targets.keys().cloned().collect::<BTreeSet<_>>();
        let reachable = Self::reachable(&state.lock, roots.iter().cloned());
        let mut complete = true;
        for key in &roots {
            let Some(entry) = state.lock.files.get(key) else {
                complete = false;
                break;
            };
            if !entry.direct
                || targets
                    .get(key)
                    .is_none_or(|target| target.version() != &entry.version)
            {
                complete = false;
                break;
            }
        }
        for key in &reachable {
            if !state.lock.files.contains_key(key) {
                complete = false;
                break;
            }
        }

        let mut installed = 0;
        if complete {
            for key in &reachable {
                let entry = &state.lock.files[key];
                let destination = Self::lock_destination(&state.paths, &entry.path)?;
                if Self::file_matches(&destination, &entry.hash).await? {
                    continue;
                }
                ensure!(
                    !offline,
                    "{key} is not available from the locked vendor directory in offline mode"
                );
                let target = VitSourceInput::parse_manifest_target(key, &entry.version)?;
                let download =
                    if let Some(target) = target.as_any().downcast_ref::<VitSourceGitHubTarget>() {
                        VitGitHubCache::try_new()?
                            .fetch_revision(target, &entry.revision)
                            .await?
                    } else {
                        target.source().download(target.as_ref()).await?
                    };
                let hash = format!("sha256:{:x}", Sha256::digest(&download.bytes));
                ensure!(
                    hash == entry.hash,
                    "{key} no longer matches the content hash recorded in the lock"
                );
                download.write(&destination).await?;
                installed += 1;
            }
        } else {
            ensure!(
                !offline,
                "The locked dependency graph is incomplete in offline mode"
            );
            let mut graph: BTreeMap<VitManifestTargetUrl, VitGraphFile> = BTreeMap::new();
            for (_, target) in targets {
                for (key, file) in resolve_graph(target).await? {
                    if let Some(existing) = graph.get(&key) {
                        ensure!(
                            existing.download.revision == file.download.revision
                                && existing.download.bytes == file.download.bytes,
                            "{key} resolves to conflicting revisions"
                        );
                    } else {
                        graph.insert(key, file);
                    }
                }
            }
            installed = Self::write_graph(&mut state, graph, &roots).await?;
        }
        let removed = Self::prune_unreachable(&mut state, roots).await?;

        state.lock.write_toml(&state.paths.lock).await?;
        println!("Installed {installed} and removed {removed} files");
        Ok(())
    }

    pub(super) async fn file_matches(path: &Path, expected_hash: &str) -> Result<bool> {
        let mut file = match tokio::fs::File::open(path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to read {}", path.display()));
            }
        };
        let mut hasher = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .await
                .with_context(|| format!("Failed to read {}", path.display()))?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(format!("sha256:{:x}", hasher.finalize()) == expected_hash)
    }

    pub(super) fn lock_destination(paths: &VitPaths, value: &str) -> Result<PathBuf> {
        let relative = Path::new(value);
        ensure!(
            relative
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
                && relative.starts_with("vendor"),
            "Lockfile path {value:?} is not a safe vendor path"
        );
        Ok(paths.root.join(relative))
    }

    pub(super) async fn remove_locked_file(path: &Path, vendor_root: &Path) -> Result<()> {
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to remove {}", path.display()));
            }
        }

        let mut parent = path.parent();
        while let Some(directory) = parent.filter(|directory| directory.starts_with(vendor_root)) {
            match tokio::fs::remove_dir(directory).await {
                Ok(()) => parent = directory.parent(),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) =>
                {
                    break;
                }
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to remove {}", directory.display()));
                }
            }
        }
        Ok(())
    }
}
