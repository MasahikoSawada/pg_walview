pub mod walmisc;
pub mod walreader;
pub mod walmain;
pub mod rmgr;
pub mod xlogdesc;
pub mod xactdesc;
pub mod smgrdesc;
pub mod clogdesc;
pub mod dbdesc;
pub mod tblspcdesc;
pub mod multixactdesc;
pub mod relmapdesc;
pub mod standbydesc;
pub mod heap2desc;
pub mod heapdesc;
pub mod btdesc;
pub mod hashdesc;
pub mod gindesc;
pub mod gistdesc;
pub mod seqdesc;
pub mod spgistdesc;
pub mod brindesc;
pub mod committsdesc;
pub mod replorigindesc;
pub mod genericdesc;
pub mod logicalmsgdesc;

#[allow(non_upper_case_globals)]
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[allow(dead_code)]
pub mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
