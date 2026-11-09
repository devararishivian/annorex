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
}

impl Tree {
    /// Put the result of a scan in the tree. The open directories stay open.
    pub(crate) fn load(&mut self, scan: Scan) {
        self.entries = scan.entries;
        self.truncated = scan.truncated;
        self.loaded = true;
        self.index = self.index.min(self.visible().len().saturating_sub(1));
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
                .filter_map(|(index, _)| files.get(index).map(|&entry| Row { entry, depth: 0 }))
                .collect();
        }
        self.entries
            .iter()
            .filter(|entry| self.is_shown(&entry.path))
            .map(|entry| Row { entry, depth: entry.path.components().count().saturating_sub(1) })
            .collect()
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
    pub(crate) fn file_paths(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| entry.path.display().to_string())
            .collect()
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
