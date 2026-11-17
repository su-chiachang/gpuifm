//! One pane: a tab strip, a navigation toolbar and the active tab's file list.

use crate::fm_core::format::status;
use crate::fm_core::layout::PaneId;
use crate::fm_core::places::{crumb_title, crumbs};
use crate::fm_core::sort::{SortKey, SortOrder};
use crate::fm_core::state::{PaneSnapshot, ViewMode};
use crate::fm_core::tabs::Tabs;
use crate::ui::actions::FilterDown;
use crate::ui::column_view::ColumnView;
use crate::ui::file_list::{FileList, FileListEvent};
use crate::ui::opener::Opener;
use crate::ui::ops::FileOps;
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseDownEvent, Render, Subscription, Window, div, prelude::*, px, rgb,
};
use std::path::PathBuf;
use std::rc::Rc;

pub enum PaneEvent {
    /// A mouse press landed in the pane (capture phase): it should become the focused pane.
    Activated,
    /// The pane changed its own tabs: re-focus its active list and refresh title/sidebar.
    Refocus,
    /// The active tab navigated.
    ActiveDirChanged(PathBuf),
    ToggleHidden,
    /// Close request for a pane's only tab.
    CloseLastTab,
    SplitRight,
    SplitDown,
    /// Tabs, a tab's folder or the view mode changed: persist the layout.
    Changed,
    /// The active tab's selection changed.
    SelectionChanged,
}

struct FilterBox {
    input: Entity<TextInput>,
    _subs: Vec<Subscription>,
}

struct Tab {
    list: Entity<FileList>,
    _sub: Subscription,
    /// Repaints the path bar (counts, selection) when the list changes.
    _obs: Subscription,
}

pub struct Pane {
    id: PaneId,
    tabs: Tabs<Tab>,
    ops: Entity<FileOps>,
    opener: Rc<dyn Opener>,
    home: PathBuf,
    /// Sort and view mode are per pane; every tab follows the pane's sort.
    sort: SortOrder,
    view_mode: ViewMode,
    show_hidden: bool,
    filter: Option<FilterBox>,
    columns: Option<Entity<ColumnView>>,
    focus_filter_pending: bool,
    focus: FocusHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PaneEvent> for Pane {}

impl Focusable for Pane {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Pane {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: PaneId,
        start: PathBuf,
        ops: Entity<FileOps>,
        opener: Rc<dyn Opener>,
        home: PathBuf,
        show_hidden: bool,
        sort: SortOrder,
        view_mode: ViewMode,
        cx: &mut Context<Self>,
    ) -> Self {
        let first = Self::make_tab(start, &ops, &opener, show_hidden, sort, view_mode == ViewMode::Column, cx);
        let busy = cx.observe(&ops, |_, _, cx| cx.notify());
        Self {
            id,
            tabs: Tabs::new(first),
            ops,
            opener,
            home,
            sort,
            view_mode,
            show_hidden,
            filter: None,
            columns: None,
            focus_filter_pending: false,
            focus: cx.focus_handle(),
            _subs: vec![busy],
        }
    }

