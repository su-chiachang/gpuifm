use crate::fm_core::dnd::DragSource;
use crate::fm_core::places::{Place, list_volumes};
use crate::fm_core::state::SidebarEntry;
use crate::ui::dnd::{self, DragPreview, DraggedFiles, DraggedPlace};
use crate::ui::icons::{IconCache, icon_el};
use crate::ui::ops::FileOps;
use crate::ui::theme;
use gpui::{
    App, Context, DragMoveEvent, Entity, EventEmitter, ExternalPaths, IntoElement, MouseButton, MouseDownEvent, Pixels,
    Point, Render, Window, anchored, deferred, div, prelude::*, px, rgb,
};
use std::any::Any;
use std::path::{Path, PathBuf};

pub use crate::fm_core::places::Section as Group;

pub enum SidebarEvent {
    Open { path: PathBuf, new_tab: bool },
    OpenSplit { path: PathBuf },
    Remove { group: Group, ix: usize },
    /// Files dropped onto a sidebar folder: move/copy them into `dest`.
    DropFiles { paths: Vec<PathBuf>, dest: PathBuf, alt: bool, source: DragSource },
    /// Folders dropped onto a section heading: add them to that section.
    AddFolders { group: Group, paths: Vec<PathBuf> },
    /// A sidebar entry dragged to an insertion slot (index in the destination as it was before the move).
    Reorder { from: (Group, usize), to: (Group, usize) },
}

/// Insertion slot of a sidebar reorder drag; `row` is the row whose hover produced it.
#[derive(Clone, Copy, PartialEq)]
struct Slot {
    group: Group,
    at: usize,
    row: usize,
}

struct Menu {
    pos: Point<Pixels>,
    group: Group,
    ix: usize,
    path: PathBuf,
}

pub struct Sidebar {
    favorites: Vec<SidebarEntry>,
    pinned: Vec<SidebarEntry>,
    locations: Vec<Place>,
    home: Place,
    active: Option<PathBuf>,
    menu: Option<Menu>,
    ops: Entity<FileOps>,
    slot: Option<Slot>,
    icons: Option<Entity<IconCache>>,
    icons_sub: Option<gpui::Subscription>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(
        active: PathBuf,
        home_path: PathBuf,
        favorites: Vec<SidebarEntry>,
        pinned: Vec<SidebarEntry>,
        ops: Entity<FileOps>,
    ) -> Self {
        let title = home_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Home".into());
        Self {
            favorites,
            pinned,
            locations: list_volumes(Path::new("/Volumes")),
            home: Place { title, path: home_path },
            active: Some(active),
            menu: None,
            ops,
            slot: None,
            icons: None,
            icons_sub: None,
        }
    }

    /// Shows native icons from `icons` next to the entries (none without it).
    pub fn set_icons(&mut self, icons: Entity<IconCache>, cx: &mut Context<Self>) {
        self.icons_sub = Some(cx.observe(&icons, |_, _, cx| cx.notify()));
        self.icons = Some(icons);
    }

    pub fn set_active(&mut self, p: PathBuf, cx: &mut Context<Self>) {
        self.active = Some(p);
        cx.notify();
    }

    pub fn set_entries(&mut self, favorites: Vec<SidebarEntry>, pinned: Vec<SidebarEntry>, cx: &mut Context<Self>) {
        self.favorites = favorites;
        self.pinned = pinned;
        self.menu = None;
        cx.notify();
    }

