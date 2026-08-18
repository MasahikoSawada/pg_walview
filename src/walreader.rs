use std::cmp;
use std::fs::File;
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::Path;

use thiserror::Error;

use crate::bindings::*;
use crate::crc32c::Crc32c;
use crate::walmisc::*;

pub const XLR_INFO_MASK: u8 = 0x0F;
pub const XLR_RMGR_INFO_MASK: u8 = 0xF0;

/// Sentinel for "no page is loaded".
const NO_PAGE: u32 = u32::MAX;

#[derive(Error, Debug)]
pub enum WALReaderError {
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),

    #[error("record with invalid length {} at {}", len, lsn_format(*lsn))]
    InvalidRecordLength { len: u32, lsn: XLogRecPtr },

    #[error("record is shorter than its header claims at {}", lsn_format(*lsn))]
    TruncatedRecord { lsn: XLogRecPtr },

    #[error("invalid block_id {} at {}", block_id, lsn_format(*lsn))]
    InvalidBlockId { block_id: u8, lsn: XLogRecPtr },

    #[error("out-of-order block_id {} at {}", block_id, lsn_format(*lsn))]
    OutOfOrderBlockId { block_id: u8, lsn: XLogRecPtr },

    #[error("BKPBLOCK_HAS_DATA set, but no data included at {}", lsn_format(*lsn))]
    NoDataIncluded { lsn: XLogRecPtr },

    #[error("BKPBLOCK_HAS_DATA not set, but data length is {} at {}", len, lsn_format(*lsn))]
    InvalidDataLength { len: u16, lsn: XLogRecPtr },

    #[error("BKPIMAGE_HAS_HOLE set, but hole offset {} length {} block image length {} at {}", hole_offset, hole_len, img_len, lsn_format(*lsn))]
    InvalidHoleData {
        hole_offset: u16,
        hole_len: u16,
        img_len: u16,
        lsn: XLogRecPtr,
    },

    #[error("BKPIMAGE_HAS_HOLE not set, but hole offset {} length {} at {}", hole_offset, hole_len, lsn_format(*lsn))]
    InvalidHoleFlag {
        hole_offset: u16,
        hole_len: u16,
        lsn: XLogRecPtr,
    },

    #[error("BKPIMAGE_COMPRESSED set, but block image length {} at {}", len, lsn_format(*lsn))]
    InvalidCompressFlag { len: u16, lsn: XLogRecPtr },

    #[error("BKPBLOCK_SAME_REL set but no previous rel at {}", lsn_format(*lsn))]
    SameRelNotFound { lsn: XLogRecPtr },

    #[error("incorrect resource manager data checksum in record at {}", lsn_format(*lsn))]
    InvalidRecordCrc { lsn: XLogRecPtr },

    #[error(
        "invalid magic number {:04X} at {}; pg_walview was built for PostgreSQL {} and reads only magic {:04X}",
        magic,
        lsn_format(*lsn),
        crate::buildinfo::pg_version(),
        crate::buildinfo::xlog_page_magic()
    )]
    InvalidPageMagicNumber { magic: u16, lsn: XLogRecPtr },

    #[error("invalid page flags {:04X} at {}", flags, lsn_format(*lsn))]
    InvalidPageFlags { flags: u16, lsn: XLogRecPtr },

    #[error("reached a zero-filled page at {}; end of the WAL written so far", lsn_format(*lsn))]
    ZeroedPage { lsn: XLogRecPtr },

    #[error("unexpected pageaddr {} on page at {}; this segment has most likely been recycled and not yet reused", lsn_format(*found), lsn_format(*expected))]
    UnexpectedPageAddr {
        found: XLogRecPtr,
        expected: XLogRecPtr,
    },

    #[error("out-of-sequence timeline ID {} (expected {}) at {}", found, expected, lsn_format(*lsn))]
    UnexpectedTimeLineId {
        found: TimeLineID,
        expected: TimeLineID,
        lsn: XLogRecPtr,
    },

    #[error("first page of the segment has no long header")]
    MissingLongHeader,

    #[error("the segment has not been written to yet")]
    EmptySegment,

    #[error("invalid WAL page size {} in long page header", size)]
    InvalidXLogBlockSize { size: u32 },

    #[error("invalid WAL segment size {} in long page header", size)]
    InvalidSegmentSize { size: u32 },

    #[error(
        "file is {} bytes, larger than the {} byte WAL segment size",
        file_size,
        seg_size
    )]
    FileTooLarge { file_size: u64, seg_size: u64 },
}

#[derive(Clone, Debug, Default)]
pub struct WALFullPageImage {
    pub compressed: bool,
    pub apply_image: bool,
    pub hole_offset: u16,
    pub hole_len: u16,
    pub bimg_info: u8,
    pub bimg_len: u16,
    /// Where the image bytes live inside `WALRecordInfo::raw`.
    pub bimg_range: Range<usize>,
}

// XLogRecordBlockHeader
#[derive(Clone, Debug, Default)]
pub struct WALBlockData {
    pub block_id: u8,
    pub rlocator: RelFileLocator,
    pub forknum: ForkNumber,
    pub flags: u8,
    pub blocknum: BlockNumber,

    // full-page image
    pub image: Option<WALFullPageImage>,

    // Length of data (not including page image).
    pub data_len: u16,

    /// Where the rmgr-specific block data lives inside `WALRecordInfo::raw`.
    pub data_range: Option<Range<usize>>,
}

// Struct for one WAL record.
#[derive(Debug, Clone, Default)]
pub struct WALRecordInfo {
    // Record's LSN
    pub lsn: XLogRecPtr,

    pub xlrec: XLogRecord,

    pub blocks: Vec<WALBlockData>,
    pub main_len: u32,
    /// Where the main data lives inside `raw`.
    pub main_range: Option<Range<usize>>,
    pub origin: Option<u16>,
    pub top_xid: Option<TransactionId>,

    /// Whether xl_crc matched the record contents.
    pub crc_ok: bool,

    /// Byte ranges the record occupies in the segment file.  A record that
    /// crosses a page boundary is split by the next page's header, so this
    /// has one entry per page it lives on.
    pub file_ranges: Vec<Range<usize>>,

    // Raw bytes of the full record (header + payload).  All the ranges above
    // point into this, so the payload is stored exactly once.
    pub raw: Vec<u8>,
}

impl WALRecordInfo {
    pub fn nblocks_inuse(&self) -> usize {
        self.blocks.len()
    }

