use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::{WALRecordInfo, XLR_RMGR_INFO_MASK};

// Heap xl_info op values (PG17)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeapOp {
    Insert,
    Delete,
    Update,
    Truncate,
    HotUpdate,
    Confirm,
    Lock,
    Inplace,
    InsertInit,
    UpdateInit,
    HotUpdateInit,
    Unknown,
}

impl HeapOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => HeapOp::Insert,
	    0x10 => HeapOp::Delete,
	    0x20 => HeapOp::Update,
	    0x30 => HeapOp::Truncate,
	    0x40 => HeapOp::HotUpdate,
	    0x50 => HeapOp::Confirm,
	    0x60 => HeapOp::Lock,
	    0x70 => HeapOp::Inplace,
	    0x80 => HeapOp::InsertInit,
	    0xA0 => HeapOp::UpdateInit,
	    0xB0 => HeapOp::HotUpdateInit,
	    _ => HeapOp::Unknown,
	}
    }
}

impl fmt::Display for HeapOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    HeapOp::Insert => write!(f, "INSERT"),
	    HeapOp::Delete => write!(f, "DELETE"),
	    HeapOp::Update => write!(f, "UPDATE"),
	    HeapOp::Truncate => write!(f, "TRUNCATE"),
	    HeapOp::HotUpdate => write!(f, "HOT_UPDATE"),
	    HeapOp::Confirm => write!(f, "CONFIRM"),
	    HeapOp::Lock => write!(f, "LOCK"),
	    HeapOp::Inplace => write!(f, "INPLACE"),
	    HeapOp::InsertInit => write!(f, "INSERT+INIT"),
	    HeapOp::UpdateInit => write!(f, "UPDATE+INIT"),
	    HeapOp::HotUpdateInit => write!(f, "HOT_UPDATE+INIT"),
	    HeapOp::Unknown => write!(f, "Unknown"),
	}
    }
}

// ---------------------------------------------------------------------------
// Flag / infobits decoders (pub so heap2desc can reuse them)
// ---------------------------------------------------------------------------

pub fn decode_heap_insert_flags(flags: u8) -> String {
    let mut p = Vec::new();
    if flags & (1 << 0) != 0 { p.push("ALL_VISIBLE_CLEARED"); }
    if flags & (1 << 1) != 0 { p.push("LAST_IN_MULTI"); }
    if flags & (1 << 2) != 0 { p.push("IS_SPECULATIVE"); }
    if flags & (1 << 3) != 0 { p.push("CONTAINS_NEW_TUPLE"); }
    if flags & (1 << 4) != 0 { p.push("ON_TOAST_RELATION"); }
    if flags & (1 << 5) != 0 { p.push("ALL_FROZEN_SET"); }
    if p.is_empty() { format!("0x{:02x}", flags) } else { format!("0x{:02x} ({})", flags, p.join(" | ")) }
}

pub fn decode_infobits(b: u8) -> String {
    let mut p = Vec::new();
    if b & 0x01 != 0 { p.push("XMAX_IS_MULTI"); }
    if b & 0x02 != 0 { p.push("XMAX_LOCK_ONLY"); }
    if b & 0x04 != 0 { p.push("XMAX_EXCL_LOCK"); }
    if b & 0x08 != 0 { p.push("XMAX_KEYSHR_LOCK"); }
    if b & 0x10 != 0 { p.push("KEYS_UPDATED"); }
    if p.is_empty() { format!("0x{:02x}", b) } else { format!("0x{:02x} ({})", b, p.join(" | ")) }
}

fn decode_heap_delete_flags(flags: u8) -> String {
    let mut p = Vec::new();
    if flags & (1 << 0) != 0 { p.push("ALL_VISIBLE_CLEARED"); }
    if flags & (1 << 1) != 0 { p.push("CONTAINS_OLD_TUPLE"); }
    if flags & (1 << 2) != 0 { p.push("CONTAINS_OLD_KEY"); }
    if flags & (1 << 3) != 0 { p.push("IS_SUPER"); }
    if flags & (1 << 4) != 0 { p.push("IS_PARTITION_MOVE"); }
    if p.is_empty() { format!("0x{:02x}", flags) } else { format!("0x{:02x} ({})", flags, p.join(" | ")) }
}

fn decode_heap_update_flags(flags: u8) -> String {
    let mut p = Vec::new();
    if flags & (1 << 0) != 0 { p.push("OLD_ALL_VISIBLE_CLEARED"); }
    if flags & (1 << 1) != 0 { p.push("NEW_ALL_VISIBLE_CLEARED"); }
    if flags & (1 << 2) != 0 { p.push("CONTAINS_OLD_TUPLE"); }
    if flags & (1 << 3) != 0 { p.push("CONTAINS_OLD_KEY"); }
    if flags & (1 << 4) != 0 { p.push("CONTAINS_NEW_TUPLE"); }
    if flags & (1 << 5) != 0 { p.push("PREFIX_FROM_OLD"); }
    if flags & (1 << 6) != 0 { p.push("SUFFIX_FROM_OLD"); }
    if p.is_empty() { format!("0x{:02x}", flags) } else { format!("0x{:02x} ({})", flags, p.join(" | ")) }
}