    fn make_tab(
        start: PathBuf,
        ops: &Entity<FileOps>,
        opener: &Rc<dyn Opener>,
        show_hidden: bool,
        sort: SortOrder,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> Tab {
        let list = cx.new(|cx| {
            let mut l = FileList::new(start, ops.clone(), opener.clone(), show_hidden, sort, cx);
            l.set_compact_initial(compact);
            l
        });
        let sub = cx.subscribe(&list, |this, l, ev: &FileListEvent, cx| match ev {
            FileListEvent::Navigated(p) => {
                if l.entity_id() == this.active_list().entity_id() {
                    this.clear_filter(cx);
                    cx.emit(PaneEvent::ActiveDirChanged(p.clone()));
                }
                cx.emit(PaneEvent::Changed);
                cx.notify();
            }
            FileListEvent::SortRequested(key) => this.request_sort(*key, cx),
            FileListEvent::SelectionChanged => {
                if l.entity_id() == this.active_list().entity_id() {
                    cx.emit(PaneEvent::SelectionChanged);
                }
            }
            FileListEvent::ColumnLeft | FileListEvent::ColumnRight => {}
        });
        let obs = cx.observe(&list, |_, _, cx| cx.notify());
        Tab { list, _sub: sub, _obs: obs }
    }

    /// Adds the remaining tabs of a saved pane (the first one was the pane's start) and
    /// re-selects the saved active tab.
    pub fn restore_tabs(&mut self, snap: &PaneSnapshot, cx: &mut Context<Self>) {
        for dir in snap.tabs.iter().skip(1) {
            let compact = self.view_mode == ViewMode::Column;
            let tab = Self::make_tab(dir.clone(), &self.ops, &self.opener, self.show_hidden, self.sort, compact, cx);
            self.tabs.add_after_active(tab);
        }
        self.tabs.select(snap.active);
        self.sync_columns(cx);
        cx.notify();
    }

    pub fn snapshot(&self, cx: &App) -> PaneSnapshot {
        PaneSnapshot {
            tabs: self.tabs.items().iter().map(|t| t.list.read(cx).current_dir().to_path_buf()).collect(),
            active: self.tabs.active(),
            view_mode: self.view_mode,
        }
    }

    pub fn sort(&self) -> SortOrder {
        self.sort
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    /// Pushes a sort order to every tab (used for a freshly split pane too).
    pub fn set_sort(&mut self, o: SortOrder, cx: &mut Context<Self>) {
        self.sort = o;
        for t in self.tabs.items() {
            t.list.update(cx, |l, cx| l.set_sort(o, cx));
        }
        if let Some(c) = &self.columns {
            c.update(cx, |c, cx| c.set_sort(o, cx));
        }
        cx.notify();
    }

    fn request_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        let o = self.sort.header_click(key);
        self.set_sort(o, cx);
    }

    // ---- filter ------------------------------------------------------------------------------

    pub fn filter_input(&self) -> Option<&Entity<TextInput>> {
        self.filter.as_ref().map(|f| &f.input)
    }

    /// Shows (or re-focuses) the filter field.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_renaming(cx) {
            return;
        }
        match &self.filter {
            Some(f) => {
                let n = f.input.read(cx).text().len();
                f.input.update(cx, |i, cx| i.select_range(0..n, cx));
            }
            None => {
                let input = cx.new(|cx| TextInput::new("", cx));
                let subs = vec![
                    cx.subscribe_in(&input, window, |this, _, ev: &TextInputEvent, window, cx| match ev {
                        TextInputEvent::Confirm => this.leave_filter(false, window, cx),
                        TextInputEvent::Cancel => this.leave_filter(true, window, cx),
                    }),
                    cx.observe(&input, |this, _, cx| this.apply_filter(cx)),
                ];
                self.filter = Some(FilterBox { input, _subs: subs });
            }
        }
        self.focus_filter_pending = true;
        cx.notify();
    }

    /// Return / Esc in the field: hand focus back to the list (Esc also clears the filter).
    fn leave_filter(&mut self, clear: bool, window: &mut Window, cx: &mut Context<Self>) {
        if clear {
            self.clear_filter(cx);
        }
        self.active_list().read(cx).focus_handle(cx).focus(window);
    }

    pub fn clear_filter(&mut self, cx: &mut Context<Self>) {
        if self.filter.take().is_some() {
            self.apply_filter(cx);
            cx.notify();
        }
    }

    /// Only the active tab is filtered; the text survives tab switches.
    fn apply_filter(&mut self, cx: &mut Context<Self>) {
        let text = self.filter.as_ref().map(|f| f.input.read(cx).text().to_string());
        let active = self.tabs.active();
        for (i, t) in self.tabs.items().iter().enumerate() {
            let f = if i == active { text.clone() } else { None };
            t.list.update(cx, |l, cx| l.set_filter(f, cx));
        }
        if let Some(c) = &self.columns {
            let list = self.active_list().clone();
            c.update(cx, |c, cx| c.set_list(list, cx));
        }
    }

