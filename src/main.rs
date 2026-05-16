use anyhow::Result;
use std::env;
use std::io;
use std::path::PathBuf;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use ratatui::{
    buffer::Buffer,
    symbols::merge::MergeStrategy,
    layout::{Constraint, Direction, Layout, Rect, Spacing},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    DefaultTerminal, Frame,
    widgets::{
        Block, Borders, Cell, Paragraph, Row, StatefulWidget, Table, TableState, Widget,
    },
};

use pg_walview::bindings::*;
use pg_walview::walmisc::*;
use pg_walview::walreader::*;
use pg_walview::rmgr::RmgrId;
use pg_walview::walmain::{describe_main_data, describe_block_data};
use pg_walview::xlogdesc::*;
use pg_walview::xactdesc::*;
use pg_walview::smgrdesc::*;
use pg_walview::clogdesc::*;
use pg_walview::dbdesc::*;
use pg_walview::tblspcdesc::*;
use pg_walview::multixactdesc::*;
use pg_walview::relmapdesc::*;
use pg_walview::standbydesc::*;
use pg_walview::heap2desc::*;
use pg_walview::heapdesc::*;
use pg_walview::btdesc::*;
use pg_walview::hashdesc::*;
use pg_walview::gindesc::*;
use pg_walview::gistdesc::*;
use pg_walview::seqdesc::*;
use pg_walview::spgistdesc::*;
use pg_walview::brindesc::*;
use pg_walview::committsdesc::*;
use pg_walview::replorigindesc::*;
use pg_walview::genericdesc::*;
use pg_walview::logicalmsgdesc::*;

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
            block_states: vec![(false, false); record.nblocks_inuse],
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
    state: TableState,
    focus: FocusPane,
    detail_tree: DetailTree,
    detail_scroll: usize,
    hex_scroll: usize,
    exit: bool,
}

const PAGE_JUMP_SIZE: usize = 20;

