//! Column browser (Miller columns) for one pane.
//!
//! The pane's active tab list is the *current* column (it keeps selection, rename, file ops and
//! Quick Look); read-only columns to its left show the ancestors, and a read-only preview column
//! on its right shows the selected folder. Moving between columns replaces the tab's folder
//! without recording history.

use crate::fm_core::entry::FileEntry;
use crate::fm_core::history::parent_of;
use crate::fm_core::listing::list_dir;
use crate::fm_core::sort::{SortOrder, sort_entries};
use crate::ui::file_list::{FileList, FileListEvent};
use crate::ui::opener::Opener;
use crate::ui::theme;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, Focusable, IntoElement, Render, ScrollHandle, Subscription, Window, div,
    prelude::*, px, rgb, uniform_list,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;

const COL_W: f32 = 220.;

struct Col {
    dir: PathBuf,
    entries: Vec<FileEntry>,
}

impl Col {
    fn load(dir: &Path, hidden: bool, sort: SortOrder) -> Self {
        let mut entries = list_dir(dir, hidden).unwrap_or_default();
        sort_entries(&mut entries, sort);
        Self { dir: dir.to_path_buf(), entries }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Left(usize),
    Preview,
}

pub struct ColumnView {
    list: Entity<FileList>,
    lefts: Vec<Col>,
    preview: Option<Col>,
    sort: SortOrder,
    hidden: bool,
    opener: Rc<dyn Opener>,
    scroll: ScrollHandle,
    shown_cols: usize,
    _sub: Subscription,
}

impl ColumnView {
    pub fn new(list: Entity<FileList>, sort: SortOrder, hidden: bool, opener: Rc<dyn Opener>, cx: &mut Context<Self>) -> Self {
        let sub = Self::subscribe(&list, cx);
        let mut s = Self { list, lefts: Vec::new(), preview: None, sort, hidden, opener, scroll: ScrollHandle::new(), shown_cols: 0, _sub: sub };
        s.sync_lefts(cx);
        s.update_preview(cx);
        s
    }

    fn subscribe(list: &Entity<FileList>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(list, |this, _, ev: &FileListEvent, cx| match ev {
            FileListEvent::Navigated(_) => {
                this.sync_lefts(cx);
                this.update_preview(cx);
            }
            FileListEvent::SelectionChanged => this.update_preview(cx),
            FileListEvent::ColumnLeft => this.left(cx),
            FileListEvent::ColumnRight => this.right(cx),
            FileListEvent::SortRequested(_) => {}
        })
    }

    /// The pane switched to another tab.
    pub fn set_list(&mut self, list: Entity<FileList>, cx: &mut Context<Self>) {
        if list.entity_id() == self.list.entity_id() {
            return;
        }
        self._sub = Self::subscribe(&list, cx);
        self.list = list;
        self.lefts.clear();
        self.preview = None;
        self.sync_lefts(cx);
        self.update_preview(cx);
        cx.notify();
    }

    pub fn set_sort(&mut self, o: SortOrder, cx: &mut Context<Self>) {
        self.sort = o;
        self.reload(cx);
    }

    pub fn set_show_hidden(&mut self, v: bool, cx: &mut Context<Self>) {
        self.hidden = v;
        self.reload(cx);
    }

    /// Re-reads the read-only columns from disk.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (h, s) = (self.hidden, self.sort);
        for c in &mut self.lefts {
            *c = Col::load(&c.dir, h, s);
        }
        self.preview = None;
        self.update_preview(cx);
        cx.notify();
    }