    // ---- view mode ---------------------------------------------------------------------------

    pub fn columns(&self) -> Option<&Entity<ColumnView>> {
        self.columns.as_ref()
    }

    /// Switches between the list and the column browser.
    pub fn set_view_mode(&mut self, m: ViewMode, cx: &mut Context<Self>) {
        if m == self.view_mode || self.active_renaming(cx) {
            return;
        }
        self.view_mode = m;
        self.sync_columns(cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Refocus);
        cx.notify();
    }

    /// Makes the lists and the column view match `view_mode`.
    fn sync_columns(&mut self, cx: &mut Context<Self>) {
        let column = self.view_mode == ViewMode::Column;
        for t in self.tabs.items() {
            t.list.update(cx, |l, cx| l.set_compact(column, cx));
        }
        if !column {
            self.columns = None;
        } else if self.columns.is_none() {
            let (list, sort, hidden, opener) = (self.active_list().clone(), self.sort, self.show_hidden, self.opener.clone());
            self.columns = Some(cx.new(|cx| ColumnView::new(list, sort, hidden, opener, cx)));
        }
    }

    /// The folder contents changed on disk: re-read the read-only columns.
    pub fn refresh_columns(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &self.columns {
            c.update(cx, |c, cx| c.reload(cx));
        }
    }

    /// Path-bar crumb titles for the active tab, outermost first.
    pub fn crumbs(&self, cx: &App) -> Vec<String> {
        let dir = self.active_list().read(cx).current_dir().to_path_buf();
        crumbs(&dir, &self.home).iter().map(|p| crumb_title(p, &self.home)).collect()
    }

    /// Path-bar status text: item count, or selected-of-total.
    pub fn status_text(&self, cx: &App) -> String {
        let l = self.active_list().read(cx);
        status(l.selection_count(), l.entry_count())
    }

