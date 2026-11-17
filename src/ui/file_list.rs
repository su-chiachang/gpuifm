use crate::fm_core::entry::FileEntry;
use crate::fm_core::filter;
use crate::fm_core::format::{format_modified, format_size};
use crate::fm_core::history::{History, parent_of};
use crate::fm_core::listing::list_dir;
use crate::fm_core::ops::{OpRequest, split_ext};
use crate::fm_core::selection::Selection;
use crate::fm_core::sort::{SortKey, SortOrder, sort_entries};
use crate::ui::actions::*;
use crate::ui::ops::{FileOps, FileOpsEvent};
use crate::ui::opener::Opener;
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme;
use gpui::{
    Action, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollStrategy, Subscription, Task, UniformListScrollHandle, Window,
    anchored, deferred, div, prelude::*, px, rgb, uniform_list,
};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

pub enum FileListEvent {
    Navigated(PathBuf),
    /// A column header was clicked; the pane decides the new order and pushes it to all tabs.
    SortRequested(SortKey),
    /// The selection changed (user input or a finished listing).
    SelectionChanged,
    /// Column view: Left / Right arrow.
    ColumnLeft,
    ColumnRight,
}

/// Selection to apply (and optionally a rename to start) once the next listing lands.
#[derive(Clone)]
struct Pending {
    paths: Vec<PathBuf>,
    rename: bool,
}

