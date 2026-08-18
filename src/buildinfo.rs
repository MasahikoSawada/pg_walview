//! What PostgreSQL this copy of pg_walview was built against.
//!
//! Both the struct layouts and the WAL page magic come from the server headers
//! at build time, so a binary only reads WAL whose page magic matches.

use crate::bindings;

/// pg_walview's own version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `PG_VERSION_NUM` of the server headers, e.g. 180004 or 200000.
pub const PG_VERSION_NUM: u32 = bindings::PG_VERSION_NUM;

/// Turn one of bindgen's NUL-terminated byte-string constants into a `str`.
fn from_c_str(bytes: &'static [u8]) -> &'static str {
    let end = bytes.len().saturating_sub(1);
    std::str::from_utf8(&bytes[..end]).unwrap_or("unknown")
}

/// Full server version the headers came from, e.g. "18.4" or "20devel".
pub fn pg_version() -> &'static str {
    from_c_str(bindings::PG_VERSION)
}

/// Major version only, e.g. "18" or "20".
pub fn pg_major_version() -> &'static str {
    from_c_str(bindings::PG_MAJORVERSION)
}

/// The WAL page magic this build accepts.  This, not the version number, is
/// what decides whether a segment can be read: it changes whenever the WAL
/// format changes, which happens during a development cycle too.
pub fn xlog_page_magic() -> u16 {
    bindings::XLOG_PAGE_MAGIC as u16
}

/// One line naming what this build reads, for --help and --version.
pub fn build_line() -> String {
    format!(
        "Built for PostgreSQL {} (WAL page magic 0x{:04X})",
        pg_version(),
        xlog_page_magic()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pg_version_comes_from_the_headers_we_built_against() {
        // "18.4", "20devel", ... -- never empty, never NUL-terminated.
        let v = pg_version();
        assert!(!v.is_empty());
        assert!(!v.contains('\0'), "{:?}", v);
        assert!(
            v.starts_with(pg_major_version()),
            "{:?} vs {:?}",
            v,
            pg_major_version()
        );

        let major: u32 = pg_major_version()
            .parse()
            .expect("major version is a number");
        assert_eq!(major, PG_VERSION_NUM / 10_000);
    }

    #[test]
    fn the_page_magic_is_the_one_the_reader_checks() {
        assert_eq!(
            xlog_page_magic() as u32,
            crate::bindings::XLOG_PAGE_MAGIC,
            "the reader compares against this exact value"
        );
        // Every WAL magic PostgreSQL has ever used is 0xD0xx or 0xD1xx.
        assert_eq!(xlog_page_magic() & 0xFF00, 0xD100);
    }

    #[test]
    fn the_build_line_names_both_the_version_and_the_magic() {
        let line = build_line();
        assert!(line.contains(pg_version()), "{}", line);
        assert!(
            line.contains(&format!("{:04X}", xlog_page_magic())),
            "{}",
            line
        );
    }
}
