use std::fmt;

use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashOp {
    InitMetaPage,
    InitBitmapPage,
    Insert,
    AddOvflPage,
    SplitAllocatePage,
    SplitPage,
    SplitComplete,
    MovePageContents,
    SqueezePage,
    Delete,
    SplitCleanup,
    UpdateMetaPage,
    VacuumOnePage,
    Unknown,
}

impl HashOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => HashOp::InitMetaPage,
	    0x10 => HashOp::InitBitmapPage,
	    0x20 => HashOp::Insert,
	    0x30 => HashOp::AddOvflPage,
	    0x40 => HashOp::SplitAllocatePage,
	    0x50 => HashOp::SplitPage,
	    0x60 => HashOp::SplitComplete,
	    0x70 => HashOp::MovePageContents,
	    0x80 => HashOp::SqueezePage,
	    0x90 => HashOp::Delete,
	    0xA0 => HashOp::SplitCleanup,
	    0xB0 => HashOp::UpdateMetaPage,
	    0xC0 => HashOp::VacuumOnePage,
	    _ => HashOp::Unknown,
	}
    }
}

impl fmt::Display for HashOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    HashOp::InitMetaPage => write!(f, "INIT_META_PAGE"),
	    HashOp::InitBitmapPage => write!(f, "INIT_BITMAP_PAGE"),
	    HashOp::Insert => write!(f, "INSERT"),
	    HashOp::AddOvflPage => write!(f, "ADD_OVFL_PAGE"),
	    HashOp::SplitAllocatePage => write!(f, "SPLIT_ALLOCATE_PAGE"),
	    HashOp::SplitPage => write!(f, "SPLIT_PAGE"),
	    HashOp::SplitComplete => write!(f, "SPLIT_COMPLETE"),
	    HashOp::MovePageContents => write!(f, "MOVE_PAGE_CONTENTS"),
	    HashOp::SqueezePage => write!(f, "SQUEEZE_PAGE"),
	    HashOp::Delete => write!(f, "DELETE"),
	    HashOp::SplitCleanup => write!(f, "SPLIT_CLEANUP"),
	    HashOp::UpdateMetaPage => write!(f, "UPDATE_META_PAGE"),
	    HashOp::VacuumOnePage => write!(f, "VACUUM_ONE_PAGE"),
	    HashOp::Unknown => write!(f, "Unknown"),
	}
    }
}
