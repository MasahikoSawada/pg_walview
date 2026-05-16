use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClogOp {
    ZeroPage,
    Truncate,
    Unknown,
}

impl ClogOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => ClogOp::ZeroPage,
	    0x10 => ClogOp::Truncate,
	    _ => ClogOp::Unknown,
	}
    }
}

impl fmt::Display for ClogOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    ClogOp::ZeroPage => write!(f, "ZEROPAGE"),
	    ClogOp::Truncate => write!(f, "TRUNCATE"),
	    ClogOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_clog_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = ClogOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        ClogOp::ZeroPage => {
            let pageno = r.read_i64_le().unwrap_or(0);
            lines.push(format!("  pageno: {}", pageno));
        }
        ClogOp::Truncate => {
            let pageno         = r.read_i64_le().unwrap_or(0);
            let oldest_xact    = r.read_u32_le().unwrap_or(0);
            let oldest_xact_db = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  pageno:       {}", pageno));
            lines.push(format!("  oldestXact:   {}", oldest_xact));
            lines.push(format!("  oldestXactDB: {}", oldest_xact_db));
        }
        ClogOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown clog op)", main.len()));
        }
    }
    lines
}
