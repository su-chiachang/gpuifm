//! Thin wrappers over NSFileManager (move / copy / trash). Safe to call from any thread.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::rc::autoreleasepool;
    use objc2_foundation::{NSError, NSFileManager, NSString, NSURL};

    fn url(p: &Path) -> objc2::rc::Retained<NSURL> {
        NSURL::fileURLWithPath(&NSString::from_str(&p.to_string_lossy()))
    }

    fn err(e: objc2::rc::Retained<NSError>) -> io::Error {
        io::Error::other(e.localizedDescription().to_string())
    }

    pub fn move_item(src: &Path, dst: &Path) -> io::Result<()> {
        autoreleasepool(|_| {
            let fm = NSFileManager::defaultManager();
            fm.moveItemAtURL_toURL_error(&url(src), &url(dst)).map_err(err)
        })
    }

    pub fn copy_item(src: &Path, dst: &Path) -> io::Result<()> {
        autoreleasepool(|_| {
            let fm = NSFileManager::defaultManager();
            fm.copyItemAtURL_toURL_error(&url(src), &url(dst)).map_err(err)
        })
    }

    pub fn trash_item(p: &Path) -> io::Result<PathBuf> {
        autoreleasepool(|_| {
            let fm = NSFileManager::defaultManager();
            let mut out = None;
            fm.trashItemAtURL_resultingItemURL_error(&url(p), Some(&mut out))
                .map_err(err)?;
            let resulting = out.and_then(|u| u.path()).map(|s| PathBuf::from(s.to_string()));
            Ok(resulting.unwrap_or_else(|| {
                let leaf = p.file_name().unwrap_or_default();
                crate::fm_core::places::home_dir().join(".Trash").join(leaf)
            }))
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    fn unsupported<T>() -> io::Result<T> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "only supported on macOS"))
    }
    pub fn move_item(_: &Path, _: &Path) -> io::Result<()> {
        unsupported()
    }
    pub fn copy_item(_: &Path, _: &Path) -> io::Result<()> {
        unsupported()
    }
    pub fn trash_item(_: &Path) -> io::Result<PathBuf> {
        unsupported()
    }
}

pub use imp::{copy_item, move_item};

thread_local! {
    static TRASH_LOG: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Trashes `p` and remembers (per thread) the exact path the system returned, so tests can clean up.
pub fn trash_item(p: &Path) -> io::Result<PathBuf> {
    let t = imp::trash_item(p)?;
    TRASH_LOG.with(|l| l.borrow_mut().push(t.clone()));
    Ok(t)
}

/// Test cleanup: on drop, removes exactly the paths `trash_item` returned on this thread (and
/// only those whose leaf carries the `nanofm-test-` prefix and that still exist).
#[doc(hidden)]
#[derive(Default)]
pub struct TrashCleanup;

impl Drop for TrashCleanup {
    fn drop(&mut self) {
        let paths = TRASH_LOG.with(|l| std::mem::take(&mut *l.borrow_mut()));
        for p in paths {
            let ours = p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("nanofm-test-"));
            if !ours {
                continue;
            }
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => {
                    let _ = std::fs::remove_dir_all(&p);
                }
                Ok(_) => {
                    let _ = std::fs::remove_file(&p);
                }
                Err(_) => {}
            }
        }
    }
}
