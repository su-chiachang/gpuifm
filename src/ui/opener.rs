//! Seam for "open with the system" and Quick Look: the real one talks to the OS, tests record calls.

use gpui::App;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub trait Opener {
    fn open(&self, cx: &mut App, path: &Path);
    /// Shows a Quick Look preview of `paths`; false when it could not be started.
    fn quick_look(&self, _paths: &[PathBuf]) -> bool {
        false
    }
    fn close_quick_look(&self) {}
    /// Whether a preview started by `quick_look` is still showing.
    fn quick_look_open(&self) -> bool {
        false
    }
}

#[derive(Default)]
pub struct SystemOpener {
    ql: RefCell<Option<Child>>,
}

impl Opener for SystemOpener {
    fn open(&self, cx: &mut App, path: &Path) {
        cx.open_with_system(path);
    }

    fn quick_look(&self, paths: &[PathBuf]) -> bool {
        self.close_quick_look();
        if paths.is_empty() {
            return false;
        }
        let child = Command::new("qlmanage")
            .arg("-p")
            .args(paths)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match child {
            Ok(child) => {
                *self.ql.borrow_mut() = Some(child);
                true
            }
            Err(_) => false,
        }
    }

    fn close_quick_look(&self) {
        if let Some(mut c) = self.ql.borrow_mut().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn quick_look_open(&self) -> bool {
        let mut ql = self.ql.borrow_mut();
        match ql.as_mut().map(|c| c.try_wait()) {
            Some(Ok(None)) => true,
            Some(_) => {
                *ql = None;
                false
            }
            None => false,
        }
    }
}

impl Drop for SystemOpener {
    fn drop(&mut self) {
        self.close_quick_look();
    }
}
