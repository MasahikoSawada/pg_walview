# pg_walview 🔍

A modernized, interactive TUI alternative to `pg_waldump` for exploring PostgreSQL Write-Ahead Logs.

![pg_walview screenshot](./assets/demo.gif)

## Features

- Visual Transaction Tracking: Visually track `COMMIT`s and `ABORT`s with colored, dynamically drawn graph lines.
- Deep Drill-down: Detail split view. Inspect `XLogRecord` details, block-level information (`RelFileNode`), and Full Page Images (FPI) instantly.
- Record checksums are verified (`xl_crc`), and a record that fails is flagged in the list and in the detail pane.
- The segment a server is currently writing can be opened; reading stops at the point the WAL has been written up to.

### Build from source

```bash
git clone https://github.com/MasahikoSawada/pg_walview.git
cd pg_walview

# Standard build (relies on `pg_config` in your PATH)
cargo build --release

# If you have a custom PostgreSQL installation, specify the include path:
PG_INCLUDE_DIR=/path/to/pgsql/include/server cargo build --release
```

pg_walview reads WAL written by the PostgreSQL version whose headers it was
built against: the WAL page magic and the on-disk struct layouts are taken from
those headers at build time. Opening a segment from a different major version
reports the magic number mismatch rather than misreading it. It has been tested
against PostgreSQL 18 and later.

The WAL page size and segment size are read from the segment's own long page
header, so a cluster initdb'd with a non-default `--wal-segsize` works.

# Usage

Simply pass the path to a PostgreSQL WAL file as an argument:

```bash
pg_walview /path/to/pg_wal/000000010000000000000001
```

# Keybindings

| Key        | Action                                    |
|------------|-------------------------------------------|
| `j` / `↓` | Move selection down (Next record)         |
| `k` / `↑` | Move selection up (Previous record)       |
| `g`        | Jump to the first record                  |
| `G`        | Jump to the last record                   |
| `s`        | Jump to next record with the same XID     |
| `r`        | Jump to previous record with the same XID |
| `Space` / `PageDown` | Jump forward (Page Down)            |
| `-` / `PageUp`       | Jump backward (Page Up)             |
| `Tab`                | Switch Pane                         |
| `q`                  | Quit the application               |

In the DETAILS pane:

| Key        | Action                                    |
|------------|-------------------------------------------|
| `↑` / `↓` | Move the cursor between items              |
| `Enter`    | Expand / collapse the item under the cursor |

In the HEX DUMP pane:

| Key        | Action                                    |
|------------|-------------------------------------------|
| `↑` / `↓` | Scroll one line                            |
| `Space` / `PageDown` | Scroll down a page               |
| `b` / `PageUp`       | Scroll up a page                 |


# License

pg_walview is released under the [MIT License](LICENSE). PostgreSQL header files included via `bindgen` at build time are covered by the [PostgreSQL License](https://www.postgresql.org/about/licence/).
