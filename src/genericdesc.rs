use std::fmt;

// Generic xlog xl_info layout (PG18: generic_xlog.h)
//
// xl_info does not use the standard RMGR info nibble for op dispatch.
// The lower nibble holds the number of modified pages (1..MAX_GENERIC_XLOG_PAGES=4),
// and GENERIC_XLOG_FULL_IMAGE (0x0001) is a per-page flag stored in the record
// data, not in xl_info itself.  We therefore just display the page count.
pub struct GenericOp {
    pub page_count: u8,
}

impl GenericOp {
    pub fn from_xl_info(info: u8) -> Self {
	GenericOp {
	    page_count: info & 0x0F,
	}
    }
}

impl fmt::Display for GenericOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	write!(f, "MODIFY({} page{})", self.page_count,
	       if self.page_count == 1 { "" } else { "s" })
    }
}
