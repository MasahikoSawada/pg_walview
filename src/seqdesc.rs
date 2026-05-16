use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceOp {
    Log,
    Unknown,
}

impl SequenceOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => SequenceOp::Log,
	    _ => SequenceOp::Unknown,
	}
    }
}

impl fmt::Display for SequenceOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    SequenceOp::Log => write!(f, "LOG"),
	    SequenceOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_seq_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = SequenceOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        SequenceOp::Log => {
            let spc = r.read_u32_le().unwrap_or(0);
            let db  = r.read_u32_le().unwrap_or(0);
            let rel = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  locator:    {}/{}/{}", spc, db, rel));
            lines.push(format!("  tuple data: {} bytes follow", r.remaining()));
        }
        SequenceOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown sequence op)", main.len()));
        }
    }
    lines
}
