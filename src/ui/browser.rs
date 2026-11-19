use crate::fm_core::layout::{Axis, LayoutTree, PaneId, Side, focus_after_close};
use crate::fm_core::places::{self, GoTarget, Places};
use crate::fm_core::search_index::{self, IndexEntry};
use crate::fm_core::sort::SortOrder;
use crate::fm_core::watch::{DirWatcher, canon};
use crate::fm_core::state::{PaneSnapshot, PersistedState, SidebarEntry, StateStore, ViewMode, restore};
use crate::ui::actions::*;
use crate::ui::commands;
use crate::ui::dnd;
use crate::ui::file_list::FileList;
use crate::ui::menus;
use crate::ui::opener::{Opener, SystemOpener};
use crate::ui::ops::{FileOps, FileOpsEvent};
use crate::ui::palette::{Choice, OpenMode, Palette, PaletteEvent};
use crate::ui::pane::{Pane, PaneEvent};
use crate::ui::sidebar::{Group, Sidebar, SidebarEvent};
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme;
use gpui::{
    AnyElement, App, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle, Focusable, IntoElement, Render, Subscription,
    Task, Window, deferred, div, prelude::*, px, relative, rgb,
};
use futures::StreamExt;
use std::collections::{BTreeSet, HashMap};
use crate::ui::icons::{IconCache, IconSource, NoIcons};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(600);
const INDEX_DELAY: Duration = Duration::from_secs(3);
const INDEX_STALE: Duration = Duration::from_secs(600);
const REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);

/// Everything the browser needs from outside; tests pass tempdirs and fakes.
pub struct BrowserOptions {
    /// Folder for the first pane; when set, saved layouts are not restored.
    pub start: Option<PathBuf>,
    /// Where state is loaded from / saved to. None: nothing is persisted and the sidebar starts
    /// empty (no default favorites).
    pub store: Option<StateStore>,
    /// Explicit index roots. None: derived from favorites and pins (only when `store` is set).
    pub index_roots: Option<Vec<PathBuf>>,
    pub opener: Rc<dyn Opener>,
    /// Watch the open tabs' folders for outside changes (tests drive `dirs_changed` instead).
    pub watch: bool,
    /// Home and the Go menu's folders; tests root them in a tempdir.
    pub places: Places,
    /// Where file icons come from (tests: none, or a fake).
    pub icons: Rc<dyn IconSource>,
}

impl Default for BrowserOptions {
    fn default() -> Self {
        Self {
            start: None,
            store: None,
            index_roots: None,
            opener: Rc::new(SystemOpener::default()),
            watch: false,
            places: Places::system(),
            icons: Rc::new(NoIcons),
        }
    }
}

fn not_found(raw: &str) -> String {
    format!("The folder \u{201C}{raw}\u{201D} can\u{2019}t be found.")
}

/// Drag payload for a split divider: which split (by child-index path) and which divider in it.
#[derive(Clone)]
struct DraggedDivider {
    path: Vec<usize>,
    ix: usize,
}

struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub struct Browser {
    sidebar: Entity<Sidebar>,
    ops: Entity<FileOps>,
    icons: Entity<IconCache>,
    panes: HashMap<PaneId, Entity<Pane>>,
    pane_subs: HashMap<PaneId, Vec<Subscription>>,
    layout: LayoutTree<PaneId>,
    focused: PaneId,
    next_pane_id: u64,
    show_hidden: bool,
    sidebar_visible: bool,
    title: String,
    banner: Option<(u64, String)>,
    banner_id: u64,
    focus: FocusHandle,
    favorites: Vec<SidebarEntry>,
    pinned: Vec<SidebarEntry>,
    opener: Rc<dyn Opener>,
    places: Places,
    store: Option<StateStore>,
    last_saved: Option<PersistedState>,
    save_gen: u64,
    _save_task: Option<Task<()>>,
    index: Arc<Vec<IndexEntry>>,
    index_fixed_roots: Option<Vec<PathBuf>>,
    index_enabled: bool,
    index_built: Option<Instant>,
    index_gen: u64,
    _index_task: Option<Task<()>>,
    palette: Option<(Entity<Palette>, Subscription)>,
    goto: Option<(Entity<TextInput>, Vec<Subscription>)>,
    goto_focus_pending: bool,
    ql_paths: Vec<PathBuf>,
    watcher: Option<DirWatcher>,
    dirty: BTreeSet<PathBuf>,
    refresh_gen: u64,
    _refresh_task: Option<Task<()>>,
    _watch_task: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl Browser {
    /// No persistence, no index, one pane (the headless tests' default).
    pub fn new(start: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_options(BrowserOptions { start, ..Default::default() }, window, cx)
    }

    pub fn with_options(opts: BrowserOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let BrowserOptions { start, store, index_roots, opener, watch, places, icons } = opts;
        let icons = cx.new(|_| IconCache::new(icons, places.home.clone()));
        let loaded = store.as_ref().map(|s| s.load_with_home(&places.home));
        let (favorites, pinned, show_hidden, sidebar_visible) = match &loaded {
            Some(l) => (l.favorites.clone(), l.pinned.clone(), l.show_hidden, l.sidebar_visible),
            None => (Vec::new(), Vec::new(), false, true),
        };
        let restored = match (&start, &loaded) {
            (None, Some(l)) => restore(l, |p| p.is_dir()),
            _ => None,
        };
        let tree = restored.unwrap_or_else(|| {
            LayoutTree::Leaf(PaneSnapshot { tabs: vec![start.clone().unwrap_or_else(|| places.home.clone())], ..Default::default() })
        });
        let mut next = 1u64;
        let layout = tree.map(&mut |_| {
            let id = PaneId(next);
            next += 1;
            id
        });
        let snaps = tree.leaves();
        let first = PaneId(1);
        let first_dir = {
            let s = &snaps[0];
            s.tabs.get(s.active).cloned().unwrap_or_else(|| places.home.clone())
        };

        let ops = cx.new(|_| FileOps::new());
        let sidebar = cx.new(|cx| {
            let mut s = Sidebar::new(first_dir, places.home.clone(), favorites.clone(), pinned.clone(), ops.clone());
            s.set_icons(icons.clone(), cx);
            s
        });
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(
            &sidebar,
            window,
            |this, _, ev: &SidebarEvent, window, cx| match ev {
                SidebarEvent::Open { path, new_tab } => {
                    let mode = if *new_tab { OpenMode::NewTab } else { OpenMode::Here };
                    this.open_location(path.clone(), None, mode, window, cx);
                }
                SidebarEvent::OpenSplit { path } => this.open_location(path.clone(), None, OpenMode::Split, window, cx),
                SidebarEvent::Remove { group, ix } => {
                    match group {
                        Group::Favorites => places::remove(&mut this.favorites, *ix),
                        Group::Pinned => places::remove(&mut this.pinned, *ix),
                    };
                    this.entries_changed(cx);
                }
                SidebarEvent::DropFiles { paths, dest, alt, source } => {
                    dnd::perform(&this.ops, paths, dest, *alt, *source, cx);
                }
                SidebarEvent::AddFolders { group, paths } => {
                    if paths.is_empty() || !paths.iter().all(|p| p.is_dir()) {
                        return;
                    }
                    let list = match group {
                        Group::Favorites => &mut this.favorites,
                        Group::Pinned => &mut this.pinned,
                    };
                    if places::add_folders(list, paths, *group) {
                        this.entries_changed(cx);
                    }
                }
                SidebarEvent::Reorder { from, to } => {
                    if places::move_entry(&mut this.favorites, &mut this.pinned, *from, *to) {
                        this.entries_changed(cx);
                    }
                }
            },
        ));
        subs.push(cx.subscribe(&ops, |this, _, ev: &FileOpsEvent, cx| match ev {
            FileOpsEvent::StackChanged => this.rebuild_menus(cx),
            FileOpsEvent::Error(m) => this.show_banner(m.clone(), cx),
            FileOpsEvent::Done { .. } => {}
        }));
        subs.push(cx.observe(&ops, |_, _, cx| cx.notify()));
        subs.push(cx.on_app_quit(|this, cx| {
            this.opener.close_quick_look();
            this.save_now(cx);
            async {}
        }));
        let index_enabled = index_roots.is_some() || store.is_some();
        let focus = cx.focus_handle();
        let mut this = Self {
            sidebar,
            ops,
            icons,
            panes: HashMap::new(),
            pane_subs: HashMap::new(),
            layout,
            focused: first,
            next_pane_id: next,
            show_hidden,
            sidebar_visible,
            title: String::new(),
            banner: None,
            banner_id: 0,
            focus,
            favorites,
            pinned,
            opener,
            places,
            store,
            last_saved: loaded,
            save_gen: 0,
            _save_task: None,
            index: Arc::new(Vec::new()),
            index_fixed_roots: index_roots,
            index_enabled,
            index_built: None,
            index_gen: 0,
            _index_task: None,
            palette: None,
            goto: None,
            goto_focus_pending: false,
            ql_paths: Vec::new(),
            watcher: None,
            dirty: BTreeSet::new(),
            refresh_gen: 0,
            _refresh_task: None,
            _watch_task: None,
            _subs: subs,
        };
        for (i, snap) in snaps.into_iter().enumerate() {
            this.create_pane(PaneId(i as u64 + 1), snap, SortOrder::default(), window, cx);
        }
        if watch {
            let (tx, mut rx) = futures::channel::mpsc::unbounded::<Vec<PathBuf>>();
            this.watcher = Some(DirWatcher::new(move |v| {
                let _ = tx.unbounded_send(v);
            }));
            this._watch_task = Some(cx.spawn(async move |this, cx| {
                while let Some(dirs) = rx.next().await {
                    if this.update(cx, |this, cx| this.dirs_changed(dirs, cx)).is_err() {
                        break;
                    }
                }
            }));
            this.sync_watch(cx);
        }
        if this.index_enabled {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(INDEX_DELAY).await;
                this.update(cx, |this, cx| this.rebuild_index(cx)).ok();
            })
            .detach();
        }
        if !sidebar_visible {
            this.rebuild_menus(cx);
        }
        // Keys only fire when something focused is in the dispatch path.
        this.focus_pane(first, window, cx);
        this
    }

    // ---- accessors (also used by the UI tests) -----------------------------------------------

    /// Canonical folders currently watched (empty when watching is off).
    pub fn watched_dirs(&self) -> BTreeSet<PathBuf> {
        self.watcher.as_ref().map(|w| w.watched().clone()).unwrap_or_default()
    }

    /// The shared file-operations queue.
    pub fn ops(&self) -> &Entity<FileOps> {
        &self.ops
    }

    /// The shared icon cache.
    pub fn icons(&self) -> &Entity<IconCache> {
        &self.icons
    }

    pub fn active_list(&self, cx: &App) -> Entity<FileList> {
        self.focused_pane().read(cx).active_list().clone()
    }

    /// Last window title set (the platform window's title is not readable in tests).
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn focused(&self) -> PaneId {
        self.focused
    }

    pub fn layout(&self) -> &LayoutTree<PaneId> {
        &self.layout
    }

    pub fn favorites(&self) -> &[SidebarEntry] {
        &self.favorites
    }

    pub fn pinned(&self) -> &[SidebarEntry] {
        &self.pinned
    }

    pub fn banner(&self) -> Option<&str> {
        self.banner.as_ref().map(|(_, m)| m.as_str())
    }

    pub fn palette(&self) -> Option<&Entity<Palette>> {
        self.palette.as_ref().map(|(p, _)| p)
    }

    /// The Go to Folder field while the overlay is open.
    pub fn go_to_input(&self) -> Option<&Entity<TextInput>> {
        self.goto.as_ref().map(|(i, _)| i)
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub fn pane(&self, id: PaneId) -> Option<&Entity<Pane>> {
        self.panes.get(&id)
    }

    fn focused_pane(&self) -> &Entity<Pane> {
        &self.panes[&self.focused]
    }

    // ---- pane management ---------------------------------------------------------------------

    fn create_pane(&mut self, id: PaneId, snap: PaneSnapshot, sort: SortOrder, window: &mut Window, cx: &mut Context<Self>) {
        let (ops, opener, hidden, home, icons) =
            (self.ops.clone(), self.opener.clone(), self.show_hidden, self.places.home.clone(), self.icons.clone());
        let first = snap.tabs.first().cloned().unwrap_or_else(|| self.places.home.clone());
        let pane = cx.new(|cx| {
            let mut p = Pane::new(id, first, ops, opener, home, hidden, sort, snap.view_mode, icons, cx);
            p.restore_tabs(&snap, cx);
            p
        });
        let focus = pane.read(cx).focus().clone();
        let subs = vec![
            cx.subscribe_in(&pane, window, move |this, _, ev: &PaneEvent, window, cx| {
                this.on_pane_event(id, ev, window, cx)
            }),
            // Keyboard backstop: anything focusing inside the pane makes it the focused pane.
            cx.on_focus_in(&focus, window, move |this, window, cx| {
                if this.focused != id && this.panes.contains_key(&id) {
                    this.focus_pane(id, window, cx);
                }
            }),
        ];
        self.panes.insert(id, pane);
        self.pane_subs.insert(id, subs);
    }

    fn on_pane_event(&mut self, id: PaneId, ev: &PaneEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panes.contains_key(&id) {
            return;
        }
        match ev {
            PaneEvent::Activated => {
                if self.focused != id {
                    self.focus_pane(id, window, cx);
                }
            }
            PaneEvent::Refocus => self.focus_pane(id, window, cx),
            PaneEvent::ActiveDirChanged(_) => {
                if self.focused == id {
                    self.sync_chrome(window, cx);
                }
            }
            PaneEvent::ToggleHidden => self.toggle_hidden(cx),
            PaneEvent::CloseLastTab => {
                self.focus_pane(id, window, cx);
                self.close_tab(window, cx);
            }
            PaneEvent::SplitRight => {
                self.focus_pane(id, window, cx);
                self.split(Side::Right, window, cx);
            }
            PaneEvent::SplitDown => {
                self.focus_pane(id, window, cx);
                self.split(Side::Bottom, window, cx);
            }
            PaneEvent::Changed => {
                self.schedule_save(cx);
                self.sync_watch(cx);
            }
            PaneEvent::SelectionChanged => self.selection_changed(id, cx),
            PaneEvent::InspectorChanged => {
                if self.focused == id {
                    self.rebuild_menus(cx);
                }
            }
        }
    }

    /// Window title and sidebar highlight follow the focused pane's active directory.
    fn sync_chrome(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.active_list(cx);
        let (title, dir) = {
            let l = list.read(cx);
            (l.title(), l.current_dir().to_path_buf())
        };
        window.set_window_title(&title);
        self.title = title;
        self.sidebar.update(cx, |s, cx| s.set_active(dir, cx));
    }

    /// The single funnel for programmatic focus changes.
    fn focus_pane(&mut self, id: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.panes.get(&id).cloned() else { return };
        self.focused = id;
        let list = pane.read(cx).active_list().clone();
        // Re-focusing would blur (and so commit) an in-progress rename.
        if !list.read(cx).is_renaming() {
            list.read(cx).focus_handle(cx).focus(window);
        }
        self.sync_chrome(window, cx);
        self.rebuild_menus(cx);
        cx.notify();
    }

    fn split(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        if self.focused_pane().read(cx).active_list().read(cx).is_renaming() {
            return;
        }
        let dir = self.active_list(cx).read(cx).current_dir().to_path_buf();
        // The new pane inherits the source pane's sort and view mode.
        let (sort, view_mode) = {
            let p = self.focused_pane().read(cx);
            (p.sort(), p.view_mode())
        };
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        self.layout = self.layout.docking(id, &self.focused, side);
        self.create_pane(id, PaneSnapshot { tabs: vec![dir], active: 0, view_mode }, sort, window, cx);
        self.focus_pane(id, window, cx);
        self.schedule_save(cx);
        self.sync_watch(cx);
    }

    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let old = self.layout.leaves();
        if old.len() < 2 || self.focused_pane().read(cx).active_list().read(cx).is_renaming() {
            return;
        }
        let Some(closed_ix) = old.iter().position(|p| *p == self.focused) else { return };
        let Some(layout) = self.layout.removing(&self.focused) else { return };
        let new = layout.leaves();
        let next = focus_after_close(&old, closed_ix, &new);
        self.layout = layout;
        let closing = self.focused;
        self.focus_pane(next, window, cx);
        self.panes.remove(&closing);
        self.pane_subs.remove(&closing);
        self.schedule_save(cx);
        self.sync_watch(cx);
        cx.notify();
    }

    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.focused_pane().clone();
        if pane.read(cx).active_list().read(cx).is_renaming() {
            return;
        }
        if pane.read(cx).tab_count() > 1 {
            pane.update(cx, |p, cx| p.close_active_tab(cx));
        } else if self.layout.leaves().len() > 1 {
            self.close_pane(window, cx);
        } else {
            window.remove_window();
        }
    }

    fn cycle_focus(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.focused_pane().read(cx).active_list().read(cx).is_renaming() {
            return;
        }
        if let Some(next) = self.layout.next_leaf(&self.focused, delta) {
            self.focus_pane(next, window, cx);
        }
    }

    fn rebuild_menus(&self, cx: &mut Context<Self>) {
        let (u, r) = {
            let o = self.ops.read(cx);
            (o.undo_title(), o.redo_title())
        };
        let inspector_visible = self.panes.get(&self.focused).is_some_and(|p| p.read(cx).inspector_visible());
        menus::rebuild(
            cx,
            &menus::MenuState { undo: u.as_deref(), redo: r.as_deref(), sidebar_visible: self.sidebar_visible, inspector_visible },
        );
    }

    fn show_banner(&mut self, msg: String, cx: &mut Context<Self>) {
        self.banner_id += 1;
        let id = self.banner_id;
        self.banner = Some((id, msg));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(7)).await;
            this.update(cx, |this, cx| {
                if this.banner.as_ref().is_some_and(|(i, _)| *i == id) {
                    this.banner = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        self.show_hidden = !self.show_hidden;
        let v = self.show_hidden;
        for pane in self.panes.values() {
            pane.update(cx, |p, cx| p.set_show_hidden(v, cx));
        }
        self.schedule_save(cx);
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        self.ops.update(cx, |o, cx| o.undo(cx));
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        self.ops.update(cx, |o, cx| o.redo(cx));
    }

    // ---- Quick Look and view mode --------------------------------------------------------------

    fn toggle_quick_look(&mut self, cx: &mut Context<Self>) {
        let list = self.active_list(cx);
        if list.read(cx).is_renaming() {
            return;
        }
        if self.opener.quick_look_open() {
            self.opener.close_quick_look();
            self.ql_paths.clear();
            return;
        }
        let paths = list.read(cx).selection_paths();
        if !paths.is_empty() && self.opener.quick_look(&paths) {
            self.ql_paths = paths;
        }
    }

    /// While Quick Look is up it follows the focused pane's selection.
    fn selection_changed(&mut self, id: PaneId, cx: &mut Context<Self>) {
        if id != self.focused || !self.opener.quick_look_open() {
            return;
        }
        let paths = self.active_list(cx).read(cx).selection_paths();
        if paths == self.ql_paths {
            return;
        }
        if paths.is_empty() {
            self.opener.close_quick_look();
        } else {
            self.opener.quick_look(&paths);
        }
        self.ql_paths = paths;
    }

    fn set_view_mode(&mut self, m: ViewMode, cx: &mut Context<Self>) {
        self.focused_pane().clone().update(cx, |p, cx| p.set_view_mode(m, cx));
    }

    // ---- watching ----------------------------------------------------------------------------

    /// Watches exactly the folders of all open tabs (non-recursive).
    fn sync_watch(&mut self, cx: &mut Context<Self>) {
        if self.watcher.is_none() {
            return;
        }
        let dirs: BTreeSet<PathBuf> = self
            .panes
            .values()
            .flat_map(|p| p.read(cx).lists())
            .map(|l| l.read(cx).current_dir().to_path_buf())
            .collect();
        if let Some(w) = &mut self.watcher {
            w.set_dirs(&dirs);
        }
    }

    /// Folders changed on disk (from the watcher; also the test hook). Refreshes the affected
    /// tabs once things have been quiet for 150ms.
    pub fn dirs_changed(&mut self, dirs: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.dirty.extend(dirs.iter().map(|d| canon(d)));
        self.refresh_gen += 1;
        let g = self.refresh_gen;
        self._refresh_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REFRESH_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.refresh_gen == g {
                    this.flush_dirty(cx);
                }
            })
            .ok();
        }));
    }

    fn flush_dirty(&mut self, cx: &mut Context<Self>) {
        let dirty = std::mem::take(&mut self.dirty);
        for pane in self.panes.values() {
            for list in pane.read(cx).lists() {
                if dirty.contains(&canon(list.read(cx).current_dir())) {
                    // Deferred by the list itself while a rename is in progress.
                    list.update(cx, |l, cx| l.reload(cx));
                }
            }
            pane.update(cx, |p, cx| p.refresh_columns(cx));
        }
    }

    // ---- persistence -------------------------------------------------------------------------

    fn persisted(&self, cx: &App) -> PersistedState {
        PersistedState {
            favorites: self.favorites.clone(),
            pinned: self.pinned.clone(),
            layout: self.layout.map(&mut |id| self.panes[id].read(cx).snapshot(cx)),
            show_hidden: self.show_hidden,
            sidebar_visible: self.sidebar_visible,
        }
    }

    /// Debounced (0.6s) save; a newer request supersedes the pending one.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() {
            return;
        }
        self.save_gen += 1;
        let g = self.save_gen;
        self._save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.save_gen == g {
                    this.save_now(cx);
                }
            })
            .ok();
        }));
    }

    /// Writes the state unless it equals what was last saved.
    fn save_now(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else { return };
        let state = self.persisted(cx);
        if self.last_saved.as_ref() == Some(&state) {
            return;
        }
        match store.save(&state) {
            Ok(()) => self.last_saved = Some(state),
            Err(e) => eprintln!("nanofm: could not save state: {e}"),
        }
    }

    // ---- sidebar entries and search index ----------------------------------------------------

    fn entries_changed(&mut self, cx: &mut Context<Self>) {
        let (f, p) = (self.favorites.clone(), self.pinned.clone());
        self.sidebar.update(cx, |s, cx| s.set_entries(f, p, cx));
        self.schedule_save(cx);
        self.rebuild_index(cx);
        cx.notify();
    }

    /// `pin` false: add to Favorites instead. Folders only; no selection means the current folder.
    fn pin_selection(&mut self, pin: bool, cx: &mut Context<Self>) {
        let list = self.active_list(cx);
        if list.read(cx).is_renaming() {
            return;
        }
        let targets = list.read(cx).pin_targets();
        if targets.is_empty() {
            let msg = if pin { "Only folders can be pinned to the sidebar." } else { "Only folders can be added to Favorites." };
            self.show_banner(msg.into(), cx);
            return;
        }
        for t in targets {
            if pin {
                places::pin(&mut self.pinned, &t);
            } else {
                places::add_favorite(&mut self.favorites, &t);
            }
        }
        self.entries_changed(cx);
    }

    fn rebuild_index(&mut self, cx: &mut Context<Self>) {
        if !self.index_enabled {
            return;
        }
        let roots = match &self.index_fixed_roots {
            Some(r) => r.clone(),
            None => {
                let f: Vec<PathBuf> = self.favorites.iter().map(|e| e.path.clone()).collect();
                let p: Vec<PathBuf> = self.pinned.iter().map(|e| e.path.clone()).collect();
                search_index::index_roots(&self.places.home, &f, &p)
            }
        };
        self.index_gen += 1;
        let g = self.index_gen;
        self._index_task = Some(cx.spawn(async move |this, cx| {
            let entries = cx
                .background_executor()
                .spawn(async move { search_index::crawl(&roots, search_index::MAX_DEPTH, search_index::MAX_ENTRIES) })
                .await;
            this.update(cx, |this, cx| {
                if this.index_gen != g {
                    return;
                }
                this.index = Arc::new(entries);
                this.index_built = Some(Instant::now());
                if let Some((p, _)) = &this.palette {
                    let idx = this.index.clone();
                    p.update(cx, |p, cx| p.set_index(idx, cx));
                }
            })
            .ok();
        }));
    }

    // ---- navigation helpers and the palette --------------------------------------------------

    /// Shows `dir` (optionally selecting `select`) in the focused pane, a new tab or a new split.
    fn open_location(&mut self, dir: PathBuf, select: Option<PathBuf>, mode: OpenMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        match mode {
            OpenMode::Here => {}
            OpenMode::NewTab => self.focused_pane().clone().update(cx, |p, cx| p.new_tab(cx)),
            OpenMode::Split => self.split(Side::Right, window, cx),
        }
        self.active_list(cx).update(cx, |l, cx| l.go_to(dir, select, cx));
        self.focus_pane(self.focused, window, cx);
    }

    fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        if self.index_built.is_none_or(|t| t.elapsed() > INDEX_STALE) {
            self.rebuild_index(cx);
        }
        let (f, p, idx) = (self.favorites.clone(), self.pinned.clone(), self.index.clone());
        let palette = cx.new(|cx| Palette::new(f, p, idx, window, cx));
        let sub = cx.subscribe_in(&palette, window, |this, _, ev: &PaletteEvent, window, cx| match ev {
            PaletteEvent::Close => this.close_palette(window, cx),
            PaletteEvent::Activate(c, m) => this.palette_activate(c.clone(), *m, window, cx),
        });
        self.palette = Some((palette, sub));
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.take().is_some() {
            self.focus_pane(self.focused, window, cx);
        }
    }

    fn palette_activate(&mut self, choice: Choice, mode: OpenMode, window: &mut Window, cx: &mut Context<Self>) {
        self.close_palette(window, cx);
        match choice {
            Choice::Folder(path) => {
                if path.is_dir() {
                    self.open_location(path, None, mode, window, cx);
                } else {
                    self.show_banner(format!("Folder not found: {}", path.display()), cx);
                }
            }
            Choice::File(path) => {
                if !path.exists() {
                    self.show_banner(format!("File not found: {}", path.display()), cx);
                } else if mode == OpenMode::Here {
                    let opener = self.opener.clone();
                    opener.open(cx, &path);
                } else if let Some(parent) = path.parent() {
                    self.open_location(parent.to_path_buf(), Some(path.clone()), mode, window, cx);
                }
            }
            Choice::Command(id) => {
                let Some(def) = commands::find(id) else { return };
                let action = (def.action)();
                // Run once the palette is gone and focus is back in the list.
                cx.defer_in(window, move |_, window, cx| window.dispatch_action(action, cx));
            }
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        self.rebuild_menus(cx);
        self.schedule_save(cx);
        cx.notify();
    }

    /// Open in New Tab / Open in Split: the selection (or the current folder when nothing is
    /// selected). Files go to the system; folders become one tab each (the first one active), or
    /// for a split the first in the new pane and the rest as unselected tabs there.
    fn open_selection(&mut self, mode: OpenMode, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.active_list(cx);
        if list.read(cx).is_renaming() {
            return;
        }
        let (dirs, files): (Vec<PathBuf>, Vec<PathBuf>) =
            list.read(cx).action_targets().into_iter().partition(|p| p.is_dir());
        let opener = self.opener.clone();
        for f in &files {
            opener.open(cx, f);
        }
        let Some((first, rest)) = dirs.split_first() else { return };
        match mode {
            OpenMode::NewTab => self.focused_pane().clone().update(cx, |p, cx| p.add_tab(first.clone(), true, cx)),
            _ => self.open_location(first.clone(), None, OpenMode::Split, window, cx),
        }
        self.focused_pane().clone().update(cx, |p, cx| {
            for d in rest.iter().rev() {
                p.add_tab(d.clone(), false, cx);
            }
        });
    }

    fn reveal(&mut self, cx: &mut Context<Self>) {
        let list = self.active_list(cx);
        if list.read(cx).is_renaming() {
            return;
        }
        let targets = list.read(cx).action_targets();
        self.opener.reveal(&targets);
    }

    // ---- Go menu -------------------------------------------------------------------------------

    /// Go to Home / Desktop / ...: the folder must exist.
    fn go_place(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        if path.is_dir() {
            self.open_location(path, None, OpenMode::Here, window, cx);
        } else {
            self.show_banner(not_found(&path.display().to_string()), cx);
        }
    }

    fn toggle_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.goto.is_some() {
            self.close_goto(window, cx);
            return;
        }
        if self.active_list(cx).read(cx).is_renaming() {
            return;
        }
        if self.palette.is_some() {
            self.palette = None;
        }
        let cur = self.active_list(cx).read(cx).current_dir().to_path_buf();
        let text = places::abbreviate(&cur, &self.places.home);
        let n = text.len();
        let input = cx.new(|cx| {
            let mut i = TextInput::new(&text, cx);
            i.select_range(0..n, cx);
            i
        });
        let subs = vec![cx.subscribe_in(&input, window, |this, input, ev: &TextInputEvent, window, cx| match ev {
            TextInputEvent::Confirm => {
                let raw = input.read(cx).text().to_string();
                this.confirm_goto(raw, window, cx);
            }
            TextInputEvent::Cancel => this.close_goto(window, cx),
        })];
        self.goto = Some((input, subs));
        self.goto_focus_pending = true;
        cx.notify();
    }

    fn close_goto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.goto.take().is_some() {
            self.goto_focus_pending = false;
            self.focus_pane(self.focused, window, cx);
        }
    }

    fn confirm_goto(&mut self, raw: String, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.active_list(cx).read(cx).current_dir().to_path_buf();
        let target = places::resolve_go_to(&raw, &self.places.home, &cwd);
        self.close_goto(window, cx);
        match target {
            GoTarget::Empty => {}
            GoTarget::Folder(dir) => self.open_location(dir, None, OpenMode::Here, window, cx),
            GoTarget::File { dir, select } => self.open_location(dir, Some(select), OpenMode::Here, window, cx),
            GoTarget::NotFound => self.show_banner(not_found(&raw), cx),
        }
    }

    fn render_node(&self, node: &LayoutTree<PaneId>, path: Vec<usize>, cx: &mut Context<Self>) -> AnyElement {
        match node {
            LayoutTree::Leaf(id) => {
                let ring = self.layout.leaves().len() > 1 && *id == self.focused;
                div()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .border_2()
                    .border_color(rgb(if ring { theme::ACCENT } else { theme::BG }))
                    .child(self.panes[id].clone())
                    .into_any_element()
            }
            LayoutTree::Split { axis, children, ratios } => {
                let side = *axis == Axis::SideBySide;
                let mut d = div().flex().size_full().min_w_0().min_h_0();
                d = if side { d.flex_row() } else { d.flex_col() };
                for (i, c) in children.iter().enumerate() {
                    if i > 0 {
                        let ix = i - 1;
                        let line = div().flex_none().bg(rgb(theme::BORDER));
                        let (hit, line) = if side {
                            (div().w(px(5.)).h_full().flex().justify_center(), line.w(px(1.)).h_full())
                        } else {
                            (div().h(px(5.)).w_full().flex().flex_col().justify_center(), line.h(px(1.)).w_full())
                        };
                        let payload = DraggedDivider { path: path.clone(), ix };
                        let n = path.iter().fold(ix, |a, p| a * 31 + p + 1);
                        d = d.child(
                            hit.id(("divider", n))
                                .debug_selector({
                                    let p = path.clone();
                                    move || format!("divider-{}-{ix}", p.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("."))
                                })
                                .flex_none()
                                .cursor(if side { CursorStyle::ResizeLeftRight } else { CursorStyle::ResizeUpDown })
                                .on_drag(payload, |_, _, _, cx| cx.new(|_| DragGhost))
                                .child(line),
                        );
                    }
                    let mut cell = div().min_w_0().min_h_0().overflow_hidden().flex_basis(relative(0.));
                    cell.style().flex_grow = Some(ratios[i]);
                    let mut child_path = path.clone();
                    child_path.push(i);
                    d = d.child(cell.child(self.render_node(c, child_path, cx)));
                }
                let my_path = path.clone();
                d = d.on_drag_move(cx.listener(move |this, ev: &DragMoveEvent<DraggedDivider>, _, cx| {
                    let (dpath, ix) = {
                        let dr = ev.drag(cx);
                        (dr.path.clone(), dr.ix)
                    };
                    if dpath != my_path {
                        return;
                    }
                    let (pos, ext, min_px) = if side {
                        (ev.event.position.x - ev.bounds.origin.x, ev.bounds.size.width, 320.)
                    } else {
                        (ev.event.position.y - ev.bounds.origin.y, ev.bounds.size.height, 200.)
                    };
                    let (pos, ext): (f32, f32) = (pos.into(), ext.into());
                    if ext <= 0. {
                        return;
                    }
                    this.layout.drag_divider(&my_path, ix, pos / ext, (min_px / ext).min(0.45));
                    cx.notify();
                }));
                d.into_any_element()
            }
        }
    }
}

