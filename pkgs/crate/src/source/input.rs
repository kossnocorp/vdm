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
            "Unsupported source {input:?}; expected gh:owner/repo/path[@ref], git:<repository-url>//path[@ref], or an HTTP URL"
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sources_in_order() {
        assert!(
            VdmSourceInput::parse_target("gh:js-fns/js-fns/vitest.config.ts@main")
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
    fn restores_targets_from_manifest_entries() {
        let github = VdmSourceInput::parse_manifest_target(
            &VdmManifestTargetUrl::new("gh:js-fns/js-fns/vitest.config.ts"),
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
        assert_eq!(http.key(), &key);
        assert_eq!(http.version().as_str(), hash);
        assert!(
            VdmSourceInput::parse_manifest_target(&key, &VdmManifestSourceVersion::new("other"))
                .is_err()
        );
    }
}
