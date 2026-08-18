pub mod brindesc;
pub mod btdesc;
pub mod buildinfo;
pub mod clogdesc;
pub mod committsdesc;
pub mod crc32c;
pub mod dbdesc;
pub mod genericdesc;
pub mod gindesc;
pub mod gistdesc;
pub mod hashdesc;
pub mod heap2desc;
pub mod heapdesc;
pub mod logicalmsgdesc;
pub mod multixactdesc;
pub mod relmapdesc;
pub mod replorigindesc;
pub mod rmgr;
pub mod seqdesc;
pub mod smgrdesc;
pub mod spgistdesc;
pub mod standbydesc;
pub mod tblspcdesc;
pub mod walmain;
pub mod walmisc;
pub mod walreader;
pub mod xactdesc;
pub mod xlogdesc;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(dead_code)]
pub mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