fn decode_t_infomask(mask: u16) -> String {
    let mut p = Vec::new();
    if mask & 0x0001 != 0 { p.push("HASNULL"); }
    if mask & 0x0002 != 0 { p.push("HASVARWIDTH"); }
    if mask & 0x0004 != 0 { p.push("HASEXTERNAL"); }
    if mask & 0x0008 != 0 { p.push("HASOID_OLD"); }
    if mask & 0x0010 != 0 { p.push("XMAX_KEYSHR_LOCK"); }
    if mask & 0x0020 != 0 { p.push("COMBOCID"); }
    if mask & 0x0040 != 0 { p.push("XMAX_EXCL_LOCK"); }
    if mask & 0x0080 != 0 { p.push("XMAX_LOCK_ONLY"); }
    if mask & 0x0100 != 0 { p.push("XMIN_COMMITTED"); }
    if mask & 0x0200 != 0 { p.push("XMIN_INVALID"); }
    if mask & 0x0400 != 0 { p.push("XMAX_COMMITTED"); }
    if mask & 0x0800 != 0 { p.push("XMAX_INVALID"); }
    if mask & 0x1000 != 0 { p.push("XMAX_IS_MULTI"); }
    if mask & 0x2000 != 0 { p.push("UPDATED"); }
    if mask & 0x4000 != 0 { p.push("MOVED_OFF"); }
    if mask & 0x8000 != 0 { p.push("MOVED_IN"); }
    if p.is_empty() { format!("0x{:04x}", mask) } else { format!("0x{:04x} ({})", mask, p.join("|")) }
}

fn decode_t_infomask2(mask: u16) -> String {
    let natts = mask & 0x07FF;
    let mut p = Vec::new();
    if mask & 0x0800 != 0 { p.push("KEYS_UPDATED"); }
    if mask & 0x1000 != 0 { p.push("HOT_UPDATED"); }
    if mask & 0x2000 != 0 { p.push("ONLY_FROZEN"); }
    if mask & 0x4000 != 0 { p.push("NOT_ONLY_FROZEN"); }
    if mask & 0x8000 != 0 { p.push("HEAP_ONLY"); }
    format!(
        "0x{:04x} (natts={}, {})",
        mask,
        natts,
        if p.is_empty() { "-".to_string() } else { p.join("|") }
    )
}

/// Parse an xl_heap_header (5 bytes) and return descriptive lines + bytes consumed.
pub fn parse_xl_heap_header(data: &[u8]) -> (Vec<String>, usize) {
    if data.len() < 5 {
        return (vec!["  xl_heap_header: (truncated)".to_string()], 0);
    }
    let mut r = Reader::new(data);
    let t_infomask2 = r.read_u16_le().unwrap();
    let t_infomask  = r.read_u16_le().unwrap();
    let t_hoff      = r.read_u8().unwrap();
    let lines = vec![
        format!("  t_infomask2: {}", decode_t_infomask2(t_infomask2)),
        format!("  t_infomask:  {}", decode_t_infomask(t_infomask)),
        format!("  t_hoff:      {} (header size incl. nulls bitmap)", t_hoff),
    ];
    (lines, r.pos)
}

// ---------------------------------------------------------------------------
// Main-data parser
// ---------------------------------------------------------------------------

