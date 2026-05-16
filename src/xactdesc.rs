use std::fmt;

use crate::walmisc::{fmt_pg_ts, lsn_format, Reader};

pub const XLOG_XACT_OPMASK: u8 = 0x70;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XactOp {
    Commit,
    Prepare,
    Abort,
    CommitPrepared,
    AbortPrepared,
    Assignment,
    Invalidations,
    Unknown,
}

impl XactOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLOG_XACT_OPMASK {
	    0x00 => XactOp::Commit,
	    0x10 => XactOp::Prepare,
	    0x20 => XactOp::Abort,
	    0x30 => XactOp::CommitPrepared,
	    0x40 => XactOp::AbortPrepared,
	    0x50 => XactOp::Assignment,
	    0x60 => XactOp::Invalidations,
	    _ => XactOp::Unknown,
	}
    }
}

impl fmt::Display for XactOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    XactOp::Commit => write!(f, "COMMIT"),
	    XactOp::Prepare => write!(f, "PREPARE"),
	    XactOp::Abort => write!(f, "ABORT"),
	    XactOp::CommitPrepared => write!(f, "COMMIT_PREPARED"),
	    XactOp::AbortPrepared => write!(f, "ABORT_PREPARED"),
	    XactOp::Assignment => write!(f, "ASSIGNMENT"),
	    XactOp::Invalidations => write!(f, "INVALIDATION"),
	    _ => write!(f, "Unknown"),
	}
    }
}

fn decode_xact_xinfo(xinfo: u32) -> String {
    let mut p = Vec::new();
    if xinfo & (1 << 0)  != 0 { p.push("HAS_DBINFO"); }
    if xinfo & (1 << 1)  != 0 { p.push("HAS_SUBXACTS"); }
    if xinfo & (1 << 2)  != 0 { p.push("HAS_RELFILELOCATORS"); }
    if xinfo & (1 << 3)  != 0 { p.push("HAS_INVALS"); }
    if xinfo & (1 << 4)  != 0 { p.push("HAS_TWOPHASE"); }
    if xinfo & (1 << 5)  != 0 { p.push("HAS_ORIGIN"); }
    if xinfo & (1 << 6)  != 0 { p.push("HAS_AE_LOCKS"); }
    if xinfo & (1 << 7)  != 0 { p.push("HAS_GID"); }
    if xinfo & (1 << 8)  != 0 { p.push("HAS_DROPPED_STATS"); }
    if xinfo & (1 << 29) != 0 { p.push("APPLY_FEEDBACK"); }
    if xinfo & (1 << 30) != 0 { p.push("UPDATE_RELCACHE_FILE"); }
    if xinfo & (1 << 31) != 0 { p.push("FORCE_SYNC_COMMIT"); }
    if p.is_empty() { format!("0x{:08x}", xinfo) } else { format!("0x{:08x} ({})", xinfo, p.join(" | ")) }
}

