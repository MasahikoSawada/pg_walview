use anyhow::{Context, Result};
use std::env;
use std::io;
use std::ops::Range;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect, Spacing},
    style::Style,
    symbols::merge::MergeStrategy,
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState, Widget},
};

mod theme;

use pg_walview::bindings::*;
use pg_walview::buildinfo;
use pg_walview::rmgr::RmgrId;
use pg_walview::walmain::{describe_block_data, describe_main_data, describe_record};
use pg_walview::walmisc::*;
use pg_walview::walreader::*;
use pg_walview::xactdesc::XactOp;

#[derive(Debug, PartialEq, Clone)]
enum FocusPane {
    RecordList,
    Details,
    HexDump,
}

/// Items that can be navigated to in the DETAILS accordion.
#[derive(Debug, Clone, PartialEq)]
enum NavItem {
    Header,
    Block(usize),
    BlockFpi(usize),
    MainData,
}

/// Per-record accordion state for the DETAILS pane.
#[derive(Debug, Clone, Default)]
struct DetailTree {
    header_expanded: bool,
    /// (block_expanded, fpi_expanded) per block
    block_states: Vec<(bool, bool)>,
    main_expanded: bool,
    /// Index into the flat navigable list (see nav_items()).
    cursor: usize,
}

impl DetailTree {
    fn new_for(record: &WALRecordInfo) -> Self {
        DetailTree {
            header_expanded: false,
            block_states: vec![(false, false); record.nblocks_inuse()],
            main_expanded: false,
            cursor: 0,
        }
    }

    /// Build the flat list of navigable items given the current expand state.
    fn nav_items(&self, blocks: &[WALBlockData], has_main: bool) -> Vec<NavItem> {
        let mut items = vec![NavItem::Header];
        for i in 0..self.block_states.len() {
            items.push(NavItem::Block(i));
            if self.block_states[i].0 && i < blocks.len() && blocks[i].image.is_some() {
                items.push(NavItem::BlockFpi(i));
            }
        }
        if has_main {
            items.push(NavItem::MainData);
        }
        items
    }

    /// Which parts of the record the cursor is on, so the hex dump can pick
    /// out exactly those bytes.  A block item covers everything belonging to
    /// that block: a block whose only payload is a full-page image would
    /// otherwise light up nothing at all.
    fn focused_parts(&self, record: &WALRecordInfo) -> Vec<RecordPart> {
        let nav = self.nav_items(&record.blocks, record.has_main_data());
        match nav.get(self.cursor) {
            Some(NavItem::Header) => vec![RecordPart::Header],
            Some(NavItem::Block(i)) => vec![RecordPart::Fpi(*i), RecordPart::BlockData(*i)],
            Some(NavItem::BlockFpi(i)) => vec![RecordPart::Fpi(*i)],
            Some(NavItem::MainData) => vec![RecordPart::MainData],
            None => Vec::new(),
        }
    }

    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_down(&mut self, blocks: &[WALBlockData], has_main: bool) {
        let nav = self.nav_items(blocks, has_main);
        if self.cursor + 1 < nav.len() {
            self.cursor += 1;
        }
    }

    fn toggle(&mut self, blocks: &[WALBlockData], has_main: bool) {
        let nav = self.nav_items(blocks, has_main);
        if self.cursor >= nav.len() {
            return;
        }
        match nav[self.cursor] {
            NavItem::Header => self.header_expanded = !self.header_expanded,
            NavItem::Block(i) => {
                self.block_states[i].0 = !self.block_states[i].0;
                // collapse FPI when parent block collapses
                if !self.block_states[i].0 {
                    self.block_states[i].1 = false;
                }
            }
            NavItem::BlockFpi(i) => self.block_states[i].1 = !self.block_states[i].1,
            NavItem::MainData => self.main_expanded = !self.main_expanded,
        }
    }
}

/// Repositioning of the hex pane requested by a key or by a change of
/// selection, resolved at render time when the pane geometry is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HexJump {
    None,
    ToSelection,
    ToEnd,
}

/// The raw segment behind the hex dump pane.
#[derive(Debug, Default)]
struct Segment {
    bytes: Vec<u8>,
    /// LSN of byte 0.
    start_lsn: XLogRecPtr,
    xlog_blcksz: usize,
}

#[derive(Debug)]
pub struct App {
    records: Vec<WALRecordInfo>,
    segment: Segment,
    current_file: PathBuf,
    /// Why the reader stopped short of the end of the segment, if it did.
    stop_reason: Option<String>,
    state: TableState,
    /// First visible row of the record list.  Scrolling is managed here rather
    /// than by the Table widget so that only the visible rows are built.
    list_offset: usize,
    /// Index range of the records sharing the selected XID, recomputed only
    /// when the selection changes.
    xid_range: Option<(usize, usize)>,
    focus: FocusPane,
    detail_tree: DetailTree,
    detail_scroll: usize,
    hex_scroll: usize,
    hex_jump: HexJump,
    exit: bool,
}

const PAGE_JUMP_SIZE: usize = 20;
/// Lines of context kept above the selected record in the hex pane.
const HEX_JUMP_MARGIN: usize = 2;

impl App {
    pub fn new(path: &str) -> Result<Self> {
        let mut reader =
            WALReader::open(path).with_context(|| format!("could not read WAL file '{}'", path))?;
        let mut records: Vec<WALRecordInfo> = Vec::new();
        let mut stop_reason = None;

        loop {
            match reader.load_next_record() {
                Ok(Some(record)) => records.push(record),
                Ok(None) => break,
                // Anything unreadable ends the scan, but the records already
                // decoded are still worth showing; report where it stopped.
                Err(e) => {
                    stop_reason = Some(e.to_string());
                    break;
                }
            }
        }

        // A file we cannot get a single record out of is a hard error; there
        // would be nothing to display.
        if records.is_empty()
            && let Some(reason) = stop_reason
        {
            return Err(anyhow::anyhow!(reason))
                .with_context(|| format!("could not read WAL file '{}'", path));
        }

        // The hex pane dumps the whole segment, not just the selected
        // record, so keep the file itself around.
        let segment = Segment {
            bytes: std::fs::read(path)
                .with_context(|| format!("could not read WAL file '{}'", path))?,
            start_lsn: reader.segment_start_lsn(),
            xlog_blcksz: reader.xlog_blcksz(),
        };

        Ok(Self::with_records(
            PathBuf::from(path),
            records,
            stop_reason,
            segment,
        ))
    }