struct RenameSession {
    path: PathBuf,
    input: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

pub struct FileList {
    history: History,
    /// Every entry of the directory, in sort order.
    all_entries: Vec<FileEntry>,
    /// What is on screen: `all_entries` through the filter.
    entries: Vec<FileEntry>,
    filter: Option<String>,
    /// Column-view mode: name-only rows, no header.
    compact: bool,
    opener: Rc<dyn Opener>,
    /// Directory the current `entries` belong to; None while a navigation is loading.
    entries_dir: Option<PathBuf>,
    selection: Selection,
    show_hidden: bool,
    sort: SortOrder,
    error: Option<String>,
    loading: bool,
    pending: Option<Pending>,
    load_gen: u64,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    load_task: Option<Task<()>>,
    ops: Entity<FileOps>,
    rename: Option<RenameSession>,
    /// Row context menu: where it was opened.
    menu: Option<Point<Pixels>>,
    focus_rename: bool,
    rename_request: bool,
    reload_after_rename: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FileListEvent> for FileList {}

impl Focusable for FileList {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

const SIZE_W: f32 = 90.;
const DATE_W: f32 = 170.;
const KIND_W: f32 = 140.;

fn leaf_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl FileList {
    pub fn new(
        start: PathBuf,
        ops: Entity<FileOps>,
        opener: Rc<dyn Opener>,
        show_hidden: bool,
        sort: SortOrder,
        cx: &mut Context<Self>,
    ) -> Self {
        let sub = cx.subscribe(&ops, |this, _, ev: &FileOpsEvent, cx| match ev {
            FileOpsEvent::Done { touched, select, rename, origin } => {
                if touched.iter().any(|t| t.as_path() == this.history.current()) {
                    if *origin == Some(cx.entity_id()) && (!select.is_empty() || *rename) {
                        this.pending = Some(Pending { paths: select.clone(), rename: *rename });
                    }
                    this.reload(cx);
                }
            }
            FileOpsEvent::Error(_) | FileOpsEvent::StackChanged => {}
        });
        let mut s = Self {
            history: History::new(start),
            all_entries: Vec::new(),
            entries: Vec::new(),
            filter: None,
            compact: false,
            opener,
            entries_dir: None,
            selection: Selection::default(),
            show_hidden,
            sort,
            error: None,
            loading: false,
            pending: None,
            load_gen: 0,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            load_task: None,
            ops,
            rename: None,
            menu: None,
            focus_rename: false,
            rename_request: false,
            reload_after_rename: false,
            _subs: vec![sub],
        };
        s.reload(cx);
        s
    }

    pub fn current_dir(&self) -> &Path {
        self.history.current()
    }

    pub fn can_back(&self) -> bool {
        self.history.can_back()
    }

    pub fn can_forward(&self) -> bool {
        self.history.can_forward()
    }

    pub fn can_parent(&self) -> bool {
        parent_of(self.history.current()).is_some()
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn is_renaming(&self) -> bool {
        self.rename.is_some()
    }

    /// Editor of the active rename session, if any.
    pub fn rename_input(&self) -> Option<&Entity<TextInput>> {
        self.rename.as_ref().map(|r| &r.input)
    }

    pub fn sort(&self) -> SortOrder {
        self.sort
    }

    pub(crate) fn set_compact_initial(&mut self, v: bool) {
        self.compact = v;
    }

    pub fn set_compact(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.compact != v {
            self.compact = v;
            cx.notify();
        }
    }

    /// Paths of the selected entries.
    pub fn selection_paths(&self) -> Vec<PathBuf> {
        self.selected_paths()
    }

    pub fn context_menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Entries on screen (after the filter).
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn selection_count(&self) -> usize {
        self.selection.count()
    }

    /// What path-based commands act on: the selection, or the current folder when nothing is selected.
    pub fn action_targets(&self) -> Vec<PathBuf> {
        let sel = self.selected_paths();
        if sel.is_empty() { vec![self.history.current().to_path_buf()] } else { sel }
    }

    /// The selected entry when it is the only one and a folder.
    pub fn selected_dir(&self) -> Option<PathBuf> {
        if self.selection.count() != 1 {
            return None;
        }
        self.selection.get().and_then(|i| self.entries.get(i)).filter(|e| e.is_dir).map(|e| e.path.clone())
    }

    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    pub fn selected_name(&self) -> Option<String> {
        self.selected_path().map(|p| leaf_of(&p))
    }

    pub fn entry_names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    /// Entries on screen belong to the current directory.
    fn ready(&self) -> bool {
        self.entries_dir.as_deref() == Some(self.history.current())
    }

    /// Selection-dependent actions need a settled listing and no rename in progress.
    fn can_act(&self) -> bool {
        self.ready() && self.rename.is_none()
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selection
            .get()
            .and_then(|i| self.entries.get(i))
            .map(|e| e.path.clone())
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection
            .indices()
            .into_iter()
            .filter_map(|i| self.entries.get(i).map(|e| e.path.clone()))
            .collect()
    }

    pub fn navigate(&mut self, p: PathBuf, select: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        if self.history.navigate(p) {
            self.after_nav(select, cx);
        }
    }

    /// Column view: makes `dir` the current folder without recording history.
    pub fn replace_current(&mut self, dir: PathBuf, select: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.rename.is_some() || dir == self.history.current() {
            return;
        }
        self.history.replace(dir);
        self.after_nav(select, cx);
    }

    /// Selects the first entry when nothing is selected.
    pub fn select_first(&mut self, cx: &mut Context<Self>) {
        if self.can_act() && self.selection.get().is_none() {
            self.move_selection(1, false, cx);
        }
    }

    /// Shows `dir`; when it is already current, just selects `select` (after a reload).
    pub fn go_to(&mut self, dir: PathBuf, select: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        if dir == self.history.current() {
            if let Some(p) = select {
                self.pending = Some(Pending { paths: vec![p], rename: false });
                self.reload(cx);
            }
        } else {
            self.navigate(dir, select, cx);
        }
    }

    /// Folders to pin: the selected folders, or the current folder when nothing is selected.
    pub fn pin_targets(&self) -> Vec<PathBuf> {
        if self.selection.count() == 0 {
            return vec![self.history.current().to_path_buf()];
        }
        self.selection
            .indices()
            .into_iter()
            .filter_map(|i| self.entries.get(i))
            .filter(|e| e.is_dir)
            .map(|e| e.path.clone())
            .collect()
    }

    fn after_nav(&mut self, select: Option<PathBuf>, cx: &mut Context<Self>) {
        self.pending = select.map(|p| Pending { paths: vec![p], rename: false });
        self.filter = None;
        self.all_entries.clear();
        self.entries.clear();
        self.entries_dir = None;
        self.selection.clear();
        self.menu = None;
        self.reload(cx);
        cx.emit(FileListEvent::Navigated(self.history.current().to_path_buf()));
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            self.reload_after_rename = true;
            return;
        }
        let dir = self.history.current().to_path_buf();
        let hidden = self.show_hidden;
        let sort = self.sort;
        let (prev_paths, prev_cursor) = match &self.pending {
            Some(p) => (p.paths.clone(), None),
            None => (self.selected_paths(), self.selected_path()),
        };
        // Only a requested selection scrolls into view; a plain refresh keeps the scroll position.
        let reveal = self.pending.is_some();
        self.load_gen += 1;
        let gen_ = self.load_gen;
        self.loading = true;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let d = dir.clone();
            let res = cx
                .background_executor()
                .spawn(async move {
                    list_dir(&d, hidden).map(|mut v| {
                        sort_entries(&mut v, sort);
                        v
                    })
                })
                .await;
            this.update(cx, |this, cx| {
                if this.load_gen != gen_ {
                    return;
                }
                this.loading = false;
                match res {
                    Ok(v) => {
                        this.all_entries = v;
                        this.rebuild_entries();
                        this.error = None;
                    }
                    Err(e) => {
                        this.all_entries.clear();
                        this.entries.clear();
                        this.error = Some(e.to_string());
                    }
                }
                this.entries_dir = Some(dir.clone());
                this.selection
                    .restore(&this.entries, &prev_paths, prev_cursor.as_deref());
                if reveal && let Some(i) = this.selection.get() {
                    this.scroll.scroll_to_item(i, ScrollStrategy::Center);
                }
                if let Some(p) = this.pending.take()
                    && p.rename
                    && this.selection.count() == 1
                {
                    this.rename_request = true;
                }
                cx.emit(FileListEvent::SelectionChanged);
                cx.notify();
            })
            .ok();
        }));
    }

