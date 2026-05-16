use std::fmt;

use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrinOp {
    CreateIndex,
    Insert,
    Update,
    SamepageUpdate,
    RevmapExtend,
    Desummarize,
    Unknown,
}

impl BrinOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => BrinOp::CreateIndex,
	    0x10 => BrinOp::Insert,
	    0x20 => BrinOp::Update,
	    0x30 => BrinOp::SamepageUpdate,
	    0x40 => BrinOp::RevmapExtend,
	    0x50 => BrinOp::Desummarize,
	    _ => BrinOp::Unknown,
	}
    }
}

impl fmt::Display for BrinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    BrinOp::CreateIndex => write!(f, "CREATE_INDEX"),
	    BrinOp::Insert => write!(f, "INSERT"),
	    BrinOp::Update => write!(f, "UPDATE"),
	    BrinOp::SamepageUpdate => write!(f, "SAMEPAGE_UPDATE"),
	    BrinOp::RevmapExtend => write!(f, "REVMAP_EXTEND"),
	    BrinOp::Desummarize => write!(f, "DESUMMARIZE"),
	    BrinOp::Unknown => write!(f, "Unknown"),
	}
    }
}