    pub fn main_data(&self) -> Option<&[u8]> {
        self.main_range.clone().map(|r| &self.raw[r])
    }

    pub fn has_main_data(&self) -> bool {
        self.main_range.is_some()
    }

    pub fn block_data(&self, block_idx: usize) -> Option<&[u8]> {
        let block = self.blocks.get(block_idx)?;
        block.data_range.clone().map(|r| &self.raw[r])
    }

    pub fn block_image_data(&self, block_idx: usize) -> Option<&[u8]> {
        let image = self.blocks.get(block_idx)?.image.as_ref()?;
        Some(&self.raw[image.bimg_range.clone()])
    }
}

pub fn fork_name(forknum: ForkNumber) -> &'static str {
    match forknum {
        0 => "main",
        1 => "fsm",
        2 => "vm",
        3 => "init",
        _ => "unknown",
    }
}

impl WALBlockData {
    pub fn flags_str(&self) -> String {
        let mut parts = Vec::new();
        if self.flags & BKPBLOCK_HAS_IMAGE != 0 {
            parts.push("HAS_IMAGE");
        }
        if self.flags & BKPBLOCK_HAS_DATA != 0 {
            parts.push("HAS_DATA");
        }
        if self.flags & BKPBLOCK_WILL_INIT != 0 {
            parts.push("WILL_INIT");
        }
        if self.flags & BKPBLOCK_SAME_REL != 0 {
            parts.push("SAME_REL");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(" | ")
        }
    }
}

impl WALFullPageImage {
    pub fn compression_str(&self) -> &'static str {
        if self.bimg_info & BKPIMAGE_COMPRESS_ZSTD != 0 {
            "zstd"
        } else if self.bimg_info & BKPIMAGE_COMPRESS_LZ4 != 0 {
            "lz4"
        } else if self.bimg_info & BKPIMAGE_COMPRESS_PGLZ != 0 {
            "pglz"
        } else {
            "none"
        }
    }
}

#[derive(Debug)]
pub struct WALReader<R> {
    tli: TimeLineID,
    seg_no: u64,
    seg_size: u64,
    xlog_blcksz: usize,
    sysid: u64,

    /// Pages actually present in the file.  Less than a full segment for a
    /// partially written file (e.g. pg_receivewal's *.partial).
    readable_pages: u32,

    source: R,

    page_buffer: Vec<u8>,
    page_no: u32,

    // current read pointer. Note that this could point to anywhere, e.g.,
    // the middle of record, inside of the page header.
    read_lsn: XLogRecPtr,

    // the record's LSN (i.e., the first byte of the record).
    record_lsn: XLogRecPtr,
    record_buffer: Vec<u8>,

    /// Set once the reader has run off the end of the readable data, so that
    /// further calls keep returning None instead of spinning.
    done: bool,
}

impl WALReader<File> {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, WALReaderError> {
        let path_ref = path.as_ref();
        let fname = path_ref
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidInput, "Invalid path or non-UTF8 filename")
            })?;

        // Accept a trailing ".partial" as written by pg_receivewal.
        let stem = fname.strip_suffix(".partial").unwrap_or(fname);

        if stem.len() != 24 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("'{}' is not a WAL segment file name", fname),
            )
            .into());
        }

        let parse_hex = |s: &str| -> io::Result<u32> {
            u32::from_str_radix(s, 16)
                .map_err(|e| Error::new(ErrorKind::InvalidData, format!("Parse error: {}", e)))
        };

        let tli = parse_hex(&stem[0..8])?;
        let log = parse_hex(&stem[8..16])?;
        let seg = parse_hex(&stem[16..24])?;

        Self::new(File::open(path_ref)?, tli, log, seg)
    }
}

impl<R: Read + Seek> WALReader<R> {
    /// `tli`, `log` and `seg` come from the segment file name; the geometry
    /// (page size, segment size) comes from the long page header, so a cluster
    /// initdb'd with a non-default --wal-segsize reads correctly.
    pub fn new(mut source: R, tli: TimeLineID, log: u32, seg: u32) -> Result<Self, WALReaderError> {
        let file_size = source.seek(SeekFrom::End(0))?;

        // Read the first page assuming the default page size, just to get at
        // the long header that tells us the real geometry.
        let mut page_buffer = vec![0u8; DEFAULT_XLOG_BLCKSZ];
        source.seek(SeekFrom::Start(0))?;
        source.read_exact(&mut page_buffer)?;

        let header = XLogPageHeaderData::from_bytes(&page_buffer)
            .ok_or(WALReaderError::MissingLongHeader)?;
        if header.xlp_magic != XLOG_PAGE_MAGIC as u16 {
            // A segment that has been preallocated but never written to has
            // nothing wrong with it, it is just empty.
            if page_buffer.iter().all(|&b| b == 0) {
                return Err(WALReaderError::EmptySegment);
            }
            return Err(WALReaderError::InvalidPageMagicNumber {
                magic: header.xlp_magic,
                lsn: 0,
            });
        }
        if !header.is_long() {
            return Err(WALReaderError::MissingLongHeader);
        }

        let long = XLogLongPageHeaderData::from_bytes(&page_buffer)
            .ok_or(WALReaderError::MissingLongHeader)?;
        let xlog_blcksz = long.xlp_xlog_blcksz;
        let seg_size = long.xlp_seg_size;
        let sysid = long.xlp_sysid;

        if !xlog_blcksz.is_power_of_two()
            || xlog_blcksz < size_of::<XLogLongPageHeaderData>() as u32
            || xlog_blcksz > 1024 * 1024
        {
            return Err(WALReaderError::InvalidXLogBlockSize { size: xlog_blcksz });
        }
        if !seg_size.is_power_of_two()
            || seg_size < xlog_blcksz
            || !(1024 * 1024..=1024 * 1024 * 1024).contains(&seg_size)
        {
            return Err(WALReaderError::InvalidSegmentSize { size: seg_size });
        }

        let xlog_blcksz = xlog_blcksz as usize;
        let seg_size = seg_size as u64;

        if file_size > seg_size {
            return Err(WALReaderError::FileTooLarge {
                file_size,
                seg_size,
            });
        }

        // Now that the real page size is known, size the buffer accordingly.
        page_buffer.resize(xlog_blcksz, 0);

        let seg_no = log as u64 * (0x1_0000_0000u64 / seg_size) + seg as u64;
        let readable_pages = (file_size / xlog_blcksz as u64) as u32;

        let mut reader = Self {
            tli,
            seg_no,
            seg_size,
            xlog_blcksz,
            sysid,
            readable_pages,
            source,
            page_buffer,
            page_no: NO_PAGE,
            read_lsn: seg_no * seg_size,
            record_lsn: 0,
            record_buffer: Vec::with_capacity(8192),
            done: false,
        };

        // Runs the full page-header validation (pageaddr, tli, flags) that the
        // quick checks above skipped.
        reader.load_page(0)?;

        Ok(reader)
    }

