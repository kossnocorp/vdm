use crate::prelude::*;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use similar::{ChangeTag, TextDiff};
use std::io::{self, IsTerminal, Write};

enum ReviewKind {
    Upsert(VitGraphFile, Box<VitLockFile>),
    Delete,
}

struct ReviewFile {
    key: VitManifestTargetUrl,
    path: String,
    old: Option<VitLockFile>,
    kind: ReviewKind,
    additions: usize,
    deletions: usize,
    diff: String,
    accepted: bool,
}

enum ReviewChoice {
    Accept,
    Reject,
    Cancel,
}

impl VitVendor {
    pub async fn update(manifest_path: Option<&Path>, input: &str, review: bool) -> Result<()> {
        let target = VitSourceInput::parse_target(input)?;
        let state = VitState::create().initialize(manifest_path).await?;

        let VitState::Initialized(state) = state else {
            bail!("Failed to initialize Vit state, expected initialized state");
        };

        let targets = state.manifest.targets()?;
        targets.get(target.key()).with_context(|| {
            format!(
                "{} is not present in {}",
                target.key(),
                state.paths.manifest.display()
            )
        })?;
        let requested_version = target.version().clone();

        let mut state = state.as_locked().await?;
        let key = target.key().clone();
        let graph = resolve_graph(target).await?;
        let direct = targets.keys().cloned().collect::<BTreeSet<_>>();

        if !review {
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
        let mut files = Vec::new();
        for (file_key, file) in graph {
            let destination = state.paths.target(file.target.as_ref());
            let next = VitLockFile::new(
                file.target.as_ref(),
                &file.download,
                &state.paths,
                direct.contains(&file_key),
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
            files.push(ReviewFile {
                key: file_key,
                path: next.path.clone(),
                old,
                kind: ReviewKind::Upsert(file, Box::new(next)),
                additions,
                deletions,
                diff,
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
                accepted: false,
            });
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));

        if files.is_empty() {
            println!("{key} is already up to date");
            return Ok(());
        }

        let file_count = files.len();
        for (index, file) in files.iter_mut().enumerate() {
            println!("\n[{}/{}] {}", index + 1, file_count, file.path);
            print!("{}", file.diff);
            match prompt("Accept, reject, or cancel? [a/r/c] ", true)? {
                ReviewChoice::Accept => file.accepted = true,
                ReviewChoice::Reject => file.accepted = false,
                ReviewChoice::Cancel => {
                    println!("Update cancelled");
                    return Ok(());
                }
            }
        }

        println!("\nReview recap:");
        for file in &files {
            let decision = if file.accepted {
                "accepted"
            } else {
                "rejected"
            };
            println!(
                "  {}  +{} -{}  {decision}",
                file.path, file.additions, file.deletions
            );
        }
        if !matches!(
            prompt("Apply accepted changes? [a/r] ", false)?,
            ReviewChoice::Accept
        ) {
            println!("Update rejected");
            return Ok(());
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
                VitLockUpdate {
                    from,
                    to,
                    decision: if file.accepted {
                        VitLockUpdateDecision::Accepted
                    } else {
                        VitLockUpdateDecision::Rejected
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

fn prompt(message: &str, allow_cancel: bool) -> Result<ReviewChoice> {
    loop {
        print!("{message}");
        io::stdout().flush()?;
        let character = if io::stdin().is_terminal() {
            enable_raw_mode().context("Failed to enable terminal raw mode")?;
            let result = loop {
                match event::read() {
                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => match key.code {
                        KeyCode::Char(character) => break Ok(character),
                        KeyCode::Esc => break Ok('c'),
                        _ => {}
                    },
                    Ok(_) => {}
                    Err(error) => break Err(error),
                }
            };
            disable_raw_mode().context("Failed to restore terminal mode")?;
            let character = result.context("Failed to read terminal input")?;
            println!("{character}");
            character
        } else {
            let mut input = String::new();
            io::stdin().read_line(&mut input)?;
            input.chars().next().unwrap_or_default()
        };
        match character.to_ascii_lowercase() {
            'a' => return Ok(ReviewChoice::Accept),
            'r' => return Ok(ReviewChoice::Reject),
            'c' if allow_cancel => return Ok(ReviewChoice::Cancel),
            _ => println!(
                "Please press {}.",
                if allow_cancel { "a, r, or c" } else { "a or r" }
            ),
        }
    }
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
