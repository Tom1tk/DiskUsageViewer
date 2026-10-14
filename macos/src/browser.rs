//! The state behind the tree and treemap panes: which folders are open, what is selected, and
//! which folder the treemap shows. Pure logic, so it can be tested without a window.
use std::collections::HashSet;
use std::sync::Arc;

use crate::tree::Tree;

/// One visible line of the tree pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub depth: usize,
}

pub struct Browser {
    tree: Arc<Tree>,
    selection: Option<u32>,
    /// Folders whose children are shown. The root starts open.
    expanded: HashSet<u32>,
    rows: Vec<Row>,
    /// Set by anything that changes which rows are visible. `refresh` clears it.
    rows_stale: bool,
    /// Set by `reveal`, so the tree pane scrolls the selected row into view once.
    scroll_pending: bool,
}

impl Browser {
    pub fn new(tree: Arc<Tree>) -> Self {
        let root = tree.root();
        Browser {
            tree,
            selection: Some(root),
            expanded: HashSet::from([root]),
            rows: Vec::new(),
            rows_stale: true,
            scroll_pending: false,
        }
    }

    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn tree_handle(&self) -> &Arc<Tree> {
        &self.tree
    }

    pub fn selection(&self) -> Option<u32> {
        self.selection
    }

    pub fn is_expanded(&self, id: u32) -> bool {
        self.expanded.contains(&id)
    }

    /// The folder the treemap shows: the selection if it is a folder, else the folder holding it.
    pub fn focus(&self) -> u32 {
        match self.selection {
            Some(id) if self.tree.is_dir(id) => id,
            Some(id) => self.tree.parent(id).unwrap_or(self.tree.root()),
            None => self.tree.root(),
        }
    }

    /// Rebuilds the visible rows if anything changed since the last call.
    pub fn refresh(&mut self) {
        if self.rows_stale {
            self.rows.clear();
            self.push_rows(self.tree.root(), 0);
            self.rows_stale = false;
        }
    }

    /// The visible rows. Call `refresh` first after any change.
    pub fn rows(&self) -> &[Row] {
        debug_assert!(!self.rows_stale, "refresh before reading rows");
        &self.rows
    }

    /// The index of the selected row, once the rows are refreshed.
    pub fn selected_row(&self) -> Option<usize> {
        let selected = self.selection?;
        self.rows.iter().position(|row| row.id == selected)
    }

    /// Opens or closes a folder. Files and empty folders have nothing to open.
    pub fn toggle(&mut self, id: u32) {
        if !self.tree.is_dir(id) || self.tree.child_count(id) == 0 {
            return;
        }
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        self.rows_stale = true;
    }

    pub fn select(&mut self, id: u32) {
        self.selection = Some(id);
    }

    /// Selects `id` and opens every folder above it, so its row is visible in the tree.
    pub fn reveal(&mut self, id: u32) {
        let mut above = self.tree.parent(id);
        while let Some(folder) = above {
            self.expanded.insert(folder);
            above = self.tree.parent(folder);
        }
        self.selection = Some(id);
        self.rows_stale = true;
        self.scroll_pending = true;
    }

    /// True once after a `reveal`, so the caller scrolls the tree to the selection a single time.
    pub fn take_scroll_request(&mut self) -> bool {
        std::mem::take(&mut self.scroll_pending)
    }

    fn push_rows(&mut self, id: u32, depth: usize) {
        self.rows.push(Row { id, depth });
        if self.expanded.contains(&id) {
            for child in self.tree.sorted_children(id) {
                self.push_rows(child, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{NO_PARENT, Parts, build};

    /// `/` holds Users/ (tom/ holds a.bin 300 and b.bin 100), Notes.txt 50 and Empty/ (no children).
    /// Ids: 0 root, 1 Users, 2 tom, 3 a.bin, 4 b.bin, 5 Notes.txt, 6 Empty.
    fn sample() -> Arc<Tree> {
        let mut p = Parts::new(7);
        let set = |p: &mut Parts, id: u32, parent: u32, name: &str, size: u64, dir: bool| {
            let units: Vec<u16> = name.encode_utf16().collect();
            p.set(id, parent, &units, size, dir);
        };
        set(&mut p, 0, NO_PARENT, "/", 0, true);
        set(&mut p, 1, 0, "Users", 0, true);
        set(&mut p, 2, 1, "tom", 0, true);
        set(&mut p, 3, 2, "a.bin", 300, false);
        set(&mut p, 4, 2, "b.bin", 100, false);
        set(&mut p, 5, 0, "Notes.txt", 50, false);
        set(&mut p, 6, 0, "Empty", 0, true);
        Arc::new(build(p))
    }

    fn visible(browser: &mut Browser) -> Vec<u32> {
        browser.refresh();
        browser.rows().iter().map(|row| row.id).collect()
    }

    #[test]
    fn root_starts_open_and_shows_its_children_only() {
        let mut browser = Browser::new(sample());
        assert_eq!(visible(&mut browser), vec![0, 1, 5, 6]);
        assert_eq!(browser.selection(), Some(0));
    }

    #[test]
    fn toggling_a_folder_shows_and_hides_its_children() {
        let mut browser = Browser::new(sample());
        browser.toggle(1);
        assert_eq!(visible(&mut browser), vec![0, 1, 2, 5, 6]);
        browser.toggle(1);
        assert_eq!(visible(&mut browser), vec![0, 1, 5, 6]);
    }

    #[test]
    fn toggling_a_file_or_an_empty_folder_does_nothing() {
        let mut browser = Browser::new(sample());
        browser.toggle(5);
        browser.toggle(6);
        assert_eq!(visible(&mut browser), vec![0, 1, 5, 6]);
        assert!(!browser.is_expanded(6));
    }

    #[test]
    fn reveal_opens_the_path_and_selects_the_entry() {
        let mut browser = Browser::new(sample());
        browser.reveal(4);
        assert_eq!(visible(&mut browser), vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(browser.selection(), Some(4));
        assert_eq!(browser.selected_row(), Some(4));
        assert!(browser.take_scroll_request());
        assert!(!browser.take_scroll_request(), "the scroll request is taken once");
    }

    #[test]
    fn focus_is_the_folder_or_the_parent_of_a_file() {
        let mut browser = Browser::new(sample());
        browser.select(2);
        assert_eq!(browser.focus(), 2, "a folder shows itself");
        browser.select(4);
        assert_eq!(browser.focus(), 2, "a file shows the folder holding it");
        browser.select(5);
        assert_eq!(browser.focus(), 0);
    }
}
