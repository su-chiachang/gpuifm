use std::path::{Path, PathBuf};

/// What an icon is cached under: one per kind, except packages and special folders which are per path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconKey {
    Folder,
    Plain,
    Ext(String),
    Exact(PathBuf),
}

pub fn icon_key(path: &Path, is_dir: bool, is_package: bool, is_special: &dyn Fn(&Path) -> bool) -> IconKey {
    if is_package || (is_dir && is_special(path)) {
        return IconKey::Exact(path.to_path_buf());
    }
    if is_dir {
        return IconKey::Folder;
    }
    let ext = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.rsplit_once('.'))
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase());
    match ext {
        Some(e) => IconKey::Ext(e),
        None => IconKey::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &Path) -> bool {
        false
    }

    #[test]
    fn generic_folder() {
        assert_eq!(icon_key(Path::new("/a/d"), true, false, &none), IconKey::Folder);
        assert_eq!(icon_key(Path::new("/a/e"), true, false, &none), IconKey::Folder);
    }

    #[test]
    fn special_exact() {
        let sp = |p: &Path| p == Path::new("/Users/me/Desktop");
        assert_eq!(
            icon_key(Path::new("/Users/me/Desktop"), true, false, &sp),
            IconKey::Exact(PathBuf::from("/Users/me/Desktop"))
        );
        assert_eq!(icon_key(Path::new("/Users/me/Other"), true, false, &sp), IconKey::Folder);
    }

    #[test]
    fn package_exact_per_path() {
        let a = icon_key(Path::new("/A.app"), false, true, &none);
        let b = icon_key(Path::new("/B.app"), false, true, &none);
        assert_eq!(a, IconKey::Exact(PathBuf::from("/A.app")));
        assert_ne!(a, b);
    }

    #[test]
    fn ext_lowercased() {
        assert_eq!(icon_key(Path::new("/a/X.TXT"), false, false, &none), IconKey::Ext("txt".into()));
        assert_eq!(icon_key(Path::new("/a/y.txt"), false, false, &none), IconKey::Ext("txt".into()));
        assert_eq!(icon_key(Path::new("/a/arch.tar.GZ"), false, false, &none), IconKey::Ext("gz".into()));
    }

    #[test]
    fn dotfile_plain() {
        assert_eq!(icon_key(Path::new("/a/.gitignore"), false, false, &none), IconKey::Plain);
        assert_eq!(icon_key(Path::new("/a/Makefile"), false, false, &none), IconKey::Plain);
        assert_eq!(icon_key(Path::new("/a/trailing."), false, false, &none), IconKey::Plain);
    }
}
