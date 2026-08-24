use crate::prelude::*;

mod file;
pub use file::*;

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct VitLock {
    #[serde(default)]
    pub files: BTreeMap<VitManifestTargetUrl, VitLockFile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub updates: BTreeMap<VitManifestTargetUrl, VitLockUpdate>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct VitLockUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub decision: VitLockUpdateDecision,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VitLockUpdateDecision {
    Accepted,
    Rejected,
}

impl VitFileToml for VitLock {}
