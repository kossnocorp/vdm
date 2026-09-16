use crate::prelude::*;

use chumsky::prelude::*;

impl VdmGithubSource {
    pub(super) fn parse_target(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        if !input.starts_with("gh:") {
            return Ok(None);
        }

        let (owner, repo, path, version) = Self::parse(input).with_context(|| {
            format!("Invalid GitHub target {input:?}; expected gh:owner/repo/path[@version]")
        })?;
        ensure!(!path.ends_with('@'), "GitHub version must not be empty");
        let version = version.unwrap_or("HEAD");

        ensure!(!owner.is_empty(), "Repository owner must not be empty");
        ensure!(!repo.is_empty(), "Repository name must not be empty");
        ensure!(
            [owner, repo].iter().all(|part| matches!(
                Path::new(part).components().next(),
                Some(Component::Normal(_))
            )),
            "Repository owner and name must be safe path components"
        );
        ensure!(!path.is_empty(), "Repository file path must not be empty");
        ensure!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "Repository file path must not contain absolute or traversal components"
        );

        let valid_ref = version.len() == 40 && version.bytes().all(|byte| byte.is_ascii_hexdigit())
            || git2::Reference::is_valid_name(&format!("refs/heads/{version}"));
        ensure!(valid_ref, "Version must be a valid Git ref or commit SHA");

        let owner = owner.to_owned();
        let repo = repo.to_owned();
        let path = VdmManifestTargetPath::new(path);
        let version = VdmManifestSourceVersion::new(version);

        let target = VdmGitHubTarget::new(owner, repo, path, version);
        target.glob()?;
        Ok(Some(Box::new(target)))
    }

    fn parse(input: &str) -> Option<(&str, &str, &str, Option<&str>)> {
        let component = any::<_, extra::Default>()
            .filter(|c: &char| *c != '/' && *c != '@')
            .repeated()
            .at_least(1)
            .to_slice();
        let version = just('@')
            .ignore_then(
                any()
                    .filter(|c: &char| *c != '@')
                    .repeated()
                    .at_least(1)
                    .to_slice(),
            )
            .then_ignore(end());
        let path = version
            .not()
            .ignore_then(any())
            .repeated()
            .at_least(1)
            .to_slice();
        just("gh:")
            .ignore_then(component)
            .then_ignore(just('/'))
            .then(component)
            .then_ignore(just('/'))
            .then(path)
            .then(version.or_not())
            .map(|(((owner, repo), path), version)| (owner, repo, path, version))
            .parse(input)
            .into_result()
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_prefixed_github_target() {
        let target = VDM_GITHUB_SOURCE
            .parse("gh:js-fns/js-fns/vitest.config.ts@main")
            .unwrap()
            .unwrap();
        assert_eq!(
            target.key(),
            &VdmManifestTargetUrl::new("gh:js-fns/js-fns/vitest.config.ts")
        );
        assert_eq!(target.version(), &VdmManifestSourceVersion::new("main"));
        assert_eq!(
            target.vendor_path(),
            Path::new("@js-fns/js-fns/vitest.config.ts")
        );
        assert!(
            VDM_GITHUB_SOURCE
                .parse("js-fns/js-fns/file@main")
                .unwrap()
                .is_none()
        );
        assert!(
            VDM_GITHUB_SOURCE
                .parse("gh:js-fns/js-fns/../secret@main")
                .is_err()
        );
    }

    #[test]
    fn parses_github_paths_and_optional_refs() {
        let glob = VDM_GITHUB_SOURCE
            .parse("gh:omacom/omarchy/**/*.sh")
            .unwrap()
            .unwrap();
        assert_eq!(glob.key().as_str(), "gh:omacom/omarchy/**/*.sh");
        assert_eq!(glob.version().as_str(), "HEAD");
        assert!(VDM_GITHUB_SOURCE.parse("gh:owner/repo/[broken").is_err());
        assert_eq!(
            VdmGithubSource::parse("gh:owner/repo/path/file"),
            Some(("owner", "repo", "path/file", None))
        );
        assert_eq!(
            VdmGithubSource::parse("gh:owner/repo/@scope/file@feature/branch"),
            Some(("owner", "repo", "@scope/file", Some("feature/branch")))
        );
        for input in [
            "gh:/repo/file",
            "gh:owner//file",
            "gh:owner/repo/",
            "gh:owner/repo",
        ] {
            assert!(VdmGithubSource::parse(input).is_none(), "{input}");
        }
    }
}
