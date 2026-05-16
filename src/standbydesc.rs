use std::fmt;

use crate::walmisc::Reader;
use crate::walreader::XLR_RMGR_INFO_MASK;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StandbyOp {
    Lock,
    RunningXacts,
    Invalidations,
    Unknown,
}

impl StandbyOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => StandbyOp::Lock,
	    0x10 => StandbyOp::RunningXacts,
	    0x20 => StandbyOp::Invalidations,
	    _ => StandbyOp::Unknown,
	}
    }
}

impl fmt::Display for StandbyOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    StandbyOp::Lock => write!(f, "LOCK"),
	    StandbyOp::RunningXacts => write!(f, "RUNNING_XACTS"),
	    StandbyOp::Invalidations => write!(f, "INVALIDATIONS"),
	    StandbyOp::Unknown => write!(f, "Unknown"),
	}
    }
}

pub fn describe_standby_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = StandbyOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        StandbyOp::Lock => {
            let nlocks = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  nlocks: {}", nlocks));
            let show = (nlocks.max(0) as usize).min(16);
            for i in 0..show {
                let xid    = r.read_u32_le().unwrap_or(0);
                let db_oid = r.read_u32_le().unwrap_or(0);
                let rel_oid = r.read_u32_le().unwrap_or(0);
                lines.push(format!("  lock[{}]: xid={} db={} rel={}", i, xid, db_oid, rel_oid));
            }
        }
        StandbyOp::RunningXacts => {
            let xcnt             = r.read_i32_le().unwrap_or(0);
            let subxcnt          = r.read_i32_le().unwrap_or(0);
            let overflow         = r.read_bool().unwrap_or(false);
            r.skip(3);
            let next_xid         = r.read_u32_le().unwrap_or(0);
            let oldest_running   = r.read_u32_le().unwrap_or(0);
            let latest_completed = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  xcnt:               {}", xcnt));
            lines.push(format!("  subxcnt:            {}", subxcnt));
            lines.push(format!("  subxid_overflow:    {}", overflow));
            lines.push(format!("  nextXid:            {}", next_xid));
            lines.push(format!("  oldestRunningXid:   {}", oldest_running));
            lines.push(format!("  latestCompletedXid: {}", latest_completed));
            let total = (xcnt + subxcnt).max(0) as usize;
            let show  = total.min(16);
            for i in 0..show {
                if let Some(xid) = r.read_u32_le() {
                    lines.push(format!("  xid[{}]: {}", i, xid));
                }
            }
            if total > 16 {
                lines.push(format!("  ... ({} more xids)", total - 16));
            }
        }
        StandbyOp::Invalidations => {
            let db_id          = r.read_u32_le().unwrap_or(0);
            let ts_id          = r.read_u32_le().unwrap_or(0);
            let relcache_inval = r.read_bool().unwrap_or(false);
            r.skip(3);
            let nmsgs          = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  dbId:                  {}", db_id));
            lines.push(format!("  tsId:                  {}", ts_id));
            lines.push(format!("  relcacheInitFileInval: {}", relcache_inval));
            lines.push(format!("  nmsgs:                 {}", nmsgs));
        }
        StandbyOp::Unknown => {
            lines.push(format!("  ({} bytes, unknown standby op)", main.len()));
        }
    }
    lines
}