impl Focusable for Browser {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.goto_focus_pending)
            && let Some((i, _)) = &self.goto
        {
            i.read(cx).focus_handle(cx).focus(window);
        }
        let (can_undo, can_redo) = {
            let o = self.ops.read(cx);
            (o.can_undo(), o.can_redo())
        };
        let multi = self.layout.leaves().len() > 1;

        let banner = self.banner.as_ref().map(|(_, msg)| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .bg(rgb(theme::TOOLBAR_BG))
                .border_b_1()
                .border_color(rgb(theme::BORDER))
                .text_sm()
                .text_color(rgb(theme::ERROR))
                .child(div().flex_1().min_w_0().child(msg.clone()))
                .child(
                    div()
                        .id("banner-close")
                        .px_2()
                        .cursor_pointer()
                        .text_color(rgb(theme::TEXT_DIM))
                        .hover(|d| d.text_color(rgb(theme::TEXT)))
                        .child("\u{00D7}")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.banner = None;
                            cx.notify();
                        })),
                )
        });

        let tree = self.layout.clone();
        let panes = self.render_node(&tree, Vec::new(), cx);
        let overlay = self.palette.as_ref().map(|(p, _)| {
            deferred(
                div().absolute().top_0().left_0().size_full().flex().justify_center().pt(px(80.)).child(
                    div()
                        .id("palette-overlay")
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_palette(window, cx)))
                        .child(p.clone()),
                ),
            )
        });

        let goto_overlay = self.goto.as_ref().map(|(input, _)| {
            deferred(
                div().absolute().top_0().left_0().size_full().flex().justify_center().pt(px(80.)).child(
                    div()
                        .id("goto-overlay")
                        .key_context("GoTo")
                        .occlude()
                        .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_goto(window, cx)))
                        .w(px(480.))
                        .h_auto()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .rounded_md()
                        .bg(rgb(theme::TOOLBAR_BG))
                        .border_1()
                        .border_color(rgb(theme::BORDER))
                        .text_sm()
                        .text_color(rgb(theme::TEXT))
                        .child("Go to Folder")
                        .child(
                            div()
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .border_1()
                                .border_color(rgb(theme::ACCENT))
                                .child(input.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(theme::TEXT_DIM))
                                .child("Type a folder path; ~ is your home folder. Return to go, Esc to cancel."),
                        ),
                ),
            )
        });

        div()
            .key_context("Browser")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &GoParent, _, cx| {
                this.active_list(cx).update(cx, |l, cx| l.on_parent(cx));
            }))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| {
                this.active_list(cx).update(cx, |l, cx| l.on_back(cx));
            }))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| {
                this.active_list(cx).update(cx, |l, cx| l.on_forward(cx));
            }))
            .on_action(cx.listener(|this, _: &GoHome, window, cx| this.go_place(this.places.home.clone(), window, cx)))
            .on_action(cx.listener(|this, _: &GoDesktop, window, cx| this.go_place(this.places.desktop.clone(), window, cx)))
            .on_action(cx.listener(|this, _: &GoDocuments, window, cx| this.go_place(this.places.documents.clone(), window, cx)))
            .on_action(cx.listener(|this, _: &GoDownloads, window, cx| this.go_place(this.places.downloads.clone(), window, cx)))
            .on_action(cx.listener(|this, _: &GoApplications, window, cx| {
                this.go_place(this.places.applications.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &GoToFolder, window, cx| this.toggle_goto(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &ToggleInspector, _, cx| {
                this.focused_pane().clone().update(cx, |p, cx| p.toggle_inspector(cx));
            }))
            .on_action(cx.listener(|this, _: &OpenInNewTab, window, cx| this.open_selection(OpenMode::NewTab, window, cx)))
            .on_action(cx.listener(|this, _: &OpenInSplit, window, cx| this.open_selection(OpenMode::Split, window, cx)))
            .on_action(cx.listener(|this, _: &RevealInFinder, _, cx| this.reveal(cx)))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| this.toggle_hidden(cx)))
            .on_action(cx.listener(|this, _: &NewTab, _, cx| {
                this.focused_pane().clone().update(cx, |p, cx| p.new_tab(cx));
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| this.close_tab(window, cx)))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                this.focused_pane().clone().update(cx, |p, cx| p.cycle_tab(1, cx));
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                this.focused_pane().clone().update(cx, |p, cx| p.cycle_tab(-1, cx));
            }))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| this.split(Side::Right, window, cx)))
            .on_action(cx.listener(|this, _: &SplitDown, window, cx| this.split(Side::Bottom, window, cx)))
            .on_action(cx.listener(|this, _: &FocusNextPane, window, cx| this.cycle_focus(1, window, cx)))
            .on_action(cx.listener(|this, _: &FocusPrevPane, window, cx| this.cycle_focus(-1, window, cx)))
            .when(multi, |d| {
                d.on_action(cx.listener(|this, _: &ClosePane, window, cx| this.close_pane(window, cx)))
            })
            .on_action(cx.listener(|this, _: &TogglePalette, window, cx| this.toggle_palette(window, cx)))
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                if this.active_list(cx).read(cx).is_renaming() {
                    return;
                }
                this.focused_pane().clone().update(cx, |p, cx| p.focus_filter(window, cx));
            }))
            .on_action(cx.listener(|this, _: &QuickLook, _, cx| this.toggle_quick_look(cx)))
            .on_action(cx.listener(|this, _: &ViewList, _, cx| this.set_view_mode(ViewMode::List, cx)))
            .on_action(cx.listener(|this, _: &ViewColumn, _, cx| this.set_view_mode(ViewMode::Column, cx)))
            .on_action(cx.listener(|this, _: &PinSelection, _, cx| this.pin_selection(true, cx)))
            .on_action(cx.listener(|this, _: &AddFavorite, _, cx| this.pin_selection(false, cx)))
            .on_action(cx.listener(|this, _: &DismissBanner, _, cx| {
                this.banner = None;
                cx.notify();
            }))
            .when(can_undo, |d| {
                d.on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            })
            .when(can_redo, |d| {
                d.on_action(cx.listener(|this, _: &Redo, _, cx| this.redo(cx)))
            })
            .size_full()
            .flex()
            .bg(rgb(theme::BG))
            .when(self.sidebar_visible, |d| d.child(self.sidebar.clone()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(banner)
                    .child(div().flex_1().min_h_0().child(panes)),
            )
            .children(overlay)
            .children(goto_overlay)
    }
}
