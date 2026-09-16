use crate::prelude::*;

mod file;
pub use file::*;

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
#[serde(try_from = "LockDocument", into = "LockDocument")]
pub struct VdmLock {
    // Keep a shared file graph in memory; serialize glob members as arrays
    // under their original manifest keys rather than duplicating file tables.
    pub files: BTreeMap<VdmManifestTargetUrl, VdmLockFile>,
    pub globs: BTreeMap<VdmManifestTargetUrl, Vec<VdmManifestTargetUrl>>,
    pub updates: BTreeMap<VdmManifestTargetUrl, VdmLockUpdate>,
}

#[derive(Deserialize, Serialize, Default)]
struct LockDocument {
    #[serde(default)]
    files: BTreeMap<VdmManifestTargetUrl, LockEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    updates: BTreeMap<VdmManifestTargetUrl, VdmLockUpdate>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum LockEntry {
    File(VdmLockFile),
    Glob(Vec<VdmLockFile>),
}

impl From<VdmLock> for LockDocument {
    fn from(lock: VdmLock) -> Self {
        let grouped = lock.globs.values().flatten().collect::<BTreeSet<_>>();
        let mut files = lock
            .files
            .iter()
            .filter(|(key, _)| !grouped.contains(key))
            .map(|(key, file)| (key.clone(), LockEntry::File(file.clone())))
            .collect::<BTreeMap<_, _>>();

        for (key, members) in &lock.globs {
            files.insert(
                key.clone(),
                LockEntry::Glob(
                    members
                        .iter()
                        .filter_map(|member| lock.files.get(member).cloned())
                        .collect(),
                ),
            );
        }

        Self {
            files,
            updates: lock.updates,
        }
    }
}

impl TryFrom<LockDocument> for VdmLock {
    type Error = Error;

    fn try_from(document: LockDocument) -> Result<Self> {
        let mut lock = Self {
            updates: document.updates,
            ..Self::default()
        };
        for (key, entry) in document.files {
            match entry {
                LockEntry::File(file) => {
                    insert_file(&mut lock.files, key, file)?;
                }

                LockEntry::Glob(files) => {
                    let mut members = Vec::new();
                    for file in files {
                        let target = VdmSourceInput::parse_manifest_target(&key, &file.version)?;
                        let target = target
                            .as_any()
                            .downcast_ref::<VdmGitHubTarget>()
                            .context("Lockfile arrays require a GitHub glob")?;
                        ensure!(
                            target.glob()?.is_some(),
                            "Lockfile array key must be a glob"
                        );
                        let prefix =
                            PathBuf::from(format!("vendor/@{}/{}", target.owner(), target.repo()));
                        let path = Path::new(&file.path)
                            .strip_prefix(&prefix)
                            .context("Glob file is outside its repository")?;
                        let member = target.with_path(path)?.key().clone();
                        insert_file(&mut lock.files, member.clone(), file)?;
                        members.push(member);
                    }
                    lock.globs.insert(key, members);
                }
            }
        }
        Ok(lock)
    }
}

fn insert_file(
    files: &mut BTreeMap<VdmManifestTargetUrl, VdmLockFile>,
    key: VdmManifestTargetUrl,
    file: VdmLockFile,
) -> Result<()> {
    if let Some(existing) = files.get(&key) {
        ensure!(existing == &file, "Conflicting lock entries for {key}");
    } else {
        files.insert(key, file);
    }
    Ok(())
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct VdmLockUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub decision: VdmLockUpdateDecision,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VdmLockUpdateDecision {
    Accepted,
    Rejected,
}

impl VdmFileToml for VdmLock {}