    pub fn tli(&self) -> TimeLineID {
        self.tli
    }

    pub fn seg_size(&self) -> u64 {
        self.seg_size
    }

    /// LSN of the first byte of the segment.
    pub fn segment_start_lsn(&self) -> XLogRecPtr {
        self.seg_no * self.seg_size
    }

    pub fn xlog_blcksz(&self) -> usize {
        self.xlog_blcksz
    }

    pub fn sysid(&self) -> u64 {
        self.sysid
    }

    /// LSN just past the last byte the reader has consumed.
    pub fn read_lsn(&self) -> XLogRecPtr {
        self.read_lsn
    }

    // -----------------------------------------------------------------------
    // LSN / page arithmetic — keep the casts in one place.
    // -----------------------------------------------------------------------

    fn pages_per_segment(&self) -> u32 {
        (self.seg_size / self.xlog_blcksz as u64) as u32
    }

    fn lsn_to_page_no(&self, lsn: XLogRecPtr) -> u32 {
        ((lsn % self.seg_size) / self.xlog_blcksz as u64) as u32
    }

    fn lsn_to_page_offset(&self, lsn: XLogRecPtr) -> usize {
        ((lsn % self.seg_size) % self.xlog_blcksz as u64) as usize
    }

    fn page_start_lsn(&self, page_no: u32) -> XLogRecPtr {
        self.seg_no * self.seg_size + page_no as u64 * self.xlog_blcksz as u64
    }

    // -----------------------------------------------------------------------

    fn load_page(&mut self, page_no: u32) -> Result<(), WALReaderError> {
        self.source
            .seek(SeekFrom::Start(page_no as u64 * self.xlog_blcksz as u64))?;
        self.source.read_exact(&mut self.page_buffer)?;

        self.validate_page_header(page_no)?;

        self.page_no = page_no;

        Ok(())
    }

    /// Equivalent of PostgreSQL's XLogReaderValidatePageHeader().
    fn validate_page_header(&self, page_no: u32) -> Result<(), WALReaderError> {
        let page_lsn = self.page_start_lsn(page_no);
        let header = XLogPageHeaderData::from_bytes(&self.page_buffer)
            .ok_or(WALReaderError::ZeroedPage { lsn: page_lsn })?;

        if header.xlp_magic != XLOG_PAGE_MAGIC as u16 {
            // A never-written page inside an otherwise valid segment is the
            // normal way a WAL file that is still being filled ends; report
            // that as such rather than as corruption.
            if self.page_buffer.iter().all(|&b| b == 0) {
                return Err(WALReaderError::ZeroedPage { lsn: page_lsn });
            }
            return Err(WALReaderError::InvalidPageMagicNumber {
                magic: header.xlp_magic,
                lsn: page_lsn,
            });
        }

        if header.xlp_info & !XLP_ALL_FLAGS != 0 {
            return Err(WALReaderError::InvalidPageFlags {
                flags: header.xlp_info,
                lsn: page_lsn,
            });
        }

        // Only the first page of a segment carries a long header.
        if header.is_long() != (page_no == 0) {
            return Err(WALReaderError::InvalidPageFlags {
                flags: header.xlp_info,
                lsn: page_lsn,
            });
        }

        // The check that catches a recycled-but-not-yet-reused segment: its
        // pages carry a valid magic but the pageaddr of their former life.
        if header.xlp_pageaddr != page_lsn {
            return Err(WALReaderError::UnexpectedPageAddr {
                found: header.xlp_pageaddr,
                expected: page_lsn,
            });
        }

        if header.xlp_tli != self.tli {
            return Err(WALReaderError::UnexpectedTimeLineId {
                found: header.xlp_tli,
                expected: self.tli,
                lsn: page_lsn,
            });
        }

        Ok(())
    }

