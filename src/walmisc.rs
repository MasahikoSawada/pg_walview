use crate::bindings::*;
use std::mem;

pub const BLCKSZ: u32 = 8192;
pub const XLOG_SEGMENT_SIZE: u64 = 16 * 1024 * 1024;
pub const XLOG_BLCKSZ: usize = 8192;
pub const XLOG_PAGE_MAGIC: u16 = 0xD118;
pub const XLP_FIRST_IS_CONTRECORD: u16 = 0x0001;
pub const XLP_LONG_HEADER: u16 = 0x0002;
pub const INVALID_BLOCK_NUMBER: BlockNumber = 0xFFFFFFFF;

pub const XLR_MAX_BLOCK_ID: u8 = 32;
pub const XLR_BLOCK_ID_DATA_SHORT: u8 = 255;
pub const XLR_BLOCK_ID_DATA_LONG: u8 = 254;
pub const XLR_BLOCK_ID_ORIGIN: u8 = 253;
pub const XLR_BLOCK_ID_TOPLEVEL_XID: u8 = 252;

pub const BKPBLOCK_FORK_MASK: u8 = 0x0F;
pub const BKPBLOCK_FLAG_MASK: u8 = 0xF0;
pub const BKPBLOCK_HAS_IMAGE: u8 = 0x10;
pub const BKPBLOCK_HAS_DATA: u8 = 0x20;
pub const BKPBLOCK_WILL_INIT: u8 = 0x30;
pub const BKPBLOCK_SAME_REL: u8 = 0x80;

pub const BKPIMAGE_HAS_HOLE: u8 = 0x01;
pub const BKPIMAGE_APPLY: u8 = 0x01;
pub const BKPIMAGE_BKPIMAGE_COMPRESS_PGLZ: u8 = 0x04;
pub const BKPIMAGE_BKPIMAGE_COMPRESS_LZ4: u8 = 0x08;
pub const BKPIMAGE_BKPIMAGE_COMPRESS_ZSTD: u8 = 0x10;

impl XLogPageHeaderData {
    pub fn from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() < mem::size_of::<Self>() {
            return None;
        }
        Some(unsafe { &*(data.as_ptr() as *const Self) })
    }

    pub fn is_long(&self) -> bool {
        (self.xlp_info & XLP_LONG_HEADER) != 0
    }

    pub fn header_size(&self) -> usize {
        if self.is_long() {
            mem::size_of::<XLogLongPageHeaderData>()
        } else {
            mem::size_of::<XLogPageHeaderData>()
        }
    }
}

impl XLogRecord {
    pub fn from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() < mem::size_of::<Self>() {
            return None;
        }
        Some(unsafe { &*(data.as_ptr() as *const Self) })
    }
}

pub fn lsn_format(lsn: XLogRecPtr) -> String {
    let hi: u32 = (lsn >> 32) as u32;
    let lo: u32 = lsn as u32;

    format!("{:X}/{:08X}", hi, lo)
}

pub fn format_rel(r: &RelFileLocator) -> String {
    format!("{}/{}/{}", r.spcOid, r.dbOid, r.relNumber)
}

// ---------------------------------------------------------------------------
// Simple byte reader — used by all *desc parsers
// ---------------------------------------------------------------------------

pub struct Reader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        if self.pos < self.buf.len() {
            let v = self.buf[self.pos];
            self.pos += 1;
            Some(v)
        } else {
            None
        }
    }

    pub fn read_bool(&mut self) -> Option<bool> {
        self.read_u8().map(|v| v != 0)
    }

    pub fn read_u16_le(&mut self) -> Option<u16> {
        if self.pos + 2 <= self.buf.len() {
            let v = u16::from_le_bytes(self.buf[self.pos..self.pos + 2].try_into().unwrap());
            self.pos += 2;
            Some(v)
        } else {
            None
        }
    }

    pub fn read_u32_le(&mut self) -> Option<u32> {
        if self.pos + 4 <= self.buf.len() {
            let v = u32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
            self.pos += 4;
            Some(v)
        } else {
            None
        }
    }

    pub fn read_i32_le(&mut self) -> Option<i32> {
        self.read_u32_le().map(|v| v as i32)
    }

    pub fn read_u64_le(&mut self) -> Option<u64> {
        if self.pos + 8 <= self.buf.len() {
            let v = u64::from_le_bytes(self.buf[self.pos..self.pos + 8].try_into().unwrap());
            self.pos += 8;
            Some(v)
        } else {
            None
        }
    }

    pub fn read_i64_le(&mut self) -> Option<i64> {
        self.read_u64_le().map(|v| v as i64)
    }

    pub fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.buf.len());
    }

    pub fn align_to(&mut self, align: usize) {
        if align > 1 {
            let r = self.pos % align;
            if r != 0 {
                self.pos += align - r;
                self.pos = self.pos.min(self.buf.len());
            }
        }
    }

    pub fn read_cstr(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        let s = String::from_utf8_lossy(&self.buf[start..self.pos]).into_owned();
        if self.pos < self.buf.len() {
            self.pos += 1; // skip null
        }
        s
    }

    pub fn peek_bytes(&self, n: usize) -> &'a [u8] {
        let end = (self.pos + n).min(self.buf.len());
        &self.buf[self.pos..end]
    }
}

// ---------------------------------------------------------------------------
// Timestamp helper — shared by xlog, xact, commit_ts parsers
// ---------------------------------------------------------------------------

/// Convert PostgreSQL TimestampTz (microseconds since 2000-01-01 UTC) to
/// a human-readable string.
pub fn fmt_pg_ts(pg_us: i64) -> String {
    const PG_EPOCH_OFFSET: i64 = 946_684_800; // 2000-01-01 in Unix seconds
    let unix_secs = pg_us / 1_000_000 + PG_EPOCH_OFFSET;
    let us = (pg_us.rem_euclid(1_000_000)) as u32;

    let secs_in_day: i64 = 86_400;
    let rem = unix_secs.rem_euclid(secs_in_day);
    let days = (unix_secs - rem) / secs_in_day;

    let h = (rem / 3600) as u32;
    let m = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;

    let (year, month, day) = civil_from_days(days);
    if us == 0 {
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", year, month, day, h, m, s)
    } else {
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06} UTC", year, month, day, h, m, s, us)
    }
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}
