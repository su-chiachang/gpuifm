use super::state::SidebarEntry;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub title: String,
    pub path: PathBuf,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// Swift defaults: Desktop, Documents, Downloads, Home (not filtered by existence).
pub fn default_favorites(home: &Path) -> Vec<SidebarEntry> {
    let mut v: Vec<SidebarEntry> = ["Desktop", "Documents", "Downloads"]
        .into_iter()
        .map(|n| SidebarEntry { path: home.join(n), title: None })
        .collect();
    v.push(SidebarEntry { path: home.to_path_buf(), title: Some("Home".into()) });
    v
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Appends `path` (canonical) unless already present. Returns whether it was added.
pub fn pin(list: &mut Vec<SidebarEntry>, path: &Path) -> bool {
    let c = canon(path);
    if list.iter().any(|e| canon(&e.path) == c) {
        return false;
    }
    list.push(SidebarEntry { path: c, title: None });
    true
}

/// Adds (or moves to the end) a favorite, deduped by canonical path.
pub fn add_favorite(list: &mut Vec<SidebarEntry>, path: &Path) {
    let c = canon(path);
    list.retain(|e| canon(&e.path) != c);
    list.push(SidebarEntry { path: c, title: None });
}

pub fn remove(list: &mut Vec<SidebarEntry>, ix: usize) -> Option<SidebarEntry> {
    (ix < list.len()).then(|| list.remove(ix))
}

pub fn list_volumes(root: &Path) -> Vec<Place> {
    let mut vols: Vec<Place> = std::fs::read_dir(root)
        .map(|rd| {
            rd.flatten()
                .filter(|e| std::fs::canonicalize(e.path()).is_ok_and(|c| c != Path::new("/")))
                .map(|e| Place { title: e.file_name().to_string_lossy().into_owned(), path: e.path() })
                .collect()
        })
        .unwrap_or_default();
    vols.sort_by_key(|p| p.title.to_lowercase());
    let mut out = vec![Place { title: "Macintosh HD".into(), path: PathBuf::from("/") }];
    out.extend(vols);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn volumes_sorted() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("zeta")).unwrap();
        fs::create_dir(t.path().join("Alpha")).unwrap();
        let v = list_volumes(t.path());
        let titles: Vec<_> = v.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["Macintosh HD", "Alpha", "zeta"]);
        assert_eq!(v[0].path, Path::new("/"));
        assert_eq!(list_volumes(&t.path().join("missing")).len(), 1);
    }

    #[test]
    fn defaults_are_data_only() {
        let home = Path::new("/nonexistent-home");
        let f = default_favorites(home);
        let paths: Vec<_> = f.iter().map(|e| e.path.clone()).collect();
        assert_eq!(
            paths,
            [home.join("Desktop"), home.join("Documents"), home.join("Downloads"), home.to_path_buf()]
        );
    }

    #[test]
    fn pin_dedupes_canonical_and_remove() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("d");
        fs::create_dir(&d).unwrap();
        let mut l = Vec::new();
        assert!(pin(&mut l, &d));
        assert!(!pin(&mut l, &d.join("..").join("d")));
        assert_eq!(l.len(), 1);
        assert!(remove(&mut l, 5).is_none());
        assert!(remove(&mut l, 0).is_some());
        assert!(l.is_empty());
    }

    #[test]
    fn add_favorite_moves_to_end() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let mut l = Vec::new();
        add_favorite(&mut l, &a);
        add_favorite(&mut l, &b);
        add_favorite(&mut l, &a);
        let names: Vec<_> = l.iter().map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["b", "a"]);
    }
}
