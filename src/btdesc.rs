use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BtreeOp {
    InsertLeaf,
    InsertUpper,
    InsertMeta,
    SplitL,
    SplitR,
    InsertPost,
    Dedup,
    Delete,
    UnlinkPage,
    UnlinkPageMeta,
    NewRoot,
    MarkPageHalfDead,
    Vacuum,
    ReusePage,
    MetaCleanup,
    Unknown,
}

impl BtreeOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => BtreeOp::InsertLeaf,
	    0x10 => BtreeOp::InsertUpper,
	    0x20 => BtreeOp::InsertMeta,
	    0x30 => BtreeOp::SplitL,
	    0x40 => BtreeOp::SplitR,
	    0x50 => BtreeOp::InsertPost,
	    0x60 => BtreeOp::Dedup,
	    0x70 => BtreeOp::Delete,
	    0x80 => BtreeOp::UnlinkPage,
	    0x90 => BtreeOp::UnlinkPageMeta,
	    0xA0 => BtreeOp::NewRoot,
	    0xB0 => BtreeOp::MarkPageHalfDead,
	    0xC0 => BtreeOp::Vacuum,
	    0xD0 => BtreeOp::ReusePage,
	    0xE0 => BtreeOp::MetaCleanup,
	    _ => BtreeOp::Unknown,
	}
    }
}

impl fmt::Display for BtreeOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    BtreeOp::InsertLeaf => write!(f, "INSERT_LEAF"),
	    BtreeOp::InsertUpper => write!(f, "INSERT_UPPER"),
	    BtreeOp::InsertMeta => write!(f, "INSERT_META"),
	    BtreeOp::SplitL => write!(f, "SPLIT_L"),
	    BtreeOp::SplitR => write!(f, "SPLIT_R"),
	    BtreeOp::InsertPost => write!(f, "INSERT_POST"),
	    BtreeOp::Dedup => write!(f, "DEDUP"),
	    BtreeOp::Delete => write!(f, "DELETE"),
	    BtreeOp::UnlinkPage => write!(f, "UNLINK_PAGE"),
	    BtreeOp::UnlinkPageMeta => write!(f, "UNLINK_PAGE_META"),
	    BtreeOp::NewRoot => write!(f, "NEWROOT"),
	    BtreeOp::MarkPageHalfDead => write!(f, "MARK_PAGE_HALFDEAD"),
	    BtreeOp::Vacuum => write!(f, "VACUUM"),
	    BtreeOp::ReusePage => write!(f, "REUSE_PAGE"),
	    BtreeOp::MetaCleanup => write!(f, "META_CLEANUP"),
	    BtreeOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn parse_btree_metadata(r: &mut Reader) -> Vec<String> {
    let version              = r.read_u32_le().unwrap_or(0);
    let root                 = r.read_u32_le().unwrap_or(0);
    let level                = r.read_u32_le().unwrap_or(0);
    let fastroot             = r.read_u32_le().unwrap_or(0);
    let fastlevel            = r.read_u32_le().unwrap_or(0);
    let last_cleanup_delpages = r.read_u32_le().unwrap_or(0);
    let allequalimage        = r.read_bool().unwrap_or(false);
    vec![
        format!("  meta.version:               {}", version),
        format!("  meta.root:                  {}", root),
        format!("  meta.level:                 {}", level),
        format!("  meta.fastroot:              {}", fastroot),
        format!("  meta.fastlevel:             {}", fastlevel),
        format!("  meta.last_cleanup_delpages: {}", last_cleanup_delpages),
        format!("  meta.allequalimage:         {}", allequalimage),
    ]
}

