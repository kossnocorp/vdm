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
        let destination = state.paths.target(target.as_ref());
        let graph = resolve_graph(target).await?;
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

        let root = state
            .lock
            .files
            .get(&key)
            .context("Resolved graph is missing its root")?;
        state.manifest.add(&key, &root.version);
        state.manifest.write_toml(&state.paths.manifest).await?;
        state.lock.write_toml(&state.paths.lock).await?;

        println!("Added {key} and {added} files to {}", destination.display());
        Ok(())
    }
}
