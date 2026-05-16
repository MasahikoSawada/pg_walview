use std::fmt;

use crate::heapdesc::{decode_heap_insert_flags, decode_infobits};
use crate::walmisc::{lsn_format, Reader};
use crate::walreader::XLR_RMGR_INFO_MASK;

// Heap2 xl_info op values (PG18: heapam_xlog.h)
// Note: FREEZE_PAGE and VACUUM were removed in PG17+; prune was split into
// PRUNE_ON_ACCESS / PRUNE_VACUUM_SCAN / PRUNE_VACUUM_CLEANUP in PG18.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Heap2Op {
    Rewrite,
    PruneOnAccess,
    PruneVacuumScan,
    PruneVacuumCleanup,
    Visible,
    MultiInsert,
    LockUpdated,
    NewCid,
    MultiInsertInit,
    Unknown,
}

impl Heap2Op {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => Heap2Op::Rewrite,
	    0x10 => Heap2Op::PruneOnAccess,
	    0x20 => Heap2Op::PruneVacuumScan,
	    0x30 => Heap2Op::PruneVacuumCleanup,
	    0x40 => Heap2Op::Visible,
	    0x50 => Heap2Op::MultiInsert,
	    0x60 => Heap2Op::LockUpdated,
	    0x70 => Heap2Op::NewCid,
	    0xD0 => Heap2Op::MultiInsertInit,
	    _ => Heap2Op::Unknown,
	}
    }
}

impl fmt::Display for Heap2Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    Heap2Op::Rewrite => write!(f, "REWRITE"),
	    Heap2Op::PruneOnAccess => write!(f, "PRUNE_ON_ACCESS"),
	    Heap2Op::PruneVacuumScan => write!(f, "PRUNE_VACUUM_SCAN"),
	    Heap2Op::PruneVacuumCleanup => write!(f, "PRUNE_VACUUM_CLEANUP"),
	    Heap2Op::Visible => write!(f, "VISIBLE"),
	    Heap2Op::MultiInsert => write!(f, "MULTI_INSERT"),
	    Heap2Op::LockUpdated => write!(f, "LOCK_UPDATED"),
	    Heap2Op::NewCid => write!(f, "NEW_CID"),
	    Heap2Op::MultiInsertInit => write!(f, "MULTI_INSERT+INIT"),
	    Heap2Op::Unknown => write!(f, "Unknown"),
	}
    }
}

fn decode_xl_heap_visible_flags(flags: u8) -> String {
    let mut p = Vec::new();
    if flags & 0x01 != 0 { p.push("ALL_VISIBLE"); }
    if flags & 0x02 != 0 { p.push("ALL_FROZEN"); }
    if flags & 0x04 != 0 { p.push("CATALOG_REL"); }
    if p.is_empty() { format!("0x{:02x}", flags) } else { format!("0x{:02x} ({})", flags, p.join(" | ")) }
}

fn fmt_prune_reason(reason: u8) -> &'static str {
    match reason {
        0 => "on_access",
        1 => "vacuum_scan",
        2 => "vacuum_cleanup",
        _ => "unknown",
    }
}

fn fmt_prune_flags(flags: u8) -> String {
    let mut p = Vec::new();
    if flags & (1 << 1) != 0 { p.push("IS_CATALOG_REL"); }
    if flags & (1 << 2) != 0 { p.push("CLEANUP_LOCK"); }
    if flags & (1 << 3) != 0 { p.push("HAS_CONFLICT_HORIZON"); }
    if flags & (1 << 4) != 0 { p.push("HAS_FREEZE_PLANS"); }
    if flags & (1 << 5) != 0 { p.push("HAS_REDIRECTIONS"); }
    if flags & (1 << 6) != 0 { p.push("HAS_DEAD_ITEMS"); }
    if flags & (1 << 7) != 0 { p.push("HAS_NOW_UNUSED_ITEMS"); }
    if p.is_empty() { format!("0x{:02x}", flags) } else { format!("0x{:02x} ({})", flags, p.join(" | ")) }
}

fn fmt_itemptr(bi_hi: u16, bi_lo: u16, posid: u16) -> String {
    let blkno = ((bi_hi as u32) << 16) | (bi_lo as u32);
    format!("({}, {})", blkno, posid)
}