pub fn describe_btree_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = BtreeOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        BtreeOp::InsertLeaf | BtreeOp::InsertUpper | BtreeOp::InsertMeta | BtreeOp::InsertPost => {
            let offnum = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  offnum: {}", offnum));
            if op == BtreeOp::InsertMeta && r.remaining() >= 28 {
                lines.extend(parse_btree_metadata(&mut r));
            }
        }

        BtreeOp::SplitL | BtreeOp::SplitR => {
            let level        = r.read_u32_le().unwrap_or(0);
            let firstrightoff = r.read_u16_le().unwrap_or(0);
            let newitemoff   = r.read_u16_le().unwrap_or(0);
            let postingoff   = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  level:         {}", level));
            lines.push(format!("  firstrightoff: {}", firstrightoff));
            lines.push(format!("  newitemoff:    {}", newitemoff));
            lines.push(format!("  postingoff:    {}", postingoff));
        }

        BtreeOp::Dedup => {
            let nintervals = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  nintervals: {}", nintervals));
        }

        BtreeOp::Delete => {
            let horizon    = r.read_u32_le().unwrap_or(0);
            let ndeleted   = r.read_u16_le().unwrap_or(0);
            let nupdated   = r.read_u16_le().unwrap_or(0);
            let is_catalog = r.read_bool().unwrap_or(false);
            lines.push(format!("  snapshotConflictHorizon: {}", horizon));
            lines.push(format!("  ndeleted:     {}", ndeleted));
            lines.push(format!("  nupdated:     {}", nupdated));
            lines.push(format!("  isCatalogRel: {}", is_catalog));
        }

        BtreeOp::Vacuum => {
            let ndeleted = r.read_u16_le().unwrap_or(0);
            let nupdated = r.read_u16_le().unwrap_or(0);
            lines.push(format!("  ndeleted: {}", ndeleted));
            lines.push(format!("  nupdated: {}", nupdated));
        }

        BtreeOp::MarkPageHalfDead => {
            let poffset   = r.read_u16_le().unwrap_or(0);
            r.skip(2);
            let leafblk   = r.read_u32_le().unwrap_or(0);
            let leftblk   = r.read_u32_le().unwrap_or(0);
            let rightblk  = r.read_u32_le().unwrap_or(0);
            let topparent = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  poffset:   {}", poffset));
            lines.push(format!("  leafblk:   {}", leafblk));
            lines.push(format!("  leftblk:   {}", leftblk));
            lines.push(format!("  rightblk:  {}", rightblk));
            lines.push(format!("  topparent: {}", topparent));
        }

        BtreeOp::UnlinkPage | BtreeOp::UnlinkPageMeta => {
            let leftsib      = r.read_u32_le().unwrap_or(0);
            let rightsib     = r.read_u32_le().unwrap_or(0);
            let level        = r.read_u32_le().unwrap_or(0);
            r.skip(4);
            let safexid      = r.read_u64_le().unwrap_or(0);
            let leafleftsib  = r.read_u32_le().unwrap_or(0);
            let leafrightsib = r.read_u32_le().unwrap_or(0);
            let leaftopparent = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  leftsib:       {}", leftsib));
            lines.push(format!("  rightsib:      {}", rightsib));
            lines.push(format!("  level:         {}", level));
            lines.push(format!("  safexid:       {}", safexid));
            lines.push(format!("  leafleftsib:   {}", leafleftsib));
            lines.push(format!("  leafrightsib:  {}", leafrightsib));
            lines.push(format!("  leaftopparent: {}", leaftopparent));
            if op == BtreeOp::UnlinkPageMeta {
                lines.extend(parse_btree_metadata(&mut r));
            }
        }

        BtreeOp::NewRoot => {
            let rootblk = r.read_u32_le().unwrap_or(0);
            let level   = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  rootblk: {}", rootblk));
            lines.push(format!("  level:   {}", level));
        }

        BtreeOp::ReusePage => {
            let spc        = r.read_u32_le().unwrap_or(0);
            let db         = r.read_u32_le().unwrap_or(0);
            let rel        = r.read_u32_le().unwrap_or(0);
            let block      = r.read_u32_le().unwrap_or(0);
            let horizon    = r.read_u64_le().unwrap_or(0);
            let is_catalog = r.read_bool().unwrap_or(false);
            lines.push(format!("  locator:                 {}/{}/{}", spc, db, rel));
            lines.push(format!("  block:                   {}", block));
            lines.push(format!("  snapshotConflictHorizon: {}", horizon));
            lines.push(format!("  isCatalogRel:            {}", is_catalog));
        }

        BtreeOp::MetaCleanup => {
            lines.extend(parse_btree_metadata(&mut r));
        }

        BtreeOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown btree op)", main.len()));
        }
    }
    lines
}
