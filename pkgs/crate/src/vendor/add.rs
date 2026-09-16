use crate::prelude::*;

impl VdmVendor {
    pub async fn add(manifest_path: Option<&Path>, input: &str) -> Result<()> {
        let target = VdmSourceInput::parse_target(input)?;
        let state = VdmState::create().initialize(manifest_path).await?;

        let VdmState::Initialized(state) = state else {
            bail!("Failed to initialize Vdm state, expected initialized state");
        };

        let targets = state.manifest.targets()?;
        ensure!(
            !targets.contains_key(target.key()),
            "{} is already present in {}",
            target.key(),
            state.paths.manifest.display()
        );

        let mut state = state.as_locked().await?;

        let key = target.key().clone();
        let mut destination = state.paths.target(target.as_ref());
        let glob_target = target.as_any().downcast_ref::<VdmGitHubTarget>().cloned();
        if let Some(target) = &glob_target
            && target.glob()?.is_some()
        {
            destination = state
                .paths
                .root
                .join("vendor")
                .join(format!("@{}", target.owner()))
                .join(target.repo());
        }
        let graph = resolve_graph(target).await?;
        let version = Self::graph_version(&graph, &key)?.clone();
        Self::record_glob(&mut state.lock, glob_target.as_ref(), &graph)?;
        let mut direct = targets.keys().cloned().collect::<BTreeSet<_>>();
        direct.insert(key.clone());
        for (graph_key, file) in &graph {
            if let Some(existing) = state.lock.files.get(graph_key) {
                ensure!(
                    existing.revision == file.download.revision
                        && existing.hash
                            == format!("sha256:{:x}", Sha256::digest(&file.download.bytes)),
                    "{graph_key} is already locked at a conflicting revision"
                );
            }
        }
        let added = Self::write_graph(&mut state, graph, &direct).await?;

        state.manifest.add(&key, &version);
        state.manifest.write_toml(&state.paths.manifest).await?;
        state.lock.write_toml(&state.paths.lock).await?;

        println!("Added {key} and {added} files to {}", destination.display());
        Ok(())
    }
}
