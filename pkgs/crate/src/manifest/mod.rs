use crate::prelude::*;

mod source;
pub use source::*;

mod target;
pub use target::*;

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VitManifest {
    #[serde(default)]
    sources: BTreeMap<VitManifestTargetUrl, VitManifestSource>,
}

impl VitManifest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, url: &VitManifestTargetUrl, version: &VitManifestSourceVersion) {
        self.sources.insert(
            url.clone(),
            VitManifestSource::File(VitManifestSourceFile::Version(version.clone())),
        );
    }

    pub fn update(
        &mut self,
        url: &VitManifestTargetUrl,
        version: &VitManifestSourceVersion,
    ) -> Result<()> {
        for (base, source) in &mut self.sources {
            match source {
                VitManifestSource::File(file) if base == url => {
                    file.set_version(version);
                    return Ok(());
                }
                VitManifestSource::Files(files) => {
                    if files.set_version(base, url, version)? {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        bail!("{url} is not present in the manifest")
    }

    pub fn iter_targets(&self) -> impl Iterator<Item = Result<Box<dyn VitTarget>>> + '_ {
        self.sources.iter().flat_map(|(url, source)| match source {
            VitManifestSource::File(file) => {
                vec![VitSourceInput::parse_manifest_target(url, file.version())]
            }

            VitManifestSource::Files(source) => source
                .iter_versions()
                .map(|(path, version)| {
                    let url = url.join(path)?;
                    VitSourceInput::parse_manifest_target(&url, version)
                })
                .collect(),
        })
    }

    pub fn targets(&self) -> Result<BTreeMap<VitManifestTargetUrl, Box<dyn VitTarget>>> {
        let mut targets = BTreeMap::new();
        for target in self.iter_targets() {
            let target = target?;
            ensure!(
                !targets.contains_key(target.key()),
                "Manifest target {:?} is defined more than once",
                target.key()
            );
            targets.insert(target.key().clone(), target);
        }
        Ok(targets)
    }
}

impl VitFileToml for VitManifest {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_all_manifest_source_forms() {
        let manifest: VitManifest = toml::from_str(
            r#"
[sources]
"gh:kossnocorp/dev/README.md" = "main"
"gh:kossnocorp/dev/LICENSE" = { version = "v1" }

[sources."gh:kossnocorp/dev"]
version = "v2"
files = [
  "mise.toml",
  { path = "package.json", version = "v3" },
]
"#,
        )
        .unwrap();

        assert!(matches!(
            manifest
                .sources
                .get(&VitManifestTargetUrl::new("gh:kossnocorp/dev")),
            Some(VitManifestSource::Files(_))
        ));

        let targets = manifest.targets().unwrap();
        assert_eq!(targets.len(), 4);
        assert_eq!(
            targets[&VitManifestTargetUrl::new("gh:kossnocorp/dev/README.md")].version(),
            &VitManifestSourceVersion::new("main")
        );
        assert_eq!(
            targets[&VitManifestTargetUrl::new("gh:kossnocorp/dev/LICENSE")].version(),
            &VitManifestSourceVersion::new("v1")
        );
        assert_eq!(
            targets[&VitManifestTargetUrl::new("gh:kossnocorp/dev/mise.toml")].version(),
            &VitManifestSourceVersion::new("v2")
        );
        assert_eq!(
            targets[&VitManifestTargetUrl::new("gh:kossnocorp/dev/package.json")].version(),
            &VitManifestSourceVersion::new("v3")
        );
    }

    #[test]
    fn rejects_duplicate_expanded_targets() {
        let manifest: VitManifest = toml::from_str(
            r#"
[sources]
"gh:kossnocorp/dev/mise.toml" = "main"

[sources."gh:kossnocorp/dev"]
version = "main"
files = ["mise.toml"]
"#,
        )
        .unwrap();

        assert!(
            manifest
                .targets()
                .err()
                .unwrap()
                .to_string()
                .contains("defined more than once")
        );
    }

    #[test]
    fn rejects_unsafe_grouped_paths() {
        let manifest: VitManifest = toml::from_str(
            r#"
[sources."gh:kossnocorp/dev"]
version = "main"
files = ["../secret"]
"#,
        )
        .unwrap();

        assert!(
            manifest
                .targets()
                .err()
                .unwrap()
                .to_string()
                .contains("must be relative")
        );
    }

    #[test]
    fn rejects_the_old_files_manifest() {
        assert!(
            toml::from_str::<VitManifest>(
                r#"
[files]
"gh:kossnocorp/dev/mise.toml" = "main"
"#,
            )
            .is_err()
        );
    }

    #[test]
    fn add_serializes_a_direct_scalar_source() {
        let mut manifest = VitManifest::new();
        manifest.add(
            &VitManifestTargetUrl::new("gh:kossnocorp/dev/mise.toml"),
            &VitManifestSourceVersion::new("main"),
        );

        let source = toml::to_string_pretty(&manifest).unwrap();
        assert_eq!(
            source,
            "[sources]\n\"gh:kossnocorp/dev/mise.toml\" = \"main\"\n"
        );
    }

    #[test]
    fn updates_direct_and_grouped_target_versions() {
        let mut manifest: VitManifest = toml::from_str(
            r#"
[sources]
"gh:kossnocorp/dev/README.md" = "old"

[sources."gh:kossnocorp/dev"]
version = "old"
files = ["mise.toml", { path = "package.json", version = "older" }]
"#,
        )
        .unwrap();
        let version = VitManifestSourceVersion::new("main");
        manifest
            .update(
                &VitManifestTargetUrl::new("gh:kossnocorp/dev/README.md"),
                &version,
            )
            .unwrap();
        manifest
            .update(
                &VitManifestTargetUrl::new("gh:kossnocorp/dev/mise.toml"),
                &version,
            )
            .unwrap();
        manifest
            .update(
                &VitManifestTargetUrl::new("gh:kossnocorp/dev/package.json"),
                &version,
            )
            .unwrap();

        let targets = manifest.targets().unwrap();
        assert!(targets.values().all(|target| target.version() == &version));
    }
}
