use crate::ui::actions::*;
use crate::ui::commands;
use gpui::{App, Menu, MenuItem};

/// (Re)installs the application menus; the Undo/Redo titles follow the undo stack.
pub fn rebuild(cx: &mut App, undo_title: Option<&str>, redo_title: Option<&str>) {
    let undo = match undo_title {
        Some(t) => format!("Undo {t}"),
        None => "Undo".to_string(),
    };
    let redo = match redo_title {
        Some(t) => format!("Redo {t}"),
        None => "Redo".to_string(),
    };
    let item = |id: &str| match id {
        "edit.undo" => MenuItem::action(undo.clone(), Undo),
        "edit.redo" => MenuItem::action(redo.clone(), Redo),
        _ => {
            let c = commands::find(id).expect("menu id in command table");
            MenuItem::Action { name: c.title.into(), action: (c.action)(), os_action: None }
        }
    };
    let menu = |name: &'static str, ids: &[&str]| Menu {
        name: name.into(),
        items: ids.iter().map(|id| if id.is_empty() { MenuItem::separator() } else { item(id) }).collect(),
    };
    cx.set_menus(vec![
        Menu { name: "nanofm".into(), items: vec![MenuItem::action("Quit nanofm", Quit)] },
        menu(
            "File",
            &["tab.new", "tab.close", "pane.close", "", "file.newFolder", "file.open", "file.rename", "file.duplicate", "", "file.pin", "file.addFavorite", "", "file.quickLook", "", "file.trash"],
        ),
        menu(
            "Edit",
            &["edit.undo", "edit.redo", "", "edit.copy", "edit.paste", "edit.pasteMove", "edit.selectAll", "", "edit.filter"],
        ),
        menu("Go", &["go.back", "go.forward", "go.enclosing"]),
        menu("View", &["view.list", "view.column", "", "view.hidden", "", "view.splitRight", "view.splitDown"]),
        menu(
            "Window",
            &["window.nextTab", "window.prevTab", "", "window.nextPane", "window.prevPane", "", "window.palette"],
        ),
    ]);
}
