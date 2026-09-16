use crate::prelude::*;

pub struct VdmSourceFile {
    pub revision: String,
    pub bytes: Vec<u8>,
}

impl VdmFileWritable for VdmSourceFile {
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
