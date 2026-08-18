use crate::bindings::*;
use std::mem;

/// Data page size the server was built with.  Not recorded in WAL, so it has
/// to come from the headers we were built against; it is only used to derive
/// the length of the "hole" in an uncompressed full-page image.
pub use crate::bindings::BLCKSZ;

/// Default WAL page size.  The authoritative value for a given segment is
/// `xlp_xlog_blcksz` in its long page header; this is only what we assume
/// while reading that very first header.
pub const DEFAULT_XLOG_BLCKSZ: usize = crate::bindings::XLOG_BLCKSZ as usize;

pub const XLP_FIRST_IS_CONTRECORD: u16 = 0x0001;
pub const XLP_LONG_HEADER: u16 = 0x0002;
pub const XLP_FIRST_IS_OVERWRITE_CONTRECORD: u16 = 0x0004;
pub const XLP_ALL_FLAGS: u16 = 0x0007;

pub const INVALID_BLOCK_NUMBER: BlockNumber = 0xFFFFFFFF;

pub const XLR_MAX_BLOCK_ID: u8 = 32;
pub const XLR_BLOCK_ID_DATA_SHORT: u8 = 255;
pub const XLR_BLOCK_ID_DATA_LONG: u8 = 254;
pub const XLR_BLOCK_ID_ORIGIN: u8 = 253;
pub const XLR_BLOCK_ID_TOPLEVEL_XID: u8 = 252;

pub const BKPBLOCK_FORK_MASK: u8 = 0x0F;
pub const BKPBLOCK_HAS_IMAGE: u8 = 0x10;
pub const BKPBLOCK_HAS_DATA: u8 = 0x20;
pub const BKPBLOCK_WILL_INIT: u8 = 0x40;
pub const BKPBLOCK_SAME_REL: u8 = 0x80;

pub const BKPIMAGE_HAS_HOLE: u8 = 0x01;
pub const BKPIMAGE_APPLY: u8 = 0x02;
pub const BKPIMAGE_COMPRESS_PGLZ: u8 = 0x04;
pub const BKPIMAGE_COMPRESS_LZ4: u8 = 0x08;
pub const BKPIMAGE_COMPRESS_ZSTD: u8 = 0x10;

/// Same as PostgreSQL's `BKPIMAGE_COMPRESSED()`.
pub fn bkpimage_compressed(bimg_info: u8) -> bool {
    (bimg_info & (BKPIMAGE_COMPRESS_PGLZ | BKPIMAGE_COMPRESS_LZ4 | BKPIMAGE_COMPRESS_ZSTD)) != 0
}

/// `SizeOfXLogRecord`.  The C struct has no trailing padding beyond `xl_crc`,
/// so `size_of` matches; `record_header_checks()` in the tests asserts it.
pub const SIZE_OF_XLOG_RECORD: usize = mem::size_of::<XLogRecord>();

/// `offsetof(XLogRecord, xl_crc)` — the CRC covers the header only up to here.
pub const XLOG_RECORD_CRC_OFFSET: usize = mem::offset_of!(XLogRecord, xl_crc);

/// `XLogRecordMaxSize` from access/xlogrecord.h.
pub const XLOG_RECORD_MAX_SIZE: u32 = 1020 * 1024 * 1024;

/// PostgreSQL's `MAXALIGN()`.  MAXIMUM_ALIGNOF is 8 on every platform we
/// can read WAL for.
pub const MAXIMUM_ALIGNOF: usize = 8;

pub fn maxalign(n: usize) -> usize {
    n.next_multiple_of(MAXIMUM_ALIGNOF)
}

/// First transaction id that a real transaction can be assigned;
/// `FirstNormalTransactionId` in access/transam.h.
pub const FIRST_NORMAL_TRANSACTION_ID: TransactionId = 3;

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

impl XLogLongPageHeaderData {
    pub fn from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() < mem::size_of::<Self>() {
            return None;
        }
        Some(unsafe { &*(data.as_ptr() as *const Self) })
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
// Simple byte reader — used by the record decoder and all *desc parsers
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

    /// Consume exactly `n` bytes, or nothing at all if fewer remain.
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        if end > self.buf.len() {
            return None;
        }
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Some(s)
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
    let unix_secs = pg_us.div_euclid(1_000_000) + PG_EPOCH_OFFSET;
    let us = (pg_us.rem_euclid(1_000_000)) as u32;

    let secs_in_day: i64 = 86_400;
    let rem = unix_secs.rem_euclid(secs_in_day);
    let days = (unix_secs - rem) / secs_in_day;

    let h = (rem / 3600) as u32;
    let m = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;