    fn with_records(
        current_file: PathBuf,
        records: Vec<WALRecordInfo>,
        stop_reason: Option<String>,
        segment: Segment,
    ) -> Self {
        let mut state = TableState::default();
        if !records.is_empty() {
            state.select(Some(0));
        }

        let mut app = App {
            records,
            segment,
            state,
            current_file,
            stop_reason,
            list_offset: 0,
            xid_range: None,
            focus: FocusPane::RecordList,
            detail_tree: DetailTree::default(),
            detail_scroll: 0,
            hex_scroll: 0,
            hex_jump: HexJump::ToSelection,
            exit: false,
        };
        app.on_record_change();

        app
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());
    }

    fn handle_events(&mut self) -> io::Result<()> {
        if let Event::Key(key_event) = event::read()?
            && key_event.kind == KeyEventKind::Press
        {
            self.handle_key_event(key_event);
        }
        Ok(())
    }

    // main key handler function.
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Tab => self.cycle_focus(),
            _ => match self.focus {
                FocusPane::RecordList => self.handle_record_list_key(key_event),
                FocusPane::Details => self.handle_details_key(key_event),
                FocusPane::HexDump => self.handle_hex_dump_key(key_event),
            },
        }
    }

    // switch focus to the next pane.
    fn cycle_focus(&mut self) {
        self.focus = match self.focus {
            FocusPane::RecordList => FocusPane::Details,
            FocusPane::Details => FocusPane::HexDump,
            FocusPane::HexDump => FocusPane::RecordList,
        };
    }

    fn handle_record_list_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_record_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_record_down(),
            KeyCode::Char('g') => self.move_top(),
            KeyCode::Char('G') => self.move_bottom(),
            KeyCode::Char('s') => self.move_next_same_xid(),
            KeyCode::Char('r') => self.move_prev_same_xid(),
            KeyCode::PageUp | KeyCode::Char('-') => self.page_up(),
            KeyCode::PageDown | KeyCode::Char(' ') => self.page_down(),
            _ => {}
        }
    }

    fn handle_details_key(&mut self, key_event: KeyEvent) {
        let Some(record) = self.selected_record() else {
            return;
        };
        let blocks = record.blocks.clone();
        let has_main = record.has_main_data();
        match key_event.code {
            KeyCode::Up => self.detail_tree.move_up(),
            KeyCode::Down => self.detail_tree.move_down(&blocks, has_main),
            KeyCode::Enter => self.detail_tree.toggle(&blocks, has_main),
            _ => {}
        }
    }

    fn handle_hex_dump_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => self.hex_scroll = self.hex_scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.hex_scroll = self.hex_scroll.saturating_add(1)
            }
            KeyCode::PageUp | KeyCode::Char('-') | KeyCode::Char('b') => {
                self.hex_scroll = self.hex_scroll.saturating_sub(PAGE_JUMP_SIZE)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.hex_scroll = self.hex_scroll.saturating_add(PAGE_JUMP_SIZE)
            }
            // The dump covers the whole segment, so getting back to the
            // selected record needs its own key.
            KeyCode::Char('g') => self.hex_jump = HexJump::ToSelection,
            KeyCode::Char('G') => self.hex_jump = HexJump::ToEnd,
            _ => {}
        }
    }

    fn selected_record(&self) -> Option<&WALRecordInfo> {
        self.records.get(self.state.selected()?)
    }

    fn select(&mut self, idx: usize) {
        if self.records.is_empty() {
            return;
        }
        self.state.select(Some(idx.min(self.records.len() - 1)));
        self.on_record_change();
    }

    fn move_next_same_xid(&mut self) {
        let Some(idx) = self.state.selected() else {
            return;
        };
        let Some(selected_xid) = self.normal_xid_at(idx) else {
            return;
        };

        if let Some(i) =
            (idx + 1..self.records.len()).find(|&i| self.records[i].xlrec.xl_xid == selected_xid)
        {
            self.select(i);
        }
    }

    fn move_prev_same_xid(&mut self) {
        let Some(idx) = self.state.selected() else {
            return;
        };
        let Some(selected_xid) = self.normal_xid_at(idx) else {
            return;
        };

        if let Some(i) = (0..idx)
            .rev()
            .find(|&i| self.records[i].xlrec.xl_xid == selected_xid)
        {
            self.select(i);
        }
    }

    /// The XID of the record at `idx`, if it belongs to a real transaction.
    fn normal_xid_at(&self, idx: usize) -> Option<TransactionId> {
        let xid = self.records.get(idx)?.xlrec.xl_xid;
        (xid >= FIRST_NORMAL_TRANSACTION_ID).then_some(xid)
    }

    fn move_record_down(&mut self) {
        let i = self.state.selected().map_or(0, |i| i + 1);
        self.select(i);
    }

    fn move_record_up(&mut self) {
        let i = self.state.selected().map_or(0, |i| i.saturating_sub(1));
        self.select(i);
    }

    fn move_top(&mut self) {
        self.select(0);
    }

    fn move_bottom(&mut self) {
        self.select(self.records.len().saturating_sub(1));
    }

    fn page_up(&mut self) {
        let current = self.state.selected().unwrap_or(0);
        self.select(current.saturating_sub(PAGE_JUMP_SIZE));
    }

    fn page_down(&mut self) {
        let current = self.state.selected().unwrap_or(0);
        self.select(current.saturating_add(PAGE_JUMP_SIZE));
    }

    fn on_record_change(&mut self) {
        self.detail_scroll = 0;
        self.hex_jump = HexJump::ToSelection;
        self.xid_range = None;

        let Some(idx) = self.state.selected() else {
            self.detail_tree = DetailTree::default();
            return;
        };
        let Some(record) = self.records.get(idx) else {
            return;
        };
        self.detail_tree = DetailTree::new_for(record);

        // Scanning every record for the extent of this transaction is O(n),
        // so do it once per selection change rather than once per frame.
        let xid = record.xlrec.xl_xid;
        self.xid_range = if xid == 0 {
            Some((0, 0))
        } else {
            let first = self.records.iter().position(|r| r.xlrec.xl_xid == xid);
            let last = self.records.iter().rposition(|r| r.xlrec.xl_xid == xid);
            first.zip(last)
        };
    }
}

/// An accordion heading, coloured by the part of the record it stands for so
/// that it matches those bytes in the hex dump.
fn make_item_line(text: String, part: RecordPart, is_cursor: bool) -> Line<'static> {
    let mut style = theme::record_part(part);
    if is_cursor {
        style = style.patch(theme::cursor());
    }
    Line::from(Span::styled(text, style))
}

/// Split a field line at its first colon so the key can recede and the value
/// read at full strength.  Only the first, because a value can have colons of
/// its own -- a timestamp does.  Splitting here rather than at the source
/// leaves the twenty per-rmgr description modules alone.
fn detail_field_line(text: String) -> Line<'static> {
    match text.find(':') {
        Some(i) => Line::from(vec![
            Span::styled(text[..=i].to_string(), theme::detail_key()),
            Span::styled(text[i + 1..].to_string(), theme::detail_value()),
        ]),
        None => Line::from(Span::styled(text, theme::detail_value())),
    }
}

