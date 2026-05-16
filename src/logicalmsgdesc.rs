use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicalMsgOp {
    Message,
    Unknown,
}

impl LogicalMsgOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => LogicalMsgOp::Message,
	    _ => LogicalMsgOp::Unknown,
	}
    }
}

impl fmt::Display for LogicalMsgOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    LogicalMsgOp::Message => write!(f, "MESSAGE"),
	    LogicalMsgOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_logical_msg_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = LogicalMsgOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        LogicalMsgOp::Message => {
            let db_id         = r.read_u32_le().unwrap_or(0);
            let transactional = r.read_bool().unwrap_or(false);
            r.skip(3);
            let prefix_size  = r.read_u64_le().unwrap_or(0);
            let message_size = r.read_u64_le().unwrap_or(0);
            lines.push(format!("  dbId:          {}", db_id));
            lines.push(format!("  transactional: {}", transactional));
            lines.push(format!("  prefix_size:   {}", prefix_size));
            lines.push(format!("  message_size:  {}", message_size));
            if prefix_size > 0 && r.remaining() >= prefix_size as usize {
                let prefix_bytes = r.peek_bytes(prefix_size as usize - 1);
                let prefix = String::from_utf8_lossy(prefix_bytes).into_owned();
                lines.push(format!("  prefix:        {:?}", prefix));
            }
        }
        LogicalMsgOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown logical_msg op)", main.len()));
        }
    }
    lines
}
