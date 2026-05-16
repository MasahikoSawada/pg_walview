use std::fmt;

use crate::walmisc::{fmt_pg_ts, lsn_format, Reader};
use crate::walreader::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XlogOp {
    CheckpointShutdown,
    CheckpointOnline,
    Noop,
    NextOid,
    Switch,
    BackupEnd,
    ParameterChange,
    RestorePoint,
    FPWChange,
    EndOfRecovery,
    FPIForHint,
    FPI,
    OverwriteContRecord,
    CheckpointRedo,
    Unknown,
}

impl XlogOp {
    pub fn from_xl_info(info: u8) -> Self {
	match info & XLR_RMGR_INFO_MASK {
	    0x00 => XlogOp::CheckpointShutdown,
	    0x10 => XlogOp::CheckpointOnline,
	    0x20 => XlogOp::Noop,
	    0x30 => XlogOp::NextOid,
	    0x40 => XlogOp::Switch,
	    0x50 => XlogOp::BackupEnd,
	    0x60 => XlogOp::ParameterChange,
	    0x70 => XlogOp::RestorePoint,
	    0x80 => XlogOp::FPWChange,
	    0x90 => XlogOp::EndOfRecovery,
	    0xA0 => XlogOp::FPIForHint,
	    0xB0 => XlogOp::FPI,
	    0xD0 => XlogOp::OverwriteContRecord,
	    0xE0 => XlogOp::CheckpointRedo,
	    _ => XlogOp::Unknown,
	}
    }
}

impl fmt::Display for XlogOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    XlogOp::CheckpointShutdown => write!(f, "CHECKPOINT_SHTUDOWN"),
	    XlogOp::CheckpointOnline => write!(f, "CHECKPOINT_ONLINE"),
	    XlogOp::Noop => write!(f, "NOOP"),
	    XlogOp::NextOid => write!(f, "NEXTOID"),
	    XlogOp::Switch => write!(f, "SWITCH"),
	    XlogOp::BackupEnd => write!(f, "BACKUP_END"),
	    XlogOp::ParameterChange => write!(f, "PARAMETER_CHANGE"),
	    XlogOp::RestorePoint => write!(f, "RESTORE_POINT"),
	    XlogOp::FPWChange => write!(f, "FPW_CHANGE"),
	    XlogOp::EndOfRecovery => write!(f, "END_OF_RECOVERY"),
	    XlogOp::FPIForHint => write!(f, "FPI_FOR_HINT"),
	    XlogOp::FPI => write!(f, "FPI"),
	    XlogOp::OverwriteContRecord => write!(f, "OVERWRITE_CONTRECORD"),
	    XlogOp::CheckpointRedo => write!(f, "CHECKPOINT_REDO"),
	    _ => write!(f, "Unknown"),
	}
    }
}

fn decode_wal_level(level: i32) -> &'static str {
    match level {
        0 => "minimal",
        1 => "replica",
        2 => "logical",
        _ => "unknown",
    }
}

