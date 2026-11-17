//! Non-recursive directory watching: reports which watched directories had their contents change.
//! Paths are canonicalized on both sides (`/var` vs `/private/var`).

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Canonical form of `p`, or `p` itself when it cannot be resolved.
pub fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

pub struct DirWatcher {
    watcher: Option<RecommendedWatcher>,
    watched: BTreeSet<PathBuf>,
    shared: Arc<Mutex<BTreeSet<PathBuf>>>,
}

impl DirWatcher {
    /// `cb` runs on the watcher's thread with the canonical watched directories that changed.
    pub fn new(cb: impl Fn(Vec<PathBuf>) + Send + 'static) -> Self {
        let shared: Arc<Mutex<BTreeSet<PathBuf>>> = Arc::default();
        let s = shared.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if matches!(ev.kind, EventKind::Access(_)) {
                return;
            }
            let hit: BTreeSet<PathBuf> = {
                let set = s.lock().unwrap();
                ev.paths
                    .iter()
                    .flat_map(|p| [p.parent().map(Path::to_path_buf), Some(p.clone())])
                    .flatten()
                    .filter(|d| set.contains(d))
                    .collect()
            };
            if !hit.is_empty() {
                cb(hit.into_iter().collect());
            }
        })
        .ok();
        Self { watcher, watched: BTreeSet::new(), shared }
    }

    /// Makes exactly `dirs` (and nothing below them) the watched set.
    pub fn set_dirs(&mut self, dirs: &BTreeSet<PathBuf>) {
        let want: BTreeSet<PathBuf> = dirs.iter().map(|d| canon(d)).collect();
        if let Some(w) = &mut self.watcher {
            for gone in self.watched.difference(&want) {
                let _ = w.unwatch(gone);
            }
            let mut ok = BTreeSet::new();
            for d in &want {
                if self.watched.contains(d) || w.watch(d, RecursiveMode::NonRecursive).is_ok() {
                    ok.insert(d.clone());
                }
            }
            self.watched = ok;
        }
        *self.shared.lock().unwrap() = self.watched.clone();
    }

    pub fn watched(&self) -> &BTreeSet<PathBuf> {
        &self.watched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn wait_for(rx: &mpsc::Receiver<Vec<PathBuf>>, dir: &Path, mut poke: impl FnMut()) -> bool {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            poke();
            if let Ok(v) = rx.recv_timeout(Duration::from_millis(500))
                && v.iter().any(|d| d == dir)
            {
                return true;
            }
        }
        false
    }

    #[test]
    fn reports_create_and_delete_in_watched_dir() {
        let t = tempfile::tempdir().unwrap();
        let dir = canon(t.path());
        let (tx, rx) = mpsc::channel();
        let mut w = DirWatcher::new(move |v| {
            let _ = tx.send(v);
        });
        w.set_dirs(&BTreeSet::from([t.path().to_path_buf()]));
        assert_eq!(w.watched(), &BTreeSet::from([dir.clone()]));
        let f = t.path().join("new.txt");
        let mut n = 0;
        assert!(wait_for(&rx, &dir, || {
            n += 1;
            std::fs::write(t.path().join(format!("new{n}.txt")), "x").unwrap();
        }));
        std::fs::write(&f, "x").unwrap();
        while rx.try_recv().is_ok() {}
        assert!(wait_for(&rx, &dir, || {
            let _ = std::fs::remove_file(&f);
            let _ = std::fs::remove_file(t.path().join("new1.txt"));
        }));
    }

    #[test]
    fn unwatched_and_nested_dirs_are_silent() {
        let t = tempfile::tempdir().unwrap();
        let sub = t.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = DirWatcher::new(move |v| {
            let _ = tx.send(v);
        });
        w.set_dirs(&BTreeSet::from([t.path().to_path_buf()]));
        std::thread::sleep(Duration::from_millis(300));
        while rx.try_recv().is_ok() {}
        std::fs::write(sub.join("deep.txt"), "x").unwrap();
        let sub_c = canon(&sub);
        let got = rx.recv_timeout(Duration::from_millis(1200)).unwrap_or_default();
        assert!(!got.contains(&sub_c), "{got:?}");
        w.set_dirs(&BTreeSet::new());
        assert!(w.watched().is_empty());
    }
}
