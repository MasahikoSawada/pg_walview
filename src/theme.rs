//! Every colour the UI uses, in one place.
//!
//! This is a module of the binary rather than the library: the library reads
//! WAL and has no business depending on a terminal toolkit.
//!
//! Only the 16 ANSI colours are used, so the terminal's own theme still
//! decides what they look like.  Three rules keep the result readable:
//! background carries state (selection, the selected transaction), foreground
//! carries meaning (record structure, resource manager), and nothing is
//! encoded in colour alone.

use pg_walview::rmgr::RmgrId;
use pg_walview::walreader::RecordPart;
use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;

/// The NO_COLOR convention (https://no-color.org): the variable being present
/// and non-empty disables colour, whatever its value.
pub fn colors_enabled_for(no_color: Option<&str>) -> bool {
    !matches!(no_color, Some(v) if !v.is_empty())
}

/// Drop the colours from a style but keep its attributes.  NO_COLOR forbids
/// colour, not bold or reverse video, and without those there would be no way
/// to see which record is selected.
pub fn strip_color(style: Style) -> Style {
    Style::default().add_modifier(style.add_modifier)
}

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| colors_enabled_for(std::env::var("NO_COLOR").ok().as_deref()))
}

/// Every style below goes through here, so NO_COLOR needs handling only once.
fn c(style: Style) -> Style {
    if enabled() { style } else { strip_color(style) }
}

fn fg(color: Color) -> Style {
    c(Style::default().fg(color))
}

// --- panes -----------------------------------------------------------------

pub fn border(active: bool) -> Style {
    fg(if active { Color::Yellow } else { Color::Gray })
}

pub fn status_hint() -> Style {
    fg(Color::DarkGray)
}

pub fn status_error() -> Style {
    fg(Color::Red)
}

// --- record list -----------------------------------------------------------

pub fn list_header() -> Style {
    c(Style::default().fg(Color::Yellow)).add_modifier(Modifier::BOLD)
}

/// The row the cursor is on.  Reverse video, so it survives NO_COLOR.
pub fn selected_row() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// The other rows belonging to the selected transaction.
pub fn xid_group_row() -> Style {
    c(Style::default().bg(Color::DarkGray).fg(Color::White))
}

/// The rail drawn beside the records of the selected transaction.  Green and
/// red for committed and aborted follow the convention everywhere else.
pub fn graph_committed() -> Style {
    fg(Color::Green).add_modifier(Modifier::BOLD)
}

