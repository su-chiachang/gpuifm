use super::entry::FileEntry;
use super::kind::{is_package_ext, kind_for};
use std::fs;
use std::io;
use std::path::Path;

const UF_HIDDEN: u32 = 0x8000;

#[cfg(target_os = "macos")]
fn flag_hidden(meta: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    meta.st_flags() & UF_HIDDEN != 0
}

#[cfg(not(target_os = "macos"))]
fn flag_hidden(_meta: &fs::Metadata) -> bool {
    let _ = UF_HIDDEN;
    false
}

pub fn entry_for(path: &Path) -> Option<FileEntry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let lmeta = fs::symlink_metadata(path).ok()?;
    let is_symlink = lmeta.file_type().is_symlink();
    let meta = if is_symlink {
        fs::metadata(path).unwrap_or_else(|_| lmeta.clone())
    } else {
        lmeta.clone()
    };
    let target_is_dir = !is_symlink || meta.is_dir();
    let raw_dir = meta.is_dir() && target_is_dir;
    let is_package = raw_dir
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(is_package_ext);
    let is_dir = raw_dir && !is_package;
    let is_hidden = name.starts_with('.') || flag_hidden(&lmeta);
    let size = if raw_dir { None } else { Some(meta.len()) };
    let kind = kind_for(&name, is_dir, is_package);
    Some(FileEntry {
        path: path.to_path_buf(),
        name,
        is_dir,
        is_package,
        is_hidden,
        is_symlink,
        size,
        modified: meta.modified().ok(),
        kind,
    })
}

pub fn list_dir(dir: &Path, show_hidden: bool) -> io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for item in fs::read_dir(dir)? {
        let Ok(item) = item else { continue };
        let Some(e) = entry_for(&item.path()) else {
            continue;
        };
        if e.is_hidden && !show_hidden {
            continue;
        }
        out.push(e);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn hidden_flag() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join(".h"), "x").unwrap();
        fs::write(t.path().join("v"), "x").unwrap();
        let all = list_dir(t.path(), true).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().find(|e| e.name == ".h").unwrap().is_hidden);
        assert!(!all.iter().find(|e| e.name == "v").unwrap().is_hidden);
        let vis = list_dir(t.path(), false).unwrap();
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0].name, "v");
    }

    #[test]
    fn dir_detected() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("d")).unwrap();
        fs::write(t.path().join("f"), "abc").unwrap();
        let v = list_dir(t.path(), true).unwrap();
        let d = v.iter().find(|e| e.name == "d").unwrap();
        assert!(d.is_dir && d.size.is_none());
        let f = v.iter().find(|e| e.name == "f").unwrap();
        assert!(!f.is_dir && f.size == Some(3));
    }

    #[test]
    fn symlink_to_dir() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("d")).unwrap();
        symlink(t.path().join("d"), t.path().join("l")).unwrap();
        let e = entry_for(&t.path().join("l")).unwrap();
        assert!(e.is_dir && e.is_symlink);
    }

    #[test]
    fn broken_symlink_is_file() {
        let t = tempfile::tempdir().unwrap();
        symlink(t.path().join("nope"), t.path().join("l")).unwrap();
        let e = entry_for(&t.path().join("l")).unwrap();
        assert!(!e.is_dir && e.is_symlink);
    }

    #[test]
    fn package_not_dir() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("Foo.app")).unwrap();
        let e = entry_for(&t.path().join("Foo.app")).unwrap();
        assert!(e.is_package && !e.is_dir);
        assert_eq!(e.kind, "Application");
        assert!(e.size.is_none());
    }

    #[test]
    fn missing_is_err() {
        let t = tempfile::tempdir().unwrap();
        assert!(list_dir(&t.path().join("nope"), true).is_err());
    }

    #[test]
    fn empty_dir() {
        let t = tempfile::tempdir().unwrap();
        assert!(list_dir(t.path(), true).unwrap().is_empty());
    }
}
