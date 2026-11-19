//! Native file icons: a main-thread source (AppKit is not thread-safe), a cache that fetches each
//! icon once per kind in small chunks, and the element helper that falls back to a glyph.

use crate::fm_core::icons::{IconKey, icon_key};
use crate::fm_core::places::home_dir;
use crate::ui::theme;
use gpui::{
    AnyElement, App, Context, Entity, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, Styled,
    StyledImage, div, img, px, rgb,
};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};

/// Keys fetched per tick before yielding to the event loop.
const CHUNK: usize = 4;

/// Where icons come from. Only ever called on the main thread.
pub trait IconSource {
    /// PNG bytes of the icon for `path`, `px` pixels square; `None` when there is none.
    fn icon_png(&self, path: &Path, px: u32) -> Option<Vec<u8>>;
}

/// No icons at all (headless tests, non-macOS): every consumer shows its glyph.
pub struct NoIcons;

impl IconSource for NoIcons {
    fn icon_png(&self, _: &Path, _: u32) -> Option<Vec<u8>> {
        None
    }
}

/// NSWorkspace icons (macOS).
#[cfg(target_os = "macos")]
pub struct SystemIcons;

#[cfg(target_os = "macos")]
impl IconSource for SystemIcons {
    fn icon_png(&self, path: &Path, px: u32) -> Option<Vec<u8>> {
        use objc2::AnyThread;
        use objc2::rc::autoreleasepool;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
        use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};
        autoreleasepool(|_| {
            let image = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(&path.to_string_lossy()));
            let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(px as f64, px as f64));
            // SAFETY: `rect` is a valid pointer for the call; no context or hints.
            let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
            let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
            // SAFETY: an empty properties dictionary is valid.
            let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
            Some(data.to_vec())
        })
    }
}

enum Slot {
    Pending,
    Ready(Arc<Image>),
    /// The source had nothing; never asked again.
    Missing,
}

/// Cache of icons keyed by (kind, size); one fetch per key, on the main thread.
pub struct IconCache {
    source: Rc<dyn IconSource>,
    slots: HashMap<(IconKey, u32), Slot>,
    queue: VecDeque<(IconKey, u32, PathBuf)>,
    pumping: bool,
    home: PathBuf,
}

struct YieldNow(bool);

impl Future for YieldNow {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

impl IconCache {
    pub fn new(source: Rc<dyn IconSource>) -> Self {
        Self { source, slots: HashMap::new(), queue: VecDeque::new(), pumping: false, home: home_dir() }
    }

    /// The icon for `path` at `px` logical points, if already fetched; otherwise queues the fetch
    /// (the cache notifies observers when a batch lands).
    pub fn get(&mut self, path: &Path, is_dir: bool, is_package: bool, px: u32, cx: &mut Context<Self>) -> Option<Arc<Image>> {
        let home = &self.home;
        let key = icon_key(path, is_dir, is_package, &|p: &Path| {
            p == home.as_path()
                || p.parent() == Some(home.as_path())
                || p.parent().is_none_or(|q| q == Path::new("/Volumes"))
        });
        match self.slots.get(&(key.clone(), px)) {
            Some(Slot::Ready(i)) => return Some(i.clone()),
            Some(_) => return None,
            None => {}
        }
        self.slots.insert((key.clone(), px), Slot::Pending);
        self.queue.push_back((key, px, path.to_path_buf()));
        if !self.pumping {
            self.pumping = true;
            cx.spawn(async move |this, cx| {
                while let Ok(true) = this.update(cx, |this, cx| this.step(cx)) {
                    YieldNow(false).await;
                }
            })
            .detach();
        }
        None
    }

    /// Fetches up to `CHUNK` queued keys and notifies once; false when the queue is drained.
    fn step(&mut self, cx: &mut Context<Self>) -> bool {
        for _ in 0..CHUNK {
            let Some((key, px, path)) = self.queue.pop_front() else { break };
            // Fetch at twice the size for retina displays.
            let slot = match self.source.icon_png(&path, px * 2) {
                Some(png) => Slot::Ready(Arc::new(Image::from_bytes(ImageFormat::Png, png))),
                None => Slot::Missing,
            };
            self.slots.insert((key, px), slot);
        }
        cx.notify();
        let more = !self.queue.is_empty();
        self.pumping = more;
        more
    }

    // ---- test accessors ------------------------------------------------------------------------

    pub fn is_ready(&self, key: &IconKey, px: u32) -> bool {
        matches!(self.slots.get(&(key.clone(), px)), Some(Slot::Ready(_)))
    }

    pub fn is_missing(&self, key: &IconKey, px: u32) -> bool {
        matches!(self.slots.get(&(key.clone(), px)), Some(Slot::Missing))
    }

    pub fn ready_count(&self) -> usize {
        self.slots.values().filter(|s| matches!(s, Slot::Ready(_))).count()
    }
}

/// An `px`-square icon element for `path`; shows the folder / file glyph until the icon is ready.
pub fn icon_el(cache: Option<&Entity<IconCache>>, path: &Path, is_dir: bool, is_package: bool, px_size: f32, cx: &mut App) -> AnyElement {
    let image = cache.and_then(|c| c.update(cx, |c, cx| c.get(path, is_dir, is_package, px_size as u32, cx)));
    match image {
        Some(i) => img(i).size(px(px_size)).flex_none().object_fit(ObjectFit::Contain).into_any_element(),
        None => div()
            .size(px(px_size))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::TEXT_DIM))
            .text_size(px(px_size * 0.8))
            .child(if is_dir { "\u{25B8}" } else { "" })
            .into_any_element(),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// Run manually: `cargo test --lib system_icons_png_magic -- --ignored` (needs a main thread
    /// with AppKit; plain test threads work for iconForFile in practice).
    #[test]
    #[ignore]
    fn system_icons_png_magic() {
        let png = SystemIcons.icon_png(Path::new("/"), 32).expect("icon for /");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
