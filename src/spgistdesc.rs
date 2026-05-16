use std::fmt;

use crate::walreader::XLR_RMGR_INFO_MASK;

// SP-GiST xl_info op values (PG18: spgxlog.h)
// Note: XLOG_SPGIST_CREATE_INDEX (0x00) is no longer used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpGistOp {
    AddLeaf,
    MoveLeafs,
    AddNode,
    SplitTuple,
    PickSplit,
    VacuumLeaf,
    VacuumRoot,
    VacuumRedirect,
    Unknown,
}

impl SpGistOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x10 => SpGistOp::AddLeaf,
	    0x20 => SpGistOp::MoveLeafs,
	    0x30 => SpGistOp::AddNode,
	    0x40 => SpGistOp::SplitTuple,
	    0x50 => SpGistOp::PickSplit,
	    0x60 => SpGistOp::VacuumLeaf,
	    0x70 => SpGistOp::VacuumRoot,
	    0x80 => SpGistOp::VacuumRedirect,
	    _ => SpGistOp::Unknown,
	}
    }
}

impl fmt::Display for SpGistOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    SpGistOp::AddLeaf => write!(f, "ADD_LEAF"),
	    SpGistOp::MoveLeafs => write!(f, "MOVE_LEAFS"),
	    SpGistOp::AddNode => write!(f, "ADD_NODE"),
	    SpGistOp::SplitTuple => write!(f, "SPLIT_TUPLE"),
	    SpGistOp::PickSplit => write!(f, "PICKSPLIT"),
	    SpGistOp::VacuumLeaf => write!(f, "VACUUM_LEAF"),
	    SpGistOp::VacuumRoot => write!(f, "VACUUM_ROOT"),
	    SpGistOp::VacuumRedirect => write!(f, "VACUUM_REDIRECT"),
	    SpGistOp::Unknown => write!(f, "Unknown"),
	}
    }
}
