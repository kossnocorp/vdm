use crate::prelude::*;

impl VdmGitSource {
    pub(super) fn parse_target(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        let explicit = input.starts_with("git:") && !input.starts_with("git://");
        let input = if explicit { &input[4..] } else { input };
        let Some((scheme, _)) = input.split_once("://") else {
            ensure!(!explicit, "Invalid Git repository URL");
            return Ok(None);
        };
        if !matches!(scheme, "http" | "https" | "ssh" | "git" | "file") {
            ensure!(!explicit, "Unsupported Git repository URL scheme");
            return Ok(None);
        }
        let (repository, path, version) =
            if let Some((repository, path)) = Self::split_repository_path(input) {
                let (path, version) = Self::split_version(path);
                (repository, path, version)
            } else {
                let (repository, version) = Self::split_url_version(input);
                if !explicit
                    && matches!(scheme, "http" | "https")
                    && !repository.trim_end_matches('/').ends_with(".git")
                {
                    return Ok(None);
                }
                (repository, "", version)
            };
        let url = Url::parse(repository).context("Invalid Git repository URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https" | "ssh" | "git" | "file"),
            "Unsupported Git repository URL scheme"
        );
        ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "Git repository URL must not contain a query or fragment"
        );
        ensure!(
            url.scheme() == "file" || url.host_str().is_some(),
            "Git repository URL must have a host"
        );
        ensure!(
            !url.path().trim_matches('/').is_empty(),
            "Git repository URL must have a repository path"
        );
        let path = Self::normalize_path(path)?;
        Self::validate_git_path_and_version(&path, version)?;
        let target = VdmGitTarget::new_git(
            url.as_str().trim_end_matches('/').to_owned(),
            VdmManifestTargetPath::new(path),
            VdmManifestSourceVersion::new(version),
        );
        target.glob()?;
        Ok(Some(Box::new(target)))
    }

    pub(crate) fn split_version(input: &str) -> (&str, &str) {
        match input.rsplit_once('@') {
            Some((path, version)) if !path.is_empty() => (path, version),
            _ => (input, "HEAD"),
        }
    }

    fn split_url_version(input: &str) -> (&str, &str) {
        let start = input.find("://").unwrap() + 3;
        let Some(path_start) = input[start..].find('/').map(|offset| start + offset) else {
            return (input, "HEAD");
        };
        match input.rsplit_once('@') {
            Some((repository, version)) if repository.len() > path_start => (repository, version),
            _ => (input, "HEAD"),
        }
    }

    pub(crate) fn normalize_path(path: &str) -> Result<String> {
        let mut parts = Vec::new();
        for part in path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    ensure!(
                        parts.pop().is_some(),
                        "Repository path must not escape the repository"
                    );
                }
                part => parts.push(part),
            }
        }
        // Folder globs retain their recursive meaning in canonical form.
        if path.ends_with('/') && parts.iter().any(|part| part.contains(['*', '?', '[', '{'])) {
            parts.push("**");
        }
        Ok(parts.join("/"))
    }

    pub(crate) fn split_repository_path(input: &str) -> Option<(&str, &str)> {
        // Look only after the authority, skipping scheme, SSH user and port colons.
        let (scheme, rest) = input.split_once("://")?;
        let start = scheme.len() + 3 + rest.find('/')?;
        let path = input[start..].split(['?', '#']).next()?;
        let separator = start + path.find(':')?;
        let (repository, path) = input.split_at(separator);
        // HTTP content versions are not Git path separators.
        if repository.ends_with("@sha256") {
            return None;
        }
        Some((repository, &path[1..]))
    }

    pub(crate) fn validate_git_path_and_version(path: &str, version: &str) -> Result<()> {
        ensure!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "Repository file path must not contain absolute or traversal components"
        );
        let valid_ref = version.len() == 40 && version.bytes().all(|byte| byte.is_ascii_hexdigit())
            || git2::Reference::is_valid_name(&format!("refs/heads/{version}"));
        ensure!(
            !version.is_empty() && valid_ref,
            "Version must be a valid Git ref or commit SHA"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repository_urls_and_round_trips_manifest_entries() {
        for repository in [
            "https://gitlab.com/group/subgroup/repo.git",
            "ssh://git@example.com:2222/team/repo.git",
            "ssh://git@[::1]:2222/team/repo.git",
            "https://example.com:8443/team/repo.git",
            "git://example.com/repo",
            "file:///tmp/repo",
        ] {
            for (path, version) in [
                ("src/**/*.rs", "feature/branch"),
                ("@scope/file.ts", "v1.0"),
                ("file.txt", "HEAD"),
                ("tests", "master"),
                ("tests/", "HEAD"),
            ] {
                let key = VdmManifestTargetUrl::new(format!("{repository}:{path}"));
                let target = VdmSourceInput::parse_manifest_target(
                    &key,
                    &VdmManifestSourceVersion::new(version),
                )
                .unwrap();
                assert_eq!(
                    target.key().as_str(),
                    format!("git:{repository}:{}", path.trim_end_matches('/'))
                );
                assert_eq!(target.version().as_str(), version);
                let git = target.as_any().downcast_ref::<VdmGitTarget>().unwrap();
                assert_eq!(git.repository_url(), repository);
                assert_eq!(git.path(), path.trim_end_matches('/'));
                assert!(target.vendor_path().starts_with("@git"));
            }
            let target = VdmSourceInput::parse_target(&format!("{repository}:file.txt")).unwrap();
            assert_eq!(target.version().as_str(), "HEAD");
        }
    }

    #[test]
    fn parses_savannah_targets() {
        for (suffix, path, version) in [
            ("tests/", "tests", "HEAD"),
            ("tests", "tests", "HEAD"),
            ("tests/**/*.sh", "tests/**/*.sh", "HEAD"),
            ("tests@master", "tests", "master"),
        ] {
            let input = format!("git://git.git.savannah.gnu.org/bash.git:{suffix}");
            let parsed = VdmSourceInput::parse_target(&input).unwrap();
            let target = parsed.as_any().downcast_ref::<VdmGitTarget>().unwrap();
            assert_eq!(
                target.repository_url(),
                "git://git.git.savannah.gnu.org/bash.git"
            );
            assert_eq!(target.path(), path);
            assert_eq!(target.version().as_str(), version);
        }
    }

    #[test]
    fn expands_grouped_git_manifest_sources() {
        for base in [
            "https://example.com/team/repo.git",
            "https://example.com/team/repo.git:",
            "https://example.com/team/repo.git:src",
        ] {
            let manifest: VdmManifest = toml::from_str(&format!(
                "[sources.{base:?}]\nversion = 'main'\nfiles = ['file.ts']\n"
            ))
            .unwrap();
            let targets = manifest.targets().unwrap();
            let target = targets
                .values()
                .next()
                .unwrap()
                .as_any()
                .downcast_ref::<VdmGitTarget>()
                .unwrap();
            assert_eq!(target.repository_url(), "https://example.com/team/repo.git");
            assert_eq!(
                target.path(),
                if base.ends_with("src") {
                    "src/file.ts"
                } else {
                    "file.ts"
                }
            );
            assert_eq!(target.version().as_str(), "main");
        }
    }

    #[test]
    fn rejects_invalid_git_targets() {
        for input in [
            "git:invalid",
            "git:https://example.com/",
            "https://example.com/repo.git:../secret",
            "https://example.com/repo.git:file@",
            "https://example.com/repo.git:file@bad..ref",
            "https://example.com/repo.git:[broken",
            "ftp://example.com/repo.git:file",
        ] {
            assert!(VdmSourceInput::parse_target(input).is_err(), "{input}");
        }
    }
}
