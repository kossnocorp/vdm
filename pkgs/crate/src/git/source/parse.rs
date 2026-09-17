use crate::prelude::*;

impl VdmGitSource {
    pub(super) fn parse_target(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        let Some(input) = input.strip_prefix("git:") else {
            return Ok(None);
        };
        // Skip the URL's scheme delimiter; the next // separates repository
        // and file path, even for nested repositories and SSH usernames.
        let scheme_end = input
            .find("://")
            .context("Invalid Git target; expected git:<repository-url>//path[@ref]")?
            + 3;
        let separator = input[scheme_end..]
            .find("//")
            .map(|offset| scheme_end + offset)
            .context("Git target must separate repository URL and file path with //")?;
        let (repository, path) = input.split_at(separator);
        let path = &path[2..];
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
        let (path, version) = match path.rsplit_once('@') {
            Some((path, version)) if !path.is_empty() => (path, version),
            _ => (path, "HEAD"),
        };
        Self::validate_git_path_and_version(path, version)?;
        let target = VdmGitTarget::new_git(
            repository.to_owned(),
            VdmManifestTargetPath::new(path),
            VdmManifestSourceVersion::new(version),
        );
        target.glob()?;
        Ok(Some(Box::new(target)))
    }

    pub(crate) fn validate_git_path_and_version(path: &str, version: &str) -> Result<()> {
        ensure!(!path.is_empty(), "Repository file path must not be empty");
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
            "git://example.com/repo",
            "file:///tmp/repo",
        ] {
            for (path, version) in [
                ("src/**/*.rs", "feature/branch"),
                ("@scope/file.ts", "v1.0"),
                ("file.txt", "HEAD"),
            ] {
                let key = VdmManifestTargetUrl::new(format!("git:{repository}//{path}"));
                let target = VdmSourceInput::parse_manifest_target(
                    &key,
                    &VdmManifestSourceVersion::new(version),
                )
                .unwrap();
                assert_eq!(target.key(), &key);
                assert_eq!(target.version().as_str(), version);
                let git = target.as_any().downcast_ref::<VdmGitTarget>().unwrap();
                assert_eq!(git.repository_url(), repository);
                assert_eq!(git.path(), path);
                assert!(target.vendor_path().starts_with("@git"));
            }
            let target =
                VdmSourceInput::parse_target(&format!("git:{repository}//file.txt")).unwrap();
            assert_eq!(target.version().as_str(), "HEAD");
        }
    }

    #[test]
    fn expands_grouped_git_manifest_sources() {
        for base in [
            "git:https://example.com/team/repo.git",
            "git:https://example.com/team/repo.git//",
            "git:https://example.com/team/repo.git//src",
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
            "git:https://example.com/repo.git",
            "git:https://example.com/repo.git//",
            "git:https://example.com/repo.git//../secret",
            "git:https://example.com/repo.git///absolute",
            "git:https://example.com/repo.git//file@",
            "git:https://example.com/repo.git//file@bad..ref",
            "git:https://example.com/repo.git//[broken",
            "git:ftp://example.com/repo.git//file",
            "git:https://example.com/repo.git?query//file",
        ] {
            assert!(VdmSourceInput::parse_target(input).is_err(), "{input}");
        }
    }
}