    /// Folders of all columns, left to right: ancestors, the current folder, the preview folder.
    pub fn column_dirs(&self, cx: &gpui::App) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.lefts.iter().map(|c| c.dir.clone()).collect();
        v.push(self.list.read(cx).current_dir().to_path_buf());
        v.extend(self.preview.as_ref().map(|c| c.dir.clone()));
        v
    }

    /// Keeps the ancestor columns when they still lead to the current folder, else rebuilds
    /// them as just the parent.
    fn sync_lefts(&mut self, cx: &mut Context<Self>) {
        let cur = self.list.read(cx).current_dir().to_path_buf();
        let parent = parent_of(&cur);
        if parent.is_some() && self.lefts.last().map(|c| &c.dir) == parent.as_ref() {
            return;
        }
        self.lefts = parent.iter().map(|p| Col::load(p, self.hidden, self.sort)).collect();
        cx.notify();
    }

    fn update_preview(&mut self, cx: &mut Context<Self>) {
        let want = self.list.read(cx).selected_dir();
        if self.preview.as_ref().map(|c| &c.dir) == want.as_ref() {
            return;
        }
        self.preview = want.map(|d| Col::load(&d, self.hidden, self.sort));
        cx.notify();
    }

    fn left(&mut self, cx: &mut Context<Self>) {
        let cur = self.list.read(cx).current_dir().to_path_buf();
        let target = match self.lefts.pop() {
            Some(c) => c.dir,
            None => match parent_of(&cur) {
                Some(p) => p,
                None => return,
            },
        };
        self.list.update(cx, |l, cx| l.replace_current(target, Some(cur), cx));
    }

    /// Moves into the preview column, selecting `select` (default: its first entry).
    fn drill(&mut self, select: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(p) = self.preview.take() else { return };
        let cur = self.list.read(cx).current_dir().to_path_buf();
        self.lefts.push(Col::load(&cur, self.hidden, self.sort));
        let select = select.or_else(|| p.entries.first().map(|e| e.path.clone()));
        self.list.update(cx, |l, cx| l.replace_current(p.dir, select, cx));
    }

    fn right(&mut self, cx: &mut Context<Self>) {
        self.drill(None, cx);
    }

    fn click(&mut self, kind: Kind, ix: usize, ev: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let col = match kind {
            Kind::Left(i) => self.lefts.get(i),
            Kind::Preview => self.preview.as_ref(),
        };
        let Some(entry) = col.and_then(|c| c.entries.get(ix)).cloned() else { return };
        match kind {
            Kind::Preview => self.drill(Some(entry.path.clone()), cx),
            Kind::Left(i) => {
                if entry.is_dir {
                    self.lefts.truncate(i + 1);
                    self.list.update(cx, |l, cx| l.replace_current(entry.path.clone(), None, cx));
                } else {
                    let dir = self.lefts[i].dir.clone();
                    self.lefts.truncate(i);
                    let sel = entry.path.clone();
                    self.list.update(cx, |l, cx| l.replace_current(dir, Some(sel), cx));
                }
            }
        }
        self.list.read(cx).focus_handle(cx).focus(window);
        if ev.click_count() >= 2 && !entry.is_dir {
            let opener = self.opener.clone();
            opener.open(cx, &entry.path);
        }
    }

    fn column(&self, kind: Kind, cx: &mut Context<Self>) -> AnyElement {
        let (tag, count, selected): (usize, usize, Option<PathBuf>) = match kind {
            Kind::Left(i) => {
                let next = self.lefts.get(i + 1).map(|c| c.dir.clone()).unwrap_or_else(|| self.list.read(cx).current_dir().to_path_buf());
                (i, self.lefts[i].entries.len(), Some(next))
            }
            Kind::Preview => (usize::MAX, self.preview.as_ref().map_or(0, |c| c.entries.len()), None),
        };
        let list = uniform_list(
            ("column", tag),
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                let col = match kind {
                    Kind::Left(i) => this.lefts.get(i),
                    Kind::Preview => this.preview.as_ref(),
                };
                let Some(col) = col else { return Vec::new() };
                range
                    .filter_map(|ix| col.entries.get(ix).map(|e| (ix, e)))
                    .map(|(ix, e)| {
                        let is_sel = selected.as_deref() == Some(e.path.as_path());
                        div()
                            .id(ix)
                            .debug_selector({
                                let n = e.name.clone();
                                move || format!("col-{tag}-{n}")
                            })
                            .w_full()
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .px_2()
                            .text_sm()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(rgb(theme::TEXT))
                            .when(is_sel, |d| d.bg(rgb(theme::SIDEBAR_ACTIVE_BG)))
                            .when(!is_sel, |d| d.hover(|d| d.bg(rgb(theme::HOVER_BG))))
                            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| this.click(kind, ix, ev, window, cx)))
                            .child(if e.is_dir { format!("\u{25B8} {}", e.name) } else { format!("  {}", e.name) })
                    })
                    .collect()
            }),
        )
        .size_full();
        div().w(px(COL_W)).h_full().flex_none().border_r_1().border_color(rgb(theme::BORDER)).child(list).into_any_element()
    }
}

impl Render for ColumnView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.lefts.len() + 1 + usize::from(self.preview.is_some());
        if n != self.shown_cols {
            self.shown_cols = n;
            self.scroll.scroll_to_item(n - 1);
        }
        let mut row = div()
            .id("columns")
            .key_context("ColumnView")
            .flex()
            .flex_row()
            .size_full()
            .overflow_x_scroll()
            .track_scroll(&self.scroll);
        for i in 0..self.lefts.len() {
            row = row.child(self.column(Kind::Left(i), cx));
        }
        row = row.child(
            div().w(px(COL_W)).h_full().flex_none().border_r_1().border_color(rgb(theme::BORDER)).child(self.list.clone()),
        );
        if self.preview.is_some() {
            row = row.child(self.column(Kind::Preview, cx));
        }
        div().flex_1().min_h_0().w_full().child(row)
    }
}
