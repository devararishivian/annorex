//! The file tree and the fuzzy file finder. They hold their own state. The app decides what the
//! keys do, and the drawing code reads the rows.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::files::{Entry, Scan};
use crate::fuzzy;

/// One line of the tree.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Row<'a> {
    pub(crate) entry: &'a Entry,
    /// How many directories lie above the entry. A filtered list has depth 0.
    pub(crate) depth: usize,
    /// True for a row of the section at the top: a file that the agent changed.
    pub(crate) changed_section: bool,
}

/// The tree of the work directory with the open directories and the line under the cursor.
#[derive(Debug, Default)]
pub(crate) struct Tree {
    entries: Vec<Entry>,
    truncated: bool,
    loaded: bool,
    expanded: HashSet<PathBuf>,
    pub(crate) index: usize,
    filter: Option<String>,
    /// A directory to show with the cursor on it, when the scan has it.
    reveal: Option<PathBuf>,
    /// The files that the agent changed in its last turns, newest first.
    changed: Vec<PathBuf>,
}

impl Tree {
    /// Put the result of a scan in the tree. The open directories stay open.
    pub(crate) fn load(&mut self, scan: Scan) {
        self.entries = scan.entries;
        self.truncated = scan.truncated;
        self.loaded = true;
        self.index = self.index.min(self.visible().len().saturating_sub(1));
        self.place_cursor();
    }

    /// Set the files that the agent changed. A file that the scan does not have is not shown.
    /// The cursor stays on the same row when the section grows or shrinks above it.
    pub(crate) fn set_changed(&mut self, changed: Vec<PathBuf>) {
        let before = self.visible().get(self.index).map(|row| (row.entry.path.clone(), row.changed_section));
        self.changed = changed;
        let rows = self.visible();
        let same = before.and_then(|(path, section)| {
            rows.iter().position(|row| row.entry.path == path && row.changed_section == section)
        });
        self.index = same.unwrap_or_else(|| self.index.min(rows.len().saturating_sub(1)));
    }

    /// True when the scan has the file.
    pub(crate) fn has_file(&self, path: &Path) -> bool {
        self.entries.iter().any(|entry| !entry.is_dir && entry.path == path)
    }

    /// True when the agent changed the file in its last turns.
    pub(crate) fn is_changed(&self, path: &Path) -> bool {
        self.changed.iter().any(|changed| changed == path)
    }

    /// The changed files that the tree has, in order.
    fn changed_entries(&self) -> Vec<&Entry> {
        self.changed
            .iter()
            .filter_map(|path| self.entries.iter().find(|entry| !entry.is_dir && &entry.path == path))
            .collect()
    }

    /// The number of rows in the changed section.
    pub(crate) fn changed_count(&self) -> usize {
        self.changed_entries().len()
    }

    /// Open a directory and its parents, and put the cursor on it. The cursor moves when the scan ends
    /// if it has not ended.
    pub(crate) fn reveal(&mut self, directory: &Path) {
        if directory.as_os_str().is_empty() {
            return;
        }
        for parent in directory.ancestors().filter(|parent| !parent.as_os_str().is_empty()) {
            self.expanded.insert(parent.to_path_buf());
        }
        self.reveal = Some(directory.to_path_buf());
        if self.loaded {
            self.place_cursor();
        }
    }

    fn place_cursor(&mut self) {
        let Some(target) = self.reveal.take() else { return };
        if let Some(index) = self.visible().iter().position(|row| row.entry.path == target) {
            self.index = index;
        }
    }

    /// False until a scan ended.
    pub(crate) fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// The text of the filter, when the filter is on.
    pub(crate) fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    pub(crate) fn start_filter(&mut self) {
        self.filter = Some(String::new());
        self.index = 0;
    }

    pub(crate) fn stop_filter(&mut self) {
        self.filter = None;
        self.index = 0;
    }

    pub(crate) fn type_char(&mut self, c: char) {
        if let Some(filter) = &mut self.filter {
            filter.push(c);
            self.index = 0;
        }
    }

    pub(crate) fn backspace(&mut self) {
        if let Some(filter) = &mut self.filter {
            filter.pop();
            self.index = 0;
        }
    }

