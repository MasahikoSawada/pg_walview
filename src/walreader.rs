use bytes::Buf;
use std::cmp;
use std::fs::File;
use std::io::{self, Error, ErrorKind, Read, Seek, SeekFrom};
use std::mem;

use thiserror::Error;

use crate::bindings::*;
use crate::walmisc::*;
use std::path::Path;

pub const XLR_INFO_MASK: u8 = 0x0F;
pub const XLR_RMGR_INFO_MASK: u8 = 0xF0;

// Typed constants to avoid repeated casts in page/LSN arithmetic.
const PAGES_PER_SEGMENT: u32 = (XLOG_SEGMENT_SIZE / XLOG_BLCKSZ as u64) as u32;
const BLCKSZ_U16: u16 = BLCKSZ as u16;

#[derive(Error, Debug)]
pub enum WALReaderError {
    #[error("I/O error: {0}")]
    IO(#[from] std::io::Error),

    #[error("record with invalid length at {}", lsn_format(0))]
    InvalidRecordLength(XLogRecPtr),

    #[error("out-of-order block_id {} at {}", block_id, lsn_format(*lsn))]
    OutOfOrderBlockId { block_id: u32, lsn: XLogRecPtr },

    #[error("BKPBLOCK_HAS_DATA set, but not data included at {}", lsn_format(0))]
    NoDataIncluded(XLogRecPtr),

    #[error("BKPBLOCK_HAS_DATA not set, but data length is {} at {}", len, lsn_format(*lsn))]
    InvalidDataLength { len: u16, lsn: XLogRecPtr },

    #[error("BKPIMAGE_HAS_HOLE set, but hole offset {} length {} {} block image length as {}", hole_offset, hole_len, img_len, lsn_format(*lsn))]
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

    #[error("BKPBLOCK_SAME_REL set but not previous rel at {}", lsn_format(0))]
    SameRelNotFound(XLogRecPtr),

    #[error("invalid magic number {:04X} in WAL segment {}, LSN {}", magic, segno, lsn_format(*lsn))]
    InvalidPageMagicNumber {
        magic: u16,
        segno: u64,
        lsn: XLogRecPtr,
    },
}

#[derive(Clone, Debug, Default)]
pub struct WALFullPageImage {
    pub compressed: bool,
    pub apply_image: bool,
    pub hole_offset: u16,
    pub hole_len: u16,
    pub bimg_info: u8,
    pub bimg_len: u16,
    pub bimg_data: Vec<u8>,
}

// XLogRecordBlockHeader
#[derive(Clone, Debug, Default)]
pub struct WALBlockData {
    pub rlocator: RelFileLocator,
    pub forknum: ForkNumber,
    pub flags: u8,
    pub blocknum: BlockNumber,

    // full-page image
    pub image: Option<WALFullPageImage>,

    // Length of data (not including page image).
    pub data_len: u16,

    // rmgr-specific data
    pub data: Option<Vec<u8>>,
}

// Struct for one WAL record.
#[derive(Debug, Clone, Default)]
pub struct WALRecordInfo {
    // Record's LSN
    pub lsn: XLogRecPtr,

    pub xlrec: XLogRecord,

    //
    pub nblocks_inuse: usize,
    pub blocks: Vec<WALBlockData>,
    pub main_len: u32,
    pub main: Option<Vec<u8>>,
    pub origin: Option<u16>,
    pub top_xid: Option<TransactionId>,
    pub same_rel: Option<RelFileLocator>,