pub fn graph_aborted() -> Style {
    fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn graph_open() -> Style {
    fg(Color::Yellow).add_modifier(Modifier::BOLD)
}

pub fn graph_none() -> Style {
    fg(Color::DarkGray).add_modifier(Modifier::BOLD)
}

/// Resource managers collapse into four families.  Twenty-two colours would
/// be noise; the families are what one actually scans for.
pub fn rmgr(id: RmgrId) -> Style {
    fg(match id {
        RmgrId::Heap | RmgrId::Heap2 => Color::Green,
        RmgrId::Btree
        | RmgrId::Hash
        | RmgrId::Gin
        | RmgrId::Gist
        | RmgrId::SPGist
        | RmgrId::Brin => Color::Blue,
        RmgrId::Xact | RmgrId::CommitTs | RmgrId::MultiXact | RmgrId::Clog => Color::Magenta,
        _ => Color::Cyan,
    })
}

/// Marks a record carrying a full-page image; those are what make a segment
/// large.  The marker is a character too, so it survives NO_COLOR.
pub fn fpi_marker() -> Style {
    fg(Color::Yellow).add_modifier(Modifier::BOLD)
}

pub fn crc_bad() -> Style {
    fg(Color::Red)
}

pub fn crc_ok() -> Style {
    fg(Color::Green)
}

// --- details ---------------------------------------------------------------

pub fn detail_key() -> Style {
    fg(Color::DarkGray)
}

pub fn detail_value() -> Style {
    Style::default()
}

pub fn lsn() -> Style {
    fg(Color::Cyan)
}

pub fn flags() -> Style {
    fg(Color::Yellow)
}

/// The item the DETAILS cursor is on.
pub fn cursor() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

// --- hex dump --------------------------------------------------------------

pub fn hex_addr() -> Style {
    fg(Color::DarkGray)
}

/// Page headers, zero bytes outside the selected record, and the placeholders
/// in the ASCII column.
pub fn dim() -> Style {
    fg(Color::DarkGray)
}

/// The colour of one part of the selected record.  The block index does not
/// change it: what matters is which kind of bytes these are.
pub fn record_part(part: RecordPart) -> Style {
    fg(match part {
        RecordPart::Header => Color::Magenta,
        RecordPart::Descriptors => Color::Blue,
        RecordPart::Fpi(_) => Color::Yellow,
        RecordPart::BlockData(_) => Color::Cyan,
        RecordPart::MainData => Color::Green,
    })
}

/// Applied on top of the part colour to mark the whole selected record.  Bold
/// rather than reverse video, so reverse stays free for the part under the
/// DETAILS cursor -- and so the record is still identifiable under NO_COLOR.
pub fn selected_record() -> Modifier {
    Modifier::BOLD
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_walview::rmgr::RmgrId;
    use pg_walview::walreader::RecordPart;

    /// https://no-color.org: set and non-empty disables colour, whatever the
    /// value is.
    #[test]
    fn no_color_is_honoured() {
        assert!(colors_enabled_for(None));
        assert!(colors_enabled_for(Some("")));
        assert!(!colors_enabled_for(Some("1")));
        assert!(!colors_enabled_for(Some("0")));
        assert!(!colors_enabled_for(Some("anything")));
    }

    /// Bold and reverse are not colour.  Dropping them along with the colours
    /// would leave no way to see which record is selected.
    #[test]
    fn stripping_colour_keeps_the_attributes() {
        let styled = Style::default()
            .fg(Color::Green)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED);
        let stripped = strip_color(styled);
        assert_eq!(stripped.fg, None);
        assert_eq!(stripped.bg, None);
        assert!(stripped.add_modifier.contains(Modifier::BOLD));
        assert!(stripped.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn resource_managers_fall_into_four_families() {
        let heap = [RmgrId::Heap, RmgrId::Heap2];
        let index = [
            RmgrId::Btree,
            RmgrId::Hash,
            RmgrId::Gin,
            RmgrId::Gist,
            RmgrId::SPGist,
            RmgrId::Brin,
        ];
        let xact = [
            RmgrId::Xact,
            RmgrId::CommitTs,
            RmgrId::MultiXact,
            RmgrId::Clog,
        ];
        let system = [RmgrId::Xlog, RmgrId::Standby, RmgrId::Smgr, RmgrId::Generic];

        for group in [&heap[..], &index[..], &xact[..], &system[..]] {
            let first = rmgr(group[0]).fg;
            assert!(first.is_some());
            for id in group {
                assert_eq!(rmgr(*id).fg, first, "{:?} is in the wrong family", id);
            }
        }
        // The four families are told apart from each other.
        let mut seen = vec![
            rmgr(heap[0]).fg,
            rmgr(index[0]).fg,
            rmgr(xact[0]).fg,
            rmgr(system[0]).fg,
        ];
        seen.sort_by_key(|c| format!("{:?}", c));
        seen.dedup();
        assert_eq!(seen.len(), 4);
    }

    /// The five parts of a record have to be told apart in the dump.
    #[test]
    fn every_record_part_gets_its_own_colour() {
        let parts = [
            RecordPart::Header,
            RecordPart::Descriptors,
            RecordPart::Fpi(0),
            RecordPart::BlockData(0),
            RecordPart::MainData,
        ];
        let mut seen: Vec<String> = parts
            .iter()
            .map(|p| format!("{:?}", record_part(*p).fg))
            .collect();
        assert!(seen.iter().all(|c| c != "None"));
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), parts.len());
        // The block index does not change the colour.
        assert_eq!(
            record_part(RecordPart::Fpi(0)).fg,
            record_part(RecordPart::Fpi(3)).fg
        );
    }
}