    fn rebuild_entries(&mut self) {
        self.entries = match &self.filter {
            Some(f) => self.all_entries.iter().filter(|e| filter::matches(&e.name, f)).cloned().collect(),
            None => self.all_entries.clone(),
        };
    }

    /// Re-derives the visible entries, keeping the selection by path.
    fn refresh_view(&mut self, cx: &mut Context<Self>) {
        let (paths, cursor) = (self.selected_paths(), self.selected_path());
        self.rebuild_entries();
        self.selection.restore(&self.entries, &paths, cursor.as_deref());
        cx.emit(FileListEvent::SelectionChanged);
        cx.notify();
    }

    /// In-memory name filter (case-insensitive substring); blank clears it.
    pub fn set_filter(&mut self, f: Option<String>, cx: &mut Context<Self>) {
        let f = f.filter(|s| !s.trim().is_empty());
        if f == self.filter {
            return;
        }
        self.filter = f;
        self.refresh_view(cx);
    }

    /// Pushed by the pane (sort is per pane): re-sorts in memory.
    pub fn set_sort(&mut self, o: SortOrder, cx: &mut Context<Self>) {
        if o == self.sort {
            return;
        }
        self.sort = o;
        sort_entries(&mut self.all_entries, o);
        self.refresh_view(cx);
    }

