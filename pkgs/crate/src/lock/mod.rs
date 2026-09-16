use crate::prelude::*;

mod file;
pub use file::*;

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct VdmLock {
    #[serde(default)]
    pub files: BTreeMap<VdmManifestTargetUrl, VdmLockFile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub updates: BTreeMap<VdmManifestTargetUrl, VdmLockUpdate>,
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
