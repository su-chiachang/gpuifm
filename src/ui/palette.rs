//! The command palette (cmd-K): favorites/pins, commands and indexed files in one list.

use crate::fm_core::fuzzy;
use crate::fm_core::search_index::{IndexEntry, SearchHit, search};
use crate::fm_core::state::SidebarEntry;
use crate::ui::actions::*;
use crate::ui::commands;
use crate::ui::text_input::{TextInput, TextInputEvent};
use crate::ui::theme;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, ScrollHandle, Subscription, Task,
    Window, div, prelude::*, px, rgb,
};
use std::path::PathBuf;
use std::sync::Arc;

const MAX_COMMANDS: usize = 12;
const MAX_FILES: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    Here,
    NewTab,
    Split,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Folder(PathBuf),
    File(PathBuf),
    Command(&'static str),
}

pub enum PaletteEvent {
    Close,
    Activate(Choice, OpenMode),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Header(&'static str),
    Location { title: String, path: PathBuf },
    Command { id: &'static str, title: &'static str, shortcut: &'static str },
    File(SearchHit),
}

impl Row {
    pub fn is_header(&self) -> bool {
        matches!(self, Row::Header(_))
    }

    fn choice(&self) -> Option<Choice> {
        match self {
            Row::Header(_) => None,
            Row::Location { path, .. } => Some(Choice::Folder(path.clone())),
            Row::Command { id, .. } => Some(Choice::Command(id)),
            Row::File(h) if h.is_dir => Some(Choice::Folder(h.path.clone())),
            Row::File(h) => Some(Choice::File(h.path.clone())),
        }
    }
}

pub struct Palette {
    input: Entity<TextInput>,
    rows: Vec<Row>,
    selected: usize,
    favorites: Vec<SidebarEntry>,
    pinned: Vec<SidebarEntry>,
    index: Arc<Vec<IndexEntry>>,
    query: String,
    hits: Vec<SearchHit>,
    generation: u64,
    _search: Option<Task<()>>,
    scroll: ScrollHandle,
    focus_pending: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}

impl Palette {
    pub fn new(
        favorites: Vec<SidebarEntry>,
        pinned: Vec<SidebarEntry>,
        index: Arc<Vec<IndexEntry>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| TextInput::new("", cx));
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &TextInputEvent, _, cx| match ev {
                TextInputEvent::Confirm => this.activate(OpenMode::Here, cx),
                TextInputEvent::Cancel => cx.emit(PaletteEvent::Close),
            }),
            cx.observe(&input, |this, _, cx| this.on_input(cx)),
        ];
        let mut p = Self {
            input,
            rows: Vec::new(),
            selected: 0,
            favorites,
            pinned,
            index,
            query: String::new(),
            hits: Vec::new(),
            generation: 0,
            _search: None,
            scroll: ScrollHandle::new(),
            focus_pending: true,
            _subs: subs,
        };
        p.rebuild_rows();
        p
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// A fresh index finished building: re-run the current search.
    pub fn set_index(&mut self, index: Arc<Vec<IndexEntry>>, cx: &mut Context<Self>) {
        self.index = index;
        self.start_search(cx);
    }

