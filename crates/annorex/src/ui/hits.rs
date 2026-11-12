//! The hit map: which screen area maps to which clickable thing.
//!
//! Each draw clears the map and pushes the areas that it draws. Mouse handling reads the map.

use ratatui::layout::{Position, Rect};

use crate::keys::Action;
use crate::review::Kind;

/// A thing that a click can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hit {
    Block(usize),
    DocCell {
        row: u16,
        col: u16,
    },
    Action(Kind),
    /// The action bar row outside its buttons. A click there does nothing.
    Bar,
    Card(u64),
    CardEdit(u64),
    CardRemove(u64),
    Pick(u64),
    Send,
    TreeRow(usize),
    ListRow(usize),
    Hint(Action),
    EditorSave,
    EditorCancel,
    BoxInside,
    BoxOutside,
    /// The docked tree, the side annotation list, and the card rail. A row or a card lies over it.
    /// A click there does nothing, and the wheel scrolls the part.
    TreePane,
    NotesPane,
    RailPane,
}

/// The areas of the last frame, in the order of drawing.
#[derive(Debug, Default)]
pub(crate) struct HitMap {
    areas: Vec<(Rect, Hit)>,
}

impl HitMap {
    /// Remove all areas.
    pub(crate) fn clear(&mut self) {
        self.areas.clear();
    }

    /// Add an area. It lies over the areas that are already in the map.
    pub(crate) fn push(&mut self, area: Rect, hit: Hit) {
        self.areas.push((area, hit));
    }

    /// The hit at a point. The area that was pushed last wins.
    pub(crate) fn at(&self, column: u16, row: u16) -> Option<&Hit> {
        let point = Position::new(column, row);
        self.areas.iter().rev().find(|(area, _)| area.contains(point)).map(|(_, hit)| hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_push_wins() {
        let mut map = HitMap::default();
        map.push(Rect::new(0, 0, 10, 10), Hit::BoxOutside);
        map.push(Rect::new(2, 2, 3, 3), Hit::BoxInside);
        assert_eq!(map.at(3, 3), Some(&Hit::BoxInside));
        assert_eq!(map.at(0, 0), Some(&Hit::BoxOutside));
    }

    #[test]
    fn point_outside_gives_none() {
        let mut map = HitMap::default();
        map.push(Rect::new(2, 2, 3, 3), Hit::Send);
        assert_eq!(map.at(5, 3), None);
        assert_eq!(map.at(3, 5), None);
        assert_eq!(map.at(1, 1), None);
    }

    #[test]
    fn clear_empties() {
        let mut map = HitMap::default();
        map.push(Rect::new(0, 0, 4, 4), Hit::Send);
        map.clear();
        assert_eq!(map.at(1, 1), None);
    }
}
