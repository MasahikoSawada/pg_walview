use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelmapOp {
    Update,
    Unknown,
}

impl RelmapOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => RelmapOp::Update,
	    _ => RelmapOp::Unknown,
	}
    }
}

impl fmt::Display for RelmapOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    RelmapOp::Update => write!(f, "UPDATE"),
	    RelmapOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_relmap_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = RelmapOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        RelmapOp::Update => {
            let dbid   = r.read_u32_le().unwrap_or(0);
            let tsid   = r.read_u32_le().unwrap_or(0);
            let nbytes = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  dbid:   {}", dbid));
            lines.push(format!("  tsid:   {}", tsid));
            lines.push(format!("  nbytes: {}", nbytes));
        }
        RelmapOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown relmap op)", main.len()));
        }
    }
    lines
}
