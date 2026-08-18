/// Dispatch WAL record descriptions to the per-rmgr *desc modules.
use crate::brindesc::BrinOp;
use crate::btdesc::{BtreeOp, describe_btree_main};
use crate::clogdesc::{ClogOp, describe_clog_main};
use crate::committsdesc::{CommitTsOp, describe_commit_ts_main};
use crate::dbdesc::{DatabaseOp, describe_database_main};
use crate::genericdesc::GenericOp;
use crate::gindesc::GinOp;
use crate::gistdesc::GistOp;
use crate::hashdesc::HashOp;
use crate::heap2desc::{Heap2Op, describe_heap2_block_data, describe_heap2_main};
use crate::heapdesc::{HeapOp, describe_heap_block_data, describe_heap_main};
use crate::logicalmsgdesc::{LogicalMsgOp, describe_logical_msg_main};
use crate::multixactdesc::{MultiXactOp, describe_multixact_main};
use crate::relmapdesc::{RelmapOp, describe_relmap_main};
use crate::replorigindesc::{ReplOriginOp, describe_replorigin_main};
use crate::rmgr::RmgrId;
use crate::seqdesc::{SequenceOp, describe_seq_main};
use crate::smgrdesc::{SmgrOp, describe_smgr_main};
use crate::spgistdesc::SpGistOp;
use crate::standbydesc::{StandbyOp, describe_standby_main};
use crate::tblspcdesc::{TablespaceOp, describe_tablespace_main};
use crate::walreader::WALRecordInfo;
use crate::xactdesc::{XactOp, describe_xact_main};
use crate::xlogdesc::{XlogOp, describe_xlog_main};

/// Return human-readable field descriptions for the main-data portion of a WAL record.
pub fn describe_main_data(record: &WALRecordInfo) -> Vec<String> {
    let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
    let info = record.xlrec.xl_info;

    let main: &[u8] = match record.main_data() {
        Some(m) if !m.is_empty() => m,
        _ => return vec!["  (no main data)".to_string()],
    };

    match rmgr {
        RmgrId::Heap => describe_heap_main(info, main, record),
        RmgrId::Heap2 => describe_heap2_main(info, main),
        RmgrId::Xact => describe_xact_main(info, main),
        RmgrId::Xlog => describe_xlog_main(info, main),
        RmgrId::Smgr => describe_smgr_main(info, main),
        RmgrId::Clog => describe_clog_main(info, main),
        RmgrId::Database => describe_database_main(info, main),
        RmgrId::Tablespace => describe_tablespace_main(info, main),
        RmgrId::MultiXact => describe_multixact_main(info, main),
        RmgrId::Relmap => describe_relmap_main(info, main),
        RmgrId::Standby => describe_standby_main(info, main),
        RmgrId::Btree => describe_btree_main(info, main),
        RmgrId::Sequence => describe_seq_main(info, main),
        RmgrId::CommitTs => describe_commit_ts_main(info, main),
        RmgrId::ReplicationOrigin => describe_replorigin_main(info, main),
        RmgrId::LogicalMessage => describe_logical_msg_main(info, main),
        _ => vec![format!(
            "  ({} bytes, no parser for {:?})",
            main.len(),
            rmgr
        )],
    }
}

/// Return human-readable field descriptions for block-specific data.
pub fn describe_block_data(record: &WALRecordInfo, block_idx: usize) -> Vec<String> {
    let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
    let info = record.xlrec.xl_info;

    let data: &[u8] = match record.block_data(block_idx) {
        Some(d) if !d.is_empty() => d,
        _ => return vec![],
    };

    match rmgr {
        RmgrId::Heap => describe_heap_block_data(info, block_idx, data, record),
        RmgrId::Heap2 => describe_heap2_block_data(info, block_idx, data),
        _ => vec![],
    }
}

/// The per-rmgr one-line identifier, the same string pg_waldump prints
/// before the record description.
pub fn describe_record(record: &WALRecordInfo) -> String {
    let info = record.xlrec.xl_info;
    match RmgrId::from_u8(record.xlrec.xl_rmid) {
        RmgrId::Xlog => XlogOp::from_xl_info(info).to_string(),
        RmgrId::Xact => XactOp::from_xl_info(info).to_string(),
        RmgrId::Smgr => SmgrOp::from_xl_info(info).to_string(),
        RmgrId::Clog => ClogOp::from_xl_info(info).to_string(),
        RmgrId::Database => DatabaseOp::from_xl_info(info).to_string(),
        RmgrId::Tablespace => TablespaceOp::from_xl_info(info).to_string(),
        RmgrId::MultiXact => MultiXactOp::from_xl_info(info).to_string(),
        RmgrId::Relmap => RelmapOp::from_xl_info(info).to_string(),
        RmgrId::Standby => StandbyOp::from_xl_info(info).to_string(),
        RmgrId::Heap2 => Heap2Op::identify(info),
        RmgrId::Heap => HeapOp::identify(info),
        RmgrId::Btree => BtreeOp::from_xl_info(info).to_string(),
        RmgrId::Hash => HashOp::from_xl_info(info).to_string(),
        RmgrId::Gin => GinOp::from_xl_info(info).to_string(),
        RmgrId::Gist => GistOp::from_xl_info(info).to_string(),
        RmgrId::Sequence => SequenceOp::from_xl_info(info).to_string(),
        RmgrId::SPGist => SpGistOp::from_xl_info(info).to_string(),
        RmgrId::Brin => BrinOp::identify(info),
        RmgrId::CommitTs => CommitTsOp::from_xl_info(info).to_string(),
        RmgrId::ReplicationOrigin => ReplOriginOp::from_xl_info(info).to_string(),
        RmgrId::Generic => GenericOp::from_xl_info(info).to_string(),
        RmgrId::LogicalMessage => LogicalMsgOp::from_xl_info(info).to_string(),
        RmgrId::Unknown(_) => "Unknown".to_string(),
    }
}
