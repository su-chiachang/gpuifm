//! The Inspector: a read-only panel describing the selection of a pane.

use crate::fm_core::inspect::{self, Details};
use crate::ui::opener::Opener;
use crate::ui::theme;
use gpui::{
    AnyElement, ClipboardItem, Context, Entity, Img, InteractiveElement, IntoElement, ObjectFit, ParentElement, Render,
    RetainAllImageCache, StatefulInteractiveElement, Styled, StyledImage, Window, div, img, px, rgb,
};
use std::path::PathBuf;
use std::rc::Rc;

/// Images bigger than this are not decoded for the preview.
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;

pub struct Inspector {
    sel: Vec<PathBuf>,
    details: Option<Details>,
    /// Whether the single selected item is an image that may be decoded for the preview.
    previewable: bool,
    gen_: u64,
    home: PathBuf,
    opener: Rc<dyn Opener>,
    /// Own cache (not the app-wide asset cache) so decoded previews are dropped on selection change.
    cache: Entity<RetainAllImageCache>,
    clear_cache: bool,
}

impl Inspector {
    pub fn new(home: PathBuf, opener: Rc<dyn Opener>, cx: &mut Context<Self>) -> Self {
        let cache = RetainAllImageCache::new(cx);
        Self { sel: Vec::new(), details: None, previewable: false, gen_: 0, home, opener, cache, clear_cache: false }
    }

    /// Shows `sel`; does nothing when it is unchanged. Details load in the background.
    pub fn set_selection(&mut self, sel: Vec<PathBuf>, cx: &mut Context<Self>) {
        if sel == self.sel {
            return;
        }
        self.sel = sel;
        self.gen_ += 1;
        self.details = None;
        self.previewable = false;
        self.clear_cache = true;
        cx.notify();
        let [path] = self.sel.as_slice() else { return };
        let (path, home, gen_) = (path.clone(), self.home.clone(), self.gen_);
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    let d = inspect::details(&path, &home)?;
                    let ext_ok = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| Img::extensions().contains(&e.to_ascii_lowercase().as_str()));
                    let small = std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_PREVIEW_BYTES);
                    Some((d, ext_ok && small))
                })
                .await;
            this.update(cx, |this, cx| {
                if this.gen_ == gen_ {
                    if let Some((d, p)) = loaded {
                        this.details = Some(d);
                        this.previewable = p;
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    // ---- test accessors ------------------------------------------------------------------------

    /// "No Selection" / "N Items Selected"; `None` for a single item.
    pub fn placeholder(&self) -> Option<String> {
        match self.sel.len() {
            0 => Some("No Selection".into()),
            1 => None,
            n => Some(format!("{n} Items Selected")),
        }
    }

    /// (name, abbreviated parent) once the single item's details have loaded.
    pub fn heading(&self) -> Option<(String, String)> {
        self.details.as_ref().map(|d| (d.name.clone(), d.parent_abbrev.clone()))
    }

    pub fn rows(&self) -> Vec<(String, String)> {
        let Some(d) = &self.details else { return Vec::new() };
        let mut v = vec![("Kind".to_string(), d.kind.clone()), ("Size".to_string(), d.size.clone())];
        if let Some(r) = &d.resolution {
            v.push(("Resolution".to_string(), r.clone()));
        }
        v.push(("Created".to_string(), d.created.clone()));
        v.push(("Modified".to_string(), d.modified.clone()));
        v
    }

    /// Whether the preview is an image (as opposed to the large icon).
    pub fn shows_image(&self) -> bool {
        self.previewable
    }

    // ---- rendering -----------------------------------------------------------------------------

    /// The large icon for the preview; step 7 plugs the native icon in here.
    fn large_icon(is_dir: bool) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(rgb(theme::TEXT_DIM))
            .text_size(px(72.))
            .child(if is_dir { "\u{25B8}" } else { "\u{25A2}" })
            .into_any_element()
    }

    fn button(
        id: &'static str,
        label: &'static str,
        f: impl Fn(&mut gpui::App) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .debug_selector(move || id.to_string())
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .text_color(rgb(theme::TEXT))
            .bg(rgb(theme::HOVER_BG))
            .cursor_pointer()
            .hover(|d| d.bg(rgb(theme::SIDEBAR_ACTIVE_BG)))
            .child(label)
            .on_click(move |_, _, cx| f(cx))
    }

    fn dim(text: String) -> AnyElement {
        div().w_full().p_4().text_sm().text_color(rgb(theme::TEXT_DIM)).child(text).into_any_element()
    }
}

impl Render for Inspector {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.clear_cache) {
            let cache = self.cache.clone();
            cache.update(cx, |c, cx| c.clear(window, cx));
        }
        let root = div()
            .id("inspector")
            .debug_selector(|| "inspector".to_string())
            .flex()
            .flex_col()
            .w_full()
            .h_full()
            .overflow_hidden()
            .bg(rgb(theme::SIDEBAR_BG))
            .text_color(rgb(theme::TEXT));
        if let Some(p) = self.placeholder() {
            return root.child(Self::dim(p));
        }
        let (Some(d), [path]) = (&self.details, self.sel.as_slice()) else { return root };
        let is_dir = std::fs::metadata(path).is_ok_and(|m| m.is_dir());
        let preview: AnyElement = if self.previewable {
            img(path.clone())
                .image_cache(&self.cache)
                .object_fit(ObjectFit::Contain)
                .with_fallback(move || Self::large_icon(false))
                .size_full()
                .into_any_element()
        } else {
            Self::large_icon(is_dir)
        };
        let (open_p, reveal_p, copy_p) = (path.clone(), path.clone(), path.clone());
        let (o1, o2) = (self.opener.clone(), self.opener.clone());
        let mut table = div().flex().flex_col().gap_1().px_3().text_sm();
        for (k, v) in self.rows() {
            table = table.child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().w(px(80.)).flex_none().text_color(rgb(theme::TEXT_DIM)).child(k))
                    .child(div().flex_1().min_w_0().child(v)),
            );
        }
        root.child(div().h(px(180.)).w_full().flex_none().p_3().child(preview))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px_3()
                    .pb_2()
                    .child(div().text_base().child(d.name.clone()))
                    .child(div().text_xs().text_color(rgb(theme::TEXT_DIM)).child(d.parent_abbrev.clone())),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .px_3()
                    .pb_3()
                    .child(Self::button("inspector-open", "Open", move |cx| o1.open(cx, &open_p)))
                    .child(Self::button("inspector-reveal", "Reveal", move |_| o2.reveal(std::slice::from_ref(&reveal_p))))
                    .child(Self::button("inspector-path", "Path", move |cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_p.display().to_string()))
                    })),
            )
            .child(table)
    }
}