pub fn describe_xlog_main(info: u8, main: &[u8]) -> Vec<String> {
    let op = XlogOp::from_xl_info(info);
    let mut r = Reader::new(main);
    let mut lines = Vec::new();

    match op {
        XlogOp::CheckpointShutdown | XlogOp::CheckpointOnline => {
            let redo             = r.read_u64_le().unwrap_or(0);
            let this_tli         = r.read_u32_le().unwrap_or(0);
            let prev_tli         = r.read_u32_le().unwrap_or(0);
            let fpw              = r.read_bool().unwrap_or(false);
            r.skip(3);
            let wal_level        = r.read_i32_le().unwrap_or(0);
            let next_xid         = r.read_u64_le().unwrap_or(0);
            let next_oid         = r.read_u32_le().unwrap_or(0);
            let next_multi       = r.read_u32_le().unwrap_or(0);
            let next_multi_off   = r.read_u32_le().unwrap_or(0);
            let oldest_xid       = r.read_u32_le().unwrap_or(0);
            let oldest_xid_db    = r.read_u32_le().unwrap_or(0);
            let oldest_multi     = r.read_u32_le().unwrap_or(0);
            let oldest_multi_db  = r.read_u32_le().unwrap_or(0);
            r.skip(4);
            let time             = r.read_i64_le().unwrap_or(0);
            let oldest_commit_ts = r.read_u32_le().unwrap_or(0);
            let newest_commit_ts = r.read_u32_le().unwrap_or(0);
            let oldest_active_xid = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  redo:               {}", lsn_format(redo)));
            lines.push(format!("  ThisTimeLineID:     {}", this_tli));
            lines.push(format!("  PrevTimeLineID:     {}", prev_tli));
            lines.push(format!("  fullPageWrites:     {}", fpw));
            lines.push(format!("  wal_level:          {} ({})", wal_level, decode_wal_level(wal_level)));
            lines.push(format!("  nextXid:            {}", next_xid));
            lines.push(format!("  nextOid:            {}", next_oid));
            lines.push(format!("  nextMulti:          {}", next_multi));
            lines.push(format!("  nextMultiOffset:    {}", next_multi_off));
            lines.push(format!("  oldestXid:          {}", oldest_xid));
            lines.push(format!("  oldestXidDB:        {}", oldest_xid_db));
            lines.push(format!("  oldestMulti:        {}", oldest_multi));
            lines.push(format!("  oldestMultiDB:      {}", oldest_multi_db));
            lines.push(format!("  time:               {} ({})", time, fmt_pg_ts(time * 1_000_000)));
            lines.push(format!("  oldestCommitTsXid:  {}", oldest_commit_ts));
            lines.push(format!("  newestCommitTsXid:  {}", newest_commit_ts));
            lines.push(format!("  oldestActiveXid:    {}", oldest_active_xid));
        }

        XlogOp::NextOid => {
            let oid = r.read_u32_le().unwrap_or(0);
            lines.push(format!("  nextOid: {}", oid));
        }

        XlogOp::BackupEnd => {
            let start_lsn = r.read_u64_le().unwrap_or(0);
            lines.push(format!("  startpoint: {}", lsn_format(start_lsn)));
        }

        XlogOp::ParameterChange => {
            let max_conn        = r.read_i32_le().unwrap_or(0);
            let max_workers     = r.read_i32_le().unwrap_or(0);
            let max_wal_senders = r.read_i32_le().unwrap_or(0);
            let max_prep        = r.read_i32_le().unwrap_or(0);
            let max_locks       = r.read_i32_le().unwrap_or(0);
            let wal_level       = r.read_i32_le().unwrap_or(0);
            let wal_log_hints   = r.read_bool().unwrap_or(false);
            let track_cts       = r.read_bool().unwrap_or(false);
            lines.push(format!("  MaxConnections:          {}", max_conn));
            lines.push(format!("  max_worker_processes:    {}", max_workers));
            lines.push(format!("  max_wal_senders:         {}", max_wal_senders));
            lines.push(format!("  max_prepared_xacts:      {}", max_prep));
            lines.push(format!("  max_locks_per_xact:      {}", max_locks));
            lines.push(format!("  wal_level:               {} ({})", wal_level, decode_wal_level(wal_level)));
            lines.push(format!("  wal_log_hints:           {}", wal_log_hints));
            lines.push(format!("  track_commit_timestamp:  {}", track_cts));
        }

        XlogOp::RestorePoint => {
            let rp_time   = r.read_i64_le().unwrap_or(0);
            let name_bytes = r.peek_bytes(64);
            let name = String::from_utf8_lossy(
                &name_bytes[..name_bytes.iter().position(|&b| b == 0).unwrap_or(64)],
            ).into_owned();
            lines.push(format!("  rp_time: {} µs  ({})", rp_time, fmt_pg_ts(rp_time)));
            lines.push(format!("  rp_name: {:?}", name));
        }

        XlogOp::FPWChange => {
            let fpw = r.read_bool().unwrap_or(false);
            lines.push(format!("  fullPageWrites: {}", fpw));
        }

        XlogOp::EndOfRecovery => {
            let end_time  = r.read_i64_le().unwrap_or(0);
            let this_tli  = r.read_u32_le().unwrap_or(0);
            let prev_tli  = r.read_u32_le().unwrap_or(0);
            let wal_level = r.read_i32_le().unwrap_or(0);
            lines.push(format!("  end_time:       {} µs  ({})", end_time, fmt_pg_ts(end_time)));
            lines.push(format!("  ThisTimeLineID: {}", this_tli));
            lines.push(format!("  PrevTimeLineID: {}", prev_tli));
            lines.push(format!("  wal_level:      {} ({})", wal_level, decode_wal_level(wal_level)));
        }

        XlogOp::OverwriteContRecord => {
            let ow_lsn  = r.read_u64_le().unwrap_or(0);
            let ow_time = r.read_i64_le().unwrap_or(0);
            lines.push(format!("  overwritten_lsn: {}", lsn_format(ow_lsn)));
            lines.push(format!("  overwrite_time:  {} µs  ({})", ow_time, fmt_pg_ts(ow_time)));
        }

        _ => {
            lines.push(format!("  ({} bytes)", main.len()));
        }
    }
    lines
}
