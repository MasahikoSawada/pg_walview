use std::fmt;

use crate::walmisc::{lsn_format, Reader};
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplOriginOp {
    Set,
    Drop,
    Unknown,
}

impl ReplOriginOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => ReplOriginOp::Set,
	    0x10 => ReplOriginOp::Drop,
	    _ => ReplOriginOp::Unknown,
	}
    }
}

impl fmt::Display for ReplOriginOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    ReplOriginOp::Set => write!(f, "SET"),
	    ReplOriginOp::Drop => write!(f, "DROP"),
	    ReplOriginOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_replorigin_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = ReplOriginOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        ReplOriginOp::Set => {
            let remote_lsn = r.read_u64_le().unwrap_or(0);
            let node_id    = r.read_u16_le().unwrap_or(0);
            let force      = r.read_bool().unwrap_or(false);
            lines.push(format!("  remote_lsn: {}", lsn_format(remote_lsn)));
            lines.push(format!("  node_id:    {}", node_id));
            lines.push(format!("  force:      {}", force));
        }
        ReplOriginOp::Drop => {
            let node_id = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  node_id: {}", node_id));
        }
        ReplOriginOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown replorigin op)", main.len()));
        }
    }
    lines
}