pub fn describe_heap2_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = Heap2Op::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        Heap2Op::PruneOnAccess | Heap2Op::PruneVacuumScan | Heap2Op::PruneVacuumCleanup => {
            let reason = r.read_u8().unwrap_or(0);
            let flags  = r.read_u8().unwrap_or(0);
            lines.push(format!("  reason:  {} ({})", reason, fmt_prune_reason(reason)));
            lines.push(format!("  flags:   {}", fmt_prune_flags(flags)));
            if flags & (1 << 3) != 0 {
                let xid = r.read_u32_le().unwrap_or(0);
                lines.push(format!("  conflict_horizon: {}", xid));
            }
        }

        Heap2Op::Visible => {
            let horizon = r.read_u32_le().unwrap_or(0);
            let flags   = r.read_u8().unwrap_or(0);
            lines.push(format!("  snapshotConflictHorizon: {}", horizon));
            lines.push(format!("  flags: {}", decode_xl_heap_visible_flags(flags)));
        }

        Heap2Op::MultiInsert | Heap2Op::MultiInsertInit => {
            let flags   = r.read_u8().unwrap_or(0);
            r.skip(1);
            let ntuples = r.read_u16_le().unwrap_or(0);
            let init_page = info & 0x80 != 0;
            lines.push(format!("  flags:    {}", decode_heap_insert_flags(flags)));
            lines.push(format!("  ntuples:  {}", ntuples));
            if init_page {
                lines.push("  note:     INIT_PAGE (offsets omitted)".to_string());
            } else {
                let show = (ntuples as usize).min(16);
                for i in 0..show {
                    if let Some(off) = r.read_u16_le() {
                        lines.push(format!("  offsets[{}]: {}", i, off));
                    }
                }
                if ntuples as usize > 16 {
                    lines.push(format!("  ... ({} more offsets)", ntuples - 16));
                }
            }
        }

        Heap2Op::LockUpdated => {
            let xmax     = r.read_u32_le().unwrap_or(0);
            let offnum   = r.read_u16_le().unwrap_or(0);
            let infobits = r.read_u8().unwrap_or(0);
            let flags    = r.read_u8().unwrap_or(0);
            lines.push(format!("  xmax:         {}", xmax));
            lines.push(format!("  offnum:       {}", offnum));
            lines.push(format!("  infobits_set: {}", decode_infobits(infobits)));
            lines.push(format!(
                "  flags:        0x{:02x}{}",
                flags,
                if flags & 0x01 != 0 { " (ALL_FROZEN_CLEARED)" } else { "" }
            ));
        }

        Heap2Op::NewCid => {
            let top_xid  = r.read_u32_le().unwrap_or(0);
            let cmin     = r.read_u32_le().unwrap_or(0);
            let cmax     = r.read_u32_le().unwrap_or(0);
            let combocid = r.read_u32_le().unwrap_or(0);
            let spc      = r.read_u32_le().unwrap_or(0);
            let db       = r.read_u32_le().unwrap_or(0);
            let rel      = r.read_u32_le().unwrap_or(0);
            let bi_hi    = r.read_u16_le().unwrap_or(0);
            let bi_lo    = r.read_u16_le().unwrap_or(0);
            let posid    = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  top_xid:        {}", top_xid));
            lines.push(format!("  cmin:           {}", cmin));
            lines.push(format!("  cmax:           {}", cmax));
            lines.push(format!("  combocid:       {}", combocid));
            lines.push(format!("  target_locator: {}/{}/{}", spc, db, rel));
            lines.push(format!("  target_tid:     {}", fmt_itemptr(bi_hi, bi_lo, posid)));
        }

        Heap2Op::Rewrite => {
            let mapped_xid  = r.read_u32_le().unwrap_or(0);
            let mapped_db   = r.read_u32_le().unwrap_or(0);
            let mapped_rel  = r.read_u32_le().unwrap_or(0);
            r.skip(4);
            let offset      = r.read_i64_le().unwrap_or(0);
            let num_mappings = r.read_u32_le().unwrap_or(0);
            r.skip(4);
            let start_lsn   = r.read_u64_le().unwrap_or(0);
            lines.push(format!("  mapped_xid:   {}", mapped_xid));
            lines.push(format!("  mapped_db:    {}", mapped_db));
            lines.push(format!("  mapped_rel:   {}", mapped_rel));
            lines.push(format!("  offset:       {}", offset));
            lines.push(format!("  num_mappings: {}", num_mappings));
            lines.push(format!("  start_lsn:    {}", lsn_format(start_lsn)));
        }

        Heap2Op::Unknown => {
            lines.push(format!("  ({} bytes, unknown heap2 op)", main.len()));
        }
    }
    lines
}

pub fn describe_heap2_block_data(_info: u8, _block_idx: usize, data: &[u8]) -> Vec<String> {
    vec![format!("  block data: {} bytes (multi-insert tuple data)", data.len())]
}
