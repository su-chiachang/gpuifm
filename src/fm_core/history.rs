use std::path::{Path, PathBuf};

pub struct History {
    current: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
}

impl History {
    pub fn new(p: PathBuf) -> Self {
        Self { current: p, back: Vec::new(), forward: Vec::new() }
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    /// Returns false (no change) if `p` is the current directory.
    pub fn navigate(&mut self, p: PathBuf) -> bool {
        if p == self.current {
            return false;
        }
        let old = std::mem::replace(&mut self.current, p);
        self.back.push(old);
        self.forward.clear();
        true
    }

    /// Goes back; returns the directory that was left (to select it).
    pub fn back(&mut self) -> Option<PathBuf> {
        let prev = self.back.pop()?;
        let left = std::mem::replace(&mut self.current, prev);
        self.forward.push(left.clone());
        Some(left)
    }

    /// Goes forward; returns the directory that was left.
    pub fn forward(&mut self) -> Option<PathBuf> {
        let next = self.forward.pop()?;
        let left = std::mem::replace(&mut self.current, next);
        self.back.push(left.clone());
        Some(left)
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    pub fn replace(&mut self, p: PathBuf) {
        self.current = p;
    }
}

pub fn parent_of(p: &Path) -> Option<PathBuf> {
    p.parent().map(Path::to_path_buf).filter(|x| !x.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn same_no_push() {
        let mut h = History::new(p("/a"));
        assert!(!h.navigate(p("/a")));
        assert!(!h.can_back());
    }

    #[test]
    fn back_returns_left() {
        let mut h = History::new(p("/a"));
        h.navigate(p("/a/b"));
        assert_eq!(h.back(), Some(p("/a/b")));
        assert_eq!(h.current(), Path::new("/a"));
        assert_eq!(h.back(), None);
    }

    #[test]
    fn navigate_clears_forward() {
        let mut h = History::new(p("/a"));
        h.navigate(p("/b"));
        h.back();
        assert!(h.can_forward());
        h.navigate(p("/c"));
        assert!(!h.can_forward());
    }

    #[test]
    fn round_trip() {
        let mut h = History::new(p("/a"));
        h.navigate(p("/b"));
        h.navigate(p("/c"));
        h.back();
        h.back();
        assert_eq!(h.current(), Path::new("/a"));
        assert!(h.can_forward() && !h.can_back());
        assert_eq!(h.forward(), Some(p("/a")));
        h.forward();
        assert_eq!(h.current(), Path::new("/c"));
        assert_eq!(h.forward(), None);
        assert!(h.can_back());
    }

    #[test]
    fn replace_keeps_stacks() {
        let mut h = History::new(p("/a"));
        h.navigate(p("/b"));
        h.replace(p("/x"));
        assert_eq!(h.current(), Path::new("/x"));
        assert!(h.can_back());
    }

    #[test]
    fn parents() {
        assert_eq!(parent_of(Path::new("/")), None);
        assert_eq!(parent_of(Path::new("/a")), Some(p("/")));
        assert_eq!(parent_of(Path::new("/a/b")), Some(p("/a")));
    }
}
