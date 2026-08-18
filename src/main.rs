use anyhow::{Context, Result};
use std::env;
use std::io;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect, Spacing},
    style::{Color, Modifier, Style},
    symbols::merge::MergeStrategy,
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState, Widget},
};

use pg_walview::bindings::*;
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

#[derive(Debug)]
pub struct App {
    records: Vec<WALRecordInfo>,
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
    exit: bool,
}

const PAGE_JUMP_SIZE: usize = 20;

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

        Ok(Self::with_records(
            PathBuf::from(path),
            records,
            stop_reason,
        ))
    }

    fn with_records(
        current_file: PathBuf,
        records: Vec<WALRecordInfo>,
        stop_reason: Option<String>,
    ) -> Self {
        let mut state = TableState::default();
        if !records.is_empty() {
            state.select(Some(0));
        }

        let mut app = App {
            records,
            state,
            current_file,
            stop_reason,
            list_offset: 0,
            xid_range: None,
            focus: FocusPane::RecordList,
            detail_tree: DetailTree::default(),
            detail_scroll: 0,
            hex_scroll: 0,
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
            KeyCode::Up => self.hex_scroll = self.hex_scroll.saturating_sub(1),
            KeyCode::Down => self.hex_scroll = self.hex_scroll.saturating_add(1),
            KeyCode::PageUp | KeyCode::Char('b') => {
                self.hex_scroll = self.hex_scroll.saturating_sub(PAGE_JUMP_SIZE)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.hex_scroll = self.hex_scroll.saturating_add(PAGE_JUMP_SIZE)
            }
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
        self.hex_scroll = 0;
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

fn border_style(active: bool) -> Style {
    if active {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    }
}

fn make_item_line(text: String, is_cursor: bool) -> Line<'static> {
    if is_cursor {
        Line::from(Span::styled(
            text,
            Style::default().add_modifier(Modifier::REVERSED),
        ))
    } else {
        Line::from(text)
    }
}

fn detail_field_line(text: String) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(Color::Gray)))
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
                lines.push(make_item_line(summary, is_cursor));

                if tree.header_expanded {
                    lines.push(detail_field_line(format!(
                        "  LSN:      {}",
                        lsn_format(record.lsn)
                    )));
                    lines.push(detail_field_line(format!(
                        "  prev LSN: {}",
                        lsn_format(record.xlrec.xl_prev)
                    )));
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
                    lines.push(detail_field_line(format!(
                        "  crc:      0x{:08X} ({})",
                        record.xlrec.xl_crc,
                        if record.crc_ok { "ok" } else { "MISMATCH" }
                    )));
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
                lines.push(make_item_line(summary, is_cursor));

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
                    lines.push(detail_field_line(format!(
                        "  flags:  {}",
                        block.flags_str()
                    )));
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
                    lines.push(make_item_line(summary, is_cursor));

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
                lines.push(make_item_line(summary, is_cursor));

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

/// Build hex dump lines for the given raw bytes.
fn build_hex_lines(raw: &[u8]) -> Vec<Line<'static>> {
    raw.chunks(16)
        .enumerate()
        .map(|(chunk_idx, chunk)| {
            let offset = chunk_idx * 16;
            let hex: String = chunk
                .chunks(4)
                .map(|g| {
                    g.iter()
                        .map(|b| format!("{:02x}", b))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("  ");
            let ascii: String = chunk
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            Line::from(format!("{:04x}: {:<51}  {}", offset, hex, ascii))
        })
        .collect()
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
        let line_color = match self.xid_range {
            Some((_, last)) if self.records.get(last).is_some() => {
                let last_rec = &self.records[last];
                if RmgrId::from_u8(last_rec.xlrec.xl_rmid) == RmgrId::Xact {
                    match XactOp::from_xl_info(last_rec.xlrec.xl_info) {
                        XactOp::Commit | XactOp::CommitPrepared => Color::Cyan,
                        XactOp::Abort | XactOp::AbortPrepared => Color::Red,
                        _ => Color::Yellow,
                    }
                } else {
                    Color::DarkGray
                }
            }
            _ => Color::DarkGray,
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

                let graph_span = Span::styled(
                    prefix,
                    Style::default().fg(line_color).add_modifier(Modifier::BOLD),
                );
                let lsn_span = Span::raw(lsn_format(record.lsn));
                let combined_line = Line::from(vec![graph_span, lsn_span]);

                let desc = describe_record(record);
                let desc_cell = if record.crc_ok {
                    Cell::from(desc)
                } else {
                    Cell::from(Span::styled(
                        format!("{} [CRC ERROR]", desc),
                        Style::default().fg(Color::Red),
                    ))
                };

                let mut row = Row::new(vec![
                    Cell::from(combined_line),
                    Cell::from(record.xlrec.xl_xid.to_string()),
                    Cell::from(record.xlrec.xl_tot_len.to_string()),
                    Cell::from(RmgrId::from_u8(record.xlrec.xl_rmid).to_string()),
                    desc_cell,
                ]);

                if is_selected {
                    row = row.style(Style::default().add_modifier(Modifier::REVERSED));
                } else if let Some(xid) = selected_xid
                    && xid != 0
                    && record.xlrec.xl_xid == xid
                {
                    row = row.style(Style::default().bg(Color::DarkGray).fg(Color::White));
                }

                row
            })
            .collect();

        let header = Row::new(vec!["       LSN", "XID", "LEN", "RMID", "DESC"])
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
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
                    .border_style(border_style(list_active))
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

    fn render_details(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let detail_active = self.focus == FocusPane::Details;
        let inner_h = area.height.saturating_sub(2) as usize;

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
                    .border_style(border_style(detail_active))
                    .merge_borders(MergeStrategy::Exact),
            )
            .scroll((self.detail_scroll as u16, 0))
            .render(area, buf);
    }

    fn render_hex_dump(&mut self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let hex_active = self.focus == FocusPane::HexDump;
        let inner_h = area.height.saturating_sub(2) as usize;

        let hex_lines = match self.records.get(self.state.selected().unwrap_or(0)) {
            Some(record) => build_hex_lines(&record.raw),
            None => vec![],
        };

        // Do not let the pane scroll past its last line.
        let max_scroll = hex_lines.len().saturating_sub(inner_h.max(1));
        self.hex_scroll = self.hex_scroll.min(max_scroll);

        Paragraph::new(hex_lines)
            .block(
                Block::default()
                    .title("HEX DUMP")
                    .borders(Borders::ALL)
                    .border_style(border_style(hex_active))
                    .merge_borders(MergeStrategy::Exact),
            )
            .scroll((self.hex_scroll as u16, 0))
            .render(area, buf);
    }

    fn render_status(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let line = match &self.stop_reason {
            Some(reason) => Line::from(Span::styled(
                format!(" stopped: {}", reason),
                Style::default().fg(Color::Red),
            )),
            None => Line::from(Span::styled(
                " q:quit  Tab:pane  j/k:move  g/G:top/bottom  s/r:next/prev same XID  Space/-:page",
                Style::default().fg(Color::DarkGray),
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
        self.render_details(v_chunks[0], buf);
        self.render_hex_dump(v_chunks[1], buf);
        self.render_status(outer[1], buf);
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path_to_wal_file>", args[0]);
        std::process::exit(1);
    }

    // Load before touching the terminal, so a failure here reports the real
    // reason on stderr instead of inside the alternate screen.
    let mut app = App::new(&args[1])?;

    ratatui::run(|terminal| app.run(terminal))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

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
        App::with_records(PathBuf::from("000000010000000000000001"), records, None)
    }

    fn render(app: &mut App, area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        buf
    }

    /// A segment with nothing decodable used to index records[0] on the very
    /// first frame and take the whole program down.
    #[test]
    fn an_empty_record_list_renders() {
        let mut app = App::with_records(PathBuf::from("seg"), vec![], None);
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
        let mut app = App::with_records(PathBuf::from("seg"), vec![record(0x1000000, 0, 0)], None);
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

    #[test]
    fn hex_scroll_stays_within_the_dump() {
        let mut app = App::with_records(PathBuf::from("seg"), vec![record(0x1000000, 5, 10)], None);
        app.focus = FocusPane::HexDump;
        for _ in 0..100 {
            app.handle_hex_dump_key(KeyEvent::from(KeyCode::Down));
        }
        render(&mut app, Rect::new(0, 0, 120, 40));
        // 24 raw bytes is two hex lines, so there is nothing to scroll to.
        assert_eq!(app.hex_scroll, 0);
    }
}