/// Build the lines for the DETAILS accordion and return (lines, cursor_line_index).
/// When `show_cursor` is false all items render without highlight (auto-expand mode).
fn build_detail_lines(
    tree: &DetailTree,
    record: &WALRecordInfo,
    show_cursor: bool,
) -> (Vec<Line<'static>>, usize) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cursor_line: usize = 0;

    let blocks = record.blocks.as_slice();
    let has_main = record.has_main_data();
    let nav = tree.nav_items(blocks, has_main);

    for (nav_idx, nav_item) in nav.iter().enumerate() {
        let is_at_cursor = nav_idx == tree.cursor;
        let is_cursor = show_cursor && is_at_cursor;

        match nav_item {
            NavItem::Header => {
                let arrow = if tree.header_expanded { "▼" } else { "▶" };
                let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
                let summary = format!(
                    "{} Header  XID:{}  tot_len:{}  RMID:{}{}",
                    arrow,
                    record.xlrec.xl_xid,
                    record.xlrec.xl_tot_len,
                    rmgr,
                    if record.crc_ok { "" } else { "  [CRC ERROR]" }
                );
                if is_at_cursor {
                    cursor_line = lines.len();
                }
                lines.push(make_item_line(summary, RecordPart::Header, is_cursor));

                if tree.header_expanded {
                    lines.push(Line::from(vec![
                        Span::styled("  LSN:      ", theme::detail_key()),
                        Span::styled(lsn_format(record.lsn), theme::lsn()),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("  prev LSN: ", theme::detail_key()),
                        Span::styled(lsn_format(record.xlrec.xl_prev), theme::lsn()),
                    ]));
                    lines.push(detail_field_line(format!(
                        "  XID:      {}",
                        record.xlrec.xl_xid
                    )));
                    lines.push(detail_field_line(format!(
                        "  RMID:     {} ({})",
                        rmgr, record.xlrec.xl_rmid
                    )));
                    lines.push(detail_field_line(format!(
                        "  info:     0x{:02X}",
                        record.xlrec.xl_info
                    )));
                    lines.push(detail_field_line(format!(
                        "  tot_len:  {} bytes",
                        record.xlrec.xl_tot_len
                    )));
                    lines.push(Line::from(vec![
                        Span::styled("  crc:", theme::detail_key()),
                        Span::styled(
                            format!(
                                "      0x{:08X} ({})",
                                record.xlrec.xl_crc,
                                if record.crc_ok { "ok" } else { "MISMATCH" }
                            ),
                            if record.crc_ok {
                                theme::crc_ok()
                            } else {
                                theme::crc_bad()
                            },
                        ),
                    ]));
                    lines.push(detail_field_line(format!(
                        "  top_xid:  {}",
                        record.top_xid.map_or("-".to_string(), |x| x.to_string())
                    )));
                    lines.push(detail_field_line(format!(
                        "  origin:   {}",
                        record.origin.map_or("-".to_string(), |o| o.to_string())
                    )));
                }
            }

            NavItem::Block(i) => {
                let block = &record.blocks[*i];
                let (block_expanded, _) = tree.block_states[*i];
                let arrow = if block_expanded { "▼" } else { "▶" };
                let summary = format!(
                    "{} Block #{}  {}  {}  blk:{}  {}",
                    arrow,
                    block.block_id,
                    format_rel(&block.rlocator),
                    fork_name(block.forknum),
                    block.blocknum,
                    block.flags_str()
                );
                if is_at_cursor {
                    cursor_line = lines.len();
                }
                lines.push(make_item_line(
                    summary,
                    RecordPart::BlockData(*i),
                    is_cursor,
                ));

                if block_expanded {
                    lines.push(detail_field_line(format!(
                        "  rel:    {}",
                        format_rel(&block.rlocator)
                    )));
                    lines.push(detail_field_line(format!(
                        "  fork:   {}",
                        fork_name(block.forknum)
                    )));
                    lines.push(detail_field_line(format!("  blkno:  {}", block.blocknum)));
                    lines.push(Line::from(vec![
                        Span::styled("  flags:", theme::detail_key()),
                        Span::styled(format!("  {}", block.flags_str()), theme::flags()),
                    ]));
                    if block.flags & BKPBLOCK_HAS_DATA != 0 {
                        lines.push(detail_field_line(format!(
                            "  data:   {} bytes",
                            block.data_len
                        )));
                        // Show block-specific parsed fields
                        for s in describe_block_data(record, *i) {
                            lines.push(detail_field_line(s));
                        }
                    }
                }
            }

            NavItem::BlockFpi(i) => {
                let block = &record.blocks[*i];
                if let Some(image) = &block.image {
                    let (_, fpi_expanded) = tree.block_states[*i];
                    let arrow = if fpi_expanded { "▼" } else { "▶" };
                    let summary = format!(
                        "  {} FPI  ({} bytes, {})",
                        arrow,
                        image.bimg_len,
                        image.compression_str()
                    );
                    if is_at_cursor {
                        cursor_line = lines.len();
                    }
                    lines.push(make_item_line(summary, RecordPart::Fpi(*i), is_cursor));

                    if fpi_expanded {
                        lines.push(detail_field_line(format!(
                            "    length:      {} bytes",
                            image.bimg_len
                        )));
                        lines.push(detail_field_line(format!(
                            "    hole_offset: {}",
                            image.hole_offset
                        )));
                        lines.push(detail_field_line(format!(
                            "    hole_len:    {}",
                            image.hole_len
                        )));
                        lines.push(detail_field_line(format!(
                            "    compression: {}",
                            image.compression_str()
                        )));
                        lines.push(detail_field_line(format!(
                            "    apply:       {}",
                            if image.apply_image { "yes" } else { "no" }
                        )));
                    }
                }
            }

            NavItem::MainData => {
                let arrow = if tree.main_expanded { "▼" } else { "▶" };
                let summary = format!("{} Main Data  ({} bytes)", arrow, record.main_len);
                if is_at_cursor {
                    cursor_line = lines.len();
                }
                lines.push(make_item_line(summary, RecordPart::MainData, is_cursor));

                if tree.main_expanded {
                    lines.push(detail_field_line(format!(
                        "  length:  {} bytes",
                        record.main_len
                    )));
                    for s in describe_main_data(record) {
                        lines.push(detail_field_line(s));
                    }
                }
            }
        }
    }

    (lines, cursor_line)
}

// ---------------------------------------------------------------------------
// HEX DUMP pane
// ---------------------------------------------------------------------------

/// What a byte of the segment is, which decides how it is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByteKind {
    /// Part of a WAL page header, which splits records that cross pages.
    PageHeader,
    /// Part of the record currently selected in the list, and which part.
    Record(RecordPart),
    /// A zero byte outside the selected record.  WAL is full of them, and
    /// dimming them is what lets the structure show through.
    Zero,
    /// Anything else.
    Plain,
}

/// Where the parts of the selected record sit in the file, and which part the
/// DETAILS cursor is on.
struct RecordOverlay {
    /// File ranges, in ascending order.
    parts: Vec<(Range<usize>, RecordPart)>,
    /// The parts the DETAILS cursor is on.  Empty when the accordion is in
    /// auto-expand mode and shows no cursor.
    focus: Vec<RecordPart>,
}

impl RecordOverlay {
    fn empty() -> Self {
        RecordOverlay {
            parts: Vec::new(),
            focus: Vec::new(),
        }
    }

    /// The parts address `raw`; the dump addresses the file, so each one is
    /// mapped over, and a part split by a page header becomes two ranges.
    fn of(record: &WALRecordInfo, focus: Vec<RecordPart>) -> Self {
        let mut parts = Vec::new();
        for (raw, part) in record.parts() {
            for range in record.file_ranges_of(raw) {
                parts.push((range, part));
            }
        }
        parts.sort_by_key(|(r, _)| r.start);
        RecordOverlay { parts, focus }
    }

    fn part_at(&self, offset: usize) -> Option<RecordPart> {
        // A record has a handful of parts, so a scan beats an index.
        self.parts
            .iter()
            .find(|(r, _)| r.contains(&offset))
            .map(|(_, p)| *p)
    }

    fn first_byte(&self) -> Option<usize> {
        self.parts.first().map(|(r, _)| r.start)
    }
}

/// Width of the "<lsn> <offset>: " prefix.
const HEX_PREFIX_WIDTH: usize = 10 + 1 + 8 + 2;
/// Bytes are printed in groups of four, separated by an extra space.
const HEX_GROUP: usize = 4;

/// Width of the hex column for `n` bytes per line.
fn hex_column_width(n: usize) -> usize {
    let groups = n.div_ceil(HEX_GROUP);
    n * 3 - 1 + groups.saturating_sub(1)
}

/// Widest layout that fits the pane.  Falling back keeps the dump readable on
/// a narrow terminal instead of cutting the line off.
fn hex_bytes_per_line(inner_width: u16) -> usize {
    let width = inner_width as usize;
    [16usize, 8]
        .into_iter()
        .find(|&n| HEX_PREFIX_WIDTH + hex_column_width(n) + 2 + n <= width)
        .unwrap_or(4)
}

/// Renders the whole WAL segment as a hex dump, marking the bytes of the
/// selected record and the page headers.
struct HexDump<'a> {
    bytes: &'a [u8],
    /// LSN of byte 0 of the segment.
    start_lsn: XLogRecPtr,
    xlog_blcksz: usize,
    bytes_per_line: usize,
}

