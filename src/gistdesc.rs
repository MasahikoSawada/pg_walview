use std::fmt;

use crate::walreader::XLR_RMGR_INFO_MASK;

// GiST xl_info op values (PG18: gistxlog.h)
// Note: 0x40 (INSERT_COMPLETE) and 0x50 (CREATE_INDEX) are no longer used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GistOp {
    PageUpdate,
    Delete,
    PageReuse,
    PageSplit,
    PageDelete,
    AssignLsn,
    Unknown,
}

impl GistOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => GistOp::PageUpdate,
	    0x10 => GistOp::Delete,
	    0x20 => GistOp::PageReuse,
	    0x30 => GistOp::PageSplit,
	    0x60 => GistOp::PageDelete,
	    0x70 => GistOp::AssignLsn,
	    _ => GistOp::Unknown,
	}
    }
}

impl fmt::Display for GistOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    GistOp::PageUpdate => write!(f, "PAGE_UPDATE"),
	    GistOp::Delete => write!(f, "DELETE"),
	    GistOp::PageReuse => write!(f, "PAGE_REUSE"),
	    GistOp::PageSplit => write!(f, "PAGE_SPLIT"),
	    GistOp::PageDelete => write!(f, "PAGE_DELETE"),
	    GistOp::AssignLsn => write!(f, "ASSIGN_LSN"),
	    GistOp::Unknown => write!(f, "Unknown"),
	}
    }
}
