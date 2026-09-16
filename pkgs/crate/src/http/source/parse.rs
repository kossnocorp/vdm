use crate::prelude::*;
use chumsky::prelude::*;

impl VdmHttpSource {
    pub(super) fn parse_target(&self, input: &str) -> Result<Option<Box<dyn VdmTarget>>> {
        if !input.starts_with("http://") && !input.starts_with("https://") {
            return Ok(None);
        }

        let (input, version) = Self::parse(input);

        let url = Url::parse(input).with_context(|| format!("invalid HTTP URL {input:?}"))?;
        let host = url.host_str().context("HTTP URL must include a host")?;
        let path = url.path().trim_start_matches('/');

        ensure!(!path.is_empty(), "HTTP URL must identify a file");
        ensure!(
            !path.ends_with('/'),
            "HTTP URL must not end with a directory"
        );
        ensure!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
            "HTTP URL path must not contain traversal components"
        );

        let authority = match url.port() {
            Some(port) => format!("{host}_{port}"),
            None => host.to_owned(),
        };

        let key = VdmManifestTargetUrl::new(&url);
        let version =
            VdmManifestSourceVersion::new(version.unwrap_or_default().to_ascii_lowercase());
        let vendor_path = PathBuf::from("@http").join(authority).join(path);

        Ok(Some(Box::new(VdmHttpTarget::new(
            url,
            key,
            version,
            vendor_path,
        ))))
    }

    fn parse(input: &str) -> (&str, Option<&str>) {
        let hash = just::<_, _, extra::Default>("sha256:")
            .then(
                any()
                    .filter(|c: &char| c.is_ascii_hexdigit())
                    .repeated()
                    .exactly(64),
            )
            .to_slice();

        let suffix = just('@').ignore_then(hash).then_ignore(end());

        let parser = suffix
            .not()
            .ignore_then(any())
            .repeated()
            .to_slice()
            .then(suffix.or_not());

        parser
            .parse(input)
            .into_result()
            .expect("HTTP suffix parser accepts any input")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_direct_http_urls_only() {
        let target = VDM_HTTP_SOURCE
            .parse("https://example.com/assets/file.js?raw=1")
            .unwrap()
            .unwrap();
        assert_eq!(
            target.key(),
            &VdmManifestTargetUrl::new("https://example.com/assets/file.js?raw=1")
        );
        assert_eq!(
            target.vendor_path(),
            Path::new("@http/example.com/assets/file.js")
        );
        assert!(
            VDM_HTTP_SOURCE
                .parse("gh:owner/repo/file@main")
                .unwrap()
                .is_none()
        );
        assert!(VDM_HTTP_SOURCE.parse("https://example.com/").is_err());
    }

    #[test]
    fn recognizes_only_complete_trailing_http_hashes() {
        let url =
            "https://cdn.jsdelivr.net/gh/devicons/devicon@latest/icons/github/github-original.svg";
        let hash = format!("sha256:{}", "aB09".repeat(16));
        let pinned = format!("{url}@{hash}");
        assert_eq!(VdmHttpSource::parse(&pinned), (url, Some(hash.as_str())));
        for suffix in [
            String::new(),
            "@latest".into(),
            "@sha256:".into(),
            format!("@sha256:{}", "a".repeat(63)),
            format!("@sha256:{}", "a".repeat(65)),
            format!("@sha256:{}g", "a".repeat(63)),
            format!("@sha512:{}", "a".repeat(64)),
            format!("@{hash}/file"),
            format!("@{hash}?raw=1"),
        ] {
            let input = format!("{url}{suffix}");
            assert_eq!(VdmHttpSource::parse(&input), (input.as_str(), None));
        }
        let url = "https://user@example.com/@scope/pkg@latest/file?value=@foo#@bar";
        let pinned = format!("{url}@{hash}");
        assert_eq!(VdmHttpSource::parse(&pinned), (url, Some(hash.as_str())));
    }
}
