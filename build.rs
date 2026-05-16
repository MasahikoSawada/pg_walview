extern crate bindgen;

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn get_pg_include_dir() -> String {
    if let Ok(path) = env::var("PG_INCLUDE_DIR") {
	return path;
    }

    if let Ok(output) = Command::new("pg_config").arg("--includedir-server").output() {
	if output.status.success() {
	    if let Ok(path) = String::from_utf8(output.stdout) {
		return path.trim().to_string();
	    }
	}
    }

    panic!("could not find include directory");
}

fn main() {
    println!("cargo:rerun-if-env-changed=PG_INCLUDE_DIR");

    let pg_include_dir = get_pg_include_dir();

    let bindings = bindgen::Builder::default()
        .header("deps.h")
        .clang_arg(format!("-I{}", pg_include_dir))
        .derive_default(true)
        .allowlist_type("XLogRecord")
        .allowlist_type("XLogPageHeaderData")
        .allowlist_type("XLogLongPageHeaderData")
        .allowlist_type("XLogRecordBlockHeader")
        .allowlist_type("XLogRecordBlockImageHeader")
        .allowlist_type("XLogRecordBlockCompressHeader")
        .allowlist_file("access/heapam_xlog.h")
        .allowlist_type("RelFileLocator")
        .allowlist_type("XLogSegNo")
        .allowlist_type("BlockNumber")
        .allowlist_type("ForkNumber")
        .allowlist_var("InvalidBlockNumber")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("coudl not write bindings!");
}
