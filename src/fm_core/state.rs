//! Persisted app state (favorites, pins, pane layout) as JSON. Pure core, no UI deps.

use super::layout::{Axis, LayoutTree};
use super::places::default_favorites;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarEntry {
    pub path: PathBuf,
    #[serde(default)]
    pub title: Option<String>,
}

impl SidebarEntry {
    /// Title to show: explicit title, else the folder's leaf name.
    pub fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            self.path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.path.display().to_string())
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    List,
    Column,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneSnapshot {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
    pub view_mode: ViewMode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedState {
    pub favorites: Vec<SidebarEntry>,
    pub pinned: Vec<SidebarEntry>,
    pub layout: LayoutTree<PaneSnapshot>,
    pub show_hidden: bool,
    pub sidebar_visible: bool,
}

impl Default for PersistedState {
    /// Used for fields missing from an otherwise valid file: nothing pinned, no panes (restore
    /// then falls back to a single pane at home).
    fn default() -> Self {
        Self {
            favorites: Vec::new(),
            pinned: Vec::new(),
            layout: LayoutTree::Leaf(PaneSnapshot::default()),
            show_hidden: false,
            sidebar_visible: true,
        }
    }
}

fn pane(p: PathBuf) -> LayoutTree<PaneSnapshot> {
    LayoutTree::Leaf(PaneSnapshot { tabs: vec![p], active: 0, view_mode: ViewMode::List })
}

/// First-run state: four favorites and a 2x2 grid of panes on them.
pub fn defaults(home: &Path) -> PersistedState {
    let favorites = default_favorites(home);
    let p: Vec<PathBuf> = favorites.iter().map(|e| e.path.clone()).collect();
    let row = |a: &PathBuf, b: &PathBuf| LayoutTree::split(Axis::SideBySide, vec![pane(a.clone()), pane(b.clone())]);
    PersistedState {
        layout: LayoutTree::split(Axis::Stacked, vec![row(&p[0], &p[1]), row(&p[2], &p[3])]),
        favorites,
        pinned: Vec::new(),
        show_hidden: false,
        sidebar_visible: true,
    }
}

/// `$NANOFM_STATE_DIR` (if set and non-empty), else `~/Library/Application Support/nanofm-rs`.
/// Never the Swift app's `nanofm` directory (different schema).
pub fn state_dir(env: Option<&OsStr>, home: &Path) -> PathBuf {
    match env {
        Some(e) if !e.is_empty() => PathBuf::from(e),
        _ => home.join("Library").join("Application Support").join("nanofm-rs"),
    }
}

pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn file(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    /// Missing or corrupt file -> defaults for `home`. Never touches anything but state.json.
    pub fn load_with_home(&self, home: &Path) -> PersistedState {
        let Ok(bytes) = fs::read(self.file()) else { return defaults(home) };
        let Ok(mut s) = serde_json::from_slice::<PersistedState>(&bytes) else { return defaults(home) };
        match s.layout.clone().normalized() {
            Some(l) => s.layout = l,
            None => s.layout = defaults(home).layout,
        }
        s
    }

    pub fn load(&self) -> PersistedState {
        self.load_with_home(&super::places::home_dir())
    }

    /// Atomic write: temp file in the same directory, then rename.
    pub fn save(&self, s: &PersistedState) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let json = serde_json::to_vec_pretty(s).map_err(io::Error::other)?;
        let tmp = self.dir.join("state.json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, self.file())
    }
}

/// Drops tabs whose folder is gone, panes left without tabs, and normalizes. A dropped active
/// tab falls back to the nearest earlier kept tab (else the first kept one). None when nothing
/// survives.
pub fn restore(s: &PersistedState, is_dir: impl Fn(&Path) -> bool) -> Option<LayoutTree<PaneSnapshot>> {
    s.layout.filter_map_leaves(|snap| {
        let kept: Vec<usize> = (0..snap.tabs.len()).filter(|&i| is_dir(&snap.tabs[i])).collect();
        if kept.is_empty() {
            return None;
        }
        let active = kept.iter().rposition(|&i| i <= snap.active).unwrap_or(0);
        Some(PaneSnapshot {
            tabs: kept.iter().map(|&i| snap.tabs[i].clone()).collect(),
            active,
            view_mode: snap.view_mode,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(tabs: &[&str], active: usize) -> PaneSnapshot {
        PaneSnapshot { tabs: tabs.iter().map(PathBuf::from).collect(), active, view_mode: ViewMode::List }
    }

    #[test]
    fn defaults_shape() {
        let home = Path::new("/h");
        let d = defaults(home);
        assert_eq!(d.favorites.len(), 4);
        assert!(!d.show_hidden);
        assert!(d.pinned.is_empty());
        match &d.layout {
            LayoutTree::Split { axis: Axis::Stacked, children, .. } => {
                assert_eq!(children.len(), 2);
                for c in children {
                    assert!(matches!(c, LayoutTree::Split { axis: Axis::SideBySide, children, .. } if children.len() == 2));
                }
            }
            _ => panic!("not 2x2"),
        }
        assert_eq!(d.layout.leaves().len(), 4);
        assert_eq!(d.layout.leaves()[3].tabs, vec![PathBuf::from("/h")]);
    }

    #[test]
    fn round_trip_leaves_only_state_json() {
        let t = tempfile::tempdir().unwrap();
        let store = StateStore::new(t.path().join("sub"));
        let mut s = defaults(Path::new("/h"));
        s.show_hidden = true;
        s.pinned.push(SidebarEntry { path: "/p".into(), title: Some("P".into()) });
        s.layout = LayoutTree::split(
            Axis::SideBySide,
            vec![LayoutTree::Leaf(PaneSnapshot { tabs: vec!["/a".into(), "/b".into()], active: 1, view_mode: ViewMode::Column }), pane("/c".into())],
        );
        store.save(&s).unwrap();
        store.save(&s).unwrap();
        assert_eq!(store.load_with_home(Path::new("/h")), s);
        let files: Vec<_> = fs::read_dir(store.dir()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(files, [OsStr::new("state.json")]);
    }

    #[test]
    fn missing_or_corrupt_gives_defaults() {
        let t = tempfile::tempdir().unwrap();
        let store = StateStore::new(t.path());
        let home = Path::new("/h");
        assert_eq!(store.load_with_home(home), defaults(home));
        fs::write(store.file(), "{not json").unwrap();
        assert_eq!(store.load_with_home(home), defaults(home));
    }

    #[test]
    fn partial_file_fills_missing_fields() {
        let t = tempfile::tempdir().unwrap();
        let store = StateStore::new(t.path());
        fs::write(store.file(), r#"{"show_hidden": true}"#).unwrap();
        let s = store.load_with_home(Path::new("/h"));
        assert!(s.show_hidden && s.sidebar_visible);
    }

    #[test]
    fn restore_skips_missing_and_picks_nearest_earlier_tab() {
        let mut s = defaults(Path::new("/h"));
        s.layout = LayoutTree::split(
            Axis::SideBySide,
            vec![
                LayoutTree::Leaf(snap(&["/a", "/gone", "/c"], 1)),
                LayoutTree::Leaf(snap(&["/gone2"], 0)),
                LayoutTree::Leaf(snap(&["/gone3", "/d"], 0)),
            ],
        );
        let r = restore(&s, |p| !p.to_string_lossy().contains("gone")).unwrap();
        let leaves = r.leaves();
        assert_eq!(leaves.len(), 2);
        assert_eq!(leaves[0].tabs, [PathBuf::from("/a"), PathBuf::from("/c")]);
        assert_eq!(leaves[0].active, 0); // /gone dropped -> nearest earlier kept = /a
        assert_eq!(leaves[1].tabs, [PathBuf::from("/d")]);
        assert_eq!(leaves[1].active, 0);
    }

    #[test]
    fn restore_active_kept_stays() {
        let mut s = defaults(Path::new("/h"));
        s.layout = LayoutTree::Leaf(snap(&["/gone", "/b", "/c"], 2));
        let r = restore(&s, |p| !p.to_string_lossy().contains("gone")).unwrap();
        assert_eq!(r.leaves()[0].active, 1);
    }

    #[test]
    fn restore_empty_is_none() {
        let s = defaults(Path::new("/h"));
        assert!(restore(&s, |_| false).is_none());
    }

    #[test]
    fn state_dir_env_or_nanofm_rs() {
        let home = Path::new("/h");
        assert_eq!(state_dir(Some(OsStr::new("/x/y")), home), PathBuf::from("/x/y"));
        assert_eq!(
            state_dir(None, home),
            PathBuf::from("/h/Library/Application Support/nanofm-rs")
        );
        assert_eq!(state_dir(Some(OsStr::new("")), home), state_dir(None, home));
    }
}