    fn on_input(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().to_string();
        if text == self.query {
            return;
        }
        self.query = text;
        self.hits.clear();
        self.start_search(cx);
        self.rebuild_rows();
        cx.notify();
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let q = self.query.trim().to_string();
        if q.is_empty() {
            self._search = None;
            return;
        }
        let (gen_, index) = (self.generation, self.index.clone());
        self._search = Some(cx.spawn(async move |this, cx| {
            let hits = cx.background_executor().spawn(async move { search(&index, &q, MAX_FILES + 40) }).await;
            this.update(cx, |this, cx| {
                if this.generation == gen_ {
                    this.hits = hits;
                    this.rebuild_rows();
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn first_selectable(&self) -> usize {
        self.rows.iter().position(|r| !r.is_header()).unwrap_or(0)
    }

    fn rebuild_rows(&mut self) {
        let q = self.query.trim().to_string();
        let cmd_row = |c: &commands::CommandDef| Row::Command { id: c.id, title: c.title, shortcut: c.shortcut };
        let mut rows = Vec::new();
        if q.is_empty() {
            let loc = |e: &SidebarEntry| Row::Location { title: e.display_title(), path: e.path.clone() };
            if !self.favorites.is_empty() {
                rows.push(Row::Header("Favorites"));
                rows.extend(self.favorites.iter().map(loc));
            }
            if !self.pinned.is_empty() {
                rows.push(Row::Header("Pinned"));
                rows.extend(self.pinned.iter().map(loc));
            }
            rows.push(Row::Header("Commands"));
            rows.extend(commands::all().iter().map(cmd_row));
        } else {
            let mut locs: Vec<(i64, &SidebarEntry)> = Vec::new();
            for e in self.favorites.iter().chain(&self.pinned) {
                if locs.iter().any(|(_, x)| x.path == e.path) {
                    continue;
                }
                if let Some(s) = fuzzy::score(&q, &e.display_title()) {
                    locs.push((s, e));
                }
            }
            locs.sort_by_key(|a| std::cmp::Reverse(a.0));
            if !locs.is_empty() {
                rows.push(Row::Header("Locations"));
                rows.extend(locs.iter().map(|(_, e)| Row::Location { title: e.display_title(), path: e.path.clone() }));
            }
            let all = commands::all();
            let mut cmds: Vec<(i64, &commands::CommandDef)> =
                all.iter().filter_map(|c| fuzzy::score(&q, c.title).map(|s| (s, c))).collect();
            cmds.sort_by_key(|a| std::cmp::Reverse(a.0));
            cmds.truncate(MAX_COMMANDS);
            if !cmds.is_empty() {
                rows.push(Row::Header("Commands"));
                rows.extend(cmds.iter().map(|(_, c)| cmd_row(c)));
            }
            let files: Vec<Row> = self
                .hits
                .iter()
                .filter(|h| !locs.iter().any(|(_, e)| e.path == h.path))
                .take(MAX_FILES)
                .map(|h| Row::File(h.clone()))
                .collect();
            if !files.is_empty() {
                rows.push(Row::Header("Files"));
                rows.extend(files);
            }
        }
        self.rows = rows;
        self.selected = self.first_selectable();
    }

    pub fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let mut i = self.selected as isize;
        loop {
            i += delta;
            if i < 0 || i as usize >= self.rows.len() {
                return; // stop at the ends
            }
            if !self.rows[i as usize].is_header() {
                break;
            }
        }
        self.selected = i as usize;
        self.scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    fn activate(&mut self, mode: OpenMode, cx: &mut Context<Self>) {
        if let Some(c) = self.rows.get(self.selected).and_then(Row::choice) {
            cx.emit(PaletteEvent::Activate(c, mode));
        }
    }

    fn row_view(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> gpui::AnyElement {
        let sel = ix == self.selected;
        let base = div().id(("palette-row", ix)).w_full();
        match row {
            Row::Header(t) => div()
                .px_3()
                .pt_2()
                .pb_1()
                .text_xs()
                .text_color(rgb(theme::TEXT_DIM))
                .child(*t)
                .into_any_element(),
            other => {
                let (title, sub, key): (String, String, String) = match other {
                    Row::Location { title, path } => (title.clone(), path.display().to_string(), String::new()),
                    Row::Command { title, shortcut, .. } => (title.to_string(), String::new(), shortcut.to_string()),
                    Row::File(h) => (h.name.clone(), h.path.display().to_string(), String::new()),
                    Row::Header(_) => unreachable!(),
                };
                base.debug_selector(move || format!("palette-row-{ix}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .when(sel, |d| d.bg(rgb(theme::SELECTED_BG)))
                    .hover(|d| d.bg(rgb(theme::HOVER_BG)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        this.activate(OpenMode::Here, cx);
                    }))
                    .child(div().flex_none().text_color(rgb(theme::TEXT)).child(title))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_xs()
                            .text_color(rgb(theme::TEXT_DIM))
                            .child(sub),
                    )
                    .child(div().flex_none().text_xs().text_color(rgb(theme::TEXT_DIM)).child(key))
                    .into_any_element()
            }
        }
    }
}

impl Render for Palette {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.focus_pending) {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        let rows: Vec<gpui::AnyElement> =
            self.rows.clone().iter().enumerate().map(|(i, r)| self.row_view(i, r, cx)).collect();
        div()
            .key_context("Palette")
            .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.move_selection(-1, cx)))
            .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.move_selection(1, cx)))
            .on_action(cx.listener(|this, _: &PaletteOpenTab, _, cx| this.activate(OpenMode::NewTab, cx)))
            .on_action(cx.listener(|this, _: &PaletteOpenSplit, _, cx| this.activate(OpenMode::Split, cx)))
            .w(px(560.))
            .flex()
            .flex_col()
            .rounded_lg()
            .bg(rgb(theme::TOOLBAR_BG))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .text_color(rgb(theme::TEXT))
            .child(div().px_3().py_2().border_b_1().border_color(rgb(theme::BORDER)).child(self.input.clone()))
            .child(
                div()
                    .id("palette-rows")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pb_1()
                    .children(rows),
            )
    }
}
