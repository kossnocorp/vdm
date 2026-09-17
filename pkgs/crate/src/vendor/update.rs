use crate::prelude::*;

use similar::{ChangeTag, TextDiff};
use std::io;

pub(super) enum ReviewKind {
    Upsert(VdmGraphFile, Box<VdmLockFile>),
    Delete,
}

pub(super) struct ReviewFile {
    pub key: VdmManifestTargetUrl,
    pub path: String,
    pub old: Option<VdmLockFile>,
    pub kind: ReviewKind,
    pub additions: usize,
    pub deletions: usize,
    pub diff: String,
    pub old_source: String,
    pub new_source: String,
    pub accepted: bool,
}

impl VdmVendor {
    pub async fn update(manifest_path: Option<&Path>, input: &str, review: bool) -> Result<()> {
        let target = VdmSourceInput::parse_target(input)?;
        let state = VdmState::create().initialize(manifest_path).await?;

        let VdmState::Initialized(state) = state else {
            bail!("Failed to initialize Vdm state, expected initialized state");
        };

        let targets = state.manifest.targets()?;
        targets.get(target.key()).with_context(|| {
            format!(
                "{} is not present in {}",
                target.key(),
                state.paths.manifest.display()
            )
        })?;

        let mut state = state.as_locked().await?;
        let key = target.key().clone();
        let glob_target = target.as_any().downcast_ref::<VdmGitTarget>().cloned();
        let graph = resolve_graph(target).await?;
        let requested_version = Self::graph_version(&graph, &key)?.clone();
        let direct = targets.keys().cloned().collect::<BTreeSet<_>>();

        if !review {
            Self::record_glob(&mut state.lock, glob_target.as_ref(), &graph)?;
            let changed = Self::write_graph(&mut state, graph, &direct).await?;
            let removed = Self::prune_unreachable(&mut state, direct.iter().cloned()).await?;
            state.manifest.update(&key, &requested_version)?;
            state.manifest.write_toml(&state.paths.manifest).await?;
            state.lock.write_toml(&state.paths.lock).await?;
            if changed == 0 && removed == 0 {
                println!("{key} is already up to date");
            } else {
                println!("Updated {key}: wrote {changed} and removed {removed} files");
            }
            return Ok(());
        }

        let mut hypothetical = state.lock.clone();
        Self::record_glob(&mut hypothetical, glob_target.as_ref(), &graph)?;
        let mut files = Vec::new();
        for (file_key, file) in graph {
            let destination = state.paths.target(file.target.as_ref());
            let next = VdmLockFile::new(
                file.target.as_ref(),
                &file.download,
                &state.paths,
                direct.contains(&file_key)
                    || hypothetical
                        .globs
                        .values()
                        .any(|members| members.contains(&file_key)),
                file.dependencies.clone(),
            );
            hypothetical.files.insert(file_key.clone(), next.clone());

            let old = state.lock.files.get(&file_key).cloned();
            let current = match tokio::fs::read(&destination).await {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to read {}", destination.display()));
                }
            };
            if old.as_ref() == Some(&next) && current == file.download.bytes {
                continue;
            }
            let (diff, additions, deletions) =
                file_diff(&next.path, &current, &file.download.bytes);
            let old_source = String::from_utf8_lossy(&current).into_owned();
            let new_source = String::from_utf8_lossy(&file.download.bytes).into_owned();
            files.push(ReviewFile {
                key: file_key,
                path: next.path.clone(),
                old,
                kind: ReviewKind::Upsert(file, Box::new(next)),
                additions,
                deletions,
                diff,
                old_source,
                new_source,
                accepted: false,
            });
        }

