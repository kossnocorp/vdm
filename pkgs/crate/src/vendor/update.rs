use crate::prelude::*;

impl VitVendor {
    pub async fn update(manifest_path: Option<&Path>, input: &str) -> Result<()> {
        let target = VitSourceInput::parse_target(input)?;
        let state = VitState::create().initialize(manifest_path).await?;

        let VitState::Initialized(state) = state else {
            bail!("Failed to initialize Vit state, expected initialized state");
        };

        let targets = state.manifest.targets()?;
        let manifest_target = targets.get(target.key()).with_context(|| {
            format!(
                "{} is not present in {}",
                target.key(),
                state.paths.manifest.display()
            )
        })?;
        let version = manifest_target.version();

        ensure!(
            version == target.version(),
            "{} uses version {:?} in {}, not {:?}",
            target.key(),
            version,
            state.paths.manifest.display(),
            target.version()
        );

        let mut state = state.as_locked().await?;

        let key = target.key().clone();
        let graph = resolve_graph(target).await?;
        let direct = targets.keys().cloned().collect::<BTreeSet<_>>();
        let changed = Self::write_graph(&mut state, graph, &direct).await?;
        let removed = Self::prune_unreachable(&mut state, direct.iter().cloned()).await?;
        state.lock.write_toml(&state.paths.lock).await?;
        if changed == 0 && removed == 0 {
            println!("{key} is already up to date");
        } else {
            println!("Updated {key}: wrote {changed} and removed {removed} files");
        }
        Ok(())
    }
}
