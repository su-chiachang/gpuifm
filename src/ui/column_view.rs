//! Column browser (Miller columns) for one pane.
//!
//! The pane's active tab list is the *current* column (it keeps selection, rename, file ops and
//! Quick Look); read-only columns to its left show the ancestors, and a read-only preview column
//! on its right shows the selected folder. Moving between columns replaces the tab's folder
//! without recording history.

use crate::fm_core::entry::FileEntry;
use crate::ui::dnd::{DragPreview, DraggedFiles};
use crate::ui::icons::{IconCache, icon_el};
use crate::fm_core::history::parent_of;
use crate::fm_core::listing::list_dir;
use crate::fm_core::sort::{SortOrder, sort_entries};
use crate::ui::file_list::{FileList, FileListEvent};
use crate::ui::inspector::Inspector;
use crate::ui::opener::Opener;
use crate::ui::theme;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, ExternalPaths, Focusable, IntoElement, Render, ScrollHandle, Subscription, Window, div,
    prelude::*, px, rgb, uniform_list,
};
use std::any::Any;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const COL_W: f32 = 220.;

/// A read-only column. Its folder is known at once; the entries arrive from a background read
/// (`gen` identifies the read whose result this column accepts).
struct Col {
    dir: PathBuf,
    entries: Vec<FileEntry>,
    loaded: bool,
    gen_: u64,
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
    inspector: Option<Entity<Inspector>>,
    next_gen: u64,
    /// Drilled into this folder before its preview had loaded: select its first entry once the
    /// list has loaded (cleared when that load completes, empty or not, or on navigating away).
    select_first_after_load: Option<PathBuf>,
    icons: Option<Entity<IconCache>>,
    icons_sub: Option<Subscription>,
    _sub: Subscription,
}

impl ColumnView {
    pub fn new(list: Entity<FileList>, sort: SortOrder, hidden: bool, opener: Rc<dyn Opener>, cx: &mut Context<Self>) -> Self {
        let sub = Self::subscribe(&list, cx);
        let mut s = Self { list, lefts: Vec::new(), preview: None, sort, hidden, opener, scroll: ScrollHandle::new(), shown_cols: 0, inspector: None, next_gen: 0, select_first_after_load: None, icons: None, icons_sub: None, _sub: sub };
        s.sync_lefts(cx);
        s.update_preview(cx);
        s
    }

    /// Shows native icons from `icons` in the cells (glyphs without it).
    pub fn set_icons(&mut self, icons: Entity<IconCache>, cx: &mut Context<Self>) {
        self.icons_sub = Some(cx.observe(&icons, |_, _, cx| cx.notify()));
        self.icons = Some(icons);
    }

