# pg_walview 🔍

A modernized, interactive TUI alternative to `pg_waldump` for exploring PostgreSQL Write-Ahead Logs.

![pg_walview screenshot](./assets/demo.gif)

## Features

- Visual Transaction Tracking: Visually track `COMMIT`s and `ABORT`s with colored, dynamically drawn graph lines.
- Deep Drill-down: Detail split view. Inspect `XLogRecord` details, block-level information (`RelFileNode`), and Full Page Images (FPI) instantly.

### Build from source

```bash
git clone https://github.com/MasahikoSawada/pg_walview.git
cd pg_walview

# Standard build (relies on `pg_config` in your PATH)
cargo build --release

# If you have a custom PostgreSQL installation, specify the include path:
PG_INCLUDE_DIR=/path/to/pgsql/include/server cargo build --release
```

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


# License

pg_walview is released under the [MIT License](LICENSE). PostgreSQL header files included via `bindgen` at build time are covered by the [PostgreSQL License](https://www.postgresql.org/about/licence/).