    fn path_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (dir, status_text) = {
            let l = self.active_list().read(cx);
            (l.current_dir().to_path_buf(), status(l.selection_count(), l.entry_count()))
        };
        let pane = self.id.0;
        let trail = crumbs(&dir, &self.home);
        let last = trail.len() - 1;
        let mut bar = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(24.))
            .px_2()
            .gap_1()
            .bg(rgb(theme::TOOLBAR_BG))
            .border_t_1()
            .border_color(rgb(theme::BORDER))
            .text_xs();
        // justify_end: when the trail is too long it overflows (and is clipped) on the left, so the
        // current folder's crumb stays visible.
        let mut crumbs_row = div().flex().flex_1().min_w_0().items_center().justify_end().gap_1().overflow_hidden();
        for (i, p) in trail.into_iter().enumerate() {
            if i > 0 {
                crumbs_row = crumbs_row.child(div().flex_none().text_color(rgb(theme::TEXT_DIM)).child("\u{203A}"));
            }
            let title = crumb_title(&p, &self.home);
            crumbs_row = crumbs_row.child(
                div()
                    .id(("crumb", i))
                    .debug_selector(move || format!("crumb-{pane}-{i}"))
                    .flex_none()
                    .px_1()
                    .rounded_md()
                    .cursor_pointer()
                    .text_color(rgb(if i == last { theme::TEXT } else { theme::TEXT_DIM }))
                    .hover(|d| d.bg(rgb(theme::HOVER_BG)))
                    .child(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active_list().update(cx, |l, cx| l.navigate(p.clone(), None, cx));
                        cx.emit(PaneEvent::Refocus);
                    })),
            );
        }
        bar = bar.child(crumbs_row).child(div().flex_none().text_color(rgb(theme::TEXT_DIM)).child(status_text));
        bar
    }

    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn active_list(&self) -> &Entity<FileList> {
        &self.tabs.active_item().list
    }

    pub fn lists(&self) -> Vec<Entity<FileList>> {
        self.tabs.items().iter().map(|t| t.list.clone()).collect()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab(&self) -> usize {
        self.tabs.active()
    }

    fn active_renaming(&self, cx: &App) -> bool {
        self.active_list().read(cx).is_renaming()
    }

    /// New tab right after the active one, opening the same directory.
    pub fn new_tab(&mut self, cx: &mut Context<Self>) {
        if self.active_renaming(cx) {
            return;
        }
        let (dir, hidden) = {
            let l = self.active_list().read(cx);
            (l.current_dir().to_path_buf(), l.show_hidden())
        };
        let tab = Self::make_tab(dir, &self.ops, &self.opener, hidden, self.sort, self.view_mode == ViewMode::Column, cx);
        self.tabs.add_after_active(tab);
        self.apply_filter(cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Refocus);
        cx.notify();
    }

    /// Adds a tab for `dir` right after the active one; `activate` selects it, otherwise the
    /// active tab stays where it is. Adding several background tabs in reverse keeps their order.
    pub fn add_tab(&mut self, dir: PathBuf, activate: bool, cx: &mut Context<Self>) {
        let (hidden, prev) = (self.show_hidden, self.tabs.active());
        let tab = Self::make_tab(dir, &self.ops, &self.opener, hidden, self.sort, self.view_mode == ViewMode::Column, cx);
        self.tabs.add_after_active(tab);
        if !activate {
            self.tabs.select(prev);
        }
        self.apply_filter(cx);
        cx.emit(PaneEvent::Changed);
        if activate {
            cx.emit(PaneEvent::Refocus);
        }
        cx.notify();
    }

    /// Closes tab `ix`; a pane's only tab is left for the browser to deal with.
    pub fn close_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.tabs.len() < 2 {
            cx.emit(PaneEvent::CloseLastTab);
            return;
        }
        if self.tabs.items().get(ix).is_some_and(|t| t.list.read(cx).is_renaming()) {
            return;
        }
        self.tabs.close(ix);
        self.apply_filter(cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Refocus);
        cx.notify();
    }

    pub fn close_active_tab(&mut self, cx: &mut Context<Self>) {
        let ix = self.tabs.active();
        self.close_tab(ix, cx);
    }

    pub fn select_tab(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.active_renaming(cx) {
            return;
        }
        self.tabs.select(ix);
        self.apply_filter(cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Refocus);
        cx.notify();
    }

    pub fn cycle_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.active_renaming(cx) || self.tabs.len() < 2 {
            return;
        }
        self.tabs.cycle(delta);
        self.apply_filter(cx);
        cx.emit(PaneEvent::Changed);
        cx.emit(PaneEvent::Refocus);
        cx.notify();
    }

    pub fn set_show_hidden(&mut self, v: bool, cx: &mut Context<Self>) {
        self.show_hidden = v;
        for t in self.tabs.items() {
            t.list.update(cx, |l, cx| l.set_show_hidden(v, cx));
        }
        if let Some(c) = &self.columns {
            c.update(cx, |c, cx| c.set_show_hidden(v, cx));
        }
    }

    fn button(
        &self,
        id: &'static str,
        label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
            .id(id)
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .text_color(rgb(if enabled { theme::TEXT } else { theme::TEXT_DIM }))
            .when(enabled, |d| d.cursor_pointer().hover(|d| d.bg(rgb(theme::HOVER_BG))))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if enabled {
                    f(this, cx);
                }
            }))
    }

    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.tabs.active();
        let single = self.tabs.len() == 1;
        let pane = self.id.0;
        let mut strip = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(28.))
            .px_1()
            .gap_1()
            .bg(rgb(theme::SIDEBAR_BG))
            .border_b_1()
            .border_color(rgb(theme::BORDER));
        for (ix, t) in self.tabs.items().iter().enumerate() {
            let is_active = ix == active;
            let title = t.list.read(cx).title();
            strip = strip.child(
                div()
                    .flex()
                    .items_center()
                    .flex_none()
                    .max_w(px(180.))
                    .rounded_md()
                    .text_sm()
                    .text_color(rgb(if is_active { theme::TEXT } else { theme::TEXT_DIM }))
                    .when(is_active, |d| d.bg(rgb(theme::SIDEBAR_ACTIVE_BG)))
                    .child(
                        div()
                            .id(("tab", ix))
                            .debug_selector(move || format!("tab-{pane}-{ix}"))
                            .px_2()
                            .py_0p5()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(theme::HOVER_BG)))
                            .child(title)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(ix, cx))),
                    )
                    .when(!single, |d| {
                        d.child(
                            div()
                                .id(("tab-close", ix))
                                .debug_selector(move || format!("tab-close-{pane}-{ix}"))
                                .px_1()
                                .cursor_pointer()
                                .text_color(rgb(theme::TEXT_DIM))
                                .hover(|d| d.text_color(rgb(theme::TEXT)))
                                .child("\u{00D7}")
                                .on_click(cx.listener(move |this, _, _, cx| this.close_tab(ix, cx))),
                        )
                    }),
            );
        }
        strip
            .child(self.button("new-tab", "+", true, cx, |this, cx| this.new_tab(cx)))
            .child(div().flex_1())
            .child(self.button("split-right", "\u{25E7}", true, cx, |_, cx| {
                cx.emit(PaneEvent::SplitRight)
            }))
            .child(self.button("split-down", "\u{2B12}", true, cx, |_, cx| {
                cx.emit(PaneEvent::SplitDown)
            }))
    }
}

