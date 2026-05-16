/// Dispatch WAL record descriptions to the per-rmgr *desc modules.
use crate::btdesc::describe_btree_main;
use crate::clogdesc::describe_clog_main;
use crate::committsdesc::describe_commit_ts_main;
use crate::dbdesc::describe_database_main;
use crate::heap2desc::{describe_heap2_block_data, describe_heap2_main};
use crate::heapdesc::{describe_heap_block_data, describe_heap_main};
use crate::logicalmsgdesc::describe_logical_msg_main;
use crate::multixactdesc::describe_multixact_main;
use crate::relmapdesc::describe_relmap_main;
use crate::replorigindesc::describe_replorigin_main;
use crate::rmgr::RmgrId;
use crate::seqdesc::describe_seq_main;
use crate::smgrdesc::describe_smgr_main;
use crate::standbydesc::describe_standby_main;
use crate::tblspcdesc::describe_tablespace_main;
use crate::walreader::WALRecordInfo;
use crate::xactdesc::describe_xact_main;
use crate::xlogdesc::describe_xlog_main;

/// Return human-readable field descriptions for the main-data portion of a WAL record.
pub fn describe_main_data(record: &WALRecordInfo) -> Vec<String> {
    let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
    let info = record.xlrec.xl_info;

    let main: &[u8] = match &record.main {
        Some(m) if !m.is_empty() => m.as_slice(),
        _ => return vec!["  (no main data)".to_string()],
    };

    match rmgr {
        RmgrId::Heap              => describe_heap_main(info, main, record),
        RmgrId::Heap2             => describe_heap2_main(info, main),
        RmgrId::Xact              => describe_xact_main(info, main),
        RmgrId::Xlog              => describe_xlog_main(info, main),
        RmgrId::Smgr              => describe_smgr_main(info, main),
        RmgrId::Clog              => describe_clog_main(info, main),
        RmgrId::Database          => describe_database_main(info, main),
        RmgrId::Tablespace        => describe_tablespace_main(info, main),
        RmgrId::MultiXact         => describe_multixact_main(info, main),
        RmgrId::Relmap            => describe_relmap_main(info, main),
        RmgrId::Standby           => describe_standby_main(info, main),
        RmgrId::Btree             => describe_btree_main(info, main),
        RmgrId::Sequence          => describe_seq_main(info, main),
        RmgrId::CommitTs          => describe_commit_ts_main(info, main),
        RmgrId::ReplicationOrigin => describe_replorigin_main(info, main),
        RmgrId::LogicalMessage    => describe_logical_msg_main(info, main),
        _ => vec![format!("  ({} bytes, no parser for {:?})", main.len(), rmgr)],
    }
}

/// Return human-readable field descriptions for block-specific data.
pub fn describe_block_data(record: &WALRecordInfo, block_idx: usize) -> Vec<String> {
    let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
    let info = record.xlrec.xl_info;

    let nblocks = record.nblocks_inuse;
    if block_idx >= nblocks {
        return vec![];
    }
    let block = &record.blocks[block_idx];
    let data: &[u8] = match &block.data {
        Some(d) if !d.is_empty() => d.as_slice(),
        _ => return vec![],
    };

    match rmgr {
        RmgrId::Heap  => describe_heap_block_data(info, block_idx, data, record),
        RmgrId::Heap2 => describe_heap2_block_data(info, block_idx, data),
        _ => vec![],
    }
}
