use crate::prelude::*;

mod source;
pub use source::*;

mod target;
pub use target::*;

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(try_from = "ManifestDocument")]
pub struct VdmManifest {
    #[serde(default)]
    sources: BTreeMap<VdmManifestTargetUrl, VdmManifestSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    #[serde(default)]
    sources: BTreeMap<VdmManifestTargetUrl, VdmManifestSource>,
}

impl TryFrom<ManifestDocument> for VdmManifest {
    type Error = Error;

    fn try_from(document: ManifestDocument) -> Result<Self> {
        let mut sources = BTreeMap::new();
        for (key, source) in document.sources {
            let key = match &source {
                VdmManifestSource::File(file) => {
                    VdmSourceInput::parse_manifest_target(&key, file.version())?
                        .key()
                        .clone()
                }
                VdmManifestSource::Files(_) => VdmSourceInput::normalize_base(key.as_str())?,
            };
            ensure!(
                !sources.contains_key(&key),
                "Manifest source {key} is defined more than once"
            );
            sources.insert(key, source);
        }
        Ok(Self { sources })
    }
}

impl VdmManifest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, url: &VdmManifestTargetUrl, version: &VdmManifestSourceVersion) {
        self.sources.insert(
            url.clone(),
            VdmManifestSource::File(VdmManifestSourceFile::Version(version.clone())),
        );
    }

    pub fn update(
        &mut self,
        url: &VdmManifestTargetUrl,
        version: &VdmManifestSourceVersion,
    ) -> Result<()> {
        for (base, source) in &mut self.sources {
            match source {
                VdmManifestSource::File(file) if base == url => {
                    file.set_version(version);
                    return Ok(());
                }
                VdmManifestSource::Files(files) => {
                    if files.set_version(base, url, version)? {
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        bail!("{url} is not present in the manifest")
    }

    pub fn iter_targets(&self) -> impl Iterator<Item = Result<Box<dyn VdmTarget>>> + '_ {
        self.sources.iter().flat_map(|(url, source)| match source {
            VdmManifestSource::File(file) => {
                vec![VdmSourceInput::parse_manifest_target(url, file.version())]
            }

            VdmManifestSource::Files(source) => source
                .iter_versions()
                .map(|(path, version)| {
                    let url = url.join(path)?;
                    VdmSourceInput::parse_manifest_target(&url, version)
                })
                .collect(),
        })
    }

    pub fn targets(&self) -> Result<BTreeMap<VdmManifestTargetUrl, Box<dyn VdmTarget>>> {
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

impl VdmFileToml for VdmManifest {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_manifest_sources_before_matching_and_serializing() {
        let mut manifest: VdmManifest = toml::from_str(r#"
[sources]
"https://github.com/kossnocorp/genotype.git:tests/nested/.." = "main"
"https://example.com/repo.git:tests////" = "main"
"https://example.com/file.js" = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

[sources."git@github.com:kossnocorp/genotype.git"]
version = "main"
files = ["src/./file.txt"]
"#).unwrap();
        for key in [
            "gh:kossnocorp/genotype:tests",
            "git:https://example.com/repo.git:tests",
            "gh:kossnocorp/genotype:src/file.txt",
        ] {
            manifest
                .update(
                    &VdmManifestTargetUrl::new(key),
                    &VdmManifestSourceVersion::new("next"),
                )
                .unwrap();
        }
        let targets = manifest.targets().unwrap();
        assert_eq!(targets.len(), 4);
        assert!(targets.contains_key(&VdmManifestTargetUrl::new(
            "http:https://example.com/file.js"
        )));
        let serialized = toml::to_string(&manifest).unwrap();
        assert!(!serialized.contains("github.com"));
        assert!(serialized.contains("git:https://example.com/repo.git:tests"));
        let restored: VdmManifest = toml::from_str(&serialized).unwrap();
        assert_eq!(
            restored.targets().unwrap().keys().collect::<Vec<_>>(),
            targets.keys().collect::<Vec<_>>()
        );
        assert!(
            toml::from_str::<VdmManifest>(
                r#"
[sources]
"gh:owner/repo:tests" = "main"
"https://github.com/owner/repo.git:tests/./" = "main"
"#
            )
            .is_err()
        );
    }

    #[test]
    fn resolves_all_manifest_source_forms() {
        let manifest: VdmManifest = toml::from_str(
            r#"
[sources]
"gh:kossnocorp/dev:README.md" = "main"
"gh:kossnocorp/dev:LICENSE" = { version = "v1" }

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
                .get(&VdmManifestTargetUrl::new("gh:kossnocorp/dev")),
            Some(VdmManifestSource::Files(_))
        ));

        let targets = manifest.targets().unwrap();
        assert_eq!(targets.len(), 4);
        assert_eq!(
            targets[&VdmManifestTargetUrl::new("gh:kossnocorp/dev:README.md")].version(),
            &VdmManifestSourceVersion::new("main")
        );
        assert_eq!(
            targets[&VdmManifestTargetUrl::new("gh:kossnocorp/dev:LICENSE")].version(),
            &VdmManifestSourceVersion::new("v1")
        );
        assert_eq!(
            targets[&VdmManifestTargetUrl::new("gh:kossnocorp/dev:mise.toml")].version(),
            &VdmManifestSourceVersion::new("v2")
        );
        assert_eq!(
            targets[&VdmManifestTargetUrl::new("gh:kossnocorp/dev:package.json")].version(),
            &VdmManifestSourceVersion::new("v3")
        );
    }

    #[test]
    fn rejects_duplicate_expanded_targets() {
        let manifest: VdmManifest = toml::from_str(
            r#"
[sources]
"gh:kossnocorp/dev:mise.toml" = "main"

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
        let manifest: VdmManifest = toml::from_str(
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
                .contains("must not escape")
        );
    }

    #[test]
    fn rejects_the_old_files_manifest() {
        assert!(
            toml::from_str::<VdmManifest>(
                r#"
[files]
"gh:kossnocorp/dev:mise.toml" = "main"
"#,
            )
            .is_err()
        );
    }

    #[test]
    fn add_serializes_a_direct_scalar_source() {
        let mut manifest = VdmManifest::new();
        manifest.add(
            &VdmManifestTargetUrl::new("gh:kossnocorp/dev:mise.toml"),
            &VdmManifestSourceVersion::new("main"),
        );

        let source = toml::to_string_pretty(&manifest).unwrap();
        assert_eq!(
            source,
            "[sources]\n\"gh:kossnocorp/dev:mise.toml\" = \"main\"\n"
        );
    }

    #[test]
    fn updates_direct_and_grouped_target_versions() {
        let mut manifest: VdmManifest = toml::from_str(
            r#"
[sources]
"gh:kossnocorp/dev:README.md" = "old"

[sources."gh:kossnocorp/dev"]
version = "old"
files = ["mise.toml", { path = "package.json", version = "older" }]
"#,
        )
        .unwrap();
        let version = VdmManifestSourceVersion::new("main");
        manifest
            .update(
                &VdmManifestTargetUrl::new("gh:kossnocorp/dev:README.md"),
                &version,
            )
            .unwrap();
        manifest
            .update(
                &VdmManifestTargetUrl::new("gh:kossnocorp/dev:mise.toml"),
                &version,
            )
            .unwrap();
        manifest
            .update(
                &VdmManifestTargetUrl::new("gh:kossnocorp/dev:package.json"),
                &version,
            )
            .unwrap();

        let targets = manifest.targets().unwrap();
        assert!(targets.values().all(|target| target.version() == &version));
    }
}
