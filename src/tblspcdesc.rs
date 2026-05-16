use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TablespaceOp {
    Create,
    Drop,
    Unknown,
}

impl TablespaceOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => TablespaceOp::Create,
	    0x10 => TablespaceOp::Drop,
	    _ => TablespaceOp::Unknown,
	}
    }
}

impl fmt::Display for TablespaceOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    TablespaceOp::Create => write!(f, "CREATE"),
	    TablespaceOp::Drop => write!(f, "DROP"),
	    TablespaceOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_tablespace_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = TablespaceOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        TablespaceOp::Create => {
            let ts_id = r.read_u32_le().unwrap_or(0);
            let path  = r.read_cstr();
            lines.push(format!("  ts_id:   {}", ts_id));
            lines.push(format!("  ts_path: {:?}", path));
        }
        TablespaceOp::Drop => {
            let ts_id = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  ts_id: {}", ts_id));
        }
        TablespaceOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown tablespace op)", main.len()));
        }
    }
    lines
}
