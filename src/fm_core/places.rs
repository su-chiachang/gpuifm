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

/// The well-known folders the Go menu jumps to. Tests root them in a tempdir.
#[derive(Clone, Debug, PartialEq)]
pub struct Places {
    pub home: PathBuf,
    pub desktop: PathBuf,
    pub documents: PathBuf,
    pub downloads: PathBuf,
    pub applications: PathBuf,
}

impl Places {
    pub fn system() -> Self {
        let home = home_dir();
        Self {
            desktop: home.join("Desktop"),
            documents: home.join("Documents"),
            downloads: home.join("Downloads"),
            applications: PathBuf::from("/Applications"),
            home,
        }
    }

    /// Everything under `home` (applications is `home/Applications`): for tests.
    pub fn rooted(home: impl Into<PathBuf>) -> Self {
        let home = home.into();
        Self {
            desktop: home.join("Desktop"),
            documents: home.join("Documents"),
            downloads: home.join("Downloads"),
            applications: home.join("Applications"),
            home,
        }
    }
}

impl Default for Places {
    fn default() -> Self {
        Self::system()
    }
}

/// Lexical normalization: drops `.` and trailing slashes, resolves `..` (never above the root).
/// Does not touch the disk.
pub fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if out.as_os_str().is_empty() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() { PathBuf::from(".") } else { out }
}

/// Expands only `~` and `~/...`; `~user` and everything else is returned untouched.
pub fn expand_tilde(text: &str, home: &Path) -> String {
    if text == "~" {
        home.display().to_string()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home.join(rest).display().to_string()
    } else {
        text.to_string()
    }
}