    /// Moves focus to the list and makes sure something is selected (filter field Down).
    pub fn enter_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window);
        if self.can_act() && self.selection.get().is_none() {
            self.move_selection(1, false, cx);
        }
    }

    pub fn open_index(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.entries.get(ix) else { return };
        let path = e.path.clone();
        if e.is_dir {
            self.navigate(path, None, cx);
        } else {
            let opener = self.opener.clone();
            opener.open(cx, &path);
        }
    }

    fn move_selection(&mut self, delta: isize, extend: bool, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        self.selection.move_by(delta, self.entries.len(), extend);
        if let Some(i) = self.selection.get() {
            self.scroll.scroll_to_item(i, ScrollStrategy::Top);
        }
        cx.emit(FileListEvent::SelectionChanged);
        cx.notify();
    }

    pub fn on_select_prev(&mut self, cx: &mut Context<Self>) {
        self.move_selection(-1, false, cx);
    }

    pub fn on_select_next(&mut self, cx: &mut Context<Self>) {
        self.move_selection(1, false, cx);
    }

    pub fn on_select_prev_extend(&mut self, cx: &mut Context<Self>) {
        self.move_selection(-1, true, cx);
    }

    pub fn on_select_next_extend(&mut self, cx: &mut Context<Self>) {
        self.move_selection(1, true, cx);
    }

    pub fn on_select_all(&mut self, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        self.selection.select_all(self.entries.len());
        cx.emit(FileListEvent::SelectionChanged);
        cx.notify();
    }

    pub fn on_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let idx = self.selection.indices();
        if idx.len() == 1 {
            self.open_index(idx[0], window, cx);
        } else {
            for i in idx {
                if let Some(e) = self.entries.get(i)
                    && !e.is_dir
                {
                    let (opener, p) = (self.opener.clone(), e.path.clone());
                    opener.open(cx, &p);
                }
            }
        }
    }

    pub fn on_parent(&mut self, cx: &mut Context<Self>) {
        let cur = self.history.current().to_path_buf();
        if let Some(parent) = parent_of(&cur) {
            self.navigate(parent, Some(cur), cx);
        }
    }

    pub fn on_back(&mut self, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        if let Some(left) = self.history.back() {
            self.after_nav(Some(left), cx);
        }
    }

    pub fn on_forward(&mut self, cx: &mut Context<Self>) {
        if self.rename.is_some() {
            return;
        }
        if let Some(left) = self.history.forward() {
            self.after_nav(Some(left), cx);
        }
    }

    /// Pushed by the browser (hidden-file visibility is global); reloads when it changes.
    pub fn set_show_hidden(&mut self, v: bool, cx: &mut Context<Self>) {
        if self.show_hidden == v {
            return;
        }
        self.show_hidden = v;
        self.reload(cx);
        cx.notify();
    }

    /// Tab title: the directory's leaf name ("/" for the root).
    pub fn title(&self) -> String {
        let l = leaf_of(self.history.current());
        if l.is_empty() { "/".into() } else { l }
    }

    // ---- file operations -------------------------------------------------------------------

    pub fn on_copy(&mut self, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.ops.update(cx, |o, cx| o.set_clipboard(paths, cx));
    }

    /// Puts the target paths (newline-joined) on the system clipboard; the internal file
    /// clipboard used by Copy / Paste is left alone.
    pub fn on_copy_path(&mut self, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let text = self.action_targets().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    pub fn on_paste(&mut self, move_items: bool, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let srcs = self.ops.read(cx).clipboard().to_vec();
        if srcs.is_empty() {
            return;
        }
        let dest = self.history.current().to_path_buf();
        let req = if move_items {
            OpRequest::Move { srcs, dest }
        } else {
            OpRequest::Copy { srcs, dest }
        };
        let me = cx.entity_id();
        self.ops.update(cx, |o, cx| o.run(req, me, false, cx));
    }

    pub fn on_duplicate(&mut self, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let sel = self.selected_paths();
        if sel.is_empty() {
            return;
        }
        let me = cx.entity_id();
        self.ops.update(cx, |o, cx| o.run(OpRequest::Duplicate(sel), me, false, cx));
    }

    pub fn on_trash(&mut self, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let sel = self.selected_paths();
        if sel.is_empty() {
            return;
        }
        let me = cx.entity_id();
        self.ops.update(cx, |o, cx| o.run(OpRequest::Trash(sel), me, false, cx));
    }

    pub fn on_new_folder(&mut self, cx: &mut Context<Self>) {
        if !self.can_act() {
            return;
        }
        let dir = self.history.current().to_path_buf();
        let me = cx.entity_id();
        self.ops.update(cx, |o, cx| o.run(OpRequest::NewFolder { dir }, me, true, cx));
    }

    // ---- inline rename ---------------------------------------------------------------------

    pub fn begin_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_act() || self.selection.count() != 1 || self.ops.read(cx).busy() {
            return;
        }
        let Some(e) = self.selection.get().and_then(|i| self.entries.get(i)) else {
            return;
        };
        let (path, name, real_dir) = (e.path.clone(), e.name.clone(), e.is_dir);
        let stem_len = split_ext(&name, real_dir).0.len();
        let input = cx.new(|cx| TextInput::new(&name, cx));
        input.update(cx, |i, cx| i.select_range(0..stem_len, cx));
        let handle = input.read(cx).focus_handle(cx);
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &TextInputEvent, window, cx| match ev {
                TextInputEvent::Confirm => this.commit_rename(window, cx),
                TextInputEvent::Cancel => this.cancel_rename(window, cx),
            }),
            cx.on_blur(&handle, window, |this, window, cx| this.commit_rename(window, cx)),
        ];
        self.rename = Some(RenameSession { path, input, _subs: subs });
        self.focus_rename = true;
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.rename.take() else { return };
        let new = s.input.read(cx).text().to_string();
        self.focus.focus(window);
        if new != leaf_of(&s.path) {
            let req = OpRequest::Rename { src: s.path.clone(), name: new };
            let me = cx.entity_id();
            self.ops.update(cx, |o, cx| o.run(req, me, false, cx));
        }
        self.after_rename(cx);
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rename.take().is_none() {
            return;
        }
        self.focus.focus(window);
        self.after_rename(cx);
    }

    fn after_rename(&mut self, cx: &mut Context<Self>) {
        self.focus_rename = false;
        if std::mem::take(&mut self.reload_after_rename) {
            self.reload(cx);
        }
        cx.notify();
    }

    fn menu_item(
        &self,
        tag: &'static str,
        label: &'static str,
        action: fn() -> Box<dyn Action>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(tag)
            .debug_selector(move || tag.to_string())
            .px_3()
            .py_1()
            .cursor_pointer()
            .hover(|d| d.bg(rgb(theme::HOVER_BG)))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.menu = None;
                this.focus.focus(window);
                // Run the ordinary action once the menu is gone and focus is in the list.
                let a = action();
                cx.defer_in(window, move |_, window, cx| window.dispatch_action(a, cx));
                cx.notify();
            }))
    }

    fn separator() -> impl IntoElement {
        div().h(px(1.)).my_1().bg(rgb(theme::BORDER))
    }

    fn header_cell(&self, key: SortKey, label: &'static str, w: Option<f32>, cx: &mut Context<Self>) -> impl IntoElement {
        let tag = match key {
            SortKey::Name => "name",
            SortKey::Kind => "kind",
            SortKey::Size => "size",
            SortKey::Modified => "modified",
        };
        let text = if self.sort.key == key {
            format!("{label} {}", if self.sort.ascending { "\u{25B2}" } else { "\u{25BC}" })
        } else {
            label.to_string()
        };
        let d = div()
            .id(tag)
            .debug_selector(move || format!("header-{tag}"))
            .px_2()
            .cursor_pointer()
            .text_color(rgb(theme::TEXT_DIM))
            .hover(|d| d.text_color(rgb(theme::TEXT)))
            .child(text)
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(FileListEvent::SortRequested(key))));
        match w {
            Some(w) => d.w(px(w)).flex_none(),
            None => d.flex_1().min_w_0(),
        }
    }
}

