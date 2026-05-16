use std::fmt;

use crate::walmisc::{fmt_pg_ts, Reader};
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitTsOp {
    ZeroPage,
    Truncate,
    Unknown,
}

impl CommitTsOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => CommitTsOp::ZeroPage,
	    0x10 => CommitTsOp::Truncate,
	    _ => CommitTsOp::Unknown,
	}
    }
}

impl fmt::Display for CommitTsOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    CommitTsOp::ZeroPage => write!(f, "ZEROPAGE"),
	    CommitTsOp::Truncate => write!(f, "TRUNCATE"),
	    CommitTsOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_commit_ts_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = CommitTsOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        CommitTsOp::ZeroPage => {
            if main.len() >= 14 {
                let ts      = r.read_i64_le().unwrap_or(0);
                let nodeid  = r.read_u16_le().unwrap_or(0);
                r.skip(2);
                let mainxid = r.read_u32_le().unwrap_or(0);
                lines.push(format!("  timestamp: {} µs  ({})", ts, fmt_pg_ts(ts)));
                lines.push(format!("  nodeid:    {}", nodeid));
                lines.push(format!("  mainxid:   {}", mainxid));
                let n_subxids = r.remaining() / 4;
                if n_subxids > 0 {
                    lines.push(format!("  subxids:   {} entries follow", n_subxids));
                }
            } else {
                lines.push(format!("  ({} bytes)", main.len()));
            }
        }
        CommitTsOp::Truncate => {
            let pageno     = r.read_i64_le().unwrap_or(0);
            let oldest_xid = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  pageno:    {}", pageno));
            lines.push(format!("  oldestXid: {}", oldest_xid));
        }
        CommitTsOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown commit_ts op)", main.len()));
        }
    }
    lines
}
