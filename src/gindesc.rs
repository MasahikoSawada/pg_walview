use std::fmt;

use crate::walreader::XLR_RMGR_INFO_MASK;

// GIN xl_info op values (PG18: ginxlog.h)
// Note: XLOG_GIN_CREATE_INDEX (0x00) was removed; first active op is CREATE_PTREE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GinOp {
    CreatePtree,
    Insert,
    Split,
    VacuumPage,
    DeletePage,
    UpdateMetaPage,
    InsertListPage,
    DeleteListPage,
    VacuumDataLeafPage,
    Unknown,
}

impl GinOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x10 => GinOp::CreatePtree,
	    0x20 => GinOp::Insert,
	    0x30 => GinOp::Split,
	    0x40 => GinOp::VacuumPage,
	    0x50 => GinOp::DeletePage,
	    0x60 => GinOp::UpdateMetaPage,
	    0x70 => GinOp::InsertListPage,
	    0x80 => GinOp::DeleteListPage,
	    0x90 => GinOp::VacuumDataLeafPage,
	    _ => GinOp::Unknown,
	}
    }
}

impl fmt::Display for GinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    GinOp::CreatePtree => write!(f, "CREATE_PTREE"),
	    GinOp::Insert => write!(f, "INSERT"),
	    GinOp::Split => write!(f, "SPLIT"),
	    GinOp::VacuumPage => write!(f, "VACUUM_PAGE"),
	    GinOp::DeletePage => write!(f, "DELETE_PAGE"),
	    GinOp::UpdateMetaPage => write!(f, "UPDATE_META_PAGE"),
	    GinOp::InsertListPage => write!(f, "INSERT_LISTPAGE"),
	    GinOp::DeleteListPage => write!(f, "DELETE_LISTPAGE"),
	    GinOp::VacuumDataLeafPage => write!(f, "VACUUM_DATA_LEAF_PAGE"),
	    GinOp::Unknown => write!(f, "Unknown"),
	}
    }
}