    /// Load a page, reporting a never-written page as "no more data" rather
    /// than as an error: that is simply how a segment a server is still
    /// filling ends.
    fn load_page_or_eof(&mut self, page_no: u32) -> Result<bool, WALReaderError> {
        match self.load_page(page_no) {
            Ok(()) => Ok(true),
            Err(WALReaderError::ZeroedPage { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn page_header(&self) -> &XLogPageHeaderData {
        // new() rejects any xlog_blcksz smaller than a long page header, so
        // the buffer is always big enough.
        XLogPageHeaderData::from_bytes(&self.page_buffer)
            .expect("page buffer holds at least one page header")
    }

    // Find the beginning of the next record. On return, self.read_lsn is
    // ensured to point at the first byte of the next record.
    //
    // Return true if found, false if the segment ends first.
    fn find_next_record(&mut self) -> Result<bool, WALReaderError> {
        loop {
            let page_no = self.lsn_to_page_no(self.read_lsn);

            // Reached the end of the WAL segment file, or of the bytes that
            // have actually been written to it.
            if page_no >= cmp::min(self.pages_per_segment(), self.readable_pages) {
                return Ok(false);
            }

            if self.page_no != page_no && !self.load_page_or_eof(page_no)? {
                return Ok(false);
            }

            let header = self.page_header();
            let h_size = header.header_size();
            let xlp_info = header.xlp_info;
            let xlp_rem_len = header.xlp_rem_len;

            // The read pointer must be at least past the page header.
            let mut offset = cmp::max(h_size, self.lsn_to_page_offset(self.read_lsn));

            // If the page opens with the tail of a record that started on an
            // earlier page, that tail is not a record we can decode; skip it.
            if offset == h_size && xlp_info & XLP_FIRST_IS_CONTRECORD != 0 {
                let rem = maxalign(xlp_rem_len as usize);

                // If the continuation fills the rest of this page, the record
                // runs on into the next page as well; try there.
                if rem >= self.xlog_blcksz - h_size {
                    self.read_lsn = self.page_start_lsn(page_no + 1);
                    continue;
                }

                offset = h_size + rem;
            }

            // Adjust read_lsn as we may have advanced the page offset.
            self.read_lsn = self.page_start_lsn(page_no) + offset as u64;

            return Ok(true);
        }
    }

    /// Append `len` bytes of record content to `record_buffer`, starting at
    /// `*offset` in the current page and continuing onto following pages, and
    /// record where in the file those bytes came from.
    ///
    /// Returns the total size of the page headers stepped over, which the
    /// caller adds to `read_lsn`, or None if the readable data ends first.
    fn copy_record_bytes(
        &mut self,
        offset: &mut usize,
        len: usize,
        file_ranges: &mut Vec<Range<usize>>,
    ) -> Result<Option<u64>, WALReaderError> {
        let mut remaining = len;
        let mut header_bytes = 0u64;

        while remaining > 0 {
            debug_assert!(*offset <= self.xlog_blcksz);

            if *offset == self.xlog_blcksz {
                let next = self.page_no + 1;
                if next >= cmp::min(self.pages_per_segment(), self.readable_pages)
                    || !self.load_page_or_eof(next)?
                {
                    return Ok(None);
                }

                let h_size = self.page_header().header_size();
                *offset = h_size;
                header_bytes += h_size as u64;
                continue;
            }

            let take = cmp::min(remaining, self.xlog_blcksz - *offset);
            self.record_buffer
                .extend_from_slice(&self.page_buffer[*offset..*offset + take]);

            let start = self.page_no as usize * self.xlog_blcksz + *offset;
            match file_ranges.last_mut() {
                // The record header and its body are copied by separate calls
                // but are adjacent in the file; keep them as one range.
                Some(last) if last.end == start => last.end += take,
                _ => file_ranges.push(start..start + take),
            }

            *offset += take;
            remaining -= take;
        }

        Ok(Some(header_bytes))
    }

    pub fn load_next_record(&mut self) -> Result<Option<WALRecordInfo>, WALReaderError> {
        if self.done {
            return Ok(None);
        }

        if !self.find_next_record()? {
            self.done = true;
            return Ok(None);
        }

        // Set the record_lsn of the current record.
        self.record_lsn = self.read_lsn;
        self.record_buffer.clear();

        let mut offset = self.lsn_to_page_offset(self.read_lsn);
        let mut header_bytes = 0u64;
        let mut file_ranges: Vec<Range<usize>> = Vec::new();

        // Read the XLogRecord header first; it may itself straddle a page
        // boundary.
        match self.copy_record_bytes(&mut offset, SIZE_OF_XLOG_RECORD, &mut file_ranges)? {
            Some(n) => header_bytes += n,
            None => {
                self.done = true;
                return Ok(None);
            }
        }

        let tot_len = u32::from_le_bytes(self.record_buffer[0..4].try_into().unwrap());

        // A zeroed area is how the WAL a running server is still filling ends.
        if tot_len == 0 {
            self.done = true;
            return Ok(None);
        }

        if (tot_len as usize) < SIZE_OF_XLOG_RECORD || tot_len > XLOG_RECORD_MAX_SIZE {
            self.done = true;
            return Err(WALReaderError::InvalidRecordLength {
                len: tot_len,
                lsn: self.record_lsn,
            });
        }

        // xl_tot_len covers the XLogRecord header, which we already have.
        let body_len = tot_len as usize - SIZE_OF_XLOG_RECORD;
        match self.copy_record_bytes(&mut offset, body_len, &mut file_ranges)? {
            Some(n) => header_bytes += n,
            None => {
                self.done = true;
                return Ok(None);
            }
        }

        // Advance the read pointer past this record.  read_lsn still sits at
        // the record's first byte, so the whole MAXALIGN'ed record length goes
        // on top of the page headers we stepped over on the way.
        self.read_lsn += header_bytes + maxalign(tot_len as usize) as u64;

        let mut record = decode_wal_record(&self.record_buffer, self.record_lsn)?;
        record.crc_ok = verify_record_crc(&self.record_buffer);
        record.file_ranges = file_ranges;

        // Every record is held for the lifetime of the program, so the spare
        // capacity a grown Vec leaves behind is paid once per record: a Vec of
        // one element otherwise reserves room for four.
        record.file_ranges.shrink_to_fit();
        record.blocks.shrink_to_fit();

        if !record.crc_ok {
            // Past a bad CRC nothing in the stream can be trusted, in
            // particular not the next record's xl_tot_len.  Hand back this
            // record so the user can see where it went wrong, then stop.
            self.done = true;
        }

        Ok(Some(record))
    }
}

/// Same computation as PostgreSQL's XLogRecordValidateHeader(): the body
/// first, then the header up to (but not including) xl_crc.
fn verify_record_crc(record_bytes: &[u8]) -> bool {
    if record_bytes.len() < SIZE_OF_XLOG_RECORD {
        return false;
    }
    let Some(record) = XLogRecord::from_bytes(record_bytes) else {
        return false;
    };

    let mut crc = Crc32c::new();
    crc.update(&record_bytes[SIZE_OF_XLOG_RECORD..]);
    crc.update(&record_bytes[..XLOG_RECORD_CRC_OFFSET]);

    crc.finish() == record.xl_crc
}

fn truncated(lsn: XLogRecPtr) -> WALReaderError {
    WALReaderError::TruncatedRecord { lsn }
}

// Decode one WAL record.
//
// 'record_bytes' holds exactly one WAL record, starting at the first byte of
// the XLogRecord header.
fn decode_wal_record(
    record_bytes: &[u8],
    lsn: XLogRecPtr,
) -> Result<WALRecordInfo, WALReaderError> {
    let record = XLogRecord::from_bytes(record_bytes).ok_or_else(|| truncated(lsn))?;

    let mut record_info = WALRecordInfo {
        lsn,
        xlrec: *record,
        ..Default::default()
    };

    // Positions from this reader are offsets into record_bytes, which is what
    // gets stored as `raw`, so they can be recorded as ranges directly.
    let mut r = Reader::new(record_bytes);
    r.skip(SIZE_OF_XLOG_RECORD);

    let mut rlocator: Option<RelFileLocator> = None;
    let mut max_block_id: i16 = -1;
    let mut datatotal: usize = 0;
    let mut has_main = false;

    while r.remaining() > datatotal {
        let block_id: u8 = r.read_u8().ok_or_else(|| truncated(lsn))?;

        if block_id == XLR_BLOCK_ID_DATA_SHORT {
            // XLogRecordDataHeaderShort
            record_info.main_len = r.read_u8().ok_or_else(|| truncated(lsn))? as u32;
            has_main = true;
            datatotal += record_info.main_len as usize;
            break; // the main data is the last.
        } else if block_id == XLR_BLOCK_ID_DATA_LONG {
            // XLogRecordDataHeaderLong
            record_info.main_len = r.read_u32_le().ok_or_else(|| truncated(lsn))?;
            has_main = true;
            datatotal += record_info.main_len as usize;
            break; // the main data is the last.
        } else if block_id == XLR_BLOCK_ID_ORIGIN {
            record_info.origin = Some(r.read_u16_le().ok_or_else(|| truncated(lsn))?);
        } else if block_id == XLR_BLOCK_ID_TOPLEVEL_XID {
            record_info.top_xid = Some(r.read_u32_le().ok_or_else(|| truncated(lsn))?);
        } else if block_id <= XLR_MAX_BLOCK_ID {
            // Block references are numbered in ascending order and each may
            // appear only once.
            if (block_id as i16) <= max_block_id {
                return Err(WALReaderError::OutOfOrderBlockId { block_id, lsn });
            }
            max_block_id = block_id as i16;

            // Parse XLogRecordBlockHeader
            let mut block = WALBlockData {
                block_id,
                ..Default::default()
            };

            let fork_flags = r.read_u8().ok_or_else(|| truncated(lsn))?;

            block.forknum = (fork_flags & BKPBLOCK_FORK_MASK) as ForkNumber;
            block.flags = fork_flags;
            block.data_len = r.read_u16_le().ok_or_else(|| truncated(lsn))?;

            if ((block.flags & BKPBLOCK_HAS_DATA) != 0) && block.data_len == 0 {
                return Err(WALReaderError::NoDataIncluded { lsn });
            }

            if ((block.flags & BKPBLOCK_HAS_DATA) == 0) && block.data_len > 0 {
                return Err(WALReaderError::InvalidDataLength {
                    len: block.data_len,
                    lsn,
                });
            }

            datatotal += block.data_len as usize;

            // Process the full-page image if there is one.
            if (block.flags & BKPBLOCK_HAS_IMAGE) != 0 {
                let mut image = WALFullPageImage {
                    bimg_len: r.read_u16_le().ok_or_else(|| truncated(lsn))?,
                    hole_offset: r.read_u16_le().ok_or_else(|| truncated(lsn))?,
                    bimg_info: r.read_u8().ok_or_else(|| truncated(lsn))?,
                    ..Default::default()
                };

                image.compressed = bkpimage_compressed(image.bimg_info);
                image.apply_image = (image.bimg_info & BKPIMAGE_APPLY) != 0;

                if image.compressed {
                    image.hole_len = if (image.bimg_info & BKPIMAGE_HAS_HOLE) != 0 {
                        r.read_u16_le().ok_or_else(|| truncated(lsn))?
                    } else {
                        0
                    };
                } else {
                    image.hole_len = (BLCKSZ as u16).saturating_sub(image.bimg_len);
                }
                datatotal += image.bimg_len as usize;

                // cross-check that hole_offset > 0, hole_len > 0 and
                // bimg_len < BLCKSZ if the HAS_HOLE flag is set.
                if ((image.bimg_info & BKPIMAGE_HAS_HOLE) != 0)
                    && (image.hole_offset == 0
                        || image.hole_len == 0
                        || image.bimg_len == BLCKSZ as u16)
                {
                    return Err(WALReaderError::InvalidHoleData {
                        hole_offset: image.hole_offset,
                        hole_len: image.hole_len,
                        img_len: image.bimg_len,
                        lsn,
                    });
                }

                if ((image.bimg_info & BKPIMAGE_HAS_HOLE) == 0)
                    && (image.hole_offset != 0 || image.hole_len != 0)
                {
                    return Err(WALReaderError::InvalidHoleFlag {
                        hole_offset: image.hole_offset,
                        hole_len: image.hole_len,
                        lsn,
                    });
                }

                if image.compressed && image.bimg_len == BLCKSZ as u16 {
                    return Err(WALReaderError::InvalidCompressFlag {
                        len: image.bimg_len,
                        lsn,
                    });
                }

                block.image = Some(image);
            }

            if (block.flags & BKPBLOCK_SAME_REL) == 0 {
                // Get RelFileLocator
                block.rlocator.spcOid = r.read_u32_le().ok_or_else(|| truncated(lsn))?;
                block.rlocator.dbOid = r.read_u32_le().ok_or_else(|| truncated(lsn))?;
                block.rlocator.relNumber = r.read_u32_le().ok_or_else(|| truncated(lsn))?;

                rlocator = Some(block.rlocator);
            } else {
                // Copy from the previously-taken rlocator.
                block.rlocator = rlocator.ok_or(WALReaderError::SameRelNotFound { lsn })?;
            }

            block.blocknum = r.read_u32_le().ok_or_else(|| truncated(lsn))?;

            record_info.blocks.push(block);
        } else {
            return Err(WALReaderError::InvalidBlockId { block_id, lsn });
        }
    }

    if r.remaining() != datatotal {
        return Err(WALReaderError::InvalidRecordLength {
            len: record.xl_tot_len,
            lsn,
        });
    }

    // The payloads follow the headers, in block order, with the main data last.
    for block in record_info.blocks.iter_mut() {
        if let Some(image) = &mut block.image {
            let start = r.pos;
            r.take(image.bimg_len as usize)
                .ok_or_else(|| truncated(lsn))?;
            image.bimg_range = start..r.pos;
        }

        if (block.flags & BKPBLOCK_HAS_DATA) != 0 {
            let start = r.pos;
            r.take(block.data_len as usize)
                .ok_or_else(|| truncated(lsn))?;
            block.data_range = Some(start..r.pos);
        }
    }

    if has_main {
        let start = r.pos;
        r.take(record_info.main_len as usize)
            .ok_or_else(|| truncated(lsn))?;
        record_info.main_range = Some(start..r.pos);
    }

    record_info.raw = record_bytes.to_vec();

    Ok(record_info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const TLI: TimeLineID = 1;
    // Deliberately not the default 16MB: the geometry has to come from the
    // long page header, not from a hardcoded constant.
    const SEG_SIZE: u64 = 1024 * 1024;
    const BLK: usize = 8192;
    const SEG_NO: u64 = 3;
    const SYSID: u64 = 0x0123_4567_89AB_CDEF;

    fn page_header(page_no: u32, seg_no: u64, info: u16, rem_len: u32) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&(XLOG_PAGE_MAGIC as u16).to_le_bytes());
        h.extend_from_slice(&info.to_le_bytes());
        h.extend_from_slice(&TLI.to_le_bytes());
        h.extend_from_slice(&(seg_no * SEG_SIZE + page_no as u64 * BLK as u64).to_le_bytes());
        h.extend_from_slice(&rem_len.to_le_bytes());
        h.extend_from_slice(&[0u8; 4]); // padding up to SizeOfXLogShortPHD
        if info & XLP_LONG_HEADER != 0 {
            h.extend_from_slice(&SYSID.to_le_bytes());
            h.extend_from_slice(&(SEG_SIZE as u32).to_le_bytes());
            h.extend_from_slice(&(BLK as u32).to_le_bytes());
        }
        h
    }

    fn page_header_size(page_no: usize) -> usize {
        if page_no == 0 { 40 } else { 24 }
    }

    /// Assemble an XLogRecord around `body`, with a correct CRC.
    fn make_record(xid: TransactionId, rmid: u8, info: u8, body: &[u8]) -> Vec<u8> {
        let tot_len = (SIZE_OF_XLOG_RECORD + body.len()) as u32;
        let mut rec = Vec::with_capacity(tot_len as usize);
        rec.extend_from_slice(&tot_len.to_le_bytes());
        rec.extend_from_slice(&xid.to_le_bytes());
        rec.extend_from_slice(&0u64.to_le_bytes()); // xl_prev
        rec.push(info);
        rec.push(rmid);
        rec.extend_from_slice(&[0u8; 2]); // padding
        rec.extend_from_slice(&[0u8; 4]); // xl_crc, filled in below
        rec.extend_from_slice(body);

        let mut crc = Crc32c::new();
        crc.update(&rec[SIZE_OF_XLOG_RECORD..]);
        crc.update(&rec[..XLOG_RECORD_CRC_OFFSET]);
        rec[XLOG_RECORD_CRC_OFFSET..SIZE_OF_XLOG_RECORD]
            .copy_from_slice(&crc.finish().to_le_bytes());
        rec
    }

    /// A record body carrying only main data.
    fn main_data_body(main: &[u8]) -> Vec<u8> {
        let mut body = vec![XLR_BLOCK_ID_DATA_SHORT, main.len() as u8];
        body.extend_from_slice(main);
        body
    }

    /// Lay records out across a segment the way PostgreSQL does: MAXALIGN'ed,
    /// with a page header on every page and XLP_FIRST_IS_CONTRECORD set on a
    /// page that opens with the tail of the previous record.
    fn build_segment(seg_no: u64, records: &[Vec<u8>]) -> Vec<u8> {
        let npages = (SEG_SIZE as usize) / BLK;
        let mut buf = vec![0u8; SEG_SIZE as usize];
        let mut rem_len = vec![0u32; npages];

        let mut pos = page_header_size(0);
        for r in records {
            let mut written = 0usize;
            while written < r.len() {
                let page = pos / BLK;
                let page_off = pos % BLK;
                if page_off == 0 {
                    if written > 0 {
                        rem_len[page] = (r.len() - written) as u32;
                    }
                    pos += page_header_size(page);
                    continue;
                }
                let take = (BLK - page_off).min(r.len() - written);
                buf[pos..pos + take].copy_from_slice(&r[written..written + take]);
                pos += take;
                written += take;
            }
            pos = maxalign(pos);
        }

        let used_pages = pos.div_ceil(BLK);
        for p in 0..used_pages {
            let mut info = 0u16;
            if p == 0 {
                info |= XLP_LONG_HEADER;
            }
            if rem_len[p] != 0 {
                info |= XLP_FIRST_IS_CONTRECORD;
            }
            let h = page_header(p as u32, seg_no, info, rem_len[p]);
            buf[p * BLK..p * BLK + h.len()].copy_from_slice(&h);
        }

        buf
    }

    fn reader_for(buf: Vec<u8>) -> Result<WALReader<Cursor<Vec<u8>>>, WALReaderError> {
        // log/seg such that seg_no == SEG_NO for this segment size.
        let per_xlogid = (0x1_0000_0000u64 / SEG_SIZE) as u32;
        WALReader::new(
            Cursor::new(buf),
            TLI,
            SEG_NO as u32 / per_xlogid,
            SEG_NO as u32,
        )
    }

    fn read_all(
        reader: &mut WALReader<Cursor<Vec<u8>>>,
    ) -> (Vec<WALRecordInfo>, Option<WALReaderError>) {
        let mut out = Vec::new();
        loop {
            match reader.load_next_record() {
                Ok(Some(r)) => out.push(r),
                Ok(None) => return (out, None),
                Err(e) => return (out, Some(e)),
            }
        }
    }

    #[test]
    fn geometry_comes_from_the_long_page_header() {
        let seg = build_segment(SEG_NO, &[make_record(100, 10, 0, &main_data_body(b"hi"))]);
        let reader = reader_for(seg).unwrap();
        assert_eq!(reader.seg_size(), SEG_SIZE);
        assert_eq!(reader.xlog_blcksz(), BLK);
        assert_eq!(reader.sysid(), SYSID);
        assert_eq!(reader.tli(), TLI);
    }

    #[test]
    fn reads_records_and_verifies_crc() {
        let recs = vec![
            make_record(100, 10, 0x00, &main_data_body(b"first")),
            make_record(101, 1, 0x00, &main_data_body(b"second")),
        ];
        let mut reader = reader_for(build_segment(SEG_NO, &recs)).unwrap();
        let (got, err) = read_all(&mut reader);

        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(got.len(), 2);

        assert_eq!(got[0].xlrec.xl_xid, 100);
        assert_eq!(got[0].xlrec.xl_rmid, 10);
        assert_eq!(got[0].lsn, SEG_NO * SEG_SIZE + 40);
        assert_eq!(got[0].main_data(), Some(&b"first"[..]));
        assert!(got[0].crc_ok);
        assert_eq!(got[0].nblocks_inuse(), 0);

        assert_eq!(got[1].xlrec.xl_xid, 101);
        assert_eq!(got[1].main_data(), Some(&b"second"[..]));
        assert!(got[1].crc_ok);
    }

    /// The common case: a segment a running server is still filling.  Reading
    /// must stop cleanly at the zeroed tail rather than fail.
    #[test]
    fn stops_cleanly_at_the_zero_filled_tail() {
        let recs = vec![make_record(100, 10, 0, &main_data_body(b"only one"))];
        let mut reader = reader_for(build_segment(SEG_NO, &recs)).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(got.len(), 1);
    }

    /// Same, but with the last record ending exactly on a page boundary, so
    /// the reader steps onto a page that was never written.
    #[test]
    fn stops_cleanly_when_the_tail_starts_on_a_page_boundary() {
        // 8192 - 40 header = 8152 bytes of page 0 to fill exactly.
        let payload_len = 8152 - SIZE_OF_XLOG_RECORD - 6; // DATA_LONG header is 5 bytes
        let mut body = vec![XLR_BLOCK_ID_DATA_LONG];
        body.extend_from_slice(&(payload_len as u32).to_le_bytes());
        body.extend(std::iter::repeat_n(0xABu8, payload_len));
        let rec = make_record(100, 10, 0, &body);
        assert_eq!(rec.len(), 8152 - 1);

        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn reads_a_record_that_spans_pages() {
        let payload_len = BLK + 1000;
        let mut body = vec![XLR_BLOCK_ID_DATA_LONG];
        body.extend_from_slice(&(payload_len as u32).to_le_bytes());
        body.extend((0..payload_len).map(|i| i as u8));

        let recs = vec![
            make_record(100, 10, 0, &body),
            make_record(101, 10, 0, &main_data_body(b"after")),
        ];
        let mut reader = reader_for(build_segment(SEG_NO, &recs)).unwrap();
        let (got, err) = read_all(&mut reader);

        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(got.len(), 2);
        let main = got[0].main_data().unwrap();
        assert_eq!(main.len(), payload_len);
        assert!(main.iter().enumerate().all(|(i, &b)| b == i as u8));
        assert!(got[0].crc_ok);
        assert_eq!(got[1].main_data(), Some(&b"after"[..]));
    }

    #[test]
    fn decodes_block_references() {
        let mut body = vec![
            0u8,               // block_id 0
            BKPBLOCK_HAS_DATA, // main fork, has data
        ];
        body.extend_from_slice(&3u16.to_le_bytes()); // data_len
        body.extend_from_slice(&1663u32.to_le_bytes()); // spcOid
        body.extend_from_slice(&5u32.to_le_bytes()); // dbOid
        body.extend_from_slice(&16384u32.to_le_bytes()); // relNumber
        body.extend_from_slice(&7u32.to_le_bytes()); // blocknum
        body.push(XLR_BLOCK_ID_DATA_SHORT);
        body.push(2);
        body.extend_from_slice(b"\xAA\xBB\xCC"); // block data
        body.extend_from_slice(b"\xDD\xEE"); // main data

        let mut reader =
            reader_for(build_segment(SEG_NO, &[make_record(7, 10, 0, &body)])).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(got.len(), 1);

        let rec = &got[0];
        assert_eq!(rec.nblocks_inuse(), 1);
        // The block vector must be exactly as large as the record needs.
        assert_eq!(rec.blocks.len(), 1);
        let block = &rec.blocks[0];
        assert_eq!(block.block_id, 0);
        assert_eq!(block.rlocator.spcOid, 1663);
        assert_eq!(block.rlocator.dbOid, 5);
        assert_eq!(block.rlocator.relNumber, 16384);
        assert_eq!(block.blocknum, 7);
        assert_eq!(rec.block_data(0), Some(&b"\xAA\xBB\xCC"[..]));
        assert_eq!(rec.main_data(), Some(&b"\xDD\xEE"[..]));
        // Payloads are views into `raw`, not extra copies.
        assert_eq!(rec.raw.len(), rec.xlrec.xl_tot_len as usize);
    }

    #[test]
    fn detects_a_corrupted_record() {
        let recs = vec![
            make_record(100, 10, 0, &main_data_body(b"payload")),
            make_record(101, 10, 0, &main_data_body(b"never seen")),
        ];
        let mut seg = build_segment(SEG_NO, &recs);
        // Flip a byte of the first record's main data, leaving xl_crc alone.
        let main_off = 40 + SIZE_OF_XLOG_RECORD + 2;
        seg[main_off] ^= 0xFF;

        let mut reader = reader_for(seg).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);
        // The bad record is still reported, but reading stops there.
        assert_eq!(got.len(), 1);
        assert!(!got[0].crc_ok);
    }

    /// A recycled segment keeps a valid magic but the pageaddr of its former
    /// life; showing its stale records as current would be badly misleading.
    #[test]
    fn rejects_a_recycled_segment() {
        let seg = build_segment(
            SEG_NO + 1,
            &[make_record(100, 10, 0, &main_data_body(b"old"))],
        );
        match reader_for(seg) {
            Err(WALReaderError::UnexpectedPageAddr { found, expected }) => {
                assert_eq!(found, (SEG_NO + 1) * SEG_SIZE);
                assert_eq!(expected, SEG_NO * SEG_SIZE);
            }
            other => panic!("expected UnexpectedPageAddr, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn rejects_a_wrong_timeline() {
        let mut seg = build_segment(SEG_NO, &[make_record(100, 10, 0, &main_data_body(b"x"))]);
        seg[4..8].copy_from_slice(&2u32.to_le_bytes()); // xlp_tli
        assert!(matches!(
            reader_for(seg),
            Err(WALReaderError::UnexpectedTimeLineId { .. })
        ));
    }

    #[test]
    fn rejects_a_foreign_magic_number() {
        let mut seg = build_segment(SEG_NO, &[make_record(100, 10, 0, &main_data_body(b"x"))]);
        seg[0..2].copy_from_slice(&0xD118u16.to_le_bytes());
        assert!(matches!(
            reader_for(seg),
            Err(WALReaderError::InvalidPageMagicNumber { magic: 0xD118, .. })
        ));
    }

    // --- Malformed records must produce errors, never panics ---------------

    #[test]
    fn body_ending_mid_header_is_an_error() {
        // A DATA_LONG id with no length behind it.
        let rec = make_record(100, 10, 0, &[XLR_BLOCK_ID_DATA_LONG]);
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        assert!(matches!(
            reader.load_next_record(),
            Err(WALReaderError::TruncatedRecord { .. })
        ));
    }

    #[test]
    fn out_of_range_block_id_is_an_error() {
        // 200 is neither a block reference nor one of the special ids.
        let rec = make_record(100, 10, 0, &[200, 0, 0, 0]);
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        assert!(matches!(
            reader.load_next_record(),
            Err(WALReaderError::InvalidBlockId { block_id: 200, .. })
        ));
    }

    #[test]
    fn repeated_block_id_is_an_error() {
        let mut body = Vec::new();
        for _ in 0..2 {
            body.push(0u8); // block_id 0, twice
            body.push(0u8); // main fork, no data
            body.extend_from_slice(&0u16.to_le_bytes());
            body.extend_from_slice(&[0u8; 12]); // rlocator
            body.extend_from_slice(&0u32.to_le_bytes()); // blocknum
        }
        let rec = make_record(100, 10, 0, &body);
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        assert!(matches!(
            reader.load_next_record(),
            Err(WALReaderError::OutOfOrderBlockId { block_id: 0, .. })
        ));
    }

    #[test]
    fn declared_length_shorter_than_the_header_is_an_error() {
        let mut rec = make_record(100, 10, 0, &main_data_body(b"x"));
        rec[0..4].copy_from_slice(&8u32.to_le_bytes());
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        assert!(matches!(
            reader.load_next_record(),
            Err(WALReaderError::InvalidRecordLength { len: 8, .. })
        ));
    }

    #[test]
    fn absurd_declared_length_is_an_error() {
        let mut rec = make_record(100, 10, 0, &main_data_body(b"x"));
        rec[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        assert!(matches!(
            reader.load_next_record(),
            Err(WALReaderError::InvalidRecordLength { len: u32::MAX, .. })
        ));
    }

    #[test]
    fn declared_length_longer_than_the_body_is_an_error() {
        // Claim 64 more bytes of main data than the record actually carries.
        let mut body = vec![XLR_BLOCK_ID_DATA_LONG];
        body.extend_from_slice(&64u32.to_le_bytes());
        body.extend_from_slice(b"only eight");
        let rec = make_record(100, 10, 0, &body);
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        assert!(matches!(
            reader.load_next_record(),
            Err(WALReaderError::InvalidRecordLength { .. })
        ));
    }

    /// Random garbage must never take the process down.
    #[test]
    fn arbitrary_bodies_never_panic() {
        for seed in 0u32..512 {
            let len = (seed % 61) as usize;
            let body: Vec<u8> = (0..len)
                .map(|i| (seed.wrapping_mul(2654435761).wrapping_add(i as u32) >> 7) as u8)
                .collect();
            let rec = make_record(100, 10, 0, &body);
            let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
            let _ = reader.load_next_record();
        }
    }

    #[test]
    fn a_short_file_is_read_as_far_as_it_goes() {
        let recs = vec![make_record(100, 10, 0, &main_data_body(b"partial"))];
        let mut seg = build_segment(SEG_NO, &recs);
        seg.truncate(BLK); // a single page, as pg_receivewal's *.partial starts

        let mut reader = reader_for(seg).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);
        assert_eq!(got.len(), 1);
    }

    /// The hex dump highlights a record inside the whole segment, so it needs
    /// to know which bytes of the file the record actually occupies.
    #[test]
    fn a_record_on_one_page_has_a_single_file_range() {
        let rec = make_record(100, 10, 0, &main_data_body(b"first"));
        let len = rec.len();
        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);
        // Page 0's long header is 40 bytes, the record follows it.
        assert_eq!(got[0].file_ranges.len(), 1);
        assert_eq!(got[0].file_ranges[0], 40..40 + len);
    }

    /// A record that crosses a page boundary is split by the next page's
    /// header, so its bytes are not contiguous in the file.
    #[test]
    fn a_record_spanning_pages_has_one_file_range_per_page() {
        let payload_len = BLK + 1000;
        let mut body = vec![XLR_BLOCK_ID_DATA_LONG];
        body.extend_from_slice(&(payload_len as u32).to_le_bytes());
        body.extend((0..payload_len).map(|i| i as u8));
        let rec = make_record(100, 10, 0, &body);
        let total = rec.len();

        let mut reader = reader_for(build_segment(SEG_NO, &[rec])).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);

        // Page 0 holds what fits after its 40-byte long header, the rest
        // follows page 1's 24-byte header.
        let on_page0 = BLK - 40;
        assert!(total > on_page0 && total < on_page0 + BLK - 24);
        assert_eq!(
            got[0].file_ranges,
            vec![40..BLK, BLK + 24..BLK + 24 + (total - on_page0)]
        );
        // The ranges cover the record exactly once.
        let covered: usize = got[0].file_ranges.iter().map(|r| r.len()).sum();
        assert_eq!(covered, total);
    }

    /// Every record is kept for the lifetime of the program, so the slack a
    /// growing Vec leaves behind is multiplied by the record count -- a Vec of
    /// one element otherwise reserves room for four.
    #[test]
    fn records_do_not_carry_spare_capacity() {
        let mut body = vec![0u8, BKPBLOCK_HAS_DATA];
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&[0u8; 12]); // rlocator
        body.extend_from_slice(&0u32.to_le_bytes()); // blocknum
        body.push(XLR_BLOCK_ID_DATA_SHORT);
        body.push(1);
        body.extend_from_slice(b"\x01\x02");

        let mut reader =
            reader_for(build_segment(SEG_NO, &[make_record(7, 10, 0, &body)])).unwrap();
        let (got, err) = read_all(&mut reader);
        assert!(err.is_none(), "unexpected error: {:?}", err);

        let rec = &got[0];
        assert_eq!(rec.blocks.len(), 1);
        assert_eq!(rec.blocks.capacity(), rec.blocks.len());
        assert_eq!(rec.file_ranges.len(), 1);
        assert_eq!(rec.file_ranges.capacity(), rec.file_ranges.len());
        assert_eq!(rec.raw.capacity(), rec.raw.len());
    }

    #[test]
    fn lsn_arithmetic_round_trips() {
        let reader = reader_for(build_segment(
            SEG_NO,
            &[make_record(1, 10, 0, &main_data_body(b"x"))],
        ))
        .unwrap();

        for page in [0u32, 1, 17, (SEG_SIZE / BLK as u64) as u32 - 1] {
            let lsn = reader.page_start_lsn(page);
            assert_eq!(reader.lsn_to_page_no(lsn), page);
            assert_eq!(reader.lsn_to_page_offset(lsn), 0);
            assert_eq!(reader.lsn_to_page_offset(lsn + 123), 123);
        }
        assert_eq!(reader.pages_per_segment(), (SEG_SIZE / BLK as u64) as u32);
    }
}