        let reachable = Self::reachable(&hypothetical, direct.iter().cloned());
        for file_key in state
            .lock
            .files
            .keys()
            .filter(|file_key| !reachable.contains(*file_key))
        {
            let old = state.lock.files[file_key].clone();
            let destination = Self::lock_destination(&state.paths, &old.path)?;
            let current = match tokio::fs::read(&destination).await {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Failed to read {}", destination.display()));
                }
            };
            let (diff, additions, deletions) = file_diff(&old.path, &current, &[]);
            files.push(ReviewFile {
                key: file_key.clone(),
                path: old.path.clone(),
                old: Some(old),
                kind: ReviewKind::Delete,
                additions,
                deletions,
                diff,
                old_source: String::from_utf8_lossy(&current).into_owned(),
                new_source: String::new(),
                accepted: false,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));

        if files.is_empty() {
            println!("{key} is already up to date");
            return Ok(());
        }

        match super::review::run(&mut files)? {
            super::review::ReviewOutcome::Cancelled => {
                println!("Update cancelled");
                return Ok(());
            }
            super::review::ReviewOutcome::Rejected => {
                println!("Update rejected");
                return Ok(());
            }
            super::review::ReviewOutcome::Accepted => {}
        }

        let mut changed = 0;
        let mut removed = 0;
        let mut root_accepted = false;
        for file in files {
            let from = file.old.as_ref().map(|entry| entry.revision.clone());
            let to = match &file.kind {
                ReviewKind::Upsert(_, next) => Some(next.revision.clone()),
                ReviewKind::Delete => None,
            };
            state.lock.updates.insert(
                file.key.clone(),
                VdmLockUpdate {
                    from,
                    to,
                    decision: if file.accepted {
                        VdmLockUpdateDecision::Accepted
                    } else {
                        VdmLockUpdateDecision::Rejected
                    },
                },
            );
            if !file.accepted {
                continue;
            }
            if file.key == key {
                root_accepted = true;
            }
            match file.kind {
                ReviewKind::Upsert(graph_file, next) => {
                    let destination = state.paths.target(graph_file.target.as_ref());
                    graph_file.download.write(&destination).await?;
                    state.lock.files.insert(file.key, *next);
                    changed += 1;
                }
                ReviewKind::Delete => {
                    let old = file.old.context("Deleted review file has no lock entry")?;
                    Self::remove_locked_file(
                        &Self::lock_destination(&state.paths, &old.path)?,
                        &state.paths.root.join("vendor"),
                    )
                    .await?;
                    state.lock.files.remove(&file.key);
                    removed += 1;
                }
            }
        }
        if root_accepted {
            state.manifest.update(&key, &requested_version)?;
            state.manifest.write_toml(&state.paths.manifest).await?;
        }
        if let Some(members) = hypothetical.globs.get(&key)
            && (changed > 0 || removed > 0)
        {
            let mut retained = state.lock.globs.get(&key).cloned().unwrap_or_default();
            retained.extend(
                members
                    .iter()
                    .filter(|member| state.lock.files.contains_key(*member))
                    .cloned(),
            );
            retained.retain(|member| state.lock.files.contains_key(member));
            retained.sort();
            retained.dedup();
            state.lock.globs.insert(key.clone(), retained);
            state.manifest.update(&key, &requested_version)?;
            state.manifest.write_toml(&state.paths.manifest).await?;
        }
        state.lock.write_toml(&state.paths.lock).await?;
        println!("Updated {key}: wrote {changed} and removed {removed} files");
        Ok(())
    }
}

fn file_diff(path: &str, old: &[u8], new: &[u8]) -> (String, usize, usize) {
    let old = String::from_utf8_lossy(old);
    let new = String::from_utf8_lossy(new);
    let diff = TextDiff::from_lines(old.as_ref(), new.as_ref());
    let mut additions = 0;
    let mut deletions = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => additions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    let rendered = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    (rendered, additions, deletions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_diff_and_counts_changed_lines() {
        let (diff, additions, deletions) =
            file_diff("vendor/file.ts", b"one\ntwo\n", b"one\nthree\n");
        assert!(diff.contains("--- a/vendor/file.ts"));
        assert!(diff.contains("+++ b/vendor/file.ts"));
        assert_eq!((additions, deletions), (1, 1));
    }
}
