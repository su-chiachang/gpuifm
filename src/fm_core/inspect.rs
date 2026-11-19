use super::format::{format_long_date, format_size};
use super::listing::entry_for;
use super::places::abbreviate;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub name: String,
    pub parent_abbrev: String,
    pub kind: String,
    pub size: String,
    pub resolution: Option<String>,
    pub created: String,
    pub modified: String,
    pub is_dir: bool,
    pub is_package: bool,
}

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "tif", "tiff", "bmp"];

fn resolution(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !IMAGE_EXTS.contains(&ext.as_str()) {
        return None;
    }
    let s = imagesize::size(path).ok()?;
    Some(format!("{} × {}", s.width, s.height))
}

/// Inspector details for one item; `None` when it cannot be read.
pub fn details(path: &Path, home: &Path) -> Option<Details> {
    let e = entry_for(path)?;
    let created = std::fs::metadata(path).ok().and_then(|m| m.created().ok());
    let parent = path.parent().map(|p| abbreviate(p, home)).unwrap_or_default();
    let resolution = if e.is_dir || e.is_package { None } else { resolution(path) };
    Some(Details {
        is_dir: e.is_dir,
        is_package: e.is_package,
        name: e.name,
        parent_abbrev: parent,
        kind: e.kind,
        size: format_size(e.size),
        resolution,
        created: format_long_date(created),
        modified: format_long_date(e.modified),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00,
        0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x00, 0x02, 0x00, 0x00, 0x05, 0x00, 0x01, 0x7a, 0x5e,
        0xab, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn file_details() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("sub")).unwrap();
        let f = t.path().join("sub").join("a.txt");
        fs::write(&f, "hello").unwrap();
        let d = details(&f, t.path()).unwrap();
        assert_eq!(d.name, "a.txt");
        assert_eq!(d.parent_abbrev, "~/sub");
        assert_eq!(d.kind, "Plain Text");
        assert_eq!(d.size, "5 bytes");
        assert_eq!(d.resolution, None);
        assert_ne!(d.modified, "--");
        assert!(details(&t.path().join("nope"), t.path()).is_none());
    }

    #[test]
    fn dir_size_dashes() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("d")).unwrap();
        let d = details(&t.path().join("d"), t.path()).unwrap();
        assert_eq!(d.size, "--");
        assert_eq!(d.kind, "Folder");
        assert_eq!(d.resolution, None);
    }

    #[test]
    fn png_resolution() {
        assert_eq!(PNG_1X1.len(), 68);
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("p.PNG");
        fs::write(&f, PNG_1X1).unwrap();
        assert_eq!(details(&f, t.path()).unwrap().resolution.as_deref(), Some("1 × 1"));
    }

    #[test]
    fn non_image_no_resolution() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("fake.png.txt");
        fs::write(&f, PNG_1X1).unwrap();
        assert_eq!(details(&f, t.path()).unwrap().resolution, None);
        let g = t.path().join("bad.png");
        fs::write(&g, "not an image").unwrap();
        assert_eq!(details(&g, t.path()).unwrap().resolution, None);
    }

    #[test]
    fn created_present() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a");
        fs::write(&f, "x").unwrap();
        assert_ne!(details(&f, t.path()).unwrap().created, "--");
    }
}