    /// The lines to show. Without a filter: the entries whose directories are all open. With a
    /// filter: the files whose full path matches, best first.
    pub(crate) fn visible(&self) -> Vec<Row<'_>> {
        if let Some(filter) = self.filter.as_deref().filter(|text| !text.is_empty()) {
            let files: Vec<&Entry> = self.entries.iter().filter(|entry| !entry.is_dir).collect();
            let names: Vec<String> = files.iter().map(|entry| entry.path.display().to_string()).collect();
            return fuzzy::rank(filter, &names)
                .into_iter()
                .filter_map(|(index, _)| {
                    files.get(index).map(|&entry| Row { entry, depth: 0, changed_section: false })
                })
                .collect();
        }
        // While the user types a filter, also before the first letter, the changed section is not shown.
        let changed = if self.filter.is_some() { Vec::new() } else { self.changed_entries() };
        let top = changed.into_iter().map(|entry| Row { entry, depth: 0, changed_section: true });
        let all = self.entries.iter().filter(|entry| self.is_shown(&entry.path)).map(|entry| Row {
            entry,
            depth: entry.path.components().count().saturating_sub(1),
            changed_section: false,
        });
        top.chain(all).collect()
    }

    fn is_shown(&self, path: &Path) -> bool {
        path.ancestors()
            .skip(1)
            .filter(|parent| !parent.as_os_str().is_empty())
            .all(|parent| self.expanded.contains(parent))
    }

    /// The entry under the cursor.
    pub(crate) fn selected(&self) -> Option<&Entry> {
        self.visible().get(self.index).map(|row| row.entry)
    }

    pub(crate) fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub(crate) fn move_by(&mut self, delta: isize) {
        let last = self.visible().len().saturating_sub(1);
        self.index = self.index.saturating_add_signed(delta).min(last);
    }

    /// Open or close the directory under the cursor.
    pub(crate) fn toggle(&mut self) {
        if let Some(entry) = self.selected().filter(|entry| entry.is_dir) {
            let path = entry.path.clone();
            if !self.expanded.remove(&path) {
                self.expanded.insert(path);
            }
        }
    }

    pub(crate) fn expand(&mut self) {
        if let Some(entry) = self.selected().filter(|entry| entry.is_dir) {
            self.expanded.insert(entry.path.clone());
        }
    }

    /// Close the directory under the cursor. On a file or a closed directory the cursor goes to
    /// the directory above.
    pub(crate) fn collapse(&mut self) {
        let Some(entry) = self.selected() else { return };
        let path = entry.path.clone();
        if entry.is_dir && self.expanded.remove(&path) {
            return;
        }
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty())
            && let Some(index) = self.visible().iter().position(|row| row.entry.path == parent)
        {
            self.index = index;
        }
    }

    /// The paths of all files, for the finder.
    /// The files that the agent changed come first.
    pub(crate) fn file_paths(&self) -> Vec<String> {
        let top = self.changed_entries();
        let rest = self.entries.iter().filter(|entry| !entry.is_dir && !top.contains(entry));
        top.iter().copied().chain(rest).map(|entry| entry.path.display().to_string()).collect()
    }
}

/// The fuzzy find of a file by path.
#[derive(Debug, Default)]
pub(crate) struct Finder {
    pub(crate) query: String,
    paths: Vec<String>,
    results: Vec<usize>,
    pub(crate) index: usize,
}

impl Finder {
    pub(crate) fn new(paths: Vec<String>) -> Self {
        let mut finder = Self { query: String::new(), paths, results: Vec::new(), index: 0 };
        finder.refresh();
        finder
    }

    /// Give the finder a new list of paths. The query stays.
    pub(crate) fn set_paths(&mut self, paths: Vec<String>) {
        let selected = self.selected().map(str::to_owned);
        self.paths = paths;
        self.refresh();
        let position = selected.and_then(|selected| self.results().position(|path| path == selected));
        if let Some(position) = position {
            self.index = position;
        }
    }

    fn refresh(&mut self) {
        self.results = fuzzy::rank(&self.query, &self.paths).into_iter().map(|(index, _)| index).collect();
        self.index = 0;
    }

    pub(crate) fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.refresh();
    }

    pub(crate) fn backspace(&mut self) {
        self.query.pop();
        self.refresh();
    }

    pub(crate) fn move_by(&mut self, delta: isize) {
        let last = self.results.len().saturating_sub(1);
        self.index = self.index.saturating_add_signed(delta).min(last);
    }

    /// The matching paths, best first.
    pub(crate) fn results(&self) -> impl Iterator<Item = &str> {
        self.results.iter().filter_map(|&index| self.paths.get(index).map(String::as_str))
    }

    pub(crate) fn selected(&self) -> Option<&str> {
        self.results.get(self.index).and_then(|&index| self.paths.get(index)).map(String::as_str)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn file(path: &str) -> Entry {
        Entry { path: PathBuf::from(path), is_dir: false }
    }
    fn tree(paths: &[&str]) -> Tree {
        let mut tree = Tree::default();
        tree.load(Scan { entries: paths.iter().map(|p| file(p)).collect(), truncated: false });
        tree
    }
    fn selected(tree: &Tree) -> PathBuf {
        tree.selected().expect("a row").path.clone()
    }

    #[test]
    fn the_cursor_stays_on_its_file_when_changed_rows_appear_above() {
        let mut tree = tree(&["a.rs", "b.rs", "c.rs"]);
        tree.index = 2;
        assert_eq!(selected(&tree), PathBuf::from("c.rs"));
        tree.set_changed(vec!["b.rs".into(), "a.rs".into()]);
        assert_eq!(selected(&tree), PathBuf::from("c.rs"));
        tree.set_changed(Vec::new());
        assert_eq!(selected(&tree), PathBuf::from("c.rs"));
    }

    #[test]
    fn the_cursor_on_a_changed_row_stays_in_the_section() {
        let mut tree = tree(&["a.rs", "b.rs"]);
        tree.set_changed(vec!["b.rs".into()]);
        tree.index = 0;
        tree.set_changed(vec!["a.rs".into(), "b.rs".into()]);
        assert_eq!(selected(&tree), PathBuf::from("b.rs"));
        assert!(tree.visible()[tree.index].changed_section);
    }

    #[test]
    fn an_empty_filter_result_has_no_changed_section() {
        let mut tree = tree(&["a.rs", "b.rs"]);
        tree.set_changed(vec!["b.rs".into()]);
        assert!(tree.visible().iter().any(|row| row.changed_section));
        tree.start_filter();
        assert!(!tree.visible().iter().any(|row| row.changed_section), "no section while the filter is on");
        assert_eq!(tree.visible().len(), 2, "the normal tree stays complete");
    }

    #[test]
    fn the_finder_keeps_the_selected_path_when_the_list_changes() {
        let mut finder = Finder::new(vec!["a.rs".into(), "b.rs".into(), "c.rs".into()]);
        finder.move_by(2);
        assert_eq!(finder.selected(), Some("c.rs"));
        finder.set_paths(vec!["x.rs".into(), "a.rs".into(), "b.rs".into(), "c.rs".into()]);
        assert_eq!(finder.selected(), Some("c.rs"));
    }
}