impl App {
    pub fn new(path: &str) -> Result<Self> {
        let mut reader = WALReader::new(path)?;
        let mut records: Vec<WALRecordInfo> = Vec::new();

        while let Some(record) = reader.load_next_record()? {
            records.push(record);
        }

        let mut state = TableState::default();
        state.select(Some(0));

        let detail_tree = if !records.is_empty() {
            DetailTree::new_for(&records[0])
        } else {
            DetailTree::default()
        };

        Ok(App {
            records,
            state,
            current_file: PathBuf::from(path),
            focus: FocusPane::RecordList,
            detail_tree,
            detail_scroll: 0,
            hex_scroll: 0,
            exit: false,
        })
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
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
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
            KeyCode::PageUp | KeyCode::Char('-') => self.page_up(),
            KeyCode::PageDown | KeyCode::Char(' ') => self.page_down(),
            _ => {}
        }
    }

    fn handle_details_key(&mut self, key_event: KeyEvent) {
        if let Some(idx) = self.state.selected() {
            let record = &self.records[idx];
            let nblocks = record.nblocks_inuse;
            let blocks = &record.blocks[..nblocks];
            let has_main = record.main.is_some();
            match key_event.code {
                KeyCode::Up => self.detail_tree.move_up(),
                KeyCode::Down => self.detail_tree.move_down(blocks, has_main),
                KeyCode::Enter => self.detail_tree.toggle(blocks, has_main),
                _ => {}
            }
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

    fn move_record_down(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.records.len() - 1 {
                    i
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.on_record_change();
    }

    fn move_record_up(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    0
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
        self.on_record_change();
    }

    fn move_top(&mut self) {
	self.state.select(Some(0));
	self.on_record_change();
    }

    fn move_bottom(&mut self) {
	self.state.select(Some(self.records.len() - 1));
	self.on_record_change();
    }

    fn page_up(&mut self) {
        let current = self.state.selected().unwrap_or(0);
        self.state.select(Some(current.saturating_sub(PAGE_JUMP_SIZE)));
        self.on_record_change();
    }

    fn page_down(&mut self) {
        let current = self.state.selected().unwrap_or(0);
        let max_idx = self.records.len().saturating_sub(1);
        self.state
            .select(Some(current.saturating_add(PAGE_JUMP_SIZE).min(max_idx)));
        self.on_record_change();
    }

    fn on_record_change(&mut self) {
        if let Some(idx) = self.state.selected() {
            self.detail_tree = DetailTree::new_for(&self.records[idx]);
            self.detail_scroll = 0;
            self.hex_scroll = 0;
        }
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

    let nblocks = record.nblocks_inuse;
    let blocks = &record.blocks[..nblocks];
    let has_main = record.main.is_some();
    let nav = tree.nav_items(blocks, has_main);

    for (nav_idx, nav_item) in nav.iter().enumerate() {
        let is_at_cursor = nav_idx == tree.cursor;
        let is_cursor = show_cursor && is_at_cursor;

        match nav_item {
            NavItem::Header => {
                let arrow = if tree.header_expanded { "▼" } else { "▶" };
                let rmgr = RmgrId::from_u8(record.xlrec.xl_rmid);
                let summary = format!(
                    "{} Header  XID:{}  tot_len:{}  RMID:{}",
                    arrow, record.xlrec.xl_xid, record.xlrec.xl_tot_len, rmgr
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
                        "  top_xid:  {}",
                        record
                            .top_xid
                            .map_or("-".to_string(), |x| x.to_string())
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
                    i,
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
                    lines.push(detail_field_line(format!("  flags:  {}", block.flags_str())));
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
                    if is_cursor {
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
                .map(|&b| if b >= 0x20 && b < 0x7f { b as char } else { '.' })
                .collect();
            Line::from(format!("{:04x}: {:<51}  {}", offset, hex, ascii))
        })
        .collect()
}

/// Build a DetailTree with everything expanded (for auto-expand check).
fn fully_expanded_tree(record: &WALRecordInfo) -> DetailTree {
    let nblocks = record.nblocks_inuse;
    let block_states = (0..nblocks)
        .map(|i| (true, record.blocks[i].image.is_some()))
        .collect();
    DetailTree {
        header_expanded: true,
        block_states,
        main_expanded: true,
        cursor: 0,
    }
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Horizontal split: 40% record list | 60% right panels
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .spacing(Spacing::Overlap(1))
            .split(area);

        // Vertical split of right side: 60% details | remaining hex dump
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Fill(1)])
            .spacing(Spacing::Overlap(1))
            .split(h_chunks[1]);

        // ---- Left pane: record list ----------------------------------------

        let selected_xid: Option<TransactionId> = if let Some(idx) = self.state.selected() {
            Some(self.records[idx].xlrec.xl_xid)
        } else {
            None
        };

        let mut first_idx = None;
        let mut last_idx = None;
        if let Some(xid) = selected_xid {
            if xid != 0 {
                for (i, rec) in self.records.iter().enumerate() {
                    if rec.xlrec.xl_xid == xid {
                        if first_idx.is_none() {
                            first_idx = Some(i);
                        }
                        last_idx = Some(i);
                    }
                }
            } else {
                first_idx = Some(0);
                last_idx = Some(0);
            }
        }

        let rows: Vec<Row> = self
            .records
            .iter()
            .enumerate()
            .map(|(i, record)| {
                let is_selected = self.state.selected().map_or(false, |s| i == s);

                let mut prefix = "    ";
                let mut line_color = Color::DarkGray;
                if let Some(xid) = selected_xid {
                    let first = first_idx.unwrap();
                    let last = last_idx.unwrap();

                    if RmgrId::from_u8(self.records[last].xlrec.xl_rmid) == RmgrId::Xact {
                        match XactOp::from_xl_info(self.records[last].xlrec.xl_info) {
                            XactOp::Commit | XactOp::CommitPrepared => {
                                line_color = Color::Cyan
                            }
                            XactOp::Abort | XactOp::AbortPrepared => {
                                line_color = Color::Red
                            }
                            _ => line_color = Color::Yellow,
                        }
                    }

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
                    Style::default()
                        .fg(line_color)
                        .add_modifier(Modifier::BOLD),
                );
                let lsn_span = Span::raw(lsn_format(record.lsn));
                let combined_line = Line::from(vec![graph_span, lsn_span]);

                let info = record.xlrec.xl_info;
                let desc_cell = match RmgrId::from_u8(record.xlrec.xl_rmid) {
                    RmgrId::Xlog => Cell::from(format!("{}", XlogOp::from_xl_info(info))),
                    RmgrId::Xact => Cell::from(format!("{}", XactOp::from_xl_info(info))),
                    RmgrId::Smgr => Cell::from(format!("{}", SmgrOp::from_xl_info(info))),
                    RmgrId::Clog => Cell::from(format!("{}", ClogOp::from_xl_info(info))),
                    RmgrId::Database => {
                        Cell::from(format!("{}", DatabaseOp::from_xl_info(info)))
                    }
                    RmgrId::Tablespace => {
                        Cell::from(format!("{}", TablespaceOp::from_xl_info(info)))
                    }
                    RmgrId::MultiXact => {
                        Cell::from(format!("{}", MultiXactOp::from_xl_info(info)))
                    }
                    RmgrId::Relmap => Cell::from(format!("{}", RelmapOp::from_xl_info(info))),
                    RmgrId::Standby => {
                        Cell::from(format!("{}", StandbyOp::from_xl_info(info)))
                    }
                    RmgrId::Heap2 => Cell::from(format!("{}", Heap2Op::from_xl_info(info))),
                    RmgrId::Heap => Cell::from(format!("{}", HeapOp::from_xl_info(info))),
                    RmgrId::Btree => Cell::from(format!("{}", BtreeOp::from_xl_info(info))),
                    RmgrId::Hash => Cell::from(format!("{}", HashOp::from_xl_info(info))),
                    RmgrId::Gin => Cell::from(format!("{}", GinOp::from_xl_info(info))),
                    RmgrId::Gist => Cell::from(format!("{}", GistOp::from_xl_info(info))),
                    RmgrId::Sequence => {
                        Cell::from(format!("{}", SequenceOp::from_xl_info(info)))
                    }
                    RmgrId::SPGist => {
                        Cell::from(format!("{}", SpGistOp::from_xl_info(info)))
                    }
                    RmgrId::Brin => Cell::from(format!("{}", BrinOp::from_xl_info(info))),
                    RmgrId::CommitTs => {
                        Cell::from(format!("{}", CommitTsOp::from_xl_info(info)))
                    }
                    RmgrId::ReplicationOrigin => {
                        Cell::from(format!("{}", ReplOriginOp::from_xl_info(info)))
                    }
                    RmgrId::Generic => {
                        Cell::from(format!("{}", GenericOp::from_xl_info(info)))
                    }
                    RmgrId::LogicalMessage => {
                        Cell::from(format!("{}", LogicalMsgOp::from_xl_info(info)))
                    }
                    RmgrId::Unknown(_) => Cell::from("Unknown"),
                };

                let mut row = Row::new(vec![
                    Cell::from(combined_line),
                    Cell::from(Span::from(format!("{}", record.xlrec.xl_xid))),
                    Cell::from(Span::from(format!("{}", record.xlrec.xl_tot_len))),
                    Cell::from(Span::from(format!(
                        "{}",
                        RmgrId::from_u8(record.xlrec.xl_rmid)
                    ))),
                    desc_cell,
                ]);

                if is_selected {
                    row = row.style(Style::default().add_modifier(Modifier::REVERSED));
                } else if let Some(xid) = selected_xid {
                    if xid != 0 && record.xlrec.xl_xid == xid {
                        row = row
                            .style(Style::default().bg(Color::DarkGray).fg(Color::White));
                    }
                }

                row
            })
            .collect();

        let header = Row::new(vec![
            "    LSN",
            "XID",
            "LEN(TOTAL)",
            "RMID",
            "DESC",
        ])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

        let widths = [
            Constraint::Length(15),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Length(20),
        ];

        let selected_idx = self.state.selected().unwrap_or(0);
        let percentage = if selected_idx == self.records.len() {
            100
        } else {
            (selected_idx * 100) / self.records.len()
        };

        let list_active = self.focus == FocusPane::RecordList;
        let table = Table::new(rows, widths)
            .header(header)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style(list_active))
                    .title("WAL records")
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

        StatefulWidget::render(table, h_chunks[0], buf, &mut self.state);

        // ---- Right top: DETAILS accordion ----------------------------------

        let detail_active = self.focus == FocusPane::Details;
        let detail_inner_h = v_chunks[0].height.saturating_sub(2) as usize;

        let (detail_lines, cursor_line) = if let Some(idx) = self.state.selected() {
            let record = &self.records[idx];
            // If all items fit when fully expanded, auto-expand and skip scrolling
            let full_tree = fully_expanded_tree(record);
            let (full_lines, _) = build_detail_lines(&full_tree, record, false);
            if full_lines.len() <= detail_inner_h {
                self.detail_scroll = 0;
                (full_lines, 0)
            } else {
                build_detail_lines(&self.detail_tree, record, detail_active)
            }
        } else {
            (vec![Line::from("No record selected")], 0)
        };

        // Keep cursor in view (only reached when not auto-expanded)
        if detail_inner_h > 0 {
            if cursor_line < self.detail_scroll {
                self.detail_scroll = cursor_line;
            } else if cursor_line >= self.detail_scroll + detail_inner_h {
                self.detail_scroll = cursor_line.saturating_sub(detail_inner_h - 1);
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
            .render(v_chunks[0], buf);

        // ---- Right bottom: HEX DUMP ----------------------------------------

        let hex_active = self.focus == FocusPane::HexDump;

        let hex_lines = if let Some(idx) = self.state.selected() {
            build_hex_lines(&self.records[idx].raw)
        } else {
            vec![]
        };

        // Clamp scroll to valid range
        let hex_max_scroll = hex_lines.len().saturating_sub(1);
        if self.hex_scroll > hex_max_scroll {
            self.hex_scroll = hex_max_scroll;
        }

        Paragraph::new(hex_lines)
            .block(
                Block::default()
                    .title("HEX DUMP")
                    .borders(Borders::ALL)
                    .border_style(border_style(hex_active))
                    .merge_borders(MergeStrategy::Exact),
            )
            .scroll((self.hex_scroll as u16, 0))
            .render(v_chunks[1], buf);
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path_to_wal_file>", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];

    let _ = ratatui::run(|terminal| {
        App::new(path)
            .expect("could not open wal file")
            .run(terminal)
    });

    Ok(())
}
