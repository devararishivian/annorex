//! The layout of the viewer for the size of the pane.

use ratatui::layout::Rect;

/// Below this size, the viewer shows only "pane too small".
pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 10;
/// Below this height, the viewer has one column at all widths.
pub const SINGLE_BELOW_HEIGHT: u16 = 16;
/// From this width, the tree is on the left of the document. Below it, the viewer has one column, and
/// the tree and the annotation list open over the document.
pub const SIDE_FROM_WIDTH: u16 = 100;
/// From this width, the card rail is on the right of the document.
pub const RAIL_FROM_WIDTH: u16 = 120;
/// The width of the tree on the left, with its border.
pub const TREE_WIDTH: u16 = 28;
/// The width of the card rail on the right.
pub const RAIL_WIDTH: u16 = 34;
/// The largest width of the document area, with its margin and gutter. A wider space stays empty on
/// the right, so the card rail sits beside the text.
pub const DOC_MAX_WIDTH: u16 = 100;

/// The areas of the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    TooSmall,
    /// One column. The tree, the annotation list, and the boxes open over the document.
    Single {
        header: Rect,
        doc: Rect,
        footer: Rect,
    },
    /// The tree on the left when the view has files and the tree is open, the document, and the card
    /// rail on the right from `RAIL_FROM_WIDTH` columns.
    Wide {
        header: Rect,
        tree: Option<Rect>,
        doc: Rect,
        rail: Option<Rect>,
        footer: Rect,
    },
}

impl Layout {
    /// The header and the footer, or `None` when the pane is too small.
    pub fn header_footer(&self) -> Option<(Rect, Rect)> {
        match *self {
            Layout::TooSmall => None,
            Layout::Single { header, footer, .. } | Layout::Wide { header, footer, .. } => {
                Some((header, footer))
            }
        }
    }
}

/// The layout for `area`. `has_tree` is true when the view has files. `tree_open` is false after the
/// user hid the tree.
pub fn compute(area: Rect, has_tree: bool, tree_open: bool) -> Layout {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return Layout::TooSmall;
    }
    let header = Rect::new(area.x, area.y, area.width, 1);
    let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    if area.width < SIDE_FROM_WIDTH || area.height < SINGLE_BELOW_HEIGHT {
        return Layout::Single { header, doc: body, footer };
    }
    let tree = (has_tree && tree_open).then(|| Rect::new(body.x, body.y, TREE_WIDTH, body.height));
    let has_rail = area.width >= RAIL_FROM_WIDTH;
    let left = tree.map_or(body.x, Rect::right);
    let room = body.right() - left - if has_rail { RAIL_WIDTH } else { 0 };
    let doc = Rect::new(left, body.y, room.min(DOC_MAX_WIDTH), body.height);
    let rail = has_rail.then(|| Rect::new(doc.right(), body.y, RAIL_WIDTH, body.height));
    Layout::Wide { header, tree, doc, rail, footer }
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
    fn tree_doc_and_rail_at_120_columns() {
        match compute(area(120, 40), true, true) {
            Layout::Wide { header, tree, doc, rail, footer } => {
                assert_eq!(header, Rect::new(0, 0, 120, 1));
                assert_eq!(footer, Rect::new(0, 39, 120, 1));
                assert_eq!(tree, Some(Rect::new(0, 1, 28, 38)));
                assert_eq!(rail, Some(Rect::new(86, 1, 34, 38)));
                assert_eq!(doc, Rect::new(28, 1, 58, 38));
            }
            other => panic!("expected Wide, got {other:?}"),
        }
    }

    #[test]
    fn the_document_width_is_capped_at_240_columns() {
        match compute(area(240, 40), true, true) {
            Layout::Wide { header, tree, doc, rail, .. } => {
                assert_eq!(header.width, 240);
                assert_eq!(tree, Some(Rect::new(0, 1, 28, 38)));
                assert_eq!(doc, Rect::new(28, 1, DOC_MAX_WIDTH, 38));
                assert_eq!(rail, Some(Rect::new(doc.right(), 1, 34, 38)));
                assert!(rail.is_some_and(|rail| rail.right() < 240));
            }
            other => panic!("expected Wide, got {other:?}"),
        }
        match compute(area(240, 40), false, true) {
            Layout::Wide { tree, doc, rail, .. } => {
                assert_eq!(tree, None);
                assert_eq!(doc, Rect::new(0, 1, DOC_MAX_WIDTH, 38));
                assert_eq!(rail, Some(Rect::new(100, 1, 34, 38)));
            }
            other => panic!("expected Wide, got {other:?}"),
        }
    }

    #[test]
    fn the_cap_applies_without_a_rail() {
        match compute(area(119, 40), false, true) {
            Layout::Wide { doc, rail, .. } => {
                assert_eq!(rail, None);
                assert_eq!(doc, Rect::new(0, 1, DOC_MAX_WIDTH, 38));
            }
            other => panic!("expected Wide, got {other:?}"),
        }
    }

    #[test]
    fn tree_and_doc_from_100_to_119_columns() {
        for width in [100, 119] {
            match compute(area(width, 30), true, true) {
                Layout::Wide { tree, doc, rail, .. } => {
                    assert_eq!(tree, Some(Rect::new(0, 1, 28, 28)));
                    assert_eq!(rail, None);
                    assert_eq!(doc, Rect::new(28, 1, width - 28, 28));
                }
                other => panic!("expected Wide at {width}, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_hidden_tree_or_no_files_gives_the_document_the_space() {
        for (has_tree, tree_open) in [(true, false), (false, true)] {
            match compute(area(120, 40), has_tree, tree_open) {
                Layout::Wide { tree, doc, .. } => {
                    assert_eq!(tree, None);
                    assert_eq!(doc, Rect::new(0, 1, 86, 38));
                }
                other => panic!("expected Wide, got {other:?}"),
            }
        }
    }

    #[test]
    fn one_column_below_100_columns_or_16_rows() {
        match compute(area(99, 30), true, true) {
            Layout::Single { doc, .. } => assert_eq!(doc, Rect::new(0, 1, 99, 28)),
            other => panic!("expected Single, got {other:?}"),
        }
        assert!(matches!(compute(area(200, 15), true, true), Layout::Single { .. }));
        assert!(matches!(compute(area(200, 16), true, true), Layout::Wide { .. }));
    }

    #[test]
    fn too_small_at_39_columns_or_9_rows() {
        assert_eq!(compute(area(39, 30), true, true), Layout::TooSmall);
        assert_eq!(compute(area(80, 9), true, true), Layout::TooSmall);
        assert_eq!(compute(area(0, 0), true, true), Layout::TooSmall);
    }

    #[test]
    fn exactly_40_by_10_is_not_too_small() {
        assert!(matches!(compute(area(40, 10), true, true), Layout::Single { .. }));
    }
}
