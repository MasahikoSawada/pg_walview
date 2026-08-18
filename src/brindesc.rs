use std::fmt;

// BRIN xl_info op values (access/brin_xlog.h).  As with heap, the top bit is
// the independent XLOG_BRIN_INIT_PAGE flag, not part of the opcode; masking
// the whole high nibble made every "+INIT" record decode as Unknown.
pub const XLOG_BRIN_OPMASK: u8 = 0x70;
pub const XLOG_BRIN_INIT_PAGE: u8 = 0x80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrinOp {
    CreateIndex,
    Insert,
    Update,
    SamepageUpdate,
    RevmapExtend,
    Desummarize,
    Unknown,
}

impl BrinOp {
    pub fn from_xl_info(info: u8) -> Self {
        match info & XLOG_BRIN_OPMASK {
            0x00 => BrinOp::CreateIndex,
            0x10 => BrinOp::Insert,
            0x20 => BrinOp::Update,
            0x30 => BrinOp::SamepageUpdate,
            0x40 => BrinOp::RevmapExtend,
            0x50 => BrinOp::Desummarize,
            _ => BrinOp::Unknown,
        }
    }

    pub fn is_init_page(info: u8) -> bool {
        (info & XLOG_BRIN_INIT_PAGE) != 0
    }

    /// The identifier pg_waldump prints, e.g. "INSERT+INIT".
    pub fn identify(info: u8) -> String {
        let op = Self::from_xl_info(info);
        if Self::is_init_page(info) {
            format!("{}+INIT", op)
        } else {
            op.to_string()
        }
    }
}

impl fmt::Display for BrinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BrinOp::CreateIndex => write!(f, "CREATE_INDEX"),
            BrinOp::Insert => write!(f, "INSERT"),
            BrinOp::Update => write!(f, "UPDATE"),
            BrinOp::SamepageUpdate => write!(f, "SAMEPAGE_UPDATE"),
            BrinOp::RevmapExtend => write!(f, "REVMAP_EXTEND"),
            BrinOp::Desummarize => write!(f, "DESUMMARIZE"),
            BrinOp::Unknown => write!(f, "Unknown"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brin_opcodes_match_postgres() {
        assert_eq!(BrinOp::from_xl_info(0x00), BrinOp::CreateIndex);
        assert_eq!(BrinOp::from_xl_info(0x10), BrinOp::Insert);
        assert_eq!(BrinOp::from_xl_info(0x20), BrinOp::Update);
        assert_eq!(BrinOp::from_xl_info(0x30), BrinOp::SamepageUpdate);
        assert_eq!(BrinOp::from_xl_info(0x40), BrinOp::RevmapExtend);
        assert_eq!(BrinOp::from_xl_info(0x50), BrinOp::Desummarize);
    }

    /// Masking the whole high nibble made every "+INIT" record Unknown.
    #[test]
    fn init_page_flag_is_independent_of_the_opcode() {
        assert_eq!(BrinOp::from_xl_info(0x90), BrinOp::Insert);
        assert_eq!(BrinOp::identify(0x10), "INSERT");
        assert_eq!(BrinOp::identify(0x90), "INSERT+INIT");
        assert_eq!(BrinOp::identify(0xC0), "REVMAP_EXTEND+INIT");
    }
}
