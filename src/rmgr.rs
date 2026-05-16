use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmgrId {
    Xlog, // 0
    Xact, // 1
    Smgr, // 2
    Clog, // 3
    Database, // 4
    Tablespace, // 5
    MultiXact, // 6
    Relmap, // 7
    Standby, // 8
    Heap2, // 9
    Heap, // 10
    Btree, // 11
    Hash, // 12
    Gin, // 13
    Gist, // 14
    Sequence, // 15
    SPGist, // 16
    Brin, // 17
    CommitTs, // 18
    ReplicationOrigin, // 19
    Generic, // 20
    LogicalMessage, // 21
    Unknown(u8),
}

impl RmgrId {
    pub fn from_u8(id: u8) -> Self {
	match id {
	    0 => RmgrId::Xlog,
	    1 => RmgrId::Xact,
	    2 => RmgrId::Smgr,
	    3 => RmgrId::Clog,
	    4 => RmgrId::Database,
	    5 => RmgrId::Tablespace,
	    6 => RmgrId::MultiXact,
	    7 => RmgrId::Relmap,
	    8 => RmgrId::Standby,
	    9 => RmgrId::Heap2,
	    10 => RmgrId::Heap,
	    11 => RmgrId::Btree,
	    12 => RmgrId::Hash,
	    13 => RmgrId::Gin,
	    14 => RmgrId::Gist,
	    15 => RmgrId::Sequence,
	    16 => RmgrId::SPGist,
	    17 => RmgrId::Brin,
	    18 => RmgrId::CommitTs,
	    19 => RmgrId::ReplicationOrigin,
	    20 => RmgrId::Generic,
	    21 => RmgrId::LogicalMessage,
	    _ => RmgrId::Unknown(id),
	}
    }
}

impl fmt::Display for RmgrId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
	match self {
	    RmgrId::Xlog => write!(f, "XLOG"),
	    RmgrId::Xact => write!(f, "Transaction"),
	    RmgrId::Smgr => write!(f, "Storage"),
	    RmgrId::Clog => write!(f, "CLOG"),
	    RmgrId::Database => write!(f, "Database"),
	    RmgrId::Tablespace => write!(f, "Tablespace"),
	    RmgrId::MultiXact => write!(f, "MultiXact"),
	    RmgrId::Relmap => write!(f, "RelMap"),
	    RmgrId::Standby => write!(f, "Standby"),
	    RmgrId::Heap2 => write!(f, "Heap2"),
	    RmgrId::Heap => write!(f, "Heap"),
	    RmgrId::Btree => write!(f, "Btree"),
	    RmgrId::Hash => write!(f, "Hash"),
	    RmgrId::Gin => write!(f, "Gin"),
	    RmgrId::Gist => write!(f, "Gist"),
	    RmgrId::Sequence => write!(f, "Sequence"),
	    RmgrId::SPGist => write!(f, "SPGist"),
	    RmgrId::Brin => write!(f, "BRIN"),
	    RmgrId::CommitTs => write!(f, "CommitTs"),
	    RmgrId::ReplicationOrigin => write!(f, "ReplicationOrigin"),
	    RmgrId::Generic => write!(f, "Generic"),
	    RmgrId::LogicalMessage => write!(f, "LogicalMessage"),
	    RmgrId::Unknown(id) => write!(f, "Unknown({})", id),
	}
    }
}
