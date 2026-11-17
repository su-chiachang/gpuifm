use super::entry::FileEntry;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Multi-selection over list indices. `cursor` is the keyboard focus, `anchor` the fixed end of a
/// shift-range.
#[derive(Default)]
pub struct Selection {
    cursor: Option<usize>,
    anchor: Option<usize>,
    set: BTreeSet<usize>,
}

impl Selection {
    pub fn get(&self) -> Option<usize> {
        self.cursor
    }

    /// Single selection (or clear with None).
    pub fn set(&mut self, i: Option<usize>) {
        self.cursor = i;
        self.anchor = i;
        self.set.clear();
        if let Some(i) = i {
            self.set.insert(i);
        }
    }

    pub fn contains(&self, i: usize) -> bool {
        self.set.contains(&i)
    }

    pub fn indices(&self) -> Vec<usize> {
        self.set.iter().copied().collect()
    }

    pub fn count(&self) -> usize {
        self.set.len()
    }

    pub fn clear(&mut self) {
        self.set(None);
    }

    /// Cmd-click: add or remove one row, leaving the rest alone.
    pub fn toggle(&mut self, i: usize) {
        if self.set.remove(&i) {
            self.cursor = self.set.iter().next_back().copied();
        } else {
            self.set.insert(i);
            self.cursor = Some(i);
        }
        self.anchor = self.cursor;
    }

    /// Shift-click / shift-arrow: select anchor..=i (either direction).
    pub fn extend_to(&mut self, i: usize) {
        let a = self.anchor.or(self.cursor).unwrap_or(i);
        let (lo, hi) = if a <= i { (a, i) } else { (i, a) };
        self.set = (lo..=hi).collect();
        self.anchor = Some(a);
        self.cursor = Some(i);
    }

    pub fn select_all(&mut self, len: usize) {
        if len == 0 {
            self.clear();
            return;
        }
        self.set = (0..len).collect();
        self.anchor = Some(0);
        self.cursor = Some(self.cursor.map_or(0, |c| c.min(len - 1)));
    }

    pub fn move_by(&mut self, delta: isize, len: usize, extend: bool) {
        if len == 0 {
            self.clear();
            return;
        }
        let last = len - 1;
        let next = match self.cursor {
            None if delta >= 0 => 0,
            None => last,
            Some(i) => (i as isize + delta).clamp(0, last as isize) as usize,
        };
        if extend {
            self.extend_to(next);
        } else {
            self.set(Some(next));
        }
    }

    /// Re-select by path after a reload; else clamp the old cursor; else None.
    pub fn restore(&mut self, entries: &[FileEntry], paths: &[PathBuf], cursor: Option<&Path>) {
        if entries.is_empty() {
            self.clear();
            return;
        }
        let set: BTreeSet<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| paths.contains(&e.path))
            .map(|(i, _)| i)
            .collect();
        if !set.is_empty() {
            let cur = cursor
                .and_then(|c| entries.iter().position(|e| e.path == c))
                .filter(|i| set.contains(i))
                .or_else(|| set.iter().next_back().copied());
            self.cursor = cur;
            self.anchor = cur;
            self.set = set;
            return;
        }
        let c = self.cursor.map(|i| i.min(entries.len() - 1));
        self.set(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ents(names: &[&str]) -> Vec<FileEntry> {
        names
            .iter()
            .map(|n| FileEntry {
                path: PathBuf::from(format!("/d/{n}")),
                name: (*n).into(),
                is_dir: false,
                is_package: false,
                is_hidden: false,
                is_symlink: false,
                size: Some(0),
                modified: None,
                kind: "Document".into(),
            })
            .collect()
    }

    #[test]
    fn clamp_edges() {
        let mut s = Selection::default();
        s.set(Some(0));
        s.move_by(-1, 3, false);
        assert_eq!(s.get(), Some(0));
        s.set(Some(2));
        s.move_by(1, 3, false);
        assert_eq!(s.get(), Some(2));
    }

    #[test]
    fn none_start() {
        let mut s = Selection::default();
        s.move_by(1, 3, false);
        assert_eq!(s.get(), Some(0));
        let mut s = Selection::default();
        s.move_by(-1, 3, false);
        assert_eq!(s.get(), Some(2));
    }

    #[test]
    fn empty_noop() {
        let mut s = Selection::default();
        s.move_by(1, 0, false);
        assert_eq!(s.get(), None);
    }

    #[test]
    fn restore_cases() {
        let e = ents(&["a", "b", "c"]);
        let mut s = Selection::default();
        s.set(Some(0));
        s.restore(&e, &[PathBuf::from("/d/c")], None);
        assert_eq!(s.get(), Some(2));
        s.restore(&e[..2], &[PathBuf::from("/d/zzz")], None);
        assert_eq!(s.get(), Some(1));
        s.restore(&[], &[], None);
        assert_eq!(s.get(), None);
        s.restore(&e, &[], None);
        assert_eq!(s.get(), None);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut s = Selection::default();
        s.set(Some(1));
        s.toggle(3);
        assert_eq!(s.indices(), [1, 3]);
        assert_eq!(s.get(), Some(3));
        s.toggle(3);
        assert_eq!(s.indices(), [1]);
        assert_eq!(s.get(), Some(1));
        s.toggle(1);
        assert_eq!(s.count(), 0);
        assert_eq!(s.get(), None);
    }

    #[test]
    fn extend_both_directions() {
        let mut s = Selection::default();
        s.set(Some(3));
        s.extend_to(5);
        assert_eq!(s.indices(), [3, 4, 5]);
        s.extend_to(1);
        assert_eq!(s.indices(), [1, 2, 3]);
        assert_eq!(s.get(), Some(1));
        assert!(s.contains(2) && !s.contains(4));
    }

    #[test]
    fn select_all_and_clear() {
        let mut s = Selection::default();
        s.select_all(4);
        assert_eq!(s.indices(), [0, 1, 2, 3]);
        assert_eq!(s.count(), 4);
        s.clear();
        assert_eq!(s.count(), 0);
        s.select_all(0);
        assert_eq!(s.get(), None);
    }

    #[test]
    fn move_by_extend() {
        let mut s = Selection::default();
        s.set(Some(1));
        s.move_by(1, 5, true);
        s.move_by(1, 5, true);
        assert_eq!(s.indices(), [1, 2, 3]);
        s.move_by(-1, 5, true);
        assert_eq!(s.indices(), [1, 2]);
        s.move_by(-3, 5, true);
        assert_eq!(s.indices(), [0, 1]);
        s.move_by(1, 5, false);
        assert_eq!(s.indices(), [1]);
    }

    #[test]
    fn restore_multi() {
        let e = ents(&["a", "b", "c", "d"]);
        let mut s = Selection::default();
        s.restore(&e, &[PathBuf::from("/d/b"), PathBuf::from("/d/d")], Some(Path::new("/d/b")));
        assert_eq!(s.indices(), [1, 3]);
        assert_eq!(s.get(), Some(1));
        s.restore(&e[..2], &[PathBuf::from("/d/b"), PathBuf::from("/d/d")], None);
        assert_eq!(s.indices(), [1]);
    }
}