impl HexDump<'_> {
    fn total_lines(&self) -> usize {
        self.bytes.len().div_ceil(self.bytes_per_line)
    }

    /// Size of the page header at the start of the page holding `offset`.
    /// Only the first page of a segment carries a long header, but rather than
    /// assume that, read the flag out of the page itself.
    fn page_header_size(&self, offset: usize) -> usize {
        let page_start = offset - offset % self.xlog_blcksz;
        let info = self
            .bytes
            .get(page_start + 2..page_start + 4)
            .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
            .unwrap_or(0);
        if info & XLP_LONG_HEADER != 0 { 40 } else { 24 }
    }

    fn classify(&self, offset: usize, overlay: &RecordOverlay) -> ByteKind {
        // A page header is never record content, whatever the ranges say.
        if offset % self.xlog_blcksz < self.page_header_size(offset) {
            return ByteKind::PageHeader;
        }
        if let Some(part) = overlay.part_at(offset) {
            return ByteKind::Record(part);
        }
        if self.bytes.get(offset) == Some(&0) {
            return ByteKind::Zero;
        }
        ByteKind::Plain
    }

    fn style_at(&self, offset: usize, overlay: &RecordOverlay) -> Style {
        match self.classify(offset, overlay) {
            ByteKind::PageHeader | ByteKind::Zero => theme::dim(),
            ByteKind::Plain => Style::default(),
            ByteKind::Record(part) => {
                let style = theme::record_part(part).add_modifier(theme::selected_record());
                if overlay.focus.contains(&part) {
                    style.patch(theme::cursor())
                } else {
                    style
                }
            }
        }
    }

    /// The ASCII column dims its placeholders: a run of dots carries nothing,
    /// and letting it recede makes the printable stretches stand out.
    fn ascii_style_at(&self, offset: usize, overlay: &RecordOverlay) -> Style {
        let printable = self
            .bytes
            .get(offset)
            .is_some_and(|b| (0x20..0x7f).contains(b));
        if printable {
            self.style_at(offset, overlay)
        } else {
            match self.classify(offset, overlay) {
                ByteKind::Record(_) => self.style_at(offset, overlay).patch(theme::dim()),
                _ => theme::dim(),
            }
        }
    }

    /// One line of the dump, with runs of same-styled bytes merged into a span.
    fn line(&self, line_idx: usize, overlay: &RecordOverlay) -> Line<'static> {
        let start = line_idx * self.bytes_per_line;
        let end = (start + self.bytes_per_line).min(self.bytes.len());

        let mut spans = vec![Span::styled(
            format!(
                "{:<10} {:08x}: ",
                lsn_format(self.start_lsn + start as u64),
                start
            ),
            theme::hex_addr(),
        )];

        let flush = |spans: &mut Vec<Span<'static>>, text: &mut String, style: Style| {
            if !text.is_empty() {
                spans.push(Span::styled(std::mem::take(text), style));
            }
        };

        // Hex column, split into runs so a record that starts mid-line is
        // highlighted from exactly the right byte.
        let mut hex = String::new();
        let mut hex_width = 0usize;
        let mut run = self.style_at(start, overlay);
        for offset in start..end {
            let style = self.style_at(offset, overlay);
            if style != run {
                hex_width += hex.chars().count();
                flush(&mut spans, &mut hex, run);
                run = style;
            }
            let col = offset - start;
            if col > 0 {
                hex.push(' ');
                if col.is_multiple_of(HEX_GROUP) {
                    hex.push(' ');
                }
            }
            hex.push_str(&format!("{:02x}", self.bytes[offset]));
        }
        hex_width += hex.chars().count();
        flush(&mut spans, &mut hex, run);

        // Pad a short final line so the ASCII column stays aligned.
        let pad = hex_column_width(self.bytes_per_line).saturating_sub(hex_width) + 2;
        spans.push(Span::raw(" ".repeat(pad)));

        // ASCII column, highlighted the same way.
        let mut ascii = String::new();
        let mut run = self.ascii_style_at(start, overlay);
        for offset in start..end {
            let style = self.ascii_style_at(offset, overlay);
            if style != run {
                flush(&mut spans, &mut ascii, run);
                run = style;
            }
            let b = self.bytes[offset];
            ascii.push(if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            });
        }
        flush(&mut spans, &mut ascii, run);

        Line::from(spans)
    }
}

/// Build a DetailTree with everything expanded (for auto-expand check).
fn fully_expanded_tree(record: &WALRecordInfo) -> DetailTree {
    let block_states = record
        .blocks
        .iter()
        .map(|b| (true, b.image.is_some()))
        .collect();
    DetailTree {
        header_expanded: true,
        block_states,
        main_expanded: true,
        cursor: 0,
    }
}