/// `~` / `~/sub` for paths inside `home`, else the plain path.
pub fn abbreviate(p: &Path, home: &Path) -> String {
    if home == Path::new("/") {
        return p.display().to_string();
    }
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GoTarget {
    Empty,
    Folder(PathBuf),
    File { dir: PathBuf, select: PathBuf },
    NotFound,
}

/// Interprets Go to Folder input: trimmed, `~` expanded, relative paths resolved against `cwd`,
/// normalized lexically, then checked on disk.
pub fn resolve_go_to(text: &str, home: &Path, cwd: &Path) -> GoTarget {
    let t = text.trim();
    if t.is_empty() {
        return GoTarget::Empty;
    }
    let expanded = expand_tilde(t, home);
    let p = Path::new(&expanded);
    let full = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    let path = normalize(&full);
    if path.is_dir() {
        GoTarget::Folder(path)
    } else if path.exists() {
        match path.parent() {
            Some(dir) => GoTarget::File { dir: dir.to_path_buf(), select: path },
            None => GoTarget::NotFound,
        }
    } else {
        GoTarget::NotFound
    }
}

/// Breadcrumb folders from `home` (when `dir` is inside it) or `/` down to `dir`.
pub fn crumbs(dir: &Path, home: &Path) -> Vec<PathBuf> {
    let (root, rest) = match dir.strip_prefix(home) {
        Ok(rest) if home != Path::new("/") => (home.to_path_buf(), rest.to_path_buf()),
        _ => (PathBuf::from("/"), dir.strip_prefix("/").unwrap_or(dir).to_path_buf()),
    };
    let mut out = vec![root.clone()];
    let mut cur = root;
    for c in rest.components() {
        cur.push(c.as_os_str());
        out.push(cur.clone());
    }
    out
}

pub fn crumb_title(p: &Path, home: &Path) -> String {
    if p == Path::new("/") {
        "Macintosh HD".into()
    } else if p == home {
        "Home".into()
    } else {
        p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Favorites,
    Pinned,
}

/// Moves an entry within or between the two lists. `to.1` is an insertion slot in the destination
/// list as it was before the move (Swift adjusts it down by one when moving forward in the same list).
/// Returns false (and changes nothing) when `from` is out of range.
pub fn move_entry(
    favs: &mut Vec<SidebarEntry>,
    pins: &mut Vec<SidebarEntry>,
    from: (Section, usize),
    to: (Section, usize),
) -> bool {
    let src = match from.0 {
        Section::Favorites => &*favs,
        Section::Pinned => &*pins,
    };
    if from.1 >= src.len() {
        return false;
    }
    let entry = match from.0 {
        Section::Favorites => favs.remove(from.1),
        Section::Pinned => pins.remove(from.1),
    };
    let mut at = to.1;
    if from.0 == to.0 && from.1 < at {
        at -= 1;
    }
    let dst = match to.0 {
        Section::Favorites => favs,
        Section::Pinned => pins,
    };
    dst.insert(at.min(dst.len()), entry);
    true
}

/// Adds each folder to `list` (pinned: deduped, appended; favorites: moved to the end).
/// Returns whether anything changed.
pub fn add_folders(list: &mut Vec<SidebarEntry>, paths: &[PathBuf], section: Section) -> bool {
    let mut changed = false;
    for p in paths {
        match section {
            Section::Pinned => changed |= pin(list, p),
            Section::Favorites => {
                add_favorite(list, p);
                changed = true;
            }
        }
    }
    changed
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

    #[test]
    fn tilde_forms() {
        let h = Path::new("/h");
        assert_eq!(expand_tilde("~", h), "/h");
        assert_eq!(expand_tilde("~/a/b", h), "/h/a/b");
        assert_eq!(expand_tilde("~/", h), "/h/");
        assert_eq!(expand_tilde("~bob/x", h), "~bob/x");
        assert_eq!(expand_tilde("/x/~", h), "/x/~");
        assert_eq!(expand_tilde("a~", h), "a~");
    }

    #[test]
    fn normalize_lexical() {
        assert_eq!(normalize(Path::new("/a/b/../c/./d/")), PathBuf::from("/a/c/d"));
        assert_eq!(normalize(Path::new("/../..")), PathBuf::from("/"));
        assert_eq!(normalize(Path::new("/a/..")), PathBuf::from("/"));
    }

    #[test]
    fn abbreviate_home() {
        let h = Path::new("/h");
        assert_eq!(abbreviate(Path::new("/h"), h), "~");
        assert_eq!(abbreviate(Path::new("/h/a/b"), h), "~/a/b");
        assert_eq!(abbreviate(Path::new("/hx/a"), h), "/hx/a");
        assert_eq!(abbreviate(Path::new("/usr"), h), "/usr");
    }

    #[test]
    fn resolve_outcomes() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        fs::create_dir_all(home.join("sub").join("deep")).unwrap();
        fs::write(home.join("sub").join("f.txt"), "x").unwrap();
        assert_eq!(resolve_go_to("   ", home, home), GoTarget::Empty);
        assert_eq!(resolve_go_to("~", home, home), GoTarget::Folder(home.to_path_buf()));
        assert_eq!(resolve_go_to("  ~/sub/ ", home, home), GoTarget::Folder(home.join("sub")));
        assert_eq!(resolve_go_to("~/sub/deep/..", home, home), GoTarget::Folder(home.join("sub")));
        assert_eq!(
            resolve_go_to("~/sub/f.txt", home, home),
            GoTarget::File { dir: home.join("sub"), select: home.join("sub").join("f.txt") }
        );
        assert_eq!(resolve_go_to("~/nope", home, home), GoTarget::NotFound);
        assert_eq!(resolve_go_to("~nobody/x", home, home), GoTarget::NotFound);
        // relative input resolves against the current folder
        assert_eq!(resolve_go_to("deep", home, &home.join("sub")), GoTarget::Folder(home.join("sub").join("deep")));
        let abs = home.join("sub").display().to_string();
        assert_eq!(resolve_go_to(&abs, Path::new("/elsewhere"), home), GoTarget::Folder(home.join("sub")));
    }

    #[test]
    fn crumbs_inside_outside_root() {
        let h = Path::new("/Users/me");
        assert_eq!(crumbs(Path::new("/Users/me"), h), [PathBuf::from("/Users/me")]);
        assert_eq!(
            crumbs(Path::new("/Users/me/a/b"), h),
            [PathBuf::from("/Users/me"), PathBuf::from("/Users/me/a"), PathBuf::from("/Users/me/a/b")]
        );
        assert_eq!(
            crumbs(Path::new("/usr/bin"), h),
            [PathBuf::from("/"), PathBuf::from("/usr"), PathBuf::from("/usr/bin")]
        );
        assert_eq!(crumbs(Path::new("/"), h), [PathBuf::from("/")]);
        // a sibling that merely shares the prefix is outside home
        assert_eq!(crumbs(Path::new("/Users/me2"), h)[0], PathBuf::from("/"));
        assert_eq!(crumb_title(Path::new("/"), h), "Macintosh HD");
        assert_eq!(crumb_title(h, h), "Home");
        assert_eq!(crumb_title(Path::new("/Users/me/a"), h), "a");
    }

    #[test]
    fn rooted_places() {
        let p = Places::rooted("/r");
        assert_eq!(p.downloads, Path::new("/r/Downloads"));
        assert_eq!(p.applications, Path::new("/r/Applications"));
    }

    fn ents(names: &[&str]) -> Vec<SidebarEntry> {
        names.iter().map(|n| SidebarEntry { path: PathBuf::from(format!("/{n}")), title: None }).collect()
    }
    fn names(l: &[SidebarEntry]) -> Vec<String> {
        l.iter().map(|e| e.path.display().to_string()[1..].to_string()).collect()
    }

    #[test]
    fn move_within_forward() {
        let (mut f, mut p) = (ents(&["a", "b", "c"]), ents(&[]));
        assert!(move_entry(&mut f, &mut p, (Section::Favorites, 0), (Section::Favorites, 2)));
        assert_eq!(names(&f), ["b", "a", "c"]);
        assert!(move_entry(&mut f, &mut p, (Section::Favorites, 0), (Section::Favorites, 3)));
        assert_eq!(names(&f), ["a", "c", "b"]);
    }

    #[test]
    fn move_within_back() {
        let (mut f, mut p) = (ents(&["a", "b", "c"]), ents(&[]));
        assert!(move_entry(&mut f, &mut p, (Section::Favorites, 2), (Section::Favorites, 0)));
        assert_eq!(names(&f), ["c", "a", "b"]);
    }

    #[test]
    fn move_between_groups() {
        let (mut f, mut p) = (ents(&["a", "b"]), ents(&["x"]));
        assert!(move_entry(&mut f, &mut p, (Section::Pinned, 0), (Section::Favorites, 1)));
        assert_eq!(names(&f), ["a", "x", "b"]);
        assert!(p.is_empty());
        assert!(move_entry(&mut f, &mut p, (Section::Favorites, 0), (Section::Pinned, 99)));
        assert_eq!(names(&p), ["a"]);
    }

    #[test]
    fn move_out_of_range_noop() {
        let (mut f, mut p) = (ents(&["a"]), ents(&["x"]));
        assert!(!move_entry(&mut f, &mut p, (Section::Favorites, 1), (Section::Pinned, 0)));
        assert!(!move_entry(&mut f, &mut p, (Section::Pinned, 5), (Section::Favorites, 0)));
        assert_eq!(names(&f), ["a"]);
        assert_eq!(names(&p), ["x"]);
    }

    #[test]
    fn add_folders_dedupes() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let mut l = Vec::new();
        assert!(add_folders(&mut l, &[a.clone(), b.clone()], Section::Pinned));
        assert!(!add_folders(&mut l, &[a], Section::Pinned));
        assert_eq!(l.len(), 2);
    }
}
