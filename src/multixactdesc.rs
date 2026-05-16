use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiXactOp {
    ZeroOffPage,
    ZeroMemPage,
    CreateId,
    TruncateId,
    Unknown,
}

impl MultiXactOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => MultiXactOp::ZeroOffPage,
	    0x10 => MultiXactOp::ZeroMemPage,
	    0x20 => MultiXactOp::CreateId,
	    0x30 => MultiXactOp::TruncateId,
	    _ => MultiXactOp::Unknown,
	}
    }
}

impl fmt::Display for MultiXactOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    MultiXactOp::ZeroOffPage => write!(f, "ZERO_OFF_PAGE"),
	    MultiXactOp::ZeroMemPage => write!(f, "ZERO_MEM_PAGE"),
	    MultiXactOp::CreateId => write!(f, "CREATE_ID"),
	    MultiXactOp::TruncateId => write!(f, "TRUNCATE_ID"),
	    MultiXactOp::Unknown => write!(f, "Unknown"),
	}
    }
}

fn multixact_status(s: u32) -> &'static str {
    match s {
        0 => "ForKeyShare",
        1 => "ForShare",
        2 => "ForNoKeyUpdate",
        3 => "ForUpdate",
        4 => "NoKeyUpdate",
        5 => "Update",
        _ => "unknown",
    }
}

pub fn describe_multixact_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = MultiXactOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        MultiXactOp::ZeroOffPage | MultiXactOp::ZeroMemPage => {
            let pageno = r.read_i64_le().unwrap_or(0);
            let kind = if op == MultiXactOp::ZeroOffPage { "offsets" } else { "members" };
            lines.push(format!("  pageno: {} (zero {} page)", pageno, kind));
        }
        MultiXactOp::CreateId => {
            let mid      = r.read_u32_le().unwrap_or(0);
            let moff     = r.read_u32_le().unwrap_or(0);
            let nmembers = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  mid:      {}", mid));
            lines.push(format!("  moff:     {}", moff));
            lines.push(format!("  nmembers: {}", nmembers));
            let show = (nmembers.max(0) as usize).min(8);
            for i in 0..show {
                let xid    = r.read_u32_le().unwrap_or(0);
                let status = r.read_u32_le().unwrap_or(0);
                lines.push(format!("  member[{}]: xid={} status={} ({})", i, xid, status, multixact_status(status)));
            }
            if nmembers as usize > 8 {
                lines.push(format!("  ... ({} more members)", nmembers - 8));
            }
        }
        MultiXactOp::TruncateId => {
            let oldest_multi_db  = r.read_u32_le().unwrap_or(0);
            let start_trunc_off  = r.read_u32_le().unwrap_or(0);
            let end_trunc_off    = r.read_u32_le().unwrap_or(0);
            let start_trunc_memb = r.read_u32_le().unwrap_or(0);
            let end_trunc_memb   = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  oldestMultiDB:   {}", oldest_multi_db));
            lines.push(format!("  startTruncOff:   {}", start_trunc_off));
            lines.push(format!("  endTruncOff:     {}", end_trunc_off));
            lines.push(format!("  startTruncMemb:  {}", start_trunc_memb));
            lines.push(format!("  endTruncMemb:    {}", end_trunc_memb));
        }
        MultiXactOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown multixact op)", main.len()));
        }
    }
    lines
}
