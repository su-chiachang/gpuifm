const PACKAGE_EXTS: &[&str] = &[
    "app", "bundle", "framework", "plugin", "kext", "pkg", "photoslibrary", "xcodeproj",
    "playground", "rtfd",
];

pub fn is_package_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    PACKAGE_EXTS.contains(&e.as_str())
}

fn ext_of(name: &str) -> Option<&str> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        None
    } else {
        Some(ext)
    }
}

pub fn kind_for(name: &str, is_dir: bool, is_package: bool) -> String {
    let ext = ext_of(name).map(|e| e.to_ascii_lowercase());
    if is_package && ext.as_deref() == Some("app") {
        return "Application".into();
    }
    if is_dir && !is_package {
        return "Folder".into();
    }
    let Some(ext) = ext else {
        return "Document".into();
    };
    let known = match ext.as_str() {
        "png" => "PNG image",
        "jpg" | "jpeg" => "JPEG image",
        "gif" => "GIF image",
        "heic" => "HEIC image",
        "svg" => "SVG image",
        "pdf" => "PDF document",
        "txt" => "Plain Text",
        "md" => "Markdown document",
        "rs" => "Rust source",
        "swift" => "Swift source",
        "py" => "Python script",
        "js" => "JavaScript source",
        "json" => "JSON document",
        "toml" => "TOML document",
        "html" => "HTML document",
        "zip" => "ZIP archive",
        "dmg" => "Disk Image",
        "mp3" => "MP3 audio",
        "mp4" => "MPEG-4 movie",
        "mov" => "QuickTime movie",
        "sh" => "Shell script",
        _ => "",
    };
    if known.is_empty() {
        format!("{} file", ext.to_ascii_uppercase())
    } else {
        known.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(kind_for("a.png", false, false), "PNG image");
        assert_eq!(kind_for("a.PDF", false, false), "PDF document");
        assert_eq!(kind_for("main.rs", false, false), "Rust source");
        assert_eq!(kind_for("a.txt", false, false), "Plain Text");
    }

    #[test]
    fn fallback() {
        assert_eq!(kind_for("a.xyz", false, false), "XYZ file");
        assert_eq!(kind_for("Makefile", false, false), "Document");
        assert_eq!(kind_for(".gitignore", false, false), "Document");
    }

    #[test]
    fn folder_and_app() {
        assert_eq!(kind_for("dir", true, false), "Folder");
        assert_eq!(kind_for("Foo.app", false, true), "Application");
        assert_eq!(kind_for("x.framework", false, true), "FRAMEWORK file");
    }

    #[test]
    fn package_ext() {
        assert!(is_package_ext("app"));
        assert!(is_package_ext("XCODEPROJ"));
        assert!(!is_package_ext("txt"));
    }
}