    // Raw bytes of the full record (header + payload), for hex dump.
    pub raw: Vec<u8>,
}

impl WALRecordInfo {
    pub fn new() -> Self {
        Self {
            lsn: 0,
            xlrec: Default::default(),
            nblocks_inuse: 0,
            blocks: vec![Default::default(); 256],
            main_len: 0,
            main: None,
            origin: None,
            top_xid: None,
            same_rel: None,
            raw: Vec::new(),
        }
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
        if self.bimg_info & BKPIMAGE_BKPIMAGE_COMPRESS_ZSTD != 0 {
            "zstd"
        } else if self.bimg_info & BKPIMAGE_BKPIMAGE_COMPRESS_LZ4 != 0 {
            "lz4"
        } else if self.bimg_info & BKPIMAGE_BKPIMAGE_COMPRESS_PGLZ != 0 {
            "pglz"
        } else {
            "none"
        }
    }
}

// ---------------------------------------------------------------------------
// LSN / page arithmetic helpers — keep casts in one place.
// ---------------------------------------------------------------------------

fn lsn_to_page_no(lsn: XLogRecPtr) -> u32 {
    ((lsn % XLOG_SEGMENT_SIZE) / XLOG_BLCKSZ as u64) as u32
}

fn lsn_to_page_offset(lsn: XLogRecPtr) -> usize {
    ((lsn % XLOG_SEGMENT_SIZE) % XLOG_BLCKSZ as u64) as usize
}

fn page_start_lsn(seg_no: u64, page_no: u32) -> XLogRecPtr {
    seg_no * XLOG_SEGMENT_SIZE + page_no as u64 * XLOG_BLCKSZ as u64
}

// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct WALReader {
    seg_no: u64,
    file: File,

    page_buffer: Vec<u8>,
    page_no: BlockNumber,

    // current read pointer. Note that this could point to anywhere, e.g.,
    // the middle of record, inside of the page header.
    read_lsn: XLogRecPtr,

    // the record's LSN (i.e., the first byte of the record).
    record_lsn: XLogRecPtr,
    record_buffer: Vec<u8>,
}

