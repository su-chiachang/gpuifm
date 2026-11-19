//! The application key bindings, shared by `main` and the UI tests.

use crate::ui::actions::*;
use gpui::{App, KeyBinding};

pub fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectPrev, Some("FileList")),
        KeyBinding::new("down", SelectNext, Some("FileList")),
        KeyBinding::new("shift-up", SelectPrevExtend, Some("FileList")),
        KeyBinding::new("shift-down", SelectNextExtend, Some("FileList")),
        KeyBinding::new("enter", BeginRename, Some("FileList")),
        KeyBinding::new("cmd-o", OpenSelected, Some("FileList")),
        KeyBinding::new("cmd-down", OpenSelected, Some("FileList")),
        KeyBinding::new("cmd-a", SelectAll, Some("FileList")),
        KeyBinding::new("cmd-c", CopyItems, Some("FileList")),
        KeyBinding::new("cmd-v", PasteItems, Some("FileList")),
        KeyBinding::new("alt-cmd-v", PasteMove, Some("FileList")),
        KeyBinding::new("cmd-backspace", TrashItems, Some("FileList")),
        KeyBinding::new("cmd-shift-n", NewFolder, Some("Browser")),
        KeyBinding::new("cmd-z", Undo, Some("Browser")),
        KeyBinding::new("cmd-shift-z", Redo, Some("Browser")),
        KeyBinding::new("cmd-up", GoParent, Some("Browser")),
        KeyBinding::new("cmd-[", GoBack, Some("Browser")),
        KeyBinding::new("cmd-]", GoForward, Some("Browser")),
        KeyBinding::new("cmd-shift-.", ToggleHidden, Some("Browser")),
        // macOS reports cmd+shift+. as key ">" with shift consumed (platform/mac/events.rs:435)
        KeyBinding::new("cmd->", ToggleHidden, Some("Browser")),
        KeyBinding::new("cmd-t", NewTab, Some("Browser")),
        KeyBinding::new("cmd-w", CloseTab, Some("Browser")),
        KeyBinding::new("alt-cmd-w", ClosePane, Some("Browser")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Browser")),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Browser")),
        KeyBinding::new("cmd-d", SplitRight, Some("Browser")),
        KeyBinding::new("cmd-shift-d", SplitDown, Some("Browser")),
        KeyBinding::new("alt-cmd-right", FocusNextPane, Some("Browser")),
        KeyBinding::new("alt-cmd-left", FocusPrevPane, Some("Browser")),
        KeyBinding::new("cmd-shift-h", GoHome, Some("Browser")),
        KeyBinding::new("alt-cmd-l", GoDownloads, Some("Browser")),
        KeyBinding::new("cmd-shift-a", GoApplications, Some("Browser")),
        KeyBinding::new("cmd-shift-g", GoToFolder, Some("Browser")),
        KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some("Browser")),
        KeyBinding::new("alt-cmd-i", ToggleInspector, Some("Browser")),
        KeyBinding::new("alt-cmd-c", CopyPath, Some("FileList")),
        KeyBinding::new("cmd-k", TogglePalette, Some("Browser")),
        KeyBinding::new("cmd-f", FocusFilter, Some("Browser")),
        KeyBinding::new("ctrl-cmd-t", PinSelection, Some("Browser")),
        KeyBinding::new("cmd-1", ViewList, Some("Browser")),
        KeyBinding::new("cmd-2", ViewColumn, Some("Browser")),
        KeyBinding::new("space", QuickLook, Some("FileList")),
        KeyBinding::new("cmd-y", QuickLook, Some("FileList")),
        KeyBinding::new("left", ColumnLeft, Some("FileList && compact")),
        KeyBinding::new("right", ColumnRight, Some("FileList && compact")),
        KeyBinding::new("escape", CloseMenu, Some("FileList && menu")),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
    crate::ui::text_input::bind_keys(cx);
    // After the text-input bindings: later bindings win at equal depth.
    cx.bind_keys([
        KeyBinding::new("up", PaletteUp, Some("Palette > TextInput")),
        KeyBinding::new("down", PaletteDown, Some("Palette > TextInput")),
        KeyBinding::new("cmd-enter", PaletteOpenTab, Some("Palette > TextInput")),
        KeyBinding::new("alt-enter", PaletteOpenSplit, Some("Palette > TextInput")),
        KeyBinding::new("down", FilterDown, Some("Filter > TextInput")),
        // Shift-cmd-G inside the Go to Folder field closes it again.
        KeyBinding::new("cmd-shift-g", GoToFolder, Some("GoTo > TextInput")),
    ]);
}