    fn row(
        &self,
        id: &'static str,
        ix: usize,
        title: String,
        path: &Path,
        group: Option<Group>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let click_path = path.to_path_buf();
        let menu_path = path.to_path_buf();
        let active = self.active.as_deref() == Some(path);
        let tag = format!("place-{id}-{ix}");
        let (ops, dest, dest_ext) = (self.ops.clone(), path.to_path_buf(), path.to_path_buf());
        let is_place = group.is_some();
        let slot = self.slot.filter(|s| Some(s.group) == group && cx.has_active_drag());
        let title_for_drag = title.clone();
        let icon = icon_el(self.icons.as_ref(), path, true, false, 16., cx);
        div()
            .relative()
            .id((id, ix))
            .debug_selector(move || tag.clone())
            .px_3()
            .py_1()
            .mx_2()
            .rounded_md()
            .cursor_pointer()
            .text_color(rgb(theme::TEXT))
            .when(active, |d| d.bg(rgb(theme::SIDEBAR_ACTIVE_BG)))
            .hover(|d| d.bg(rgb(theme::HOVER_BG)))
            .drag_over::<DraggedFiles>(|s, _, _, _| s.bg(rgb(theme::DROP_BG)))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(theme::DROP_BG)))
            .can_drop(move |payload: &dyn Any, window: &mut Window, cx: &mut App| {
                if payload.is::<DraggedPlace>() {
                    return is_place;
                }
                dnd::accepts(payload, &dest, window.modifiers().alt, ops.read(cx).busy())
            })
            .on_drop(cx.listener({
                let dest = dest_ext.clone();
                move |_, f: &DraggedFiles, window, cx| {
                    cx.emit(SidebarEvent::DropFiles {
                        paths: f.paths.to_vec(),
                        dest: dest.clone(),
                        alt: window.modifiers().alt,
                        source: DragSource::Internal,
                    });
                }
            }))
            .on_drop(cx.listener(move |_, e: &ExternalPaths, _, cx| {
                cx.emit(SidebarEvent::DropFiles {
                    paths: e.paths().to_vec(),
                    dest: dest_ext.clone(),
                    alt: false,
                    source: DragSource::External,
                });
            }))
            .when(slot.is_some_and(|s| s.at == ix), |d| d.child(Self::insertion_line(true)))
            .when(slot.is_some_and(|s| s.at == ix + 1 && s.at == self.group_len(group)), |d| {
                d.child(Self::insertion_line(false))
            })
            .flex()
            .items_center()
            .gap_2()
            .child(icon)
            .child(title)
            .on_click(cx.listener(move |_, ev: &gpui::ClickEvent, _, cx| {
                cx.emit(SidebarEvent::Open { path: click_path.clone(), new_tab: ev.modifiers().platform });
            }))
            .when_some(group, |d, group| {
                d.on_drag(DraggedPlace { group, ix }, move |_, _, _, cx| cx.new(|_| DragPreview::new(title_for_drag.clone())))
                    .on_drag_move(cx.listener(move |this, ev: &DragMoveEvent<DraggedPlace>, _, cx| {
                        let mine = this.slot.is_some_and(|s| s.row == ix && s.group == group);
                        if ev.bounds.contains(&ev.event.position) {
                            let lower = ev.event.position.y > ev.bounds.center().y;
                            let s = Slot { group, at: ix + usize::from(lower), row: ix };
                            if this.slot != Some(s) {
                                this.slot = Some(s);
                                cx.notify();
                            }
                        } else if mine {
                            this.slot = None;
                            cx.notify();
                        }
                    }))
                    .on_drop(cx.listener(move |this, p: &DraggedPlace, _, cx| {
                        let at = this.slot.filter(|s| s.group == group && s.row == ix).map_or(ix, |s| s.at);
                        this.slot = None;
                        cx.emit(SidebarEvent::Reorder { from: (p.group, p.ix), to: (group, at) });
                    }))
                    .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.menu = Some(Menu { pos: ev.position, group, ix, path: menu_path.clone() });
                        cx.notify();
                    }),
                )
            })
    }

    fn group_len(&self, group: Option<Group>) -> usize {
        match group {
            Some(Group::Favorites) => self.favorites.len(),
            Some(Group::Pinned) => self.pinned.len(),
            None => usize::MAX,
        }
    }

    fn insertion_line(top: bool) -> impl IntoElement {
        let l = div().absolute().left_0().right_0().h(px(2.)).bg(rgb(theme::ACCENT));
        if top { l.top_0() } else { l.bottom_0() }
    }

    fn heading(title: &'static str) -> gpui::Div {
        div().px_4().pt_3().pb_1().text_xs().text_color(rgb(theme::TEXT_DIM)).child(title)
    }

    /// A Favorites/Pinned heading: a drop target that adds dropped folders (or appends a dragged entry).
    fn section_heading(&self, title: &'static str, group: Group, cx: &mut Context<Self>) -> impl IntoElement {
        let ops = self.ops.clone();
        let tag = match group {
            Group::Favorites => "sidebar-heading-favorites",
            Group::Pinned => "sidebar-heading-pinned",
        };
        Self::heading(title)
            .id(tag)
            .debug_selector(move || tag.to_string())
            .drag_over::<DraggedFiles>(|s, _, _, _| s.bg(rgb(theme::DROP_BG)))
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(theme::DROP_BG)))
            .drag_over::<DraggedPlace>(|s, _, _, _| s.bg(rgb(theme::DROP_BG)))
            .can_drop(move |payload: &dyn Any, _: &mut Window, cx: &mut App| {
                if payload.is::<DraggedPlace>() {
                    return true;
                }
                let paths = if let Some(d) = payload.downcast_ref::<DraggedFiles>() {
                    d.paths.to_vec()
                } else if let Some(e) = payload.downcast_ref::<ExternalPaths>() {
                    e.paths().to_vec()
                } else {
                    return false;
                };
                !ops.read(cx).busy() && !paths.is_empty() && paths.iter().all(|p| p.is_dir())
            })
            .on_drop(cx.listener(move |_, f: &DraggedFiles, _, cx| {
                cx.emit(SidebarEvent::AddFolders { group, paths: f.paths.to_vec() });
            }))
            .on_drop(cx.listener(move |_, e: &ExternalPaths, _, cx| {
                cx.emit(SidebarEvent::AddFolders { group, paths: e.paths().to_vec() });
            }))
            .on_drop(cx.listener(move |this, p: &DraggedPlace, _, cx| {
                let at = match group {
                    Group::Favorites => this.favorites.len(),
                    Group::Pinned => this.pinned.len(),
                };
                this.slot = None;
                cx.emit(SidebarEvent::Reorder { from: (p.group, p.ix), to: (group, at) });
            }))
    }

    fn menu_item(
        &self,
        tag: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
        ev: impl Fn(&Menu) -> SidebarEvent + 'static,
    ) -> impl IntoElement {
        div()
            .id(tag)
            .debug_selector(move || tag.to_string())
            .px_3()
            .py_1()
            .cursor_pointer()
            .hover(|d| d.bg(rgb(theme::HOVER_BG)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(m) = this.menu.take() {
                    cx.emit(ev(&m));
                }
                cx.notify();
            }))
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.slot = None;
        }
        let home = self.home.clone();
        let favs = self.favorites.clone();
        let pins = self.pinned.clone();
        let locs = self.locations.clone();
        let mut col = div()
            .flex()
            .flex_col()
            .w(px(220.))
            .flex_none()
            .h_full()
            .bg(rgb(theme::SIDEBAR_BG))
            .border_r_1()
            .border_color(rgb(theme::BORDER))
            .text_sm()
            .child(Self::heading("Home"))
            .child(self.row("home", 0, home.title.clone(), &home.path, None, cx));
        col = col.child(self.section_heading("Favorites", Group::Favorites, cx));
        for (i, e) in favs.iter().enumerate() {
            col = col.child(self.row("fav", i, e.display_title(), &e.path, Some(Group::Favorites), cx));
        }
        // The Pinned heading shows during any drag so an empty section can still receive a folder.
        if !pins.is_empty() || cx.has_active_drag() {
            col = col.child(self.section_heading("Pinned", Group::Pinned, cx));
            for (i, e) in pins.iter().enumerate() {
                col = col.child(self.row("pinned", i, e.display_title(), &e.path, Some(Group::Pinned), cx));
            }
        }
        col = col.child(Self::heading("Locations"));
        for (i, p) in locs.iter().enumerate() {
            col = col.child(self.row("loc", i, p.title.clone(), &p.path, None, cx));
        }
        if let Some(m) = &self.menu {
            let menu = div()
                .id("sidebar-menu")
                .occlude()
                .min_w(px(180.))
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
                .child(self.menu_item("menu-open", "Open", cx, |m| SidebarEvent::Open {
                    path: m.path.clone(),
                    new_tab: false,
                }))
                .child(self.menu_item("menu-new-tab", "Open in New Tab", cx, |m| SidebarEvent::Open {
                    path: m.path.clone(),
                    new_tab: true,
                }))
                .child(self.menu_item("menu-split", "Open in Split", cx, |m| SidebarEvent::OpenSplit {
                    path: m.path.clone(),
                }))
                .child(self.menu_item("menu-remove", "Remove from Sidebar", cx, |m| SidebarEvent::Remove {
                    group: m.group,
                    ix: m.ix,
                }));
            col = col.child(deferred(anchored().position(m.pos).snap_to_window().child(menu)));
        }
        col
    }
}
