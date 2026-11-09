//! The layout of the viewer for the size of the pane (spec §8.2).

use ratatui::layout::Rect;

/// Below this size, the viewer shows only "pane too small".
pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 10;
/// Below this size, the viewer has one column and the panel opens over the document.
pub const SINGLE_BELOW_WIDTH: u16 = 60;
pub const SINGLE_BELOW_HEIGHT: u16 = 16;
/// From this width, the panel is at the side. Below it, the panel is at the bottom.
pub const SIDE_FROM_WIDTH: u16 = 100;
/// The height of the bottom panel.
pub const BOTTOM_PANEL_ROWS: u16 = 8;

/// The areas of the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    TooSmall,
    /// One column. The panel, when open, covers the document (`overlay`).
    Single {
        header: Rect,
        doc: Rect,
        overlay: Rect,
        footer: Rect,
    },
    /// The panel is below the document. A closed panel has zero height.
    Bottom {
        header: Rect,
        doc: Rect,
        panel: Rect,
        footer: Rect,
    },
    /// The panel is at the right of the document. A closed panel has zero width.
    Side {
        header: Rect,
        doc: Rect,
        panel: Rect,
        footer: Rect,
    },
}

/// The layout for `area`.
pub fn compute(area: Rect, panel_open: bool) -> Layout {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return Layout::TooSmall;
    }
    let header = Rect::new(area.x, area.y, area.width, 1);
    let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    if area.width < SINGLE_BELOW_WIDTH || area.height < SINGLE_BELOW_HEIGHT {
        return Layout::Single { header, doc: body, overlay: body, footer };
    }
    if area.width < SIDE_FROM_WIDTH {
        let rows = if panel_open { BOTTOM_PANEL_ROWS.min(body.height) } else { 0 };
        let doc = Rect::new(body.x, body.y, body.width, body.height - rows);
        let panel = Rect::new(body.x, doc.bottom(), body.width, rows);
        return Layout::Bottom { header, doc, panel, footer };
    }
    let columns = if panel_open { (area.width / 3).clamp(30, 50) } else { 0 };
    let doc = Rect::new(body.x, body.y, body.width - columns, body.height);
    let panel = Rect::new(doc.right(), body.y, columns, body.height);
    Layout::Side { header, doc, panel, footer }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use ratatui::layout::Rect;

    use super::*;

    fn area(width: u16, height: u16) -> Rect {
        Rect::new(0, 0, width, height)
    }

    #[test]
    fn side_at_100_columns() {
        match compute(area(100, 30), true) {
            Layout::Side { header, doc, panel, footer } => {
                assert_eq!(header, Rect::new(0, 0, 100, 1));
                assert_eq!(footer, Rect::new(0, 29, 100, 1));
                assert_eq!(panel, Rect::new(67, 1, 33, 28));
                assert_eq!(doc, Rect::new(0, 1, 67, 28));
            }
            other => panic!("expected Side, got {other:?}"),
        }
        match compute(area(218, 69), true) {
            Layout::Side { panel, .. } => assert_eq!(panel.width, 50),
            other => panic!("expected Side, got {other:?}"),
        }
    }

    #[test]
    fn closed_side_panel_gives_the_document_the_space() {
        match compute(area(120, 40), false) {
            Layout::Side { doc, panel, .. } => {
                assert_eq!(doc.width, 120);
                assert_eq!(panel.width, 0);
            }
            other => panic!("expected Side, got {other:?}"),
        }
    }

    #[test]
    fn bottom_at_99_and_60_columns() {
        for width in [99, 60] {
            match compute(area(width, 24), true) {
                Layout::Bottom { doc, panel, .. } => {
                    assert_eq!(panel, Rect::new(0, 15, width, 8));
                    assert_eq!(doc, Rect::new(0, 1, width, 14));
                }
                other => panic!("expected Bottom at {width}, got {other:?}"),
            }
        }
        match compute(area(80, 24), false) {
            Layout::Bottom { doc, panel, .. } => {
                assert_eq!(doc.height, 22);
                assert_eq!(panel.height, 0);
            }
            other => panic!("expected Bottom, got {other:?}"),
        }
    }

    #[test]
    fn single_at_59_columns() {
        match compute(area(59, 30), true) {
            Layout::Single { doc, overlay, .. } => {
                assert_eq!(doc, Rect::new(0, 1, 59, 28));
                assert_eq!(overlay, doc);
            }
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn single_at_15_rows() {
        assert!(matches!(compute(area(200, 15), true), Layout::Single { .. }));
        assert!(matches!(compute(area(200, 16), true), Layout::Side { .. }));
    }

    #[test]
    fn too_small_at_39_columns_or_9_rows() {
        assert_eq!(compute(area(39, 30), true), Layout::TooSmall);
        assert_eq!(compute(area(80, 9), true), Layout::TooSmall);
        assert_eq!(compute(area(0, 0), true), Layout::TooSmall);
    }

    #[test]
    fn exactly_40_by_10_is_not_too_small() {
        assert!(matches!(compute(area(40, 10), true), Layout::Single { .. }));
    }
}