pub fn describe_xact_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = XactOp::from_xl_info(info);
    let has_info = info & 0x80 != 0;
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        XactOp::Commit | XactOp::CommitPrepared => {
            let xact_time = r.read_i64_le().unwrap_or(0);
            lines.push(format!("  xact_time: {} µs  ({})", xact_time, fmt_pg_ts(xact_time)));
            if has_info {
                if let Some(xinfo) = r.read_u32_le() {
                    lines.push(format!("  xinfo:     {}", decode_xact_xinfo(xinfo)));
                    lines.extend(parse_xact_subrecords(&mut r, xinfo, true));
                }
            }
        }

        XactOp::Abort | XactOp::AbortPrepared => {
            let xact_time = r.read_i64_le().unwrap_or(0);
            lines.push(format!("  xact_time: {} µs  ({})", xact_time, fmt_pg_ts(xact_time)));
            if has_info {
                if let Some(xinfo) = r.read_u32_le() {
                    lines.push(format!("  xinfo:     {}", decode_xact_xinfo(xinfo)));
                    lines.extend(parse_xact_subrecords(&mut r, xinfo, false));
                }
            }
        }

        XactOp::Prepare => {
            let magic        = r.read_u32_le().unwrap_or(0);
            let total_len    = r.read_u32_le().unwrap_or(0);
            let xid          = r.read_u32_le().unwrap_or(0);
            let database     = r.read_u32_le().unwrap_or(0);
            let prepared_at  = r.read_i64_le().unwrap_or(0);
            let owner        = r.read_u32_le().unwrap_or(0);
            let nsubxacts    = r.read_i32_le().unwrap_or(0);
            let ncommitrels  = r.read_i32_le().unwrap_or(0);
            let nabortrels   = r.read_i32_le().unwrap_or(0);
            let _ncommitstats = r.read_i32_le().unwrap_or(0);
            let _nabortstats  = r.read_i32_le().unwrap_or(0);
            let ninvalmsgs   = r.read_i32_le().unwrap_or(0);
            let initfileinval = r.read_bool().unwrap_or(false);
            r.skip(1);
            let gidlen       = r.read_u16_le().unwrap_or(0);
            let origin_lsn   = r.read_u64_le().unwrap_or(0);
            lines.push(format!("  magic:        0x{:08x}", magic));
            lines.push(format!("  total_len:    {}", total_len));
            lines.push(format!("  xid:          {}", xid));
            lines.push(format!("  database:     {}", database));
            lines.push(format!("  prepared_at:  {} µs  ({})", prepared_at, fmt_pg_ts(prepared_at)));
            lines.push(format!("  owner:        {}", owner));
            lines.push(format!("  nsubxacts:    {}", nsubxacts));
            lines.push(format!("  ncommitrels:  {}", ncommitrels));
            lines.push(format!("  nabortrels:   {}", nabortrels));
            lines.push(format!("  ninvalmsgs:   {}", ninvalmsgs));
            lines.push(format!("  initfileinval:{}", initfileinval));
            lines.push(format!("  gidlen:       {}", gidlen));
            lines.push(format!("  origin_lsn:   {}", lsn_format(origin_lsn)));
            if gidlen > 0 && r.remaining() >= gidlen as usize {
                let gid = String::from_utf8_lossy(r.peek_bytes(gidlen as usize - 1)).into_owned();
                lines.push(format!("  gid:          {:?}", gid));
            }
        }

        XactOp::Assignment => {
            let xtop     = r.read_u32_le().unwrap_or(0);
            let nsubxacts = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  xtop:      {}", xtop));
            lines.push(format!("  nsubxacts: {}", nsubxacts));
            let show = (nsubxacts.max(0) as usize).min(16);
            for i in 0..show {
                if let Some(xid) = r.read_u32_le() {
                    lines.push(format!("  xsub[{}]:  {}", i, xid));
                }
            }
            if nsubxacts as usize > 16 {
                lines.push(format!("  ... ({} more)", nsubxacts - 16));
            }
        }

        XactOp::Invalidations => {
            lines.push(format!("  ({} bytes of invalidation messages)", main.len()));
        }

        XactOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown xact op)", main.len()));
        }
    }
    lines
}

fn parse_xact_subrecords(r: &mut Reader, xinfo: u32, is_commit: bool) -> Vec<String> {
    let mut lines = Vec::new();

    if xinfo & (1 << 0) != 0 {
        let db_id = r.read_u32_le().unwrap_or(0);
        let ts_id = r.read_u32_le().unwrap_or(0);
        lines.push(format!("  dbId: {}", db_id));
        lines.push(format!("  tsId: {}", ts_id));
    }

    if xinfo & (1 << 1) != 0 {
        let nsubxacts = r.read_i32_le().unwrap_or(0);
        lines.push(format!("  nsubxacts: {}", nsubxacts));
        let show = (nsubxacts.max(0) as usize).min(8);
        for i in 0..show {
            if let Some(xid) = r.read_u32_le() {
                lines.push(format!("  subxid[{}]: {}", i, xid));
            }
        }
        if nsubxacts as usize > 8 {
            r.skip((nsubxacts as usize - 8) * 4);
            lines.push(format!("  ... ({} more subxids)", nsubxacts - 8));
        }
    }

    if xinfo & (1 << 2) != 0 {
        let nrels = r.read_i32_le().unwrap_or(0);
        lines.push(format!("  nrels: {}", nrels));
        let show = (nrels.max(0) as usize).min(8);
        for i in 0..show {
            let spc = r.read_u32_le().unwrap_or(0);
            let db  = r.read_u32_le().unwrap_or(0);
            let rel = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  rel[{}]: {}/{}/{}", i, spc, db, rel));
        }
        if nrels as usize > 8 {
            r.skip((nrels as usize - 8) * 12);
            lines.push(format!("  ... ({} more rels)", nrels - 8));
        }
    }

    if is_commit && xinfo & (1 << 3) != 0 {
        let nmsgs = r.read_i32_le().unwrap_or(0);
        lines.push(format!("  ninvals: {}", nmsgs));
    }

    if xinfo & (1 << 4) != 0 {
        let xid = r.read_u32_le().unwrap_or(0);
        lines.push(format!("  2pc_xid: {}", xid));
    }

    if xinfo & (1 << 7) != 0 {
        let gid = r.read_cstr();
        lines.push(format!("  gid: {:?}", gid));
    }

    if xinfo & (1 << 5) != 0 {
        let origin_lsn = r.read_u64_le().unwrap_or(0);
        let origin_ts  = r.read_i64_le().unwrap_or(0);
        lines.push(format!("  origin_lsn: {}", lsn_format(origin_lsn)));
        lines.push(format!("  origin_ts:  {} µs  ({})", origin_ts, fmt_pg_ts(origin_ts)));
    }

    lines
}