impl Render for Pane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_filter_pending)
            && let Some(f) = &self.filter
        {
            f.input.read(cx).focus_handle(cx).focus(window);
        }
        let filter_box = self.filter.as_ref().map(|f| {
            div()
                .key_context("Filter")
                .on_action(cx.listener(|this, _: &FilterDown, window, cx| {
                    this.active_list().clone().update(cx, |l, cx| l.enter_list(window, cx));
                }))
                .w(px(170.))
                .flex_none()
                .px_2()
                .py_0p5()
                .rounded_md()
                .border_1()
                .border_color(rgb(theme::ACCENT))
                .text_sm()
                .child(f.input.clone())
        });
        let (can_back, can_fwd, can_parent, hidden, list) = {
            let list = self.active_list().clone();
            let l = list.read(cx);
            (
                l.can_back(),
                l.can_forward(),
                l.can_parent(),
                l.show_hidden(),
                list,
            )
        };
        let busy = self.ops.read(cx).busy();
        let toolbar = div()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .h(px(36.))
            .px_2()
            .bg(rgb(theme::TOOLBAR_BG))
            .border_b_1()
            .border_color(rgb(theme::BORDER))
            .child(self.button("back", "\u{2039}", can_back, cx, |this, cx| {
                this.active_list().update(cx, |l, cx| l.on_back(cx));
                cx.emit(PaneEvent::Refocus);
            }))
            .child(self.button("fwd", "\u{203A}", can_fwd, cx, |this, cx| {
                this.active_list().update(cx, |l, cx| l.on_forward(cx));
                cx.emit(PaneEvent::Refocus);
            }))
            .child(self.button("up", "\u{2191}", can_parent, cx, |this, cx| {
                this.active_list().update(cx, |l, cx| l.on_parent(cx));
                cx.emit(PaneEvent::Refocus);
            }))
            .child(div().flex_1().min_w_0())
            .children(filter_box)
            .when(busy, |d| {
                d.child(
                    div()
                        .px_2()
                        .text_sm()
                        .text_color(rgb(theme::TEXT_DIM))
                        .child("Working\u{2026}"),
                )
            })
            .child(self.button(
                "hidden",
                if hidden { "Hide Hidden" } else { "Show Hidden" },
                true,
                cx,
                |_, cx| cx.emit(PaneEvent::ToggleHidden),
            ));

        div()
            .track_focus(&self.focus)
            .debug_selector({
                let id = self.id.0;
                move || format!("pane-{id}")
            })
            .capture_any_mouse_down(cx.listener(|_, _: &MouseDownEvent, _, cx| {
                cx.emit(PaneEvent::Activated);
            }))
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .child(self.tab_strip(cx))
            .child(toolbar)
            .child(match self.columns.clone() {
                Some(c) => c.into_any_element(),
                None => list.into_any_element(),
            })
            .child(self.path_bar(cx))
    }
}
