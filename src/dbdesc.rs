use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

// Database xl_info op values (PG18: dbcommands_xlog.h)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseOp {
    CreateFileCopy,
    CreateWalLog,
    Drop,
    Unknown,
}

impl DatabaseOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => DatabaseOp::CreateFileCopy,
	    0x10 => DatabaseOp::CreateWalLog,
	    0x20 => DatabaseOp::Drop,
	    _ => DatabaseOp::Unknown,
	}
    }
}

impl fmt::Display for DatabaseOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    DatabaseOp::CreateFileCopy => write!(f, "CREATE_FILE_COPY"),
	    DatabaseOp::CreateWalLog => write!(f, "CREATE_WAL_LOG"),
	    DatabaseOp::Drop => write!(f, "DROP"),
	    DatabaseOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_database_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = DatabaseOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        DatabaseOp::CreateFileCopy => {
            let db_id     = r.read_u32_le().unwrap_or(0);
            let ts_id     = r.read_u32_le().unwrap_or(0);
            let src_db_id = r.read_u32_le().unwrap_or(0);
            let src_ts_id = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  db_id:          {}", db_id));
            lines.push(format!("  tablespace_id:  {}", ts_id));
            lines.push(format!("  src_db_id:      {}", src_db_id));
            lines.push(format!("  src_tablespace: {}", src_ts_id));
        }
        DatabaseOp::CreateWalLog => {
            let db_id = r.read_u32_le().unwrap_or(0);
            let ts_id = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  db_id:         {}", db_id));
            lines.push(format!("  tablespace_id: {}", ts_id));
        }
        DatabaseOp::Drop => {
            let db_id        = r.read_u32_le().unwrap_or(0);
            let ntablespaces = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  db_id:        {}", db_id));
            lines.push(format!("  ntablespaces: {}", ntablespaces));
            let show = (ntablespaces.max(0) as usize).min(16);
            for i in 0..show {
                if let Some(ts) = r.read_u32_le() {
                    lines.push(format!("  tablespace[{}]: {}", i, ts));
                }
            }
        }
        DatabaseOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown database op)", main.len()));
        }
    }
    lines
}
