use crate::prelude::*;

pub struct VdmDirs(#[allow(dead_code)] ProjectDirs);

impl VdmDirs {
    pub fn resolve() -> Result<Self> {
        let dirs =
            ProjectDirs::from("fyi", "vdm", "vdm").with_context(|| "Failed to resolve Vdm dirs")?;
        Ok(Self(dirs))
    }
}