pub fn describe_heap_main(info: u8, main: &[u8], _record: &WALRecordInfo) -> Vec<String> {
    let op = HeapOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        HeapOp::Insert | HeapOp::InsertInit => {
            let offnum = r.read_u16_le().unwrap_or(0);
            let flags  = r.read_u8().unwrap_or(0);
            lines.push(format!("  offnum:  {}", offnum));
            lines.push(format!("  flags:   {}", decode_heap_insert_flags(flags)));
            if info & 0x80 != 0 {
                lines.push("  note:    INIT_PAGE (new page)".to_string());
            }
            lines.push("  tuple:   (see Block #0 data)".to_string());
        }

        HeapOp::Delete => {
            let xmax     = r.read_u32_le().unwrap_or(0);
            let offnum   = r.read_u16_le().unwrap_or(0);
            let infobits = r.read_u8().unwrap_or(0);
            let flags    = r.read_u8().unwrap_or(0);
            lines.push(format!("  xmax:         {}", xmax));
            lines.push(format!("  offnum:       {}", offnum));
            lines.push(format!("  infobits_set: {}", decode_infobits(infobits)));
            lines.push(format!("  flags:        {}", decode_heap_delete_flags(flags)));
            if flags & (1 << 1) != 0 || flags & (1 << 2) != 0 {
                lines.push("  old tuple header:".to_string());
                let (hlines, _) = parse_xl_heap_header(r.peek_bytes(5));
                lines.extend(hlines.into_iter().map(|s| format!("  {}", s)));
            }
        }

        HeapOp::Update | HeapOp::HotUpdate | HeapOp::UpdateInit | HeapOp::HotUpdateInit => {
            let old_xmax     = r.read_u32_le().unwrap_or(0);
            let old_offnum   = r.read_u16_le().unwrap_or(0);
            let old_infobits = r.read_u8().unwrap_or(0);
            let flags        = r.read_u8().unwrap_or(0);
            let new_xmax     = r.read_u32_le().unwrap_or(0);
            let new_offnum   = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  old_xmax:         {}", old_xmax));
            lines.push(format!("  old_offnum:       {}", old_offnum));
            lines.push(format!("  old_infobits_set: {}", decode_infobits(old_infobits)));
            lines.push(format!("  flags:            {}", decode_heap_update_flags(flags)));
            lines.push(format!("  new_xmax:         {}", new_xmax));
            lines.push(format!("  new_offnum:       {}", new_offnum));
            if flags & (1 << 2) != 0 || flags & (1 << 3) != 0 {
                lines.push("  old tuple header:".to_string());
                let (hlines, _) = parse_xl_heap_header(r.peek_bytes(5));
                lines.extend(hlines.into_iter().map(|s| format!("  {}", s)));
            }
            lines.push("  new tuple: (see Block #0 data)".to_string());
        }

        HeapOp::Truncate => {
            let db_id    = r.read_u32_le().unwrap_or(0);
            let nrelids  = r.read_u32_le().unwrap_or(0);
            let flags    = r.read_u8().unwrap_or(0);
            r.skip(3);
            lines.push(format!("  dbId:    {}", db_id));
            lines.push(format!("  nrelids: {}", nrelids));
            lines.push(format!(
                "  flags:   0x{:02x}{}{}",
                flags,
                if flags & 0x01 != 0 { " CASCADE" } else { "" },
                if flags & 0x02 != 0 { " RESTART_SEQS" } else { "" }
            ));
            let show = nrelids.min(16) as usize;
            for i in 0..show {
                if let Some(relid) = r.read_u32_le() {
                    lines.push(format!("  relids[{}]: {}", i, relid));
                }
            }
            if nrelids > 16 {
                lines.push(format!("  ... ({} more relids)", nrelids - 16));
            }
        }

        HeapOp::Confirm => {
            let offnum = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  offnum:  {} (speculative insertion confirmed)", offnum));
        }

        HeapOp::Lock => {
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

        HeapOp::Inplace => {
            let offnum        = r.read_u16_le().unwrap_or(0);
            r.skip(2);
            let db_id         = r.read_u32_le().unwrap_or(0);
            let ts_id         = r.read_u32_le().unwrap_or(0);
            let relcache_inval = r.read_bool().unwrap_or(false);
            r.skip(3);
            let nmsgs         = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  offnum:                {}", offnum));
            lines.push(format!("  dbId:                  {}", db_id));
            lines.push(format!("  tsId:                  {}", ts_id));
            lines.push(format!("  relcacheInitFileInval: {}", relcache_inval));
            lines.push(format!("  nmsgs:                 {}", nmsgs));
        }

        HeapOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown heap op)", main.len()));
        }
    }
    lines
}

// ---------------------------------------------------------------------------
// Block-data parser
// ---------------------------------------------------------------------------

pub fn describe_heap_block_data(
    info: u8,
    block_idx: usize,
    data: &[u8],
    record: &WALRecordInfo,
) -> Vec<String> {
    let op = HeapOp::from_xl_info(info);
    let mut lines = Vec::new();

    match op {
        HeapOp::Insert => {
            if block_idx == 0 {
                lines.push("  xl_heap_header:".to_string());
                let (hlines, consumed) = parse_xl_heap_header(data);
                lines.extend(hlines);
                let tuple_len = data.len().saturating_sub(consumed);
                if tuple_len > 0 {
                    lines.push(format!("  tuple data:  {} bytes", tuple_len));
                }
            }
        }
        HeapOp::Update | HeapOp::HotUpdate => {
            if block_idx == 0 {
                let mut offset = 0usize;
                if let Some(main) = &record.main {
                    if main.len() >= 8 {
                        let flags = main[7];
                        if flags & (1 << 5) != 0 {
                            lines.push(format!(
                                "  prefix_from_old: {} bytes",
                                u16::from_le_bytes([data[0], data[1]])
                            ));
                            offset += 2;
                        }
                        if flags & (1 << 6) != 0 {
                            lines.push(format!(
                                "  suffix_from_old: {} bytes",
                                u16::from_le_bytes([data[offset], data[offset + 1]])
                            ));
                            offset += 2;
                        }
                    }
                }
                lines.push("  new xl_heap_header:".to_string());
                let (hlines, consumed) = parse_xl_heap_header(&data[offset..]);
                lines.extend(hlines);
                let tuple_len = data.len().saturating_sub(offset + consumed);
                if tuple_len > 0 {
                    lines.push(format!("  new tuple data: {} bytes", tuple_len));
                }
            }
        }
        _ => {}
    }
    lines
}
