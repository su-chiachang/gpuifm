use crate::ui::actions::*;
use crate::ui::commands;
use gpui::{App, Menu, MenuItem};

/// What the menu titles depend on.
pub struct MenuState<'a> {
    pub undo: Option<&'a str>,
    pub redo: Option<&'a str>,
    pub sidebar_visible: bool,
}

/// (Re)installs the application menus; Undo/Redo titles follow the undo stack and the sidebar
/// item says Hide or Show.
pub fn rebuild(cx: &mut App, state: &MenuState) {
    let undo = match state.undo {
        Some(t) => format!("Undo {t}"),
        None => "Undo".to_string(),
    };
    let redo = match state.redo {
        Some(t) => format!("Redo {t}"),
        None => "Redo".to_string(),
    };
    let item = |id: &str| match id {
        "edit.undo" => MenuItem::action(undo.clone(), Undo),
        "edit.redo" => MenuItem::action(redo.clone(), Redo),
        "view.sidebar" => MenuItem::action(if state.sidebar_visible { "Hide Sidebar" } else { "Show Sidebar" }, ToggleSidebar),
        _ => {
            let c = commands::find(id).expect("menu id in command table");
            MenuItem::Action { name: c.title.into(), action: (c.action)(), os_action: None, checked: false, disabled: false }
        }
    };
    let menu = |name: &'static str, ids: &[&str]| Menu::new(name).items(ids.iter().map(|id| if id.is_empty() { MenuItem::separator() } else { item(id) }));
    cx.set_menus(vec![
        Menu::new("nanofm").items([MenuItem::action("Quit nanofm", Quit)]),
        menu(
            "File",
            &["tab.new", "tab.close", "pane.close", "", "file.newFolder", "file.open", "file.rename", "file.duplicate", "", "file.openNewTab", "file.openSplit", "", "file.reveal", "file.copyPath", "", "file.pin", "file.addFavorite", "", "file.quickLook", "", "file.trash"],
        ),
        menu(
            "Edit",
            &["edit.undo", "edit.redo", "", "edit.copy", "edit.paste", "edit.pasteMove", "edit.selectAll", "", "edit.filter"],
        ),
        menu(
            "Go",
            &["go.back", "go.forward", "go.enclosing", "", "go.home", "go.desktop", "go.documents", "go.downloads", "go.applications", "", "go.goto"],
        ),
        menu(
            "View",
            &["view.list", "view.column", "", "view.hidden", "view.sidebar", "", "view.splitRight", "view.splitDown"],
        ),
        menu(
            "Window",
            &["window.nextTab", "window.prevTab", "", "window.nextPane", "window.prevPane", "", "window.palette"],
        ),
    ]);
}
