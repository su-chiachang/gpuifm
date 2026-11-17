//! The command table: one place that feeds the menus and the command palette.

use crate::ui::actions::*;
use gpui::Action;

pub struct CommandDef {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    /// Display string, e.g. "⌘T"; empty when the command has no shortcut.
    pub shortcut: &'static str,
    pub action: fn() -> Box<dyn Action>,
}

macro_rules! cmd {
    ($id:expr, $title:expr, $cat:expr, $key:expr, $act:expr) => {
        CommandDef { id: $id, title: $title, category: $cat, shortcut: $key, action: || Box::new($act) }
    };
}

pub fn all() -> Vec<CommandDef> {
    vec![
        cmd!("tab.new", "New Tab", "File", "\u{2318}T", NewTab),
        cmd!("tab.close", "Close Tab", "File", "\u{2318}W", CloseTab),
        cmd!("pane.close", "Close Pane", "File", "\u{2325}\u{2318}W", ClosePane),
        cmd!("file.newFolder", "New Folder", "File", "\u{21E7}\u{2318}N", NewFolder),
        cmd!("file.open", "Open", "File", "\u{2318}O", OpenSelected),
        cmd!("file.rename", "Rename", "File", "\u{21A9}", BeginRename),
        cmd!("file.duplicate", "Duplicate", "File", "", Duplicate),
        cmd!("file.trash", "Move to Trash", "File", "\u{2318}\u{232B}", TrashItems),
        cmd!("file.pin", "Pin to Sidebar", "File", "\u{2303}\u{2318}T", PinSelection),
        cmd!("file.addFavorite", "Add to Favorites", "File", "", AddFavorite),
        cmd!("edit.undo", "Undo", "Edit", "\u{2318}Z", Undo),
        cmd!("edit.redo", "Redo", "Edit", "\u{21E7}\u{2318}Z", Redo),
        cmd!("edit.copy", "Copy", "Edit", "\u{2318}C", CopyItems),
        cmd!("edit.paste", "Paste", "Edit", "\u{2318}V", PasteItems),
        cmd!("edit.pasteMove", "Move Items Here", "Edit", "\u{2325}\u{2318}V", PasteMove),
        cmd!("edit.selectAll", "Select All", "Edit", "\u{2318}A", SelectAll),
        cmd!("edit.filter", "Filter in Folder", "Edit", "\u{2318}F", FocusFilter),
        cmd!("go.back", "Back", "Go", "\u{2318}[", GoBack),
        cmd!("go.forward", "Forward", "Go", "\u{2318}]", GoForward),
        cmd!("go.enclosing", "Enclosing Folder", "Go", "\u{2318}\u{2191}", GoParent),
        cmd!("view.hidden", "Show Hidden Files", "View", "\u{21E7}\u{2318}.", ToggleHidden),
        cmd!("view.list", "as List", "View", "\u{2318}1", ViewList),
        cmd!("view.column", "as Columns", "View", "\u{2318}2", ViewColumn),
        cmd!("file.quickLook", "Quick Look", "File", "Space", QuickLook),
        cmd!("view.splitRight", "Split Side by Side", "View", "\u{2318}D", SplitRight),
        cmd!("view.splitDown", "Split Stacked", "View", "\u{21E7}\u{2318}D", SplitDown),
        cmd!("window.nextTab", "Show Next Tab", "Window", "\u{2303}\u{21E5}", NextTab),
        cmd!("window.prevTab", "Show Previous Tab", "Window", "\u{2303}\u{21E7}\u{21E5}", PrevTab),
        cmd!("window.nextPane", "Focus Next Pane", "Window", "\u{2325}\u{2318}\u{2192}", FocusNextPane),
        cmd!("window.prevPane", "Focus Previous Pane", "Window", "\u{2325}\u{2318}\u{2190}", FocusPrevPane),
        cmd!("window.palette", "Command Palette", "Window", "\u{2318}K", TogglePalette),
    ]
}

pub fn find(id: &str) -> Option<CommandDef> {
    all().into_iter().find(|c| c.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique_and_nonempty() {
        let all = all();
        assert!(!all.is_empty());
        let mut ids: Vec<_> = all.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(find("tab.new").is_some());
    }
}
