use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmgrOp {
    Create,
    Truncate,
    Unknown,
}

impl SmgrOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x10 => SmgrOp::Create,
	    0x20 => SmgrOp::Truncate,
	    _ => SmgrOp::Unknown,
	}
    }
}

impl fmt::Display for SmgrOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    SmgrOp::Create => write!(f, "CREATE"),
	    SmgrOp::Truncate => write!(f, "TRUNCATE"),
	    SmgrOp::Unknown => write!(f, "Unknown"),
	}
    }
}

fn decode_smgr_truncate_flags(flags: i32) -> String {
    let mut p = Vec::new();
    if flags & 0x0001 != 0 { p.push("HEAP"); }
    if flags & 0x0002 != 0 { p.push("VM"); }
    if flags & 0x0004 != 0 { p.push("FSM"); }
    if p.is_empty() { format!("0x{:04x}", flags) } else { format!("0x{:04x} ({})", flags, p.join("|")) }
}

pub fn describe_smgr_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = SmgrOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        SmgrOp::Create => {
            let spc  = r.read_u32_le().unwrap_or(0);
            let db   = r.read_u32_le().unwrap_or(0);
            let rel  = r.read_u32_le().unwrap_or(0);
            let fork = r.read_i32_le().unwrap_or(0);
            let fork_name = match fork { 0 => "main", 1 => "fsm", 2 => "vm", 3 => "init", _ => "unknown" };
            lines.push(format!("  rlocator: {}/{}/{}", spc, db, rel));
            lines.push(format!("  forkNum:  {} ({})", fork, fork_name));
        }
        SmgrOp::Truncate => {
            let blkno = r.read_u32_le().unwrap_or(0);
            let spc   = r.read_u32_le().unwrap_or(0);
            let db    = r.read_u32_le().unwrap_or(0);
            let rel   = r.read_u32_le().unwrap_or(0);
            let flags = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  blkno:    {}", blkno));
            lines.push(format!("  rlocator: {}/{}/{}", spc, db, rel));
            lines.push(format!("  flags:    {}", decode_smgr_truncate_flags(flags)));
        }
        SmgrOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown smgr op)", main.len()));
        }
    }
    lines
}
