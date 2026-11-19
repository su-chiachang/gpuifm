use crate::fm_core::places::{Place, list_volumes};
use crate::fm_core::state::SidebarEntry;
use crate::ui::theme;
use gpui::{
    Context, EventEmitter, IntoElement, MouseButton, MouseDownEvent, Pixels, Point, Render, Window, anchored,
    deferred, div, prelude::*, px, rgb,
};
use std::path::{Path, PathBuf};

pub use crate::fm_core::places::Section as Group;

pub enum SidebarEvent {
    Open { path: PathBuf, new_tab: bool },
    OpenSplit { path: PathBuf },
    Remove { group: Group, ix: usize },
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
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(active: PathBuf, home_path: PathBuf, favorites: Vec<SidebarEntry>, pinned: Vec<SidebarEntry>) -> Self {
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
        }
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
        div()
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
            .child(title)
            .on_click(cx.listener(move |_, ev: &gpui::ClickEvent, _, cx| {
                cx.emit(SidebarEvent::Open { path: click_path.clone(), new_tab: ev.modifiers().platform });
            }))
            .when_some(group, |d, group| {
                d.on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.menu = Some(Menu { pos: ev.position, group, ix, path: menu_path.clone() });
                        cx.notify();
                    }),
                )
            })
    }

    fn heading(title: &'static str) -> impl IntoElement {
        div()
            .px_4()
            .pt_3()
            .pb_1()
            .text_xs()
            .text_color(rgb(theme::TEXT_DIM))
            .child(title)
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
        col = col.child(Self::heading("Favorites"));
        for (i, e) in favs.iter().enumerate() {
            col = col.child(self.row("fav", i, e.display_title(), &e.path, Some(Group::Favorites), cx));
        }
        if !pins.is_empty() {
            col = col.child(Self::heading("Pinned"));
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