impl WALReader {
    pub fn new<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path_ref = path.as_ref();
        let fname = Path::new(path_ref)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidInput, "Invalid path or non-UTF8 filename")
            })?;

        if fname.len() != 24 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "Invalid WAL filename length",
            ));
        }

        let parse_hex = |s: &str| -> io::Result<u32> {
            u32::from_str_radix(s, 16)
                .map_err(|e| Error::new(ErrorKind::InvalidData, format!("Parse error: {}", e)))
        };

        let _tli = parse_hex(&fname[0..8])?;
        let log = parse_hex(&fname[8..16])?;
        let seg = parse_hex(&fname[16..24])?;
        let segments_per_xlog_id = 0x1_0000_0000u64 / XLOG_SEGMENT_SIZE;
        let seg_no = log as u64 * segments_per_xlog_id + seg as u64;

        let _file = File::open(path_ref)?;

        Ok(Self {
            seg_no,
            file: File::open(path_ref)?,
            page_buffer: vec![0u8; XLOG_BLCKSZ],
            page_no: INVALID_BLOCK_NUMBER,
            read_lsn: seg_no * XLOG_SEGMENT_SIZE,
            record_lsn: 0,
            record_buffer: vec![0u8; 1024],
        })
    }

    fn load_page(&mut self, page_no: u32) -> Result<bool, WALReaderError> {
        self.file
            .seek(SeekFrom::Start(page_no as u64 * XLOG_BLCKSZ as u64))?;

        // Read one WAL page.
        self.file.read_exact(&mut self.page_buffer)?;

        // XXX veriy page header.

        let header = XLogPageHeaderData::from_bytes(&self.page_buffer).unwrap();

        if header.xlp_magic != XLOG_PAGE_MAGIC {
            return Err(WALReaderError::InvalidPageMagicNumber {
                magic: header.xlp_magic,
                segno: self.seg_no,
                lsn: page_start_lsn(self.seg_no, page_no),
            });
        }

        self.page_no = page_no;

        Ok(true)
    }

    // Find the beginning of the next record. On return, self.read_lsn is ensured
    // to point the first byte of the next record.
    //
    // Return true if found, false otherwise.
    fn find_next_record(&mut self) -> Result<bool, WALReaderError> {
        loop {
            let page_no = lsn_to_page_no(self.read_lsn);
            let mut offset = lsn_to_page_offset(self.read_lsn);

            // Reached to the end of WAL segment file.
            if page_no == PAGES_PER_SEGMENT {
                return Ok(false);
            }

            // Reset the record decode buffer.
            self.record_buffer.clear();

            // load a new page if needed.
            if self.page_no != page_no {
                self.load_page(page_no)?;

                // Get the header data and see the header size.
                let header = XLogPageHeaderData::from_bytes(self.get_page()).unwrap();
                let h_size = header.header_size();

                // offset must beyond at least the page header.
                offset = cmp::max(h_size, offset);

                if header.xlp_info & XLP_FIRST_IS_CONTRECORD != 0 {
                    // If the length of the remaining continuation data is more than
                    // waht can fit in this page, the continuation record cross
                    // over this page. Read the next page and try again.
                    if header.xlp_rem_len as usize >= XLOG_BLCKSZ - h_size {
                        self.read_lsn = page_start_lsn(self.seg_no, page_no + 1);
                        continue;
                    }

                    // Skip the remaining bytes.
                    offset += header.xlp_rem_len.next_multiple_of(8) as usize;
                }
            }

            // Get the header data and see the header size.
            let header = XLogPageHeaderData::from_bytes(self.get_page()).unwrap();
            let h_size = header.header_size();

            // offset must beyond at least the page header.
            offset = cmp::max(h_size, offset);

            // Adjust the read_lsn as we might have advanced the page_offset.
            self.read_lsn = page_start_lsn(self.seg_no, self.page_no) + offset as u64;

            break;
        }

        Ok(true)
    }

    pub fn load_next_record(&mut self) -> Result<Option<WALRecordInfo>, WALReaderError> {
        if !self.find_next_record()? {
            return Ok(None);
        }

        // Set the record_lsn of the current record.
        self.record_lsn = self.read_lsn;

        let offset = lsn_to_page_offset(self.read_lsn);

        // It starts from the first bytes of the header part.
        let mut contents_bytes = &self.page_buffer[offset..];

        // Try to get the XLogRecord header first.
        if contents_bytes.len() < mem::size_of::<XLogRecord>() {
            let rest_header_len = mem::size_of::<XLogRecord>() - contents_bytes.len();

            // there is even no data for XLogRecord header. Get the current
            // data so far.
            self.record_buffer.extend_from_slice(contents_bytes);

            if self.page_no + 1 == PAGES_PER_SEGMENT {
                return Ok(None);
            }

            // Load the page to get the rest of the header, at least.
            self.load_page(self.page_no + 1)?;

            // Get the header data and see the header size.
            let header = XLogPageHeaderData::from_bytes(self.get_page()).unwrap();
            let h_size = header.header_size();

            // update contents_bytes for later use.
            contents_bytes = &self.page_buffer[h_size..];

            // Push the rest of the record header.
            self.record_buffer
                .extend_from_slice(&contents_bytes[..rest_header_len]);

            contents_bytes.advance(rest_header_len);

            // While the record's tot_len doesn't include the header size, the
            // record's LSN takes it. So we add the header size for now, and
            // will increment the read pointer by the record size later.
            self.read_lsn += h_size as u64;
        } else {
            // Easy case. Just read the record header.
            self.record_buffer
                .extend_from_slice(&contents_bytes[..mem::size_of::<XLogRecord>()]);

            contents_bytes.advance(mem::size_of::<XLogRecord>());
        }

        // OK. We got at least the record header so far.
        let tot_record_len = XLogRecord::from_bytes(&self.record_buffer)
            .map(|record| record.xl_tot_len)
            .ok_or(io::Error::new(
                io::ErrorKind::InvalidData,
                "Failed to read XLogRecord header",
            ))?;

        if tot_record_len == 0 {
            return Ok(None);
        }

        // We've consumed the XLogRecord header part from contents_bytes, so
        // contents_bytes currently points to the first byte of the WAL record
        // contents. Since xl_tot_len includes the XLogRecord header size itself
        // we need to subtract it from the header size.
        let record_len = tot_record_len - mem::size_of::<XLogRecord>() as u32;

        // Check if the whole record fits in the current page.
        if record_len > contents_bytes.len() as u32 {
            // The whole record doesn't fit in the current page.
            let mut rest_record_len = record_len;

            // Add the current page contents to the record buffer.
            self.record_buffer.extend_from_slice(&contents_bytes);
            rest_record_len -= contents_bytes.len() as u32;

            while rest_record_len > 0 {
                if self.page_no + 1 == PAGES_PER_SEGMENT {
                    return Ok(None);
                }

                self.load_page(self.page_no + 1)?;

                let header = XLogPageHeaderData::from_bytes(self.get_page()).unwrap();
                let h_size = header.header_size();

                contents_bytes = &self.page_buffer[h_size..];
                let read_len = cmp::min(contents_bytes.len(), rest_record_len as usize);

                // Push the rest of the record header.
                self.record_buffer
                    .extend_from_slice(&contents_bytes[..read_len]);

                contents_bytes.advance(read_len);

                rest_record_len -= read_len as u32;

                self.read_lsn += h_size as u64;
            }
        } else {
            // easy case, read the record.
            self.record_buffer
                .extend_from_slice(&contents_bytes[..record_len as usize]);
        }

        // Advance the read pointer to the current record header.
        // We need to to use tot_record_len instead of record_len as
        // read_len currently points to the first bytes of XLogRecord
        // header of this record.
        self.read_lsn += (tot_record_len as u64).next_multiple_of(8);

        let _record = XLogRecord::from_bytes(&self.record_buffer).unwrap();

        Ok(Some(Self::decode_wal_record(
            &self.record_buffer,
            self.record_lsn,
        )?))
    }

    // Decode one WAL record.
    //
    // 'record_bytes' points to the beginning of a WAL record (i.e., the first
    // byte of the XLogRecord header.
    fn decode_wal_record(
        record_bytes: &[u8],
        lsn: XLogRecPtr,
    ) -> Result<WALRecordInfo, WALReaderError> {
        let mut rlocator: Option<RelFileLocator> = None;

        let record = XLogRecord::from_bytes(record_bytes).unwrap();

        // Skip the header part.
        let mut record_ptr = &record_bytes[mem::size_of::<XLogRecord>()..];

        // Create a new WAL record info.
        let mut record_info = WALRecordInfo::new();
        record_info.lsn = lsn;
        record_info.xlrec = *record;

        let mut datatotal: usize = 0;
        while record_ptr.len() > datatotal {
            let block_id: u8 = record_ptr.get_u8();

            // 1. Process header part.
            if block_id == XLR_BLOCK_ID_DATA_SHORT {
                // XLogRecordDataHeaderShort
                record_info.main_len = record_ptr.get_u8() as u32;
                record_info.main = Some(Vec::with_capacity(record_info.main_len as usize));

                datatotal += record_info.main_len as usize;
                break; // the main data is the last.
            } else if block_id == XLR_BLOCK_ID_DATA_LONG {
                // XLogRecordDataHeaderLong
                record_info.main_len = record_ptr.get_u32_le();
                record_info.main = Some(Vec::with_capacity(record_info.main_len as usize));

                datatotal += record_info.main_len as usize;
                break; // the main data is the last.
            } else if block_id == XLR_BLOCK_ID_ORIGIN {
                record_info.origin = Some(record_ptr.get_u16_le());
                //println!("  [{}] origin {}", block_id, record_info.origin.unwrap());
            } else if block_id == XLR_BLOCK_ID_TOPLEVEL_XID {
                record_info.top_xid = Some(record_ptr.get_u32());
                //println!("  [{}] top_xid {}", block_id, record_info.top_xid.unwrap());
            } else if block_id <= XLR_MAX_BLOCK_ID {
                // Parse XLogRecordBlockHeader
                let block = &mut (record_info.blocks[record_info.nblocks_inuse]);
                record_info.nblocks_inuse += 1;

                let fork_flags = record_ptr.get_u8();

                block.forknum = (fork_flags & BKPBLOCK_FORK_MASK) as i32;
                block.flags = fork_flags;
                block.data_len = record_ptr.get_u16_le();

                if ((block.flags & BKPBLOCK_HAS_DATA) != 0) && block.data_len == 0 {
                    return Err(WALReaderError::NoDataIncluded(lsn));
                }

                if ((block.flags & BKPBLOCK_HAS_DATA) == 0) && block.data_len > 0 {
                    return Err(WALReaderError::InvalidDataLength {
                        len: block.data_len,
                        lsn: lsn,
                    });
                }

                datatotal += block.data_len as usize;

                // Process full-page image if it has
                if (block.flags & BKPBLOCK_HAS_IMAGE) != 0 {
                    let mut image = WALFullPageImage::default();

                    image.bimg_len = record_ptr.get_u16_le();
                    image.hole_offset = record_ptr.get_u16_le();
                    image.bimg_info = record_ptr.get_u8();

                    if (image.bimg_info
                        & (BKPIMAGE_BKPIMAGE_COMPRESS_PGLZ
                            | BKPIMAGE_BKPIMAGE_COMPRESS_LZ4
                            | BKPIMAGE_BKPIMAGE_COMPRESS_ZSTD))
                        != 0
                    {
                        image.hole_len = if (image.bimg_info & BKPIMAGE_HAS_HOLE) != 0 {
                            record_ptr.get_u16_le()
                        } else {
                            0
                        };
                    } else {
                        image.hole_len = BLCKSZ_U16 - image.bimg_len;
                    }
                    datatotal += image.bimg_len as usize;

                    // cross-check that hole_offset > 0, hole_len > 0 and
                    // bimg_len > BLCKSZ if the HAS_HOLE flag is set.
                    if ((image.bimg_info & BKPIMAGE_HAS_HOLE) != 0)
                        && (image.hole_offset == 0
                            || image.hole_len == 0
                            || image.bimg_len == BLCKSZ_U16)
                    {
                        return Err(WALReaderError::InvalidHoleData {
                            hole_offset: image.hole_offset,
                            hole_len: image.hole_len,
                            img_len: image.bimg_len,
                            lsn: lsn,
                        });
                    }

                    if ((image.bimg_info & BKPIMAGE_HAS_HOLE) == 0)
                        && (image.hole_offset != 0 || image.hole_len != 0)
                    {
                        return Err(WALReaderError::InvalidHoleFlag {
                            hole_offset: image.hole_offset,
                            hole_len: image.hole_len,
                            lsn: lsn,
                        });
                    }

                    if (image.bimg_info
                        & (BKPIMAGE_BKPIMAGE_COMPRESS_PGLZ
                            | BKPIMAGE_BKPIMAGE_COMPRESS_LZ4
                            | BKPIMAGE_BKPIMAGE_COMPRESS_ZSTD))
                        != 0
                        && image.bimg_len == BLCKSZ_U16
                    {
                        return Err(WALReaderError::InvalidCompressFlag {
                            len: image.bimg_len,
                            lsn: lsn,
                        });
                    }

                    // Complete to parse full-page image block.
                    block.image = Some(image);
                }

                if (block.flags & BKPBLOCK_SAME_REL) == 0 {
                    // Get RelFileLocator
                    block.rlocator.spcOid = record_ptr.get_u32_le();
                    block.rlocator.dbOid = record_ptr.get_u32_le();
                    block.rlocator.relNumber = record_ptr.get_u32_le();

                    rlocator = Some(block.rlocator);
                } else {
                    if rlocator.is_none() {
                        return Err(WALReaderError::SameRelNotFound(lsn));
                    }

                    // Copy from the previously-taken rloc.
                    block.rlocator = rlocator.unwrap();
                }

                block.blocknum = record_ptr.get_u32_le();
            }
        }

        if record_ptr.len() != datatotal {
            return Err(WALReaderError::InvalidRecordLength(lsn));
        }

        for (i, block) in record_info.blocks.iter_mut().enumerate() {
            if i >= record_info.nblocks_inuse {
                break;
            }

            if let Some(image) = &mut block.image {
                let bimg_len = image.bimg_len as usize;
                image.bimg_data.extend_from_slice(&record_ptr[..bimg_len]);
                record_ptr.advance(bimg_len);
            }

            if (block.flags & BKPBLOCK_HAS_DATA) != 0 {
                let data_len = block.data_len as usize;
                block
                    .data
                    .get_or_insert(Vec::new())
                    .extend_from_slice(&record_ptr[..data_len]);
                record_ptr.advance(data_len);
            }
        }

        if let Some(main) = &mut record_info.main {
            let main_len = record_info.main_len as usize;
            main.extend_from_slice(&record_ptr[..main_len]);
            record_ptr.advance(main_len);
        }

        record_info.raw = record_bytes.to_vec();

        Ok(record_info)
    }

    fn get_page(&self) -> &[u8] {
        &self.page_buffer
    }
}