    fn subscribe(list: &Entity<FileList>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(list, |this, _, ev: &FileListEvent, cx| match ev {
            FileListEvent::Navigated(dir) => {
                if this.select_first_after_load.as_ref() != Some(dir) {
                    this.select_first_after_load = None;
                }
                this.sync_lefts(cx);
                this.update_preview(cx);
            }
            FileListEvent::SelectionChanged => {
                if this.select_first_after_load.is_some() && this.list.read(cx).is_ready() {
                    let target = this.select_first_after_load.take();
                    if target.as_deref() == Some(this.list.read(cx).current_dir()) {
                        this.list.update(cx, |l, cx| l.select_first(cx));
                    }
                }
                this.update_preview(cx);
            }
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

    /// Shows (or hides) the inspector as the trailing column.
    pub fn set_inspector(&mut self, i: Option<Entity<Inspector>>, cx: &mut Context<Self>) {
        self.inspector = i;
        cx.notify();
    }

    pub fn has_inspector(&self) -> bool {
        self.inspector.is_some()
    }

    pub fn set_sort(&mut self, o: SortOrder, cx: &mut Context<Self>) {
        self.sort = o;
        self.reload(cx);
    }

    pub fn set_show_hidden(&mut self, v: bool, cx: &mut Context<Self>) {
        self.hidden = v;
        self.reload(cx);
    }

    /// Starts a background read of `dir`; the result is accepted by the column carrying the
    /// returned generation (a newer read of that column supersedes it).
    fn start_load(&mut self, dir: PathBuf, cx: &mut Context<Self>) -> u64 {
        self.next_gen += 1;
        let gen_ = self.next_gen;
        let (hidden, sort) = (self.hidden, self.sort);
        cx.spawn(async move |this, cx| {
            let entries = cx
                .background_executor()
                .spawn(async move {
                    let mut e = list_dir(&dir, hidden).unwrap_or_default();
                    sort_entries(&mut e, sort);
                    e
                })
                .await;
            this.update(cx, |this, cx| this.fill(gen_, entries, cx)).ok();
        })
        .detach();
        gen_
    }

    fn fill(&mut self, gen_: u64, entries: Vec<FileEntry>, cx: &mut Context<Self>) {
        if let Some(c) = self.lefts.iter_mut().chain(self.preview.iter_mut()).find(|c| c.gen_ == gen_) {
            c.entries = entries;
            c.loaded = true;
            cx.notify();
        }
    }

    fn new_col(&mut self, dir: PathBuf, cx: &mut Context<Self>) -> Col {
        let gen_ = self.start_load(dir.clone(), cx);
        Col { dir, entries: Vec::new(), loaded: false, gen_ }
    }

    /// Re-reads the read-only columns from disk (they keep showing the old entries meanwhile).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        for i in 0..self.lefts.len() {
            let dir = self.lefts[i].dir.clone();
            self.lefts[i].gen_ = self.start_load(dir, cx);
        }
        self.preview = None;
        self.update_preview(cx);
        cx.notify();
    }

    /// A first-entry selection is still waiting for the drilled-into folder to load.
    pub fn select_first_pending(&self) -> bool {
        self.select_first_after_load.is_some()
    }

    /// Entry names of the preview column once it has loaded.
    pub fn preview_names(&self) -> Option<Vec<String>> {
        self.preview.as_ref().filter(|c| c.loaded).map(|c| c.entries.iter().map(|e| e.name.clone()).collect())
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
        self.lefts = parent.into_iter().map(|p| self.new_col(p, cx)).collect();
        cx.notify();
    }

    fn update_preview(&mut self, cx: &mut Context<Self>) {
        let want = self.list.read(cx).selected_dir();
        if self.preview.as_ref().map(|c| &c.dir) == want.as_ref() {
            return;
        }
        self.preview = want.map(|d| self.new_col(d, cx));
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
        let col = self.new_col(cur, cx);
        self.lefts.push(col);
        let select = select.or_else(|| p.entries.first().map(|e| e.path.clone()));
        let wait = select.is_none() && !p.loaded;
        self.select_first_after_load = wait.then(|| p.dir.clone());
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
        let col_dir = match kind {
            Kind::Left(i) => self.lefts[i].dir.clone(),
            Kind::Preview => self.preview.as_ref().map(|c| c.dir.clone()).unwrap_or_default(),
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
                        let (is_dir, dest) = (e.is_dir, e.path.clone());
                        let drag = DraggedFiles::new(vec![e.path.clone()], e.is_dir);
                        let icon = icon_el(this.icons.as_ref(), &e.path, e.is_dir, e.is_package, 16., cx);
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
                            .gap_1()
                            .px_2()
                            .text_sm()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(rgb(theme::TEXT))
                            .when(is_sel, |d| d.bg(rgb(theme::SIDEBAR_ACTIVE_BG)))
                            .when(!is_sel, |d| d.hover(|d| d.bg(rgb(theme::HOVER_BG))))
                            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| this.click(kind, ix, ev, window, cx)))
                            .on_drag(drag, |dragged, _, _, cx| cx.new(|_| DragPreview::for_paths(&dragged.paths)))
                            .when(is_dir, |d| Self::drop_target(d, dest, cx, theme::DROP_BG))
                            .child(icon)
                            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(e.name.clone()))
                    })
                    .collect()
            }),
        )
        .size_full();
        let wrap = div().w(px(COL_W)).h_full().flex_none().border_r_1().border_color(rgb(theme::BORDER));
        Self::drop_target(wrap, col_dir, cx, theme::DROP_BG).child(list).into_any_element()
    }

    /// Makes `el` a drop target onto folder `dest` (internal and Finder drops).
    fn drop_target<E: InteractiveElement + 'static>(el: E, dest: PathBuf, cx: &mut Context<Self>, bg: u32) -> E {
        let weak = cx.entity().downgrade();
        let pred_dest = dest.clone();
        let (dest_int, dest_ext) = (dest.clone(), dest);
        el.drag_over::<DraggedFiles>(move |s, _, _, _| s.bg(rgb(bg)))
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.bg(rgb(bg)))
            .can_drop(move |payload: &dyn Any, window: &mut Window, cx: &mut App| {
                let alt = window.modifiers().alt;
                weak.read_with(cx, |this, cx| this.list.read(cx).accepts_drop(payload, &pred_dest, alt, cx)).unwrap_or(false)
            })
            .on_drop(cx.listener(move |this, f: &DraggedFiles, window, cx| {
                let alt = window.modifiers().alt;
                let paths = f.paths.clone();
                let dest: &Path = &dest_int;
                this.list.update(cx, |l, cx| l.drop_files(&paths, dest, alt, cx));
            }))
            .on_drop(cx.listener(move |this, e: &ExternalPaths, _, cx| {
                this.list.update(cx, |l, cx| l.drop_external(e.paths(), &dest_ext, cx));
            }))
    }
}

impl Render for ColumnView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let n = self.lefts.len() + 1 + usize::from(self.preview.is_some()) + usize::from(self.inspector.is_some());
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
        if let Some(i) = &self.inspector {
            row = row.child(
                div().id("inspector-col").debug_selector(|| "inspector-col".to_string()).w(px(260.)).h_full().flex_none().border_l_1().border_color(rgb(theme::BORDER)).child(i.clone()),
            );
        }
        div().flex_1().min_h_0().w_full().child(row)
    }
}