    let (year, month, day) = civil_from_days(days);
    if us == 0 {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
            year, month, day, h, m, s
        )
    } else {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06} UTC",
            year, month, day, h, m, s, us
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The decoder relies on `size_of::<XLogRecord>()` being SizeOfXLogRecord
    /// (i.e. the C struct having no trailing padding) and on the CRC covering
    /// the header up to `xl_crc`.
    #[test]
    fn record_header_checks() {
        assert_eq!(SIZE_OF_XLOG_RECORD, 24);
        assert_eq!(XLOG_RECORD_CRC_OFFSET, 20);
        assert_eq!(mem::size_of::<XLogPageHeaderData>(), 24);
        assert_eq!(mem::size_of::<XLogLongPageHeaderData>(), 40);
    }

    #[test]
    fn bkp_flag_values_match_postgres() {
        // access/xlogrecord.h
        assert_eq!(BKPBLOCK_HAS_IMAGE, 0x10);
        assert_eq!(BKPBLOCK_HAS_DATA, 0x20);
        assert_eq!(BKPBLOCK_WILL_INIT, 0x40);
        assert_eq!(BKPBLOCK_SAME_REL, 0x80);
        assert_eq!(BKPIMAGE_HAS_HOLE, 0x01);
        assert_eq!(BKPIMAGE_APPLY, 0x02);
        // HAS_HOLE and APPLY must not collide.
        assert_ne!(BKPIMAGE_HAS_HOLE, BKPIMAGE_APPLY);
        assert!(bkpimage_compressed(BKPIMAGE_COMPRESS_LZ4));
        assert!(!bkpimage_compressed(BKPIMAGE_HAS_HOLE | BKPIMAGE_APPLY));
    }

    #[test]
    fn reader_never_reads_past_the_end() {
        let mut r = Reader::new(&[0x01, 0x02, 0x03]);
        assert_eq!(r.read_u16_le(), Some(0x0201));
        assert_eq!(r.remaining(), 1);
        assert_eq!(r.read_u16_le(), None);
        assert_eq!(r.read_u32_le(), None);
        assert_eq!(r.read_u64_le(), None);
        // A failed read must not consume anything.
        assert_eq!(r.remaining(), 1);
        assert_eq!(r.read_u8(), Some(0x03));
        assert_eq!(r.read_u8(), None);
    }

    #[test]
    fn reader_take_is_all_or_nothing() {
        let mut r = Reader::new(&[1, 2, 3, 4]);
        assert_eq!(r.take(5), None);
        assert_eq!(r.pos, 0);
        assert_eq!(r.take(4), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(r.take(1), None);
        // No overflow panic on an absurd length.
        let mut r = Reader::new(&[1, 2, 3, 4]);
        assert_eq!(r.take(usize::MAX), None);
    }

    #[test]
    fn reader_align_and_cstr() {
        let mut r = Reader::new(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
        r.skip(3);
        r.align_to(8);
        assert_eq!(r.pos, 8);
        r.align_to(8);
        assert_eq!(r.pos, 8);

        // Unterminated string must not run off the end.
        let mut r = Reader::new(b"abc");
        assert_eq!(r.read_cstr(), "abc");
        assert_eq!(r.remaining(), 0);

        let mut r = Reader::new(b"ab\0cd");
        assert_eq!(r.read_cstr(), "ab");
        assert_eq!(r.remaining(), 2);
    }

    #[test]
    fn lsn_formatting() {
        assert_eq!(lsn_format(0), "0/00000000");
        assert_eq!(lsn_format(0x1_0000_0000), "1/00000000");
        assert_eq!(lsn_format(0x0000_0001_ABCD_EF01), "1/ABCDEF01");
    }

    #[test]
    fn pg_timestamps() {
        assert_eq!(fmt_pg_ts(0), "2000-01-01 00:00:00 UTC");
        assert_eq!(fmt_pg_ts(1_000_000), "2000-01-01 00:00:01 UTC");
        assert_eq!(fmt_pg_ts(500_000), "2000-01-01 00:00:00.500000 UTC");
        // 2000 and 2024 are leap years, 2100 is not.
        assert_eq!(
            fmt_pg_ts(59 * 86_400 * 1_000_000),
            "2000-02-29 00:00:00 UTC"
        );
        assert_eq!(
            fmt_pg_ts(60 * 86_400 * 1_000_000),
            "2000-03-01 00:00:00 UTC"
        );
        // Timestamps before the PostgreSQL epoch must not go off by a day.
        assert_eq!(fmt_pg_ts(-1_000_000), "1999-12-31 23:59:59 UTC");
    }

    #[test]
    fn maxalign_matches_postgres() {
        assert_eq!(maxalign(0), 0);
        assert_eq!(maxalign(1), 8);
        assert_eq!(maxalign(8), 8);
        assert_eq!(maxalign(9), 16);
    }
}