impl App {
    /// Left pane: the record list.  Only the rows that fit on screen are
    /// built, so the cost per frame does not grow with the size of the
    /// segment.
    fn render_record_list(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        // Borders take 2 rows, the header row and its bottom margin 2 more.
        let visible = (area.height as usize).saturating_sub(4);

        let selected = self.state.selected().unwrap_or(0);
        if visible > 0 {
            if selected < self.list_offset {
                self.list_offset = selected;
            } else if selected >= self.list_offset + visible {
                self.list_offset = selected + 1 - visible;
            }
            self.list_offset = self
                .list_offset
                .min(self.records.len().saturating_sub(visible));
        }

        let end = (self.list_offset + visible).min(self.records.len());
        let window = self.list_offset..end;

        let selected_xid = self.selected_record().map(|r| r.xlrec.xl_xid);

        // The colour of the transaction's graph line is decided by how the
        // transaction ended, so it is looked up once for the whole pane.
        let graph_style = match self.xid_range {
            Some((_, last)) if self.records.get(last).is_some() => {
                let last_rec = &self.records[last];
                if RmgrId::from_u8(last_rec.xlrec.xl_rmid) == RmgrId::Xact {
                    match XactOp::from_xl_info(last_rec.xlrec.xl_info) {
                        XactOp::Commit | XactOp::CommitPrepared => theme::graph_committed(),
                        XactOp::Abort | XactOp::AbortPrepared => theme::graph_aborted(),
                        _ => theme::graph_open(),
                    }
                } else {
                    theme::graph_none()
                }
            }
            _ => theme::graph_none(),
        };

        let rows: Vec<Row> = window
            .clone()
            .map(|i| {
                let record = &self.records[i];
                let is_selected = i == selected;

                let mut prefix = "    ";
                if let (Some(xid), Some((first, last))) = (selected_xid, self.xid_range) {
                    if xid != 0 && record.xlrec.xl_xid == xid {
                        if first == last {
                            prefix = "●━━ ";
                        } else if i == first {
                            prefix = "┏━━ ";
                        } else if i == last {
                            prefix = "┗━━ ";
                        } else {
                            prefix = "┣━━ ";
                        }
                    } else if i > first && i < last {
                        prefix = "┃   ";
                    }
                }

                let combined_line = Line::from(vec![
                    Span::styled(prefix, graph_style),
                    Span::styled(lsn_format(record.lsn), theme::lsn()),
                ]);

                let desc = describe_record(record);
                let desc_cell = if record.crc_ok {
                    Cell::from(desc)
                } else {
                    Cell::from(Span::styled(
                        format!("{} [CRC ERROR]", desc),
                        theme::crc_bad(),
                    ))
                };

                // Full-page images are what make a segment large, so mark the
                // records carrying one.  A character, not just a colour.
                let has_fpi = record.blocks.iter().any(|b| b.image.is_some());
                let fpi_cell = if has_fpi {
                    Cell::from(Span::styled("*", theme::fpi_marker()))
                } else {
                    Cell::from(" ")
                };

                let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
                let mut row = Row::new(vec![
                    Cell::from(combined_line),
                    Cell::from(record.xlrec.xl_xid.to_string()),
                    Cell::from(record.xlrec.xl_tot_len.to_string()),
                    fpi_cell,
                    Cell::from(Span::styled(rmgr.to_string(), theme::rmgr(rmgr))),
                    desc_cell,
                ]);

                if is_selected {
                    row = row.style(theme::selected_row());
                } else if let Some(xid) = selected_xid
                    && xid != 0
                    && record.xlrec.xl_xid == xid
                {
                    row = row.style(theme::xid_group_row());
                }

                row
            })
            .collect();

        let header = Row::new(vec!["       LSN", "XID", "LEN", "F", "RMID", "DESC"])
            .style(theme::list_header())
            .bottom_margin(1);

        // The fixed widths used to add up to more than the pane, so the
        // columns were squeezed and the LSN -- the one thing you navigate by
        // -- got cut off mid-way.  The LSN column needs 3 for the highlight
        // symbol, 4 for the transaction graph and 10 for the LSN itself; DESC
        // takes whatever is left over.
        let widths = [
            Constraint::Length(17),
            Constraint::Length(9),
            Constraint::Length(8),
            Constraint::Length(1),
            Constraint::Length(12),
            Constraint::Fill(1),
        ];

        let percentage = if self.records.len() <= 1 {
            100
        } else {
            (selected * 100) / (self.records.len() - 1)
        };

        let list_active = self.focus == FocusPane::RecordList;
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border(list_active))
                    .title(format!("WAL records ({})", self.records.len()))
                    .title_top(
                        Line::from(format!(
                            "{} ({}%) ",
                            self.current_file
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "Unknown".to_string()),
                            percentage
                        ))
                        .right_aligned(),
                    ),
            )
            .highlight_symbol(">> ");

        // The rows handed to the table are already the visible window, so the
        // widget's own state only has to point at the row inside it.
        let mut window_state = TableState::default();
        if !self.records.is_empty() && window.contains(&selected) {
            window_state.select(Some(selected - self.list_offset));
        }

        StatefulWidget::render(table, area, buf, &mut window_state);
    }

    /// Returns the record part the cursor is on, for the hex dump to pick out.
    /// In auto-expand mode there is no cursor and so nothing is picked out.
    fn render_details(&mut self, area: Rect, buf: &mut Buffer) -> Vec<RecordPart> {
        if area.width < 2 || area.height < 2 {
            return Vec::new();
        }

        let detail_active = self.focus == FocusPane::Details;
        let inner_h = area.height.saturating_sub(2) as usize;
        let mut focus = Vec::new();

        let (detail_lines, cursor_line) = match self.records.get(self.state.selected().unwrap_or(0))
        {
            Some(record) => {
                // If all items fit when fully expanded, auto-expand and skip
                // scrolling.
                let full_tree = fully_expanded_tree(record);
                let (full_lines, _) = build_detail_lines(&full_tree, record, false);
                if full_lines.len() <= inner_h {
                    self.detail_scroll = 0;
                    (full_lines, 0)
                } else {
                    focus = self.detail_tree.focused_parts(record);
                    build_detail_lines(&self.detail_tree, record, detail_active)
                }
            }
            None => (vec![Line::from("No record selected")], 0),
        };

        // Keep cursor in view (only reached when not auto-expanded)
        if inner_h > 0 {
            if cursor_line < self.detail_scroll {
                self.detail_scroll = cursor_line;
            } else if cursor_line >= self.detail_scroll + inner_h {
                self.detail_scroll = cursor_line.saturating_sub(inner_h - 1);
            }
        }

        Paragraph::new(detail_lines)
            .block(
                Block::default()
                    .title("DETAILS")
                    .borders(Borders::ALL)
                    .border_style(theme::border(detail_active))
                    .merge_borders(MergeStrategy::Exact),
            )
            .scroll((self.detail_scroll as u16, 0))
            .render(area, buf);

        focus
    }

    fn render_hex_dump(&mut self, area: Rect, buf: &mut Buffer, focus: Vec<RecordPart>) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let hex_active = self.focus == FocusPane::HexDump;
        let visible = area.height.saturating_sub(2) as usize;

        let dump = HexDump {
            bytes: &self.segment.bytes,
            start_lsn: self.segment.start_lsn,
            xlog_blcksz: self.segment.xlog_blcksz.max(1),
            bytes_per_line: hex_bytes_per_line(area.width.saturating_sub(2)),
        };
        let total_lines = dump.total_lines();

        let selected = self.state.selected().and_then(|i| self.records.get(i));
        let record_lsn = selected.map_or(0, |r| r.lsn);
        let overlay = match selected {
            Some(r) => RecordOverlay::of(r, focus),
            None => RecordOverlay::empty(),
        };

        // Resolve a pending jump now that the geometry is known.
        match self.hex_jump {
            HexJump::ToSelection => {
                if let Some(first) = overlay.first_byte() {
                    // Leave a little context above the record.
                    self.hex_scroll = (first / dump.bytes_per_line).saturating_sub(HEX_JUMP_MARGIN);
                }
            }
            HexJump::ToEnd => self.hex_scroll = total_lines.saturating_sub(visible),
            HexJump::None => {}
        }
        self.hex_jump = HexJump::None;
        self.hex_scroll = self.hex_scroll.min(total_lines.saturating_sub(visible));

        // Only the lines on screen are built; a 16MB segment is a million of
        // them.
        let lines: Vec<Line> = (self.hex_scroll..(self.hex_scroll + visible).min(total_lines))
            .map(|i| dump.line(i, &overlay))
            .collect();

        let title = if overlay.parts.is_empty() {
            "HEX DUMP".to_string()
        } else {
            format!("HEX DUMP  (record {})", lsn_format(record_lsn))
        };

        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(theme::border(hex_active))
                    .merge_borders(MergeStrategy::Exact),
            )
            .render(area, buf);
    }

    fn render_status(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let line = match &self.stop_reason {
            Some(reason) => Line::from(Span::styled(
                format!(" stopped: {}", reason),
                theme::status_error(),
            )),
            None => Line::from(Span::styled(
                " q:quit  Tab:pane  j/k:move  g/G:top/bottom  s/r:next/prev same XID  Space/-:page",
                theme::status_hint(),
            )),
        };
        Paragraph::new(line).render(area, buf);
    }
}

/// Below this the three bordered panes have no room left, and the layout
/// arithmetic in the widgets underneath starts to underflow.
const MIN_WIDTH: u16 = 30;
const MIN_HEIGHT: u16 = 8;

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
            Paragraph::new("terminal too small").render(area, buf);
            return;
        }

        // One status line at the bottom, the panes above it.
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(area);

        // Horizontal split: 40% record list | 60% right panels
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .spacing(Spacing::Overlap(1))
            .split(outer[0]);

        // Vertical split of right side: 60% details | remaining hex dump
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Fill(1)])
            .spacing(Spacing::Overlap(1))
            .split(h_chunks[1]);

        self.render_record_list(h_chunks[0], buf);
        let focus = self.render_details(v_chunks[0], buf);
        self.render_hex_dump(v_chunks[1], buf, focus);
        self.render_status(outer[1], buf);
    }
}

// ---------------------------------------------------------------------------
// Command line
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Help,
    Version,
    Open(String),
}

/// Two flags do not justify a command-line parsing crate.
fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut path: Option<&str> = None;

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            // Asked for explicitly, so they win over anything else on the
            // line: --help has to work even when the rest is wrong.
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option '{}'", other));
            }
            other if path.is_none() => path = Some(other),
            _ => return Err("expected one WAL segment file".to_string()),
        }
    }

    match path {
        Some(p) => Ok(Command::Open(p.to_string())),
        None => Err("no WAL segment file given".to_string()),
    }
}

fn version_text() -> String {
    format!(
        "pg_walview {}\n{}",
        buildinfo::VERSION,
        buildinfo::build_line()
    )
}