impl Render for FileList {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.rename_request) {
            self.begin_rename(window, cx);
        }
        if self.focus_rename
            && let Some(s) = &self.rename
        {
            s.input.read(cx).focus_handle(cx).focus(window);
            self.focus_rename = false;
        }

        let compact = self.compact;
        let header = div()
            .flex()
            .flex_none()
            .h(px(24.))
            .items_center()
            .text_xs()
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(self.header_cell(SortKey::Name, "Name", None, cx))
            .child(self.header_cell(SortKey::Size, "Size", Some(SIZE_W), cx))
            .child(self.header_cell(SortKey::Modified, "Date Modified", Some(DATE_W), cx))
            .child(self.header_cell(SortKey::Kind, "Kind", Some(KIND_W), cx));

        let body = if let Some(err) = self.error.clone() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::ERROR))
                .child(err)
                .into_any_element()
        } else if self.entries.is_empty() && self.filter.is_some() && self.ready() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::TEXT_DIM))
                .child("No Matches")
                .into_any_element()
        } else {
            uniform_list(
                "files",
                self.entries.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                    let now = SystemTime::now();
                    range
                        .filter_map(|ix| this.entries.get(ix).map(|e| (ix, e)))
                        .map(|(ix, e)| {
                            let is_sel = this.selection.contains(ix);
                            let renaming = this
                                .rename
                                .as_ref()
                                .filter(|r| r.path == e.path)
                                .map(|r| r.input.clone());
                            let path = e.path.clone();
                            let name = if e.is_dir {
                                format!("\u{25B8} {}", e.name)
                            } else {
                                format!("  {}", e.name)
                            };
                            let dim = rgb(if is_sel { theme::TEXT } else { theme::TEXT_DIM });
                            let name_cell = match renaming {
                                Some(input) => div().flex_1().min_w_0().px_1().child(input),
                                None => div()
                                    .flex_1()
                                    .min_w_0()
                                    .px_2()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(name),
                            };
                            div()
                                .id(ix)
                                .debug_selector(move || format!("row-{ix}"))
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .flex()
                                .items_center()
                                .h(px(24.))
                                .text_sm()
                                .text_color(rgb(theme::TEXT))
                                .when(is_sel, |d| d.bg(rgb(theme::SELECTED_BG)))
                                .when(!is_sel, |d| d.hover(|d| d.bg(rgb(theme::HOVER_BG))))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                                        // The list's own right-click (empty space) must not also run.
                                        cx.stop_propagation();
                                        if !this.can_act() {
                                            return;
                                        }
                                        if !this.selection.contains(ix) {
                                            this.selection.set(Some(ix));
                                            cx.emit(FileListEvent::SelectionChanged);
                                        }
                                        this.focus.focus(window);
                                        this.menu = Some(ev.position);
                                        cx.notify();
                                    }),
                                )
                                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                                    if !this.ready() {
                                        return;
                                    }
                                    if let Some(r) = &this.rename {
                                        // Clicks inside the editor belong to it; elsewhere commit via blur.
                                        if r.path != path {
                                            this.focus.focus(window);
                                        }
                                        return;
                                    }
                                    this.focus.focus(window);
                                    let m = ev.modifiers();
                                    if m.platform {
                                        this.selection.toggle(ix);
                                    } else if m.shift {
                                        this.selection.extend_to(ix);
                                    } else {
                                        this.selection.set(Some(ix));
                                        if ev.click_count() >= 2 {
                                            this.open_index(ix, window, cx);
                                        }
                                    }
                                    cx.emit(FileListEvent::SelectionChanged);
                                    cx.notify();
                                }))
                                .child(name_cell)
                                .when(!compact, |d| {
                                    d
                                .child(
                                    div()
                                        .w(px(SIZE_W))
                                        .flex_none()
                                        .px_2()
                                        .text_color(dim)
                                        .child(format_size(e.size)),
                                )
                                .child(
                                    div()
                                        .w(px(DATE_W))
                                        .flex_none()
                                        .px_2()
                                        .text_color(dim)
                                        .child(format_modified(e.modified, now)),
                                )
                                .child(
                                    div()
                                        .w(px(KIND_W))
                                        .flex_none()
                                        .px_2()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_color(dim)
                                        .child(e.kind.clone()),
                                )
                                })
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(self.scroll.clone())
            .w_full()
            .min_w_0()
            .flex_grow()
            .into_any_element()
        };

        let menu = self.menu.map(|pos| {
            let single = self.selection.count() == 1;
            let m = div()
                .id("file-menu")
                .occlude()
                .min_w(px(190.))
                .py_1()
                .rounded_md()
                .bg(rgb(theme::TOOLBAR_BG))
                .border_1()
                .border_color(rgb(theme::BORDER))
                .text_sm()
                .text_color(rgb(theme::TEXT))
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.menu = None;
                    cx.notify();
                }))
                .child(self.menu_item("ctx-open", "Open", || Box::new(OpenSelected), cx))
                .child(self.menu_item("ctx-new-tab", "Open in New Tab", || Box::new(OpenInNewTab), cx))
                .child(self.menu_item("ctx-split", "Open in Split", || Box::new(OpenInSplit), cx))
                .child(self.menu_item("ctx-quicklook", "Quick Look", || Box::new(QuickLook), cx))
                .child(Self::separator())
                .when(single, |d| d.child(self.menu_item("ctx-rename", "Rename", || Box::new(BeginRename), cx)))
                .child(self.menu_item("ctx-duplicate", "Duplicate", || Box::new(Duplicate), cx))
                .child(self.menu_item("ctx-copy", "Copy", || Box::new(CopyItems), cx))
                .child(self.menu_item("ctx-copy-path", "Copy Path", || Box::new(CopyPath), cx))
                .child(Self::separator())
                .child(self.menu_item("ctx-trash", "Move to Trash", || Box::new(TrashItems), cx))
                .child(Self::separator())
                .child(self.menu_item("ctx-reveal", "Reveal in Finder", || Box::new(RevealInFinder), cx))
                .child(self.menu_item("ctx-pin", "Pin to Sidebar", || Box::new(PinSelection), cx));
            deferred(anchored().position(pos).snap_to_window().child(m))
        });

        div()
            .key_context(if compact { "FileList compact" } else { "FileList" })
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    // Empty space: drop the selection.
                    if !this.can_act() {
                        return;
                    }
                    this.menu = None;
                    if this.selection.count() > 0 {
                        this.selection.clear();
                        cx.emit(FileListEvent::SelectionChanged);
                    }
                    this.focus.focus(window);
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|this, _: &ColumnLeft, _, cx| {
                if this.can_act() {
                    cx.emit(FileListEvent::ColumnLeft);
                }
            }))
            .on_action(cx.listener(|this, _: &ColumnRight, _, cx| {
                if this.can_act() {
                    cx.emit(FileListEvent::ColumnRight);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.on_select_prev(cx)))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.on_select_next(cx)))
            .on_action(cx.listener(|this, _: &SelectPrevExtend, _, cx| this.on_select_prev_extend(cx)))
            .on_action(cx.listener(|this, _: &SelectNextExtend, _, cx| this.on_select_next_extend(cx)))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.on_select_all(cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, window, cx| this.on_open(window, cx)))
            .on_action(cx.listener(|this, _: &BeginRename, window, cx| this.begin_rename(window, cx)))
            .on_action(cx.listener(|this, _: &CopyItems, _, cx| this.on_copy(cx)))
            .on_action(cx.listener(|this, _: &CopyPath, _, cx| this.on_copy_path(cx)))
            .on_action(cx.listener(|this, _: &PasteItems, _, cx| this.on_paste(false, cx)))
            .on_action(cx.listener(|this, _: &PasteMove, _, cx| this.on_paste(true, cx)))
            .on_action(cx.listener(|this, _: &Duplicate, _, cx| this.on_duplicate(cx)))
            .on_action(cx.listener(|this, _: &NewFolder, _, cx| this.on_new_folder(cx)))
            .on_action(cx.listener(|this, _: &TrashItems, _, cx| this.on_trash(cx)))
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(theme::BG))
            .when(!compact, |d| d.child(header))
            .child(body)
            .children(menu)
    }
}
