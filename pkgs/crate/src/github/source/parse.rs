use crate::prelude::*;

use chumsky::prelude::*;

impl VdmGithubSource {
    pub(super) fn parse_target(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        let Some(input) = Self::github_input(input)? else {
            return Ok(None);
        };

        let (owner, repo, path, version) = Self::parse(&input).with_context(|| {
            format!("Invalid GitHub target {input:?}; expected gh:owner/repo:path[@version]")
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
        let path = VdmGitSource::normalize_path(path)?;
        VdmGitSource::validate_git_path_and_version(&path, version)?;

        let owner = owner.to_owned();
        let repo = repo.strip_suffix(".git").unwrap_or(repo).to_owned();
        ensure!(
            !repo.is_empty() && repo != "." && repo != "..",
            "Invalid GitHub repository name"
        );
        let path = VdmManifestTargetPath::new(path);
        let version = VdmManifestSourceVersion::new(version);

        let target = VdmGitTarget::new_github(owner, repo, path, version);
        target.glob()?;
        Ok(Some(Box::new(target)))
    }

    fn github_input(input: &str) -> Result<Option<String>> {
        let input = if input.starts_with("git:") && !input.starts_with("git://") {
            &input[4..]
        } else {
            input
        };
        let source = if let Some(source) = input.strip_prefix("gh:") {
            source
        } else if let Some(source) = input.strip_prefix("github.com/") {
            source
        } else if let Some(source) = input
            .strip_prefix("git@github.com:")
            .or_else(|| input.strip_prefix("git@github.com/"))
        {
            source
        } else if let Some((scheme, rest)) = input.split_once("://") {
            if !matches!(scheme, "https" | "http" | "ssh" | "git") {
                return Ok(None);
            }
            let Some((authority, source)) = rest.split_once('/') else {
                return Ok(None);
            };
            let url = Url::parse(&format!("{scheme}://{authority}/"))?;
            if url.host_str() != Some("github.com") {
                return Ok(None);
            }
            source
        } else {
            return Ok(None);
        };
        let source = if source.contains(':') {
            source
        } else {
            source.trim_end_matches('/')
        };
        Ok(Some(format!("gh:{source}")))
    }

    fn parse(input: &str) -> Option<(&str, &str, &str, Option<&str>)> {
        let component = any::<_, extra::Default>()
            .filter(|c: &char| *c != '/' && *c != '@' && *c != ':')
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
        let path = version.not().ignore_then(any()).repeated().to_slice();
        just("gh:")
            .ignore_then(component)
            .then_ignore(just('/'))
            .then(component)
            .then(just(':').ignore_then(path).or_not())
            .then(version.or_not())
            .map(|(((owner, repo), path), version)| (owner, repo, path.unwrap_or(""), version))
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
            .parse("gh:js-fns/js-fns:vitest.config.ts@main")
            .unwrap()
            .unwrap();
        assert_eq!(
            target.key(),
            &VdmManifestTargetUrl::new("gh:js-fns/js-fns:vitest.config.ts")
        );
        assert_eq!(target.version(), &VdmManifestSourceVersion::new("main"));
        assert_eq!(
            target.vendor_path(),
            Path::new("@gh/js-fns/js-fns/vitest.config.ts")
        );
        assert!(
            VDM_GITHUB_SOURCE
                .parse("js-fns/js-fns/file@main")
                .unwrap()
                .is_none()
        );
        assert!(
            VDM_GITHUB_SOURCE
                .parse("gh:js-fns/js-fns:../secret@main")
                .is_err()
        );
    }

    #[test]
    fn parses_colon_separated_github_targets() {
        for suffix in ["", "@main"] {
            let target = VDM_GITHUB_SOURCE
                .parse(&format!(
                    "gh:kossnocorp/js-fns:pkgs/dev/src/config/tsconfig.json{suffix}"
                ))
                .unwrap()
                .unwrap();
            assert_eq!(
                target.key().as_str(),
                "gh:kossnocorp/js-fns:pkgs/dev/src/config/tsconfig.json"
            );
            assert_eq!(
                target.version().as_str(),
                if suffix.is_empty() { "HEAD" } else { "main" }
            );
        }
    }

    #[test]
    fn parses_github_paths_and_optional_refs() {
        let glob = VDM_GITHUB_SOURCE
            .parse("gh:omacom/omarchy:**/*.sh")
            .unwrap()
            .unwrap();
        assert_eq!(glob.key().as_str(), "gh:omacom/omarchy:**/*.sh");
        assert_eq!(glob.version().as_str(), "HEAD");
        assert!(VDM_GITHUB_SOURCE.parse("gh:owner/repo:[broken").is_err());
        assert_eq!(
            VdmGithubSource::parse("gh:owner/repo:path/file"),
            Some(("owner", "repo", "path/file", None))
        );
        assert_eq!(
            VdmGithubSource::parse("gh:owner/repo:@scope/file@feature/branch"),
            Some(("owner", "repo", "@scope/file", Some("feature/branch")))
        );
        for input in [
            "gh:/repo/file",
            "gh:owner//file",
            "gh:owner/repo/",
            "gh:owner/repo/file",
        ] {
            assert!(VdmGithubSource::parse(input).is_none(), "{input}");
        }
    }
}
