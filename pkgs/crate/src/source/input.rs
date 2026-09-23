use crate::prelude::*;

pub struct VdmSourceInput;

impl VdmSourceInput {
    pub fn parse_target(input: &str) -> Result<Box<dyn VdmTarget>> {
        let sources: [&'static dyn VdmSource; 3] =
            [&VDM_GITHUB_SOURCE, &VDM_GIT_SOURCE, &VDM_HTTP_SOURCE];
        for source in sources {
            if let Some(target) = source.parse(input)? {
                return Ok(target);
            }
        }

        bail!(
            "Unsupported source {input:?}; expected gh:owner/repo:path[@ref], <repository-url>:path[@ref], or an HTTP URL"
        )
    }

    pub fn parse_manifest_target(
        key: &VdmManifestTargetUrl,
        version: &VdmManifestSourceVersion,
    ) -> Result<Box<dyn VdmTarget>> {
        let input = format!("{key}@{version}");
        let target = Self::parse_target(&input)
            .with_context(|| format!("Invalid manifest target {key:?}"))?;
        ensure!(
            target.version() == version,
            "Manifest version {version:?} does not match source {key:?}"
        );
        Ok(target)
    }

    pub(crate) fn normalize_base(input: &str) -> Result<VdmManifestTargetUrl> {
        for source in [&VDM_GITHUB_SOURCE as &dyn VdmSource, &VDM_GIT_SOURCE] {
            if let Some(target) = source.parse(input)? {
                return Ok(target.key().clone());
            }
        }
        let url = VdmHttpSource::normalize_url(input)?.context("Unsupported manifest source")?;
        Ok(VdmManifestTargetUrl::new(format!(
            "http:{}",
            url.as_str().trim_end_matches('/')
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_source_aliases_and_repository_paths() {
        for repository in [
            "gh:kossnocorp/genotype",
            "gh:kossnocorp/genotype.git",
            "https://github.com/kossnocorp/genotype.git",
            "github.com/kossnocorp/genotype.git",
            "git@github.com:kossnocorp/genotype.git",
            "git@github.com/kossnocorp/genotype.git",
            "ssh://git@github.com/kossnocorp/genotype.git",
            "git:https://github.com/kossnocorp/genotype.git",
        ] {
            for (suffix, key) in [
                ("", "gh:kossnocorp/genotype"),
                (":/src//nested/.././", "gh:kossnocorp/genotype:src"),
            ] {
                let target =
                    VdmSourceInput::parse_target(&format!("{repository}{suffix}@main")).unwrap();
                assert_eq!(target.key().as_str(), key);
                assert_eq!(target.version().as_str(), "main");
                assert_eq!(
                    VdmSourceInput::parse_target(&format!("{key}@main"))
                        .unwrap()
                        .key(),
                    target.key()
                );
            }
        }
        for repository in [
            "git://git.git.savannah.gnu.org/bash.git",
            "https://https.git.savannah.gnu.org/git/bash.git",
            "ssh://git.savannah.gnu.org/srv/git/bash.git",
        ] {
            for prefix in ["", "git:"] {
                let root = VdmSourceInput::parse_target(&format!("{prefix}{repository}")).unwrap();
                assert_eq!(root.key().as_str(), format!("git:{repository}"));
                for path in [
                    "tests/",
                    "tests////",
                    "tests/./",
                    "tests/nested/dir/../..",
                    "/tests",
                    "tests",
                ] {
                    let target = VdmSourceInput::parse_target(&format!(
                        "{prefix}{repository}:{path}@master"
                    ))
                    .unwrap();
                    assert_eq!(target.key().as_str(), format!("git:{repository}:tests"));
                    assert_eq!(target.version().as_str(), "master");
                }
            }
        }
        let explicit = "git:https://example.com/does/not/look/like/git/endpoint/at/all.png";
        assert_eq!(
            VdmSourceInput::parse_target(explicit)
                .unwrap()
                .key()
                .as_str(),
            explicit
        );
        for url in [
            "https://cdn.jsdelivr.net/gh/devicons/devicon@latest/icons/github/github-original.svg",
            "https://github.com/owner/repo.git",
            "https://example.com/repo.git",
        ] {
            let explicit = format!("http:{url}");
            let target = VdmSourceInput::parse_target(&explicit).unwrap();
            assert!(target.as_any().is::<VdmHttpTarget>());
            assert_eq!(target.key().as_str(), explicit);
        }
    }

    #[test]
    fn parses_sources_in_order() {
        assert!(
            VdmSourceInput::parse_target("gh:js-fns/js-fns:vitest.config.ts@main")
                .unwrap()
                .as_any()
                .is::<VdmGitTarget>()
        );
        assert!(
            VdmSourceInput::parse_target("https://example.com/assets/file.js")
                .unwrap()
                .as_any()
                .is::<VdmHttpTarget>()
        );
        assert!(VdmSourceInput::parse_target("js-fns/js-fns/file.js@main").is_err());
    }

    #[test]
    fn keeps_http_urls_distinct_from_git_targets() {
        let hash = "a".repeat(64);
        for input in [
            "https://example.com:8443/assets/file.js".to_owned(),
            "https://[::1]:8443/assets/file.js".to_owned(),
            "https://example.com/file.js?url=https://other.com/file".to_owned(),
            "https://example.com/file.js#section:one".to_owned(),
            format!("https://example.com/file.js@sha256:{hash}"),
        ] {
            let target = VdmSourceInput::parse_target(&input).unwrap();
            assert!(target.as_any().is::<VdmHttpTarget>(), "{input}");
        }
    }

    #[test]
    fn restores_targets_from_manifest_entries() {
        let github = VdmSourceInput::parse_manifest_target(
            &VdmManifestTargetUrl::new("gh:js-fns/js-fns:vitest.config.ts"),
            &VdmManifestSourceVersion::new("main"),
        )
        .unwrap();
        assert_eq!(github.version(), &VdmManifestSourceVersion::new("main"));

        let url = "https://example.com/assets/file.js";
        let key = VdmManifestTargetUrl::new(url);
        let hash = format!("sha256:{}", "a".repeat(64));
        let http =
            VdmSourceInput::parse_manifest_target(&key, &VdmManifestSourceVersion::new(&hash))
                .unwrap();
        assert_eq!(http.key().as_str(), format!("http:{url}"));
        assert_eq!(http.version().as_str(), hash);
        assert!(
            VdmSourceInput::parse_manifest_target(&key, &VdmManifestSourceVersion::new("other"))
                .is_err()
        );
    }
}
