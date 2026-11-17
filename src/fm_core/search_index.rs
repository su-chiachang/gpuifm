//! Filename index over a few roots (pure; the UI crawls on a background thread).

use super::fuzzy;
use super::kind::is_package_ext;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const EXCLUDED_NAMES: [&str; 3] = ["node_modules", ".git", "Library"];
pub const MAX_DEPTH: usize = 6;
pub const MAX_ENTRIES: usize = 200_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    path_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub score: i64,
}

fn canon(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Crawls `roots` breadth of directory per level (depth-first per root, children sorted by name).
/// Skips dotfiles, excluded names, package contents; never follows symlinks; roots are deduped.
pub fn crawl(roots: &[PathBuf], max_depth: usize, max_entries: usize) -> Vec<IndexEntry> {
    let mut out = Vec::new();
    let mut seen_roots = HashSet::new();
    for r in roots {
        if out.len() >= max_entries {
            break;
        }
        let c = canon(r);
        if seen_roots.insert(c.clone()) {
            walk(&c, 1, max_depth, max_entries, &mut out);
        }
    }
    out
}

fn walk(dir: &Path, depth: usize, max_depth: usize, max_entries: usize, out: &mut Vec<IndexEntry>) {
    if out.len() >= max_entries {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut kids: Vec<_> = rd.flatten().collect();
    kids.sort_by_key(|e| e.file_name());
    let mut subdirs = Vec::new();
    for e in kids {
        if out.len() >= max_entries {
            return;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || EXCLUDED_NAMES.contains(&name.as_str()) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        let path = e.path();
        let is_symlink = ft.is_symlink();
        let raw_dir = if is_symlink { fs::metadata(&path).is_ok_and(|m| m.is_dir()) } else { ft.is_dir() };
        let is_package = raw_dir
            && path.extension().and_then(|x| x.to_str()).is_some_and(is_package_ext);
        let is_dir = raw_dir && !is_package;
        let path_len = path.as_os_str().len();
        out.push(IndexEntry { path: path.clone(), name, is_dir, path_len });
        if is_dir && !is_symlink && depth < max_depth {
            subdirs.push(path);
        }
    }
    for s in subdirs {
        walk(&s, depth + 1, max_depth, max_entries, out);
    }
}

/// Fuzzy name search: score desc, then shorter path, then path order. Blank query -> nothing.
pub fn search(entries: &[IndexEntry], query: &str, limit: usize) -> Vec<SearchHit> {
    let q = query.trim();
    if q.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut scored: Vec<(i64, &IndexEntry)> =
        entries.iter().filter_map(|e| fuzzy::score(q, &e.name).map(|s| (s, e))).collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0).then(a.1.path_len.cmp(&b.1.path_len)).then_with(|| a.1.path.cmp(&b.1.path))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(score, e)| SearchHit { path: e.path.clone(), name: e.name.clone(), is_dir: e.is_dir, score })
        .collect()
}

/// Folders worth indexing: favorites then pinned, existing dirs only, excluding home and `/`
/// (too broad), deduped by canonical path.
pub fn index_roots(home: &Path, favorites: &[PathBuf], pinned: &[PathBuf]) -> Vec<PathBuf> {
    let home_c = canon(home);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in favorites.iter().chain(pinned) {
        if !p.is_dir() {
            continue;
        }
        let c = canon(p);
        if c == home_c || c == Path::new("/") {
            continue;
        }
        if seen.insert(c) {
            out.push(p.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    fn names(h: &[SearchHit]) -> Vec<&str> {
        h.iter().map(|h| h.name.as_str()).collect()
    }

    #[test]
    fn build_search_and_exclusions() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        file(r, "Documents/quarterly-report.txt");
        file(r, "Documents/notes.md");
        file(r, "Projects/app/main.swift");
        file(r, "Projects/node_modules/pkg/report-lib.js");
        file(r, "Projects/.git/report-config");
        file(r, "Library/Preferences/report.plist");
        file(r, ".hidden-report/inside.txt");
        file(r, ".secret-report.txt");
        file(r, "Fake.app/Contents/report-inner.txt");
        fs::create_dir(r.join("reports")).unwrap();

        let idx = crawl(&[r.to_path_buf()], MAX_DEPTH, MAX_ENTRIES);
        assert!(!idx.is_empty());
        let hits = search(&idx, "report", 50);
        let n = names(&hits);
        assert!(n.contains(&"quarterly-report.txt"));
        assert!(n.contains(&"reports"));
        for bad in ["report-lib.js", "report-config", "report.plist", ".secret-report.txt", "inside.txt", "report-inner.txt"] {
            assert!(!n.contains(&bad), "{bad}");
        }
        assert!(search(&idx, "fake", 50).iter().any(|h| h.name == "Fake.app" && !h.is_dir));
        assert!(hits.iter().find(|h| h.name == "reports").unwrap().is_dir);
        assert!(!hits.iter().find(|h| h.name == "quarterly-report.txt").unwrap().is_dir);
        assert_eq!(hits[0].name, "reports");
        assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));

        assert!(search(&idx, "   ", 50).is_empty());
        assert!(search(&idx, "", 50).is_empty());
        assert_eq!(names(&search(&idx, "main", 1)), ["main.swift"]);
        assert!(search(&idx, "zzzzqqq", 50).is_empty());
        assert!(search(&idx, "e", 3).len() <= 3);
    }

    #[test]
    fn depth_and_entry_caps() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path().to_path_buf();
        file(&r, "a/b/c/deep.txt");
        file(&r, "top.txt");
        let shallow = crawl(std::slice::from_ref(&r), 2, MAX_ENTRIES);
        assert!(!search(&shallow, "deep", 50).iter().any(|h| h.name == "deep.txt"));
        assert!(search(&shallow, "b", 50).iter().any(|h| h.name == "b"));
        assert!(!search(&shallow, "c", 50).iter().any(|h| h.name == "c"));
        let full = crawl(std::slice::from_ref(&r), MAX_DEPTH, MAX_ENTRIES);
        assert!(search(&full, "deep", 50).iter().any(|h| h.name == "deep.txt"));
        for i in 0..30 {
            file(&r, &format!("bulk/f{i}.txt"));
        }
        assert_eq!(crawl(&[r], MAX_DEPTH, 10).len(), 10);
    }

    #[test]
    fn duplicate_roots_and_rebuild() {
        let (t1, t2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        file(t1.path(), "alpha.txt");
        file(t2.path(), "beta.txt");
        let roots = vec![t1.path().to_path_buf(), t2.path().to_path_buf(), t1.path().to_path_buf()];
        assert_eq!(crawl(&roots, MAX_DEPTH, MAX_ENTRIES).len(), 2);
        file(t1.path(), "gamma.txt");
        assert_eq!(names(&search(&crawl(&roots, MAX_DEPTH, MAX_ENTRIES), "gamma", 5)), ["gamma.txt"]);
    }

    #[test]
    fn symlinks_not_followed() {
        let t = tempfile::tempdir().unwrap();
        file(t.path(), "real/inner.txt");
        std::os::unix::fs::symlink(t.path().join("real"), t.path().join("link")).unwrap();
        let idx = crawl(&[t.path().to_path_buf()], MAX_DEPTH, MAX_ENTRIES);
        assert_eq!(idx.iter().filter(|e| e.name == "inner.txt").count(), 1);
    }

    #[test]
    fn roots_exclude_home_and_root() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("A")).unwrap();
        let a = t.path().join("A");
        let r = index_roots(
            t.path(),
            &[t.path().to_path_buf(), PathBuf::from("/"), a.clone(), t.path().join("missing")],
            std::slice::from_ref(&a),
        );
        assert_eq!(r, vec![a]);
    }
}