fn help_text() -> String {
    format!(
        "\
{version}

Usage: pg_walview <WAL_SEGMENT_FILE>

An interactive viewer for PostgreSQL write-ahead log segments.  A segment can
only be read if its page magic matches the one above, which is taken from the
server headers this binary was built against.

Options:
  -h, --help     Print this help
  -V, --version  Print version information

Record list:
  j, Down        Next record
  k, Up          Previous record
  g, G           First / last record
  s, r           Next / previous record with the same XID
  Space, -       Page down / page up

DETAILS pane:
  Up, Down       Move the cursor between items
  Enter          Expand / collapse the item under the cursor

HEX DUMP pane:
  j, Down        Scroll down one line
  k, Up          Scroll up one line
  Space, -       Page down / page up
  g              Back to the selected record
  G              End of the segment

Anywhere:
  Tab            Switch pane
  q              Quit
",
        version = version_text()
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    let path = match parse_args(&args) {
        Ok(Command::Help) => {
            print!("{}", help_text());
            return Ok(());
        }
        Ok(Command::Version) => {
            println!("{}", version_text());
            return Ok(());
        }
        Ok(Command::Open(path)) => path,
        Err(message) => {
            eprintln!("pg_walview: {}", message);
            eprintln!("Try 'pg_walview --help' for more information.");
            std::process::exit(1);
        }
    };

    // Load before touching the terminal, so a failure here reports the real
    // reason on stderr instead of inside the alternate screen.
    let mut app = App::new(&path)?;

    ratatui::run(|terminal| app.run(terminal))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Modifier;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_single_path_is_the_segment_to_open() {
        assert_eq!(
            parse(&["pg_walview", "000000010000000000000001"]),
            Ok(Command::Open("000000010000000000000001".to_string()))
        );
        // A path can start with a dash-free name that looks like nothing else.
        assert_eq!(
            parse(&["pg_walview", "./pg_wal/000000010000000000000001"]),
            Ok(Command::Open(
                "./pg_wal/000000010000000000000001".to_string()
            ))
        );
    }

    #[test]
    fn help_and_version_are_recognised_in_both_spellings() {
        assert_eq!(parse(&["pg_walview", "-h"]), Ok(Command::Help));
        assert_eq!(parse(&["pg_walview", "--help"]), Ok(Command::Help));
        assert_eq!(parse(&["pg_walview", "-V"]), Ok(Command::Version));
        assert_eq!(parse(&["pg_walview", "--version"]), Ok(Command::Version));
        // They win over a path, so --help always works.
        assert_eq!(parse(&["pg_walview", "seg", "--help"]), Ok(Command::Help));
    }

    #[test]
    fn a_missing_or_unknown_argument_is_reported() {
        assert!(parse(&["pg_walview"]).is_err());
        let err = parse(&["pg_walview", "--wat"]).unwrap_err();
        assert!(err.contains("--wat"), "{}", err);
        let err = parse(&["pg_walview", "a", "b"]).unwrap_err();
        assert!(err.contains("one"), "{}", err);
    }

    #[test]
    fn help_names_the_version_the_binary_reads() {
        let text = help_text();
        assert!(text.contains(buildinfo::VERSION), "{}", text);
        assert!(text.contains(buildinfo::pg_version()), "{}", text);
        assert!(
            text.contains(&format!("{:04X}", buildinfo::xlog_page_magic())),
            "{}",
            text
        );
        // The keybindings are in there too, so they can be read without
        // starting the TUI.
        for key in ["Tab", "Enter", "same XID", "HEX DUMP"] {
            assert!(text.contains(key), "missing {:?} in:\n{}", key, text);
        }
    }

    #[test]
    fn version_names_the_version_the_binary_reads() {
        let text = version_text();
        assert!(text.contains(buildinfo::VERSION), "{}", text);
        assert!(text.contains(buildinfo::pg_version()), "{}", text);
        assert!(
            text.contains(&format!("{:04X}", buildinfo::xlog_page_magic())),
            "{}",
            text
        );
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// A stand-in segment: a long page header on page 0, then counting bytes.
    fn test_segment() -> Vec<u8> {
        let mut seg = vec![0u8; 3 * 8192];
        for (i, b) in seg.iter_mut().enumerate() {
            *b = i as u8;
        }
        // Mark page 0 as carrying a long header and pages 1-2 short ones.
        seg[2..4].copy_from_slice(&XLP_LONG_HEADER.to_le_bytes());
        seg[8192 + 2..8192 + 4].copy_from_slice(&0u16.to_le_bytes());
        seg[2 * 8192 + 2..2 * 8192 + 4].copy_from_slice(&0u16.to_le_bytes());
        seg
    }

    /// Far enough into the segment that it is off the first screen.
    const MARK_AT: usize = 20_000;

    /// A record that lives on a single page occupies one range; clippy reads
    /// a one-element list of ranges as a mistyped `(a..b).collect()`.
    #[allow(clippy::single_range_in_vec_init, reason = "one range, not a collect")]
    fn one_range(start: usize, len: usize) -> Vec<Range<usize>> {
        vec![start..start + len]
    }

    fn test_seg() -> Segment {
        Segment {
            bytes: test_segment(),
            start_lsn: 3 * 0x100000,
            xlog_blcksz: 8192,
        }
    }

    fn record(lsn: XLogRecPtr, xid: TransactionId, rmid: u8) -> WALRecordInfo {
        let mut r = WALRecordInfo {
            lsn,
            ..Default::default()
        };
        r.xlrec.xl_xid = xid;
        r.xlrec.xl_rmid = rmid;
        r.xlrec.xl_tot_len = 24;
        r.crc_ok = true;
        r.raw = vec![0u8; 24];
        r
    }

    fn app_with(n: usize) -> App {
        let records = (0..n)
            .map(|i| record(0x1000000 + i as u64 * 32, (i / 3) as u32 + 3, 10))
            .collect();
        App::with_records(
            PathBuf::from("000000010000000000000001"),
            records,
            None,
            test_seg(),
        )
    }

    fn render(app: &mut App, area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        buf
    }

    fn test_dump(bytes: &[u8], bytes_per_line: usize) -> HexDump<'_> {
        HexDump {
            bytes,
            start_lsn: 3 * 0x100000,
            xlog_blcksz: 8192,
            bytes_per_line,
        }
    }

    /// The line has to hold "<lsn> <offset>: <hex>  <ascii>"; when the pane is
    /// too narrow for 16 bytes it has to fall back rather than truncate.
    #[test]
    fn bytes_per_line_is_chosen_to_fit_the_pane() {
        assert_eq!(hex_bytes_per_line(200), 16);
        assert_eq!(hex_bytes_per_line(89), 16);
        assert_eq!(hex_bytes_per_line(88), 8);
        assert_eq!(hex_bytes_per_line(55), 8);
        assert_eq!(hex_bytes_per_line(54), 4);
        assert_eq!(hex_bytes_per_line(10), 4);
    }

    #[test]
    fn a_hex_line_shows_both_the_lsn_and_the_file_offset() {
        let seg = test_segment();
        let dump = test_dump(&seg, 8);

        // Line 1 starts at file offset 8.
        assert_eq!(
            line_text(&dump.line(1, &RecordOverlay::empty())),
            "0/00300008 00000008: 08 09 0a 0b  0c 0d 0e 0f  ........"
        );
        // Line 8 starts at offset 0x40, where the bytes are printable ASCII.
        assert_eq!(
            line_text(&dump.line(8, &RecordOverlay::empty())),
            "0/00300040 00000040: 40 41 42 43  44 45 46 47  @ABCDEFG"
        );
    }

    #[test]
    fn the_last_line_is_not_padded_with_phantom_bytes() {
        let seg = vec![0xAAu8; 20];
        let dump = test_dump(&seg, 8);
        assert_eq!(dump.total_lines(), 3);
        assert_eq!(
            line_text(&dump.line(2, &RecordOverlay::empty())),
            "0/00300010 00000010: aa aa aa aa               ...."
        );
    }

    /// A record whose parts land at known offsets in the test segment.
    fn structured_record() -> WALRecordInfo {
        let mut rec = record(0x1000000 + MARK_AT as u64, 5, 10);
        rec.xlrec.xl_tot_len = 60;
        rec.raw = vec![0u8; 60];
        rec.file_ranges = one_range(MARK_AT, 60);
        rec.main_range = Some(40..60);
        rec.blocks = vec![WALBlockData {
            block_id: 0,
            flags: BKPBLOCK_HAS_DATA,
            data_len: 10,
            data_range: Some(30..40),
            ..Default::default()
        }];
        rec
    }

    /// parts(): Header 0..24, Descriptors 24..30, BlockData 30..40, Main 40..60
    /// -- offset by MARK_AT once mapped onto the file.
    #[test]
    fn the_dump_colours_each_part_of_the_record() {
        let rec = structured_record();
        let seg = test_segment();
        let dump = test_dump(&seg, 16);
        let overlay = RecordOverlay::of(&rec, vec![]);

        assert_eq!(
            dump.classify(MARK_AT, &overlay),
            ByteKind::Record(RecordPart::Header)
        );
        assert_eq!(
            dump.classify(MARK_AT + 23, &overlay),
            ByteKind::Record(RecordPart::Header)
        );
        assert_eq!(
            dump.classify(MARK_AT + 24, &overlay),
            ByteKind::Record(RecordPart::Descriptors)
        );
        assert_eq!(
            dump.classify(MARK_AT + 30, &overlay),
            ByteKind::Record(RecordPart::BlockData(0))
        );
        assert_eq!(
            dump.classify(MARK_AT + 40, &overlay),
            ByteKind::Record(RecordPart::MainData)
        );
        assert_eq!(
            dump.classify(MARK_AT + 59, &overlay),
            ByteKind::Record(RecordPart::MainData)
        );
        // One past the end is no longer the record.
        assert_ne!(
            dump.classify(MARK_AT + 60, &overlay),
            ByteKind::Record(RecordPart::MainData)
        );
    }

    /// Zero bytes are dimmed to let the structure show, but only outside the
    /// record: a zero inside it is as meaningful as any other byte.
    #[test]
    fn zero_bytes_are_dimmed_only_outside_the_record() {
        let rec = structured_record();
        let mut seg = test_segment();
        seg[MARK_AT + 5] = 0; // inside the record header
        seg[MARK_AT + 200] = 0; // well past it
        seg[MARK_AT + 201] = 0x42;
        let dump = test_dump(&seg, 16);
        let overlay = RecordOverlay::of(&rec, vec![]);

        assert_eq!(
            dump.classify(MARK_AT + 5, &overlay),
            ByteKind::Record(RecordPart::Header)
        );
        assert_eq!(dump.classify(MARK_AT + 200, &overlay), ByteKind::Zero);
        assert_eq!(dump.classify(MARK_AT + 201, &overlay), ByteKind::Plain);
    }

    #[test]
    fn the_selected_record_is_bold_and_the_focused_part_is_reversed() {
        let rec = structured_record();
        let seg = test_segment();
        let dump = test_dump(&seg, 16);
        let overlay = RecordOverlay::of(&rec, vec![RecordPart::BlockData(0)]);

        let header = dump.style_at(MARK_AT, &overlay);
        assert!(header.add_modifier.contains(Modifier::BOLD));
        assert!(!header.add_modifier.contains(Modifier::REVERSED));

        let focused = dump.style_at(MARK_AT + 30, &overlay);
        assert!(focused.add_modifier.contains(Modifier::BOLD));
        assert!(focused.add_modifier.contains(Modifier::REVERSED));

        let outside = dump.style_at(MARK_AT + 200, &overlay);
        assert!(!outside.add_modifier.contains(Modifier::BOLD));

        // Different parts get different colours.
        assert_ne!(header.fg, dump.style_at(MARK_AT + 40, &overlay).fg);
    }

    /// The DETAILS cursor drives the emphasis in the dump.
    #[test]
    fn the_details_cursor_selects_the_focused_parts() {
        let rec = structured_record();
        let mut tree = DetailTree::new_for(&rec);
        assert_eq!(tree.focused_parts(&rec), vec![RecordPart::Header]);

        // A block item covers everything belonging to that block, so a block
        // whose only payload is a full-page image still lights up.
        tree.move_down(&rec.blocks, rec.has_main_data());
        assert_eq!(
            tree.focused_parts(&rec),
            vec![RecordPart::Fpi(0), RecordPart::BlockData(0)]
        );

        tree.move_down(&rec.blocks, rec.has_main_data());
        assert_eq!(tree.focused_parts(&rec), vec![RecordPart::MainData]);
    }

    /// Selecting a block that carries only an image used to highlight nothing:
    /// the block item mapped onto BlockData, which such a block does not have.
    #[test]
    fn a_block_holding_only_an_image_is_highlighted() {
        let mut rec = record(0x1000000 + MARK_AT as u64, 5, 10);
        rec.xlrec.xl_tot_len = 60;
        rec.raw = vec![0u8; 60];
        rec.file_ranges = one_range(MARK_AT, 60);
        rec.blocks = vec![WALBlockData {
            flags: BKPBLOCK_HAS_IMAGE,
            image: Some(WALFullPageImage {
                bimg_len: 30,
                bimg_range: 30..60,
                ..Default::default()
            }),
            ..Default::default()
        }];

        let mut tree = DetailTree::new_for(&rec);
        tree.move_down(&rec.blocks, rec.has_main_data());
        let overlay = RecordOverlay::of(&rec, tree.focused_parts(&rec));

        let seg = test_segment();
        let dump = test_dump(&seg, 16);
        assert!(
            dump.style_at(MARK_AT + 30, &overlay)
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }

    /// Splitting on the first colon only: a timestamp value has colons of its
    /// own and must stay in one piece.
    #[test]
    fn detail_lines_split_the_key_from_the_value() {
        let line = detail_field_line("  ts:  2000-01-01 00:00:00 UTC".to_string());
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "  ts:");
        assert_eq!(line.spans[1].content, "  2000-01-01 00:00:00 UTC");
        assert_ne!(line.spans[0].style, line.spans[1].style);

        // A line with no key is all value.
        let line = detail_field_line("  (no main data)".to_string());
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "  (no main data)");
    }

    #[test]
    fn the_list_marks_records_that_carry_a_full_page_image() {
        let mut plain = record(0x1000000, 5, 10);
        plain.blocks = vec![WALBlockData::default()];
        let mut with_fpi = record(0x1000020, 5, 10);
        with_fpi.blocks = vec![WALBlockData {
            image: Some(Default::default()),
            ..Default::default()
        }];

        let mut app = App::with_records(
            PathBuf::from("seg"),
            vec![plain, with_fpi],
            None,
            test_seg(),
        );
        let buf = render(&mut app, Rect::new(0, 0, 150, 45));
        let text = screen_text(&buf);
        // The header names the column, and exactly one row is marked.
        assert!(text.contains(" F "), "{}", &text[..200]);
        assert_eq!(
            text.lines().filter(|l| l.contains(" * ")).count(),
            1,
            "{}",
            text
        );
    }

    fn overlay(parts: Vec<(Range<usize>, RecordPart)>, focus: Vec<RecordPart>) -> RecordOverlay {
        RecordOverlay { parts, focus }
    }

    #[test]
    fn record_bytes_are_told_apart_from_page_headers_and_the_rest() {
        let seg = test_segment();
        let dump = test_dump(&seg, 16);
        let record = overlay(
            vec![
                (40..8192, RecordPart::Fpi(0)),
                (8192 + 24..8192 + 100, RecordPart::Fpi(0)),
            ],
            vec![],
        );

        // Page 0's long header, then the record, then untouched bytes.
        assert_eq!(dump.classify(0, &record), ByteKind::PageHeader);
        assert_eq!(dump.classify(39, &record), ByteKind::PageHeader);
        assert_eq!(
            dump.classify(40, &record),
            ByteKind::Record(RecordPart::Fpi(0))
        );
        assert_eq!(
            dump.classify(8191, &record),
            ByteKind::Record(RecordPart::Fpi(0))
        );

        // The page header that splits the record must not look like part of it.
        assert_eq!(dump.classify(8192, &record), ByteKind::PageHeader);
        assert_eq!(dump.classify(8192 + 23, &record), ByteKind::PageHeader);
        assert_eq!(
            dump.classify(8192 + 24, &record),
            ByteKind::Record(RecordPart::Fpi(0))
        );
        assert_ne!(
            dump.classify(8192 + 100, &record),
            ByteKind::Record(RecordPart::Fpi(0))
        );

        // A later page's header is dimmed even with no record near it.
        assert_eq!(dump.classify(2 * 8192 + 10, &record), ByteKind::PageHeader);
        assert_ne!(dump.classify(2 * 8192 + 24, &record), ByteKind::PageHeader);

        // Page header always wins: a range that runs straight across a page
        // boundary must not make the header look like record content.
        let spanning = overlay(vec![(8100..8300, RecordPart::MainData)], vec![]);
        assert_eq!(
            dump.classify(8191, &spanning),
            ByteKind::Record(RecordPart::MainData)
        );
        assert_eq!(dump.classify(8192, &spanning), ByteKind::PageHeader);
        assert_eq!(dump.classify(8192 + 23, &spanning), ByteKind::PageHeader);
        assert_eq!(
            dump.classify(8192 + 24, &spanning),
            ByteKind::Record(RecordPart::MainData)
        );
    }

    #[test]
    fn only_the_record_bytes_on_a_line_are_highlighted() {
        let seg = test_segment();
        let dump = test_dump(&seg, 16);
        // A record starting mid-line, at offset 0x44, with the cursor on it.
        let mark = overlay(
            vec![(0x44..0x50, RecordPart::MainData)],
            vec![RecordPart::MainData],
        );
        let line = dump.line(4, &mark);

        let highlighted: String = line
            .spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::REVERSED))
            .map(|s| s.content.as_ref())
            .collect();
        // Bytes 0x40-0x43 stay plain; 0x44 onwards are highlighted, in the
        // hex column and in the ASCII column alike.
        assert!(highlighted.contains("44 45 46 47"), "{:?}", highlighted);
        assert!(!highlighted.contains("40 41"), "{:?}", highlighted);
        assert!(highlighted.contains("DEFGHIJKLMNO"), "{:?}", highlighted);
    }
    #[test]
    fn an_empty_record_list_renders() {
        let mut app = App::with_records(PathBuf::from("seg"), vec![], None, test_seg());
        assert_eq!(app.state.selected(), None);
        render(&mut app, Rect::new(0, 0, 120, 40));

        // Navigation on an empty list must be a no-op, not an underflow.
        app.move_bottom();
        app.move_record_down();
        app.move_record_up();
        app.page_down();
        app.page_up();
        app.move_top();
        app.move_next_same_xid();
        app.move_prev_same_xid();
        assert_eq!(app.state.selected(), None);
        render(&mut app, Rect::new(0, 0, 120, 40));
    }

    #[test]
    fn a_single_record_renders() {
        let mut app = App::with_records(
            PathBuf::from("seg"),
            vec![record(0x1000000, 0, 0)],
            None,
            test_seg(),
        );
        render(&mut app, Rect::new(0, 0, 120, 40));
        app.move_bottom();
        app.move_record_down();
        assert_eq!(app.state.selected(), Some(0));
    }

    /// A tiny terminal must not make the layout arithmetic underflow.
    #[test]
    fn tiny_areas_render() {
        let mut app = app_with(50);
        for (w, h) in [
            (0u16, 0u16),
            (1, 1),
            (4, 3),
            (20, 5),
            (80, 2),
            (MIN_WIDTH, MIN_HEIGHT),
            (MIN_WIDTH - 1, MIN_HEIGHT - 1),
            (200, 60),
        ] {
            render(&mut app, Rect::new(0, 0, w, h));
        }
    }

    #[test]
    fn the_stop_reason_is_shown() {
        let mut app = App::with_records(
            PathBuf::from("seg"),
            vec![record(0x1000000, 5, 10)],
            Some("something went wrong".to_string()),
            test_seg(),
        );
        let buf = render(&mut app, Rect::new(0, 0, 120, 40));
        let last_row: String = (0..120)
            .map(|x| buf[(x, 39)].symbol().to_string())
            .collect();
        assert!(last_row.contains("something went wrong"), "{}", last_row);
    }

    /// Scrolling is managed by the app so that only the visible rows are
    /// built; the window has to keep following the selection.
    #[test]
    fn the_visible_window_follows_the_selection() {
        let mut app = app_with(100_000);
        let area = Rect::new(0, 0, 120, 40);
        // 1 status line, 2 borders, header row + its bottom margin.
        let visible = 40 - 1 - 4;

        render(&mut app, area);
        assert_eq!(app.list_offset, 0);

        app.move_bottom();
        render(&mut app, area);
        assert_eq!(app.state.selected(), Some(99_999));
        assert_eq!(app.list_offset, 100_000 - visible);

        app.move_top();
        render(&mut app, area);
        assert_eq!(app.list_offset, 0);

        // Stepping just past the bottom edge scrolls by exactly one row.
        for _ in 0..visible {
            app.move_record_down();
        }
        render(&mut app, area);
        assert_eq!(app.state.selected(), Some(visible));
        assert_eq!(app.list_offset, 1);
    }

    #[test]
    fn same_xid_navigation_skips_bootstrap_xids() {
        // records 0..3 share xid 3, 3..6 share xid 4, ...
        let mut app = app_with(9);
        assert_eq!(app.state.selected(), Some(0));
        app.move_next_same_xid();
        assert_eq!(app.state.selected(), Some(1));
        app.move_next_same_xid();
        assert_eq!(app.state.selected(), Some(2));
        app.move_next_same_xid();
        assert_eq!(app.state.selected(), Some(2)); // no more with this xid
        app.move_prev_same_xid();
        assert_eq!(app.state.selected(), Some(1));

        // A record with no real transaction has nothing to jump to.
        let mut app = App::with_records(
            PathBuf::from("seg"),
            vec![record(0x1000000, 0, 0), record(0x1000020, 0, 0)],
            None,
            test_seg(),
        );
        app.move_next_same_xid();
        assert_eq!(app.state.selected(), Some(0));
    }

    #[test]
    fn the_xid_range_is_cached_per_selection() {
        let mut app = app_with(9);
        assert_eq!(app.xid_range, Some((0, 2)));
        app.select(4);
        assert_eq!(app.xid_range, Some((3, 5)));
        app.select(8);
        assert_eq!(app.xid_range, Some((6, 8)));
    }

    /// The dump now covers the whole segment, so the pane has to be able to
    /// leave the selected record and come back to it.
    fn app_with_marked_record() -> App {
        let mut seg = test_seg();
        // A distinctive run of bytes where the record lives, so the tests can
        // tell from the rendered screen whether it is on it.
        seg.bytes[MARK_AT..MARK_AT + 4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let mut rec = record(0x1000000, 5, 10);
        rec.file_ranges = one_range(MARK_AT, 4);
        App::with_records(PathBuf::from("seg"), vec![rec], None, seg)
    }

    fn screen_text(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_dump_opens_on_the_selected_record() {
        let mut app = app_with_marked_record();
        let buf = render(&mut app, Rect::new(0, 0, 150, 45));
        assert!(
            screen_text(&buf).contains("de ad be ef"),
            "the selected record should be on screen"
        );
    }

    #[test]
    fn the_dump_can_scroll_away_from_the_record_and_back() {
        let mut app = app_with_marked_record();
        let area = Rect::new(0, 0, 150, 45);
        app.focus = FocusPane::HexDump;

        app.handle_hex_dump_key(KeyEvent::from(KeyCode::Char('G')));
        let buf = render(&mut app, area);
        let at_end = app.hex_scroll;
        assert!(at_end > 0, "G should move to the end of the segment");
        assert!(!screen_text(&buf).contains("de ad be ef"));

        app.handle_hex_dump_key(KeyEvent::from(KeyCode::Char('g')));
        let buf = render(&mut app, area);
        assert!(
            screen_text(&buf).contains("de ad be ef"),
            "g should come back to the selected record"
        );
    }

    #[test]
    fn scrolling_stops_at_the_end_of_the_segment() {
        let mut app = app_with_marked_record();
        let area = Rect::new(0, 0, 150, 45);
        app.focus = FocusPane::HexDump;

        app.handle_hex_dump_key(KeyEvent::from(KeyCode::Char('G')));
        render(&mut app, area);
        let at_end = app.hex_scroll;

        for _ in 0..5 {
            app.handle_hex_dump_key(KeyEvent::from(KeyCode::Down));
            render(&mut app, area);
        }
        assert_eq!(app.hex_scroll, at_end, "must not scroll past the last line");
    }

    #[test]
    fn changing_the_selection_moves_the_dump_to_the_new_record() {
        let mut seg = test_seg();
        seg.bytes[MARK_AT..MARK_AT + 4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        seg.bytes[100..104].copy_from_slice(&[0xCA, 0xFE, 0xBA, 0xBE]);

        let mut first = record(0x1000064, 5, 10);
        first.file_ranges = one_range(100, 4);
        let mut second = record(0x1000000 + MARK_AT as u64, 5, 10);
        second.file_ranges = one_range(MARK_AT, 4);

        let mut app = App::with_records(PathBuf::from("seg"), vec![first, second], None, seg);
        let area = Rect::new(0, 0, 150, 45);

        let buf = render(&mut app, area);
        assert!(screen_text(&buf).contains("ca fe ba be"));

        app.move_record_down();
        let buf = render(&mut app, area);
        assert!(
            screen_text(&buf).contains("de ad be ef"),
            "selecting a record should bring it into view"
        );
    }
}
