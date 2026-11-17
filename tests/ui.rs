//! Headless end-to-end UI tests: the real Browser, the real keymap, real keystrokes, tempdirs only.

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use nanofm::fm_core::places::Places;
use nanofm::fm_core::layout::{Axis, LayoutTree, PaneId};
use nanofm::fm_core::sys::TrashCleanup;
use nanofm::fm_core::sort::SortKey;
use nanofm::fm_core::state::{PaneSnapshot, PersistedState, SidebarEntry, StateStore, ViewMode};
use nanofm::ui::browser::{Browser, BrowserOptions};
use nanofm::ui::keymap;
use nanofm::ui::opener::Opener;
use nanofm::ui::palette::Row;
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

fn mount<'a>(
    cx: &'a mut TestAppContext,
    dir: &Path,
) -> (gpui::Entity<Browser>, &'a mut VisualTestContext) {
    cx.update(keymap::bind);
    let dir = dir.to_path_buf();
    let (b, vcx) = cx.add_window_view(move |window, cx| Browser::new(Some(dir), window, cx));
    vcx.run_until_parked();
    (b, vcx)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Sends keys one at a time, parking between steps so background file work completes.
fn keys(cx: &mut VisualTestContext, seq: &str) {
    for k in seq.split(' ') {
        cx.simulate_keystrokes(k);
        cx.run_until_parked();
    }
}

fn renaming(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Option<(String, std::ops::Range<usize>)> {
    cx.read(|cx| {
        let list = b.read(cx).active_list(cx).read(cx);
        list.rename_input().map(|i| {
            let i = i.read(cx);
            (i.text().to_string(), i.selected_range())
        })
    })
}

fn entry_names(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| b.read(cx).active_list(cx).read(cx).entry_names())
}

#[gpui::test]
fn enter_starts_rename_with_stem_selected(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down enter");
    assert_eq!(renaming(&b, cx), Some(("alpha.txt".into(), 0..5)));
}

#[gpui::test]
fn typing_then_enter_renames_on_disk(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down enter");
    cx.simulate_input("beta");
    cx.run_until_parked();
    keys(cx, "enter");
    assert_eq!(names(t.path()), ["beta.txt"]);
    assert!(renaming(&b, cx).is_none());
    assert_eq!(entry_names(&b, cx), ["beta.txt"]);
}

#[gpui::test]
fn trash_and_undo(cx: &mut TestAppContext) {
    let _trash = TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("nanofm-test-ui-trash.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down cmd-backspace");
    assert!(names(t.path()).is_empty());
    assert!(entry_names(&b, cx).is_empty());
    keys(cx, "cmd-z");
    assert_eq!(names(t.path()), ["nanofm-test-ui-trash.txt"]);
    assert_eq!(entry_names(&b, cx), ["nanofm-test-ui-trash.txt"]);
}

#[gpui::test]
fn cmd_o_opens_folder(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    fs::write(t.path().join("sub").join("inner.txt"), "i").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down cmd-o");
    let cur = cx.read(|cx| b.read(cx).active_list(cx).read(cx).current_dir().to_path_buf());
    assert_eq!(cur, t.path().join("sub"));
    assert_eq!(entry_names(&b, cx), ["inner.txt"]);
}

#[gpui::test]
fn shift_cmd_n_creates_folder_and_renames(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-shift-n");
    assert_eq!(names(t.path()), ["untitled folder"]);
    let (text, range) = renaming(&b, cx).expect("rename session active");
    assert_eq!(text, "untitled folder");
    assert_eq!(range, 0..text.len());
}

#[gpui::test]
fn cmd_shift_dot_toggles_hidden(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join(".secret"), "s").unwrap();
    fs::write(t.path().join("shown.txt"), "s").unwrap();
    let (b, cx) = mount(cx, t.path());
    assert_eq!(entry_names(&b, cx), ["shown.txt"]);
    keys(cx, "cmd-shift-.");
    assert_eq!(entry_names(&b, cx), [".secret", "shown.txt"]);
    keys(cx, "cmd-shift-.");
    assert_eq!(entry_names(&b, cx), ["shown.txt"]);
}

#[gpui::test]
fn cmd_greater_than_toggles_hidden(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join(".secret"), "s").unwrap();
    let (b, cx) = mount(cx, t.path());
    assert!(entry_names(&b, cx).is_empty());
    keys(cx, "cmd->");
    assert_eq!(entry_names(&b, cx), [".secret"]);
}

// ---- Phase 3: tabs and split panes --------------------------------------------------------

fn leaf(n: u64) -> LayoutTree<PaneId> {
    LayoutTree::Leaf(PaneId(n))
}

fn dir_of(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> std::path::PathBuf {
    cx.read(|cx| b.read(cx).active_list(cx).read(cx).current_dir().to_path_buf())
}

fn focused(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> u64 {
    cx.read(|cx| b.read(cx).focused().0)
}

fn tab_state(b: &gpui::Entity<Browser>, pane: u64, cx: &mut VisualTestContext) -> (usize, usize) {
    cx.read(|cx| {
        let p = b.read(cx).pane(PaneId(pane)).unwrap().read(cx);
        (p.tab_count(), p.active_tab())
    })
}

/// Entry names of every tab of a pane (including inactive tabs).
fn pane_lists(b: &gpui::Entity<Browser>, pane: u64, cx: &mut VisualTestContext) -> Vec<Vec<String>> {
    cx.read(|cx| {
        b.read(cx)
            .pane(PaneId(pane))
            .unwrap()
            .read(cx)
            .lists()
            .iter()
            .map(|l| l.read(cx).entry_names())
            .collect()
    })
}

fn selected(b: &gpui::Entity<Browser>, pane: u64, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| {
        let p = b.read(cx).pane(PaneId(pane)).unwrap().read(cx);
        p.active_list().read(cx).selected_name()
    })
}

fn is_renaming(b: &gpui::Entity<Browser>, pane: u64, ix: usize, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| {
        let p = b.read(cx).pane(PaneId(pane)).unwrap().read(cx);
        p.lists()[ix].read(cx).is_renaming()
    })
}

#[gpui::test]
fn cmd_t_new_tab_same_dir(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
    assert_eq!(dir_of(&b, cx), t.path());
}

#[gpui::test]
fn tabs_own_history_selection(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    fs::write(t.path().join("sub").join("inner.txt"), "i").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down cmd-o cmd-t cmd-up ctrl-tab");
    assert_eq!(tab_state(&b, 1, cx), (2, 0));
    assert_eq!(dir_of(&b, cx), t.path().join("sub"));
    keys(cx, "cmd-[");
    assert_eq!(dir_of(&b, cx), t.path());
    assert_eq!(selected(&b, 1, cx), Some("sub".into()));
    keys(cx, "ctrl-tab");
    assert_eq!(dir_of(&b, cx), t.path());
    assert!(cx.read(|cx| b.read(cx).active_list(cx).read(cx).can_back()));
}

#[gpui::test]
fn ctrl_tab_cycles(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-t cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (3, 2));
    keys(cx, "ctrl-tab");
    assert_eq!(tab_state(&b, 1, cx).1, 0);
    keys(cx, "ctrl-tab");
    assert_eq!(tab_state(&b, 1, cx).1, 1);
    keys(cx, "ctrl-shift-tab");
    assert_eq!(tab_state(&b, 1, cx).1, 0);
    keys(cx, "ctrl-shift-tab");
    assert_eq!(tab_state(&b, 1, cx).1, 2);
}

#[gpui::test]
fn cmd_d_split_focuses_new(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d");
    let want = LayoutTree::split(Axis::SideBySide, vec![leaf(1), leaf(2)]);
    assert_eq!(cx.read(|cx| b.read(cx).layout().clone()), want);
    assert_eq!(focused(&b, cx), 2);
    assert_eq!(dir_of(&b, cx), t.path());
    keys(cx, "cmd-shift-d");
    let want = LayoutTree::split(
        Axis::SideBySide,
        vec![leaf(1), LayoutTree::split(Axis::Stacked, vec![leaf(2), leaf(3)])],
    );
    assert_eq!(cx.read(|cx| b.read(cx).layout().clone()), want);
    assert_eq!(focused(&b, cx), 3);
}

#[gpui::test]
fn cmd_w_closes_pane_on_last_tab(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d cmd-w");
    assert_eq!(cx.read(|cx| b.read(cx).layout().clone()), leaf(1));
    assert_eq!(focused(&b, cx), 1);
    assert!(cx.read(|cx| b.read(cx).pane(PaneId(2)).is_none()));
    // keys still reach the surviving pane
    keys(cx, "cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
}

#[gpui::test]
fn cmd_w_closes_tab_then_window(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-t cmd-w");
    assert_eq!(tab_state(&b, 1, cx), (1, 0));
    keys(cx, "cmd-w");
    assert!(cx.windows().is_empty());
}

#[gpui::test]
fn alt_cmd_arrows_cycle_focus(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.txt"), "a").unwrap();
    fs::write(t.path().join("b.txt"), "b").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d cmd-d");
    assert_eq!(focused(&b, cx), 3);
    keys(cx, "alt-cmd-right");
    assert_eq!(focused(&b, cx), 1);
    keys(cx, "alt-cmd-left");
    assert_eq!(focused(&b, cx), 3);
    keys(cx, "alt-cmd-left");
    assert_eq!(focused(&b, cx), 2);
    keys(cx, "down");
    assert_eq!(selected(&b, 2, cx), Some("a.txt".into()));
    assert_eq!(selected(&b, 1, cx), None);
    assert_eq!(selected(&b, 3, cx), None);
}

#[gpui::test]
fn op_reloads_all_tabs(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d cmd-t ctrl-tab");
    assert_eq!(tab_state(&b, 2, cx), (2, 0));
    keys(cx, "cmd-shift-n");
    for pane in [1, 2] {
        for l in pane_lists(&b, pane, cx) {
            assert_eq!(l, ["untitled folder"]);
        }
    }
    assert!(is_renaming(&b, 2, 0, cx));
    assert!(!is_renaming(&b, 2, 1, cx));
    assert!(!is_renaming(&b, 1, 0, cx));
    keys(cx, "escape ctrl-tab");
    assert_eq!(tab_state(&b, 2, cx).1, 1);
    assert!(!is_renaming(&b, 2, 1, cx));
}

#[gpui::test]
fn undo_shared(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d cmd-shift-n escape");
    assert_eq!(names(t.path()), ["untitled folder"]);
    keys(cx, "alt-cmd-left cmd-z");
    assert!(names(t.path()).is_empty());
    assert!(pane_lists(&b, 1, cx)[0].is_empty());
    assert!(pane_lists(&b, 2, cx)[0].is_empty());
}

#[gpui::test]
fn clipboard_shared(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("aaa")).unwrap();
    fs::write(t.path().join("zzz.txt"), "z").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down down cmd-c cmd-d down cmd-o");
    assert_eq!(dir_of(&b, cx), t.path().join("aaa"));
    keys(cx, "cmd-v");
    assert_eq!(names(&t.path().join("aaa")), ["zzz.txt"]);
}

#[gpui::test]
fn title_follows_focus(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    let (b, cx) = mount(cx, t.path());
    let title = |cx: &mut VisualTestContext| cx.read(|cx| b.read(cx).title().to_string());
    let root = t.path().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(title(cx), root);
    keys(cx, "cmd-d down cmd-o");
    assert_eq!(title(cx), "sub");
    keys(cx, "alt-cmd-left");
    assert_eq!(title(cx), root);
    keys(cx, "alt-cmd-right");
    assert_eq!(title(cx), "sub");
}

#[gpui::test]
fn hidden_global(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join(".secret"), "s").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d cmd-t");
    keys(cx, "cmd-shift-.");
    for pane in [1, 2] {
        for l in pane_lists(&b, pane, cx) {
            assert_eq!(l, [".secret"]);
        }
    }
    // a tab created afterwards inherits the setting
    keys(cx, "cmd-t");
    assert_eq!(pane_lists(&b, 2, cx)[2], [".secret"]);
}

#[gpui::test]
fn tab_keys_blocked_in_rename(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down enter cmd-t ctrl-tab cmd-d cmd-w");
    assert_eq!(tab_state(&b, 1, cx), (1, 0));
    assert_eq!(cx.read(|cx| b.read(cx).layout().clone()), leaf(1));
    assert!(renaming(&b, cx).is_some());
    assert!(!cx.windows().is_empty());
}

#[gpui::test]
fn click_tab_activates(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
    let r = cx.debug_bounds("tab-1-0").expect("tab 0 rendered");
    cx.simulate_click(r.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(tab_state(&b, 1, cx), (2, 0));
}

#[gpui::test]
fn click_in_pane_focuses(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d");
    assert_eq!(focused(&b, cx), 2);
    let r = cx.debug_bounds("pane-1").expect("pane 1 rendered");
    cx.simulate_click(r.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(focused(&b, cx), 1);
    // keys now act on pane 1
    keys(cx, "cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
    assert_eq!(tab_state(&b, 2, cx), (1, 0));
}

#[gpui::test]
fn divider_drag_resizes_and_clamps(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-d");
    let ratios = |b: &gpui::Entity<Browser>, cx: &mut VisualTestContext| {
        cx.read(|cx| match b.read(cx).layout() {
            LayoutTree::Split { ratios, .. } => ratios.clone(),
            _ => panic!("not split"),
        })
    };
    assert_eq!(ratios(&b, cx), [0.5, 0.5]);
    let d = cx.debug_bounds("divider--0").expect("divider rendered");
    let root = cx.debug_bounds("pane-1").unwrap();
    let start = d.center();
    cx.simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::none());
    let to = gpui::point(start.x + gpui::px(150.), start.y);
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, gpui::MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let r = ratios(&b, cx);
    assert!(r[0] > 0.55, "{r:?}");
    assert!((r[0] + r[1] - 1.0).abs() < 1e-4);
    // far left: clamped at the minimum pane width, never collapsing
    let far = gpui::point(root.origin.x - gpui::px(500.), start.y);
    cx.simulate_mouse_move(far, gpui::MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    cx.simulate_mouse_up(far, gpui::MouseButton::Left, Modifiers::none());
    let r = ratios(&b, cx);
    assert!(r[0] > 0.05 && r[0] < 0.5, "{r:?}");
}

// ---- Phase 4 Batch A: persistence, sidebar pins, filter, sort, palette -------------------------

/// Records what would have been opened with the system.
#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<PathBuf>>>);

impl Opener for Recorder {
    fn open(&self, _cx: &mut gpui::App, path: &Path) {
        self.0.borrow_mut().push(path.to_path_buf());
    }
}

fn mount_opts(
    cx: &mut TestAppContext,
    make: impl FnOnce() -> BrowserOptions + 'static,
) -> (gpui::Entity<Browser>, &mut VisualTestContext) {
    cx.update(keymap::bind);
    let (b, vcx) = cx.add_window_view(move |window, cx| Browser::with_options(make(), window, cx));
    vcx.run_until_parked();
    (b, vcx)
}

fn leaf_snap(tabs: &[&Path], active: usize) -> LayoutTree<PaneSnapshot> {
    LayoutTree::Leaf(PaneSnapshot {
        tabs: tabs.iter().map(|p| p.to_path_buf()).collect(),
        active,
        view_mode: ViewMode::List,
    })
}

fn entry(p: &Path) -> SidebarEntry {
    SidebarEntry { path: p.to_path_buf(), title: None }
}

/// Writes a state file (never any real favorites) into `dir`.
fn seed(dir: &Path, favs: &[&Path], pinned: &[&Path], layout: LayoutTree<PaneSnapshot>) {
    let s = PersistedState {
        favorites: favs.iter().map(|p| entry(p)).collect(),
        pinned: pinned.iter().map(|p| entry(p)).collect(),
        layout,
        show_hidden: false,
        sidebar_visible: true,
    };
    StateStore::new(dir).save(&s).unwrap();
}

fn canon(p: &Path) -> PathBuf {
    fs::canonicalize(p).unwrap()
}

#[gpui::test]
fn restore_from_store_tempdir(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (a, b, c) = (t.path().join("a"), t.path().join("b"), t.path().join("c"));
    for d in [&a, &b, &c] {
        fs::create_dir(d).unwrap();
    }
    seed(
        state.path(),
        &[],
        &[],
        LayoutTree::split(Axis::SideBySide, vec![leaf_snap(&[&a, &b], 1), leaf_snap(&[&c], 0)]),
    );
    let dir = state.path().to_path_buf();
    let (br, cx) = mount_opts(cx, move || BrowserOptions { store: Some(StateStore::new(dir)), ..Default::default() });
    assert_eq!(cx.read(|cx| br.read(cx).layout().clone()), LayoutTree::split(Axis::SideBySide, vec![leaf(1), leaf(2)]));
    assert_eq!(tab_state(&br, 1, cx), (2, 1));
    assert_eq!(tab_state(&br, 2, cx), (1, 0));
    assert_eq!(dir_of(&br, cx), b);
}

#[gpui::test]
fn save_after_split(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), &[], &[], leaf_snap(&[t.path()], 0));
    let dir = state.path().to_path_buf();
    let (_b, cx) = mount_opts(cx, move || BrowserOptions { store: Some(StateStore::new(dir)), ..Default::default() });
    keys(cx, "cmd-d");
    // debounced: nothing written yet
    assert_eq!(StateStore::new(state.path()).load().layout.leaves().len(), 1);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let saved = StateStore::new(state.path()).load();
    assert_eq!(saved.layout.leaves().len(), 2);
    assert_eq!(saved.layout.leaves()[1].tabs, [t.path().to_path_buf()]);
    let files: Vec<_> = fs::read_dir(state.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(files, ["state.json"]);
}

#[gpui::test]
fn missing_folder_skipped(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let gone = t.path().join("gone");
    seed(
        state.path(),
        &[],
        &[],
        LayoutTree::split(Axis::SideBySide, vec![leaf_snap(&[t.path()], 0), leaf_snap(&[&gone], 0)]),
    );
    let dir = state.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || BrowserOptions { store: Some(StateStore::new(dir)), ..Default::default() });
    assert_eq!(cx.read(|cx| b.read(cx).layout().clone()), leaf(1));
    assert_eq!(dir_of(&b, cx), t.path());
}

fn pinned_paths(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.read(|cx| b.read(cx).pinned().iter().map(|e| e.path.clone()).collect())
}

#[gpui::test]
fn ctrl_cmd_t_pins_folder(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down ctrl-cmd-t");
    assert_eq!(pinned_paths(&b, cx), [canon(&t.path().join("sub"))]);
    assert!(cx.debug_bounds("place-pinned-0").is_some());
    keys(cx, "ctrl-cmd-t"); // same folder again: no duplicate
    assert_eq!(pinned_paths(&b, cx).len(), 1);
}

#[gpui::test]
fn pin_file_shows_banner(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down ctrl-cmd-t");
    assert!(pinned_paths(&b, cx).is_empty());
    assert_eq!(
        cx.read(|cx| b.read(cx).banner().map(str::to_string)),
        Some("Only folders can be pinned to the sidebar.".into())
    );
}

#[gpui::test]
fn right_click_remove_pin(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down ctrl-cmd-t");
    let r = cx.debug_bounds("place-pinned-0").expect("pinned row");
    cx.simulate_mouse_down(r.center(), gpui::MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
    let m = cx.debug_bounds("menu-remove").expect("context menu shown");
    cx.simulate_click(m.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(pinned_paths(&b, cx).is_empty());
}

fn filter_text(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| {
        let p = b.read(cx).pane(PaneId(1)).unwrap().read(cx);
        p.filter_input().map(|i| i.read(cx).text().to_string())
    })
}

#[gpui::test]
fn cmd_f_filters_and_escape_clears(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    fs::write(t.path().join("beta.txt"), "b").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-f");
    cx.simulate_input("ALP");
    cx.run_until_parked();
    assert_eq!(entry_names(&b, cx), ["alpha.txt"]);
    cx.simulate_input("zzz");
    cx.run_until_parked();
    assert!(entry_names(&b, cx).is_empty());
    keys(cx, "escape");
    assert_eq!(filter_text(&b, cx), None);
    assert_eq!(entry_names(&b, cx), ["alpha.txt", "beta.txt"]);
    // focus is back in the list
    keys(cx, "down");
    assert_eq!(selected(&b, 1, cx), Some("alpha.txt".into()));
}

#[gpui::test]
fn filter_down_returns_focus(cx: &mut TestAppContext) {
    let _trash = TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("nanofm-test-filter-alpha.txt"), "a").unwrap();
    fs::write(t.path().join("beta.txt"), "b").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-f");
    cx.simulate_input("filter-alp");
    cx.run_until_parked();
    keys(cx, "down cmd-backspace");
    assert_eq!(names(t.path()), ["beta.txt"]);
    assert!(entry_names(&b, cx).is_empty());
}

#[gpui::test]
fn filter_cleared_on_navigate(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    fs::write(t.path().join("sub").join("inner.txt"), "i").unwrap();
    fs::write(t.path().join("other.txt"), "o").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-f");
    cx.simulate_input("sub");
    cx.run_until_parked();
    assert_eq!(entry_names(&b, cx), ["sub"]);
    keys(cx, "down cmd-o");
    assert_eq!(dir_of(&b, cx), t.path().join("sub"));
    assert_eq!(filter_text(&b, cx), None);
    assert_eq!(entry_names(&b, cx), ["inner.txt"]);
}

#[gpui::test]
fn filter_kept_on_tab_switch(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    fs::write(t.path().join("beta.txt"), "b").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-f");
    cx.simulate_input("alp");
    cx.run_until_parked();
    keys(cx, "down cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
    assert_eq!(entry_names(&b, cx), ["alpha.txt"]);
    keys(cx, "ctrl-tab");
    assert_eq!(tab_state(&b, 1, cx).1, 0);
    assert_eq!(entry_names(&b, cx), ["alpha.txt"]);
    assert_eq!(filter_text(&b, cx), Some("alp".into()));
}

fn click_header(cx: &mut VisualTestContext, key: &str) {
    let tag = match key {
        "name" => "header-name",
        "size" => "header-size",
        _ => panic!("unknown header"),
    };
    let r = cx.debug_bounds(tag).expect("header rendered");
    cx.simulate_click(r.center(), Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn header_click_sorts_size_then_flips(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a_big.txt"), vec![b'x'; 100]).unwrap();
    fs::write(t.path().join("b_small.txt"), "x").unwrap();
    fs::write(t.path().join("c_mid.txt"), vec![b'x'; 10]).unwrap();
    let (b, cx) = mount(cx, t.path());
    assert_eq!(entry_names(&b, cx), ["a_big.txt", "b_small.txt", "c_mid.txt"]);
    click_header(cx, "size");
    assert_eq!(entry_names(&b, cx), ["b_small.txt", "c_mid.txt", "a_big.txt"]);
    click_header(cx, "size");
    assert_eq!(entry_names(&b, cx), ["a_big.txt", "c_mid.txt", "b_small.txt"]);
    click_header(cx, "name");
    assert_eq!(entry_names(&b, cx), ["a_big.txt", "b_small.txt", "c_mid.txt"]);
}

#[gpui::test]
fn sort_shared_by_tabs_in_pane(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a_big.txt"), vec![b'x'; 100]).unwrap();
    fs::write(t.path().join("b_small.txt"), "x").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-t");
    click_header(cx, "size");
    keys(cx, "ctrl-tab");
    assert_eq!(entry_names(&b, cx), ["b_small.txt", "a_big.txt"]);
    let key = cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).sort().key);
    assert_eq!(key, SortKey::Size);
    // a split inherits the sort
    keys(cx, "cmd-d");
    assert_eq!(entry_names(&b, cx), ["b_small.txt", "a_big.txt"]);
}

// ---- palette ------------------------------------------------------------------------------

fn palette_rows(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Option<Vec<Row>> {
    cx.read(|cx| b.read(cx).palette().map(|p| p.read(cx).rows().to_vec()))
}

fn palette_open(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> bool {
    palette_rows(b, cx).is_some()
}

fn indexed(root: &Path) -> impl FnOnce() -> BrowserOptions + 'static + use<> {
    let roots = vec![root.to_path_buf()];
    move || BrowserOptions { index_roots: Some(roots), ..Default::default() }
}

#[gpui::test]
fn cmd_k_opens_and_escape_closes(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    assert!(!palette_open(&b, cx));
    keys(cx, "cmd-k");
    let rows = palette_rows(&b, cx).expect("palette open");
    assert!(rows.iter().any(|r| matches!(r, Row::Command { id: "tab.new", .. })));
    keys(cx, "escape");
    assert!(!palette_open(&b, cx));
    // focus is back in the list
    fs::write(t.path().join("x.txt"), "x").unwrap();
    keys(cx, "cmd-t");
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
}

#[gpui::test]
fn cmd_k_inside_palette_closes(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-k");
    assert!(palette_open(&b, cx));
    keys(cx, "cmd-k");
    assert!(!palette_open(&b, cx));
}

#[gpui::test]
fn palette_blocked_while_renaming(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down enter cmd-k");
    assert!(!palette_open(&b, cx));
    assert!(renaming(&b, cx).is_some());
}

#[gpui::test]
fn palette_down_moves_selection_skipping_headers(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let (fa, fb, pc) = (t.path().join("fa"), t.path().join("fb"), t.path().join("pc"));
    for d in [&fa, &fb, &pc] {
        fs::create_dir(d).unwrap();
    }
    seed(state.path(), &[&fa, &fb], &[&pc], leaf_snap(&[t.path()], 0));
    let dir = state.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || BrowserOptions {
        store: Some(StateStore::new(dir)),
        index_roots: Some(vec![]),
        ..Default::default()
    });
    keys(cx, "cmd-k");
    let rows = palette_rows(&b, cx).unwrap();
    assert!(rows[0].is_header() && rows[3].is_header());
    let sel = |cx: &mut VisualTestContext| {
        cx.read(|cx| b.read(cx).palette().unwrap().read(cx).selected_row().cloned())
    };
    let loc = |p: &Path| Some(Row::Location { title: p.file_name().unwrap().to_string_lossy().into_owned(), path: p.to_path_buf() });
    assert_eq!(sel(cx), loc(&fa));
    keys(cx, "down");
    assert_eq!(sel(cx), loc(&fb));
    keys(cx, "down");
    assert_eq!(sel(cx), loc(&pc)); // skipped the "Pinned" header
    keys(cx, "up up up");
    assert_eq!(sel(cx), loc(&fa)); // stops at the first selectable row
}

#[gpui::test]
fn palette_enter_navigates_folder_hit(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("zebra_docs")).unwrap();
    fs::write(t.path().join("other.txt"), "o").unwrap();
    let work = tempfile::tempdir().unwrap();
    let make = indexed(t.path());
    cx.update(keymap::bind);
    let start = work.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || {
        let mut o = make();
        o.start = Some(start);
        o
    });
    keys(cx, "cmd-k");
    cx.simulate_input("zebra");
    cx.run_until_parked();
    let rows = palette_rows(&b, cx).unwrap();
    assert!(rows.iter().any(|r| matches!(r, Row::File(h) if h.name == "zebra_docs" && h.is_dir)));
    keys(cx, "enter");
    assert!(!palette_open(&b, cx));
    assert_eq!(dir_of(&b, cx), canon(&t.path().join("zebra_docs")));
}

#[gpui::test]
fn palette_command_runs_new_tab(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-k");
    cx.simulate_input("new tab");
    cx.run_until_parked();
    keys(cx, "enter");
    assert!(!palette_open(&b, cx));
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
}

#[gpui::test]
fn palette_cmd_enter_file_opens_parent_in_new_tab_selected(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    fs::write(t.path().join("sub").join("target_file.txt"), "t").unwrap();
    fs::write(t.path().join("sub").join("zzz.txt"), "z").unwrap();
    let work = tempfile::tempdir().unwrap();
    let make = indexed(t.path());
    let start = work.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || {
        let mut o = make();
        o.start = Some(start);
        o
    });
    keys(cx, "cmd-k");
    cx.simulate_input("target_file");
    cx.run_until_parked();
    keys(cx, "cmd-enter");
    assert!(!palette_open(&b, cx));
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
    assert_eq!(dir_of(&b, cx), canon(&t.path().join("sub")));
    assert_eq!(selected(&b, 1, cx), Some("target_file.txt".into()));
}

#[gpui::test]
fn palette_file_enter_calls_opener(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("readme_unique.txt"), "r").unwrap();
    let work = tempfile::tempdir().unwrap();
    let rec = Recorder::default();
    let (roots, start, op) = (vec![t.path().to_path_buf()], work.path().to_path_buf(), rec.clone());
    let (b, cx) = mount_opts(cx, move || BrowserOptions {
        start: Some(start),
        index_roots: Some(roots),
        opener: Rc::new(op),
        ..Default::default()
    });
    keys(cx, "cmd-k");
    cx.simulate_input("readme_unique");
    cx.run_until_parked();
    keys(cx, "enter");
    assert!(!palette_open(&b, cx));
    assert_eq!(*rec.0.borrow(), [canon(&t.path().join("readme_unique.txt"))]);
}

// ---- Phase 4 Batch B: watching, Quick Look, column view ------------------------------------------

fn make_dirs_changed(b: &gpui::Entity<Browser>, dirs: Vec<PathBuf>, cx: &mut VisualTestContext) {
    b.update(cx, |b, cx| b.dirs_changed(dirs, cx));
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
}

#[gpui::test]
fn external_create_appears(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    fs::write(t.path().join("b.txt"), "b").unwrap();
    // nothing happens before the debounce has elapsed
    b.update(cx, |b, cx| b.dirs_changed(vec![t.path().to_path_buf()], cx));
    cx.run_until_parked();
    assert_eq!(entry_names(&b, cx), ["a.txt"]);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(entry_names(&b, cx), ["a.txt", "b.txt"]);
}

#[gpui::test]
fn selection_survives_refresh(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("b.txt"), "b").unwrap();
    fs::write(t.path().join("c.txt"), "c").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down down");
    assert_eq!(selected(&b, 1, cx), Some("c.txt".into()));
    fs::write(t.path().join("a.txt"), "a").unwrap();
    make_dirs_changed(&b, vec![t.path().to_path_buf()], cx);
    assert_eq!(entry_names(&b, cx), ["a.txt", "b.txt", "c.txt"]);
    assert_eq!(selected(&b, 1, cx), Some("c.txt".into()));
}

#[gpui::test]
fn refresh_deferred_during_rename(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("alpha.txt"), "a").unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down enter");
    assert!(renaming(&b, cx).is_some());
    fs::write(t.path().join("zeta.txt"), "z").unwrap();
    make_dirs_changed(&b, vec![t.path().to_path_buf()], cx);
    assert_eq!(entry_names(&b, cx), ["alpha.txt"]);
    assert!(renaming(&b, cx).is_some());
    keys(cx, "escape");
    assert!(renaming(&b, cx).is_none());
    assert_eq!(entry_names(&b, cx), ["alpha.txt", "zeta.txt"]);
}

#[gpui::test]
fn watcher_follows_tab_dirs(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("sub")).unwrap();
    let (root, sub) = (canon(t.path()), canon(&t.path().join("sub")));
    let start = t.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || BrowserOptions { start: Some(start), watch: true, ..Default::default() });
    let watched = |b: &gpui::Entity<Browser>, cx: &mut VisualTestContext| cx.read(|cx| b.read(cx).watched_dirs());
    assert_eq!(watched(&b, cx), [root.clone()].into());
    keys(cx, "cmd-t down cmd-o");
    assert_eq!(watched(&b, cx), [root.clone(), sub].into());
    keys(cx, "cmd-w");
    assert_eq!(watched(&b, cx), [root].into());
}

/// Fake Quick Look: records every preview request; "open" until closed.
#[derive(Clone, Default)]
struct QlFake {
    shown: Rc<RefCell<Vec<Vec<PathBuf>>>>,
    open: Rc<std::cell::Cell<bool>>,
    closes: Rc<std::cell::Cell<u32>>,
}

impl Opener for QlFake {
    fn open(&self, _cx: &mut gpui::App, _path: &Path) {}
    fn quick_look(&self, paths: &[PathBuf]) -> bool {
        self.shown.borrow_mut().push(paths.to_vec());
        self.open.set(true);
        true
    }
    fn close_quick_look(&self) {
        self.open.set(false);
        self.closes.set(self.closes.get() + 1);
    }
    fn quick_look_open(&self) -> bool {
        self.open.get()
    }
}

fn mount_ql<'a>(cx: &'a mut TestAppContext, dir: &Path, fake: &QlFake) -> (gpui::Entity<Browser>, &'a mut VisualTestContext) {
    let (start, op) = (dir.to_path_buf(), fake.clone());
    mount_opts(cx, move || BrowserOptions { start: Some(start), opener: Rc::new(op), ..Default::default() })
}

#[gpui::test]
fn space_toggles_quicklook(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.txt"), "a").unwrap();
    let fake = QlFake::default();
    let (_b, cx) = mount_ql(cx, t.path(), &fake);
    keys(cx, "space"); // nothing selected: nothing to preview
    assert!(fake.shown.borrow().is_empty());
    keys(cx, "down space");
    assert_eq!(*fake.shown.borrow(), [vec![t.path().join("a.txt")]]);
    keys(cx, "space");
    assert!(!fake.open.get());
    assert_eq!(fake.shown.borrow().len(), 1);
}

#[gpui::test]
fn cmd_y_quicklook_and_follows_selection(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.txt"), "a").unwrap();
    fs::write(t.path().join("b.txt"), "b").unwrap();
    let fake = QlFake::default();
    let (_b, cx) = mount_ql(cx, t.path(), &fake);
    keys(cx, "down cmd-y down");
    assert_eq!(*fake.shown.borrow(), [vec![t.path().join("a.txt")], vec![t.path().join("b.txt")]]);
    assert!(fake.open.get());
}

#[gpui::test]
fn space_in_rename_types_space(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("ab"), "a").unwrap();
    let fake = QlFake::default();
    let (b, cx) = mount_ql(cx, t.path(), &fake);
    keys(cx, "down enter end space");
    assert_eq!(renaming(&b, cx).map(|r| r.0), Some("ab ".into()));
    assert!(fake.shown.borrow().is_empty());
    keys(cx, "cmd-y cmd-1 cmd-2");
    assert!(fake.shown.borrow().is_empty());
    assert_eq!(cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).view_mode()), ViewMode::List);
}

fn column_dirs(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.read(|cx| {
        let p = b.read(cx).pane(PaneId(1)).unwrap().read(cx);
        p.columns().expect("column view").read(cx).column_dirs(cx)
    })
}

fn nested(t: &Path) {
    fs::create_dir_all(t.join("sub").join("deep")).unwrap();
    fs::write(t.join("sub").join("deep").join("x.txt"), "x").unwrap();
    fs::write(t.join("sub").join("f.txt"), "f").unwrap();
}

#[gpui::test]
fn cmd_2_shows_columns(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    nested(t.path());
    let (b, cx) = mount(cx, &t.path().join("sub"));
    keys(cx, "cmd-2");
    assert_eq!(column_dirs(&b, cx), [t.path().to_path_buf(), t.path().join("sub")]);
    keys(cx, "down"); // "deep" selected: its contents preview on the right
    assert_eq!(column_dirs(&b, cx), [t.path().to_path_buf(), t.path().join("sub"), t.path().join("sub").join("deep")]);
    keys(cx, "cmd-1");
    assert!(cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).columns().is_none()));
}

#[gpui::test]
fn right_drills_without_history_and_left_truncates(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    nested(t.path());
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-2 down right");
    let (sub, deep) = (t.path().join("sub"), t.path().join("sub").join("deep"));
    assert_eq!(dir_of(&b, cx), sub);
    assert_eq!(selected(&b, 1, cx), Some("deep".into()));
    assert!(!cx.read(|cx| b.read(cx).active_list(cx).read(cx).can_back()));
    assert_eq!(column_dirs(&b, cx).len(), 4); // parent, root, sub, deep preview
    keys(cx, "right");
    assert_eq!(dir_of(&b, cx), deep);
    assert!(!cx.read(|cx| b.read(cx).active_list(cx).read(cx).can_back()));
    keys(cx, "left left");
    assert_eq!(dir_of(&b, cx), t.path());
    assert_eq!(selected(&b, 1, cx), Some("sub".into()));
    assert!(!cx.read(|cx| b.read(cx).active_list(cx).read(cx).can_back()));
}

#[gpui::test]
fn view_mode_persisted(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), &[], &[], leaf_snap(&[t.path()], 0));
    let dir = state.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || BrowserOptions { store: Some(StateStore::new(dir)), ..Default::default() });
    keys(cx, "cmd-2");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    match StateStore::new(state.path()).load().layout {
        LayoutTree::Leaf(s) => assert_eq!(s.view_mode, ViewMode::Column),
        other => panic!("{other:?}"),
    }
    let _ = b;
}

#[gpui::test]
fn view_mode_restored(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let col = LayoutTree::Leaf(PaneSnapshot { tabs: vec![t.path().to_path_buf()], active: 0, view_mode: ViewMode::Column });
    seed(state.path(), &[], &[], col);
    let dir = state.path().to_path_buf();
    let (b, cx) = mount_opts(cx, move || BrowserOptions { store: Some(StateStore::new(dir)), ..Default::default() });
    assert_eq!(cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).view_mode()), ViewMode::Column);
    assert_eq!(column_dirs(&b, cx).last(), Some(&t.path().to_path_buf()));
}

fn three_files() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    for n in ["a.txt", "b.txt", "c.txt"] {
        fs::write(t.path().join(n), "x").unwrap();
    }
    t
}

#[gpui::test]
fn list_rows_have_layout(cx: &mut TestAppContext) {
    let t = three_files();
    let (_b, cx) = mount(cx, t.path());
    for sel in ["row-0", "row-1", "row-2"] {
        let r = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{sel} not laid out"));
        assert!(r.size.height > gpui::px(0.), "{sel} has no height");
    }
}

#[gpui::test]
fn column_rows_have_layout(cx: &mut TestAppContext) {
    let t = three_files();
    let (_b, cx) = mount(cx, t.path());
    keys(cx, "cmd-2");
    let r = cx.debug_bounds("row-0").expect("row-0 laid out in column mode");
    assert!(r.size.height > gpui::px(0.));
}

// ---- Phase 5: Go menu ----------------------------------------------------------------------

/// Browser at `start` whose home (and Desktop, Downloads, ...) live under `home`: never real folders.
fn mount_home<'a>(
    cx: &'a mut TestAppContext,
    home: &Path,
    start: &Path,
) -> (gpui::Entity<Browser>, &'a mut VisualTestContext) {
    let (home, start) = (home.to_path_buf(), start.to_path_buf());
    mount_opts(cx, move || BrowserOptions { start: Some(start), places: Places::rooted(home), ..Default::default() })
}

fn banner_of(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| b.read(cx).banner().map(str::to_string))
}

fn goto_open(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| b.read(cx).go_to_input().is_some())
}

fn goto_text(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| b.read(cx).go_to_input().map(|i| i.read(cx).text().to_string()))
}

#[gpui::test]
fn shift_cmd_h_goes_home(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("a")).unwrap();
    let (b, cx) = mount_home(cx, t.path(), &t.path().join("a"));
    keys(cx, "cmd-shift-h");
    assert_eq!(dir_of(&b, cx), t.path());
}

#[gpui::test]
fn alt_cmd_l_downloads(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("Downloads")).unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "alt-cmd-l");
    assert_eq!(dir_of(&b, cx), t.path().join("Downloads"));
}

#[gpui::test]
fn go_desktop_via_dispatch_action(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("Desktop")).unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    cx.dispatch_action(nanofm::ui::actions::GoDesktop);
    cx.run_until_parked();
    assert_eq!(dir_of(&b, cx), t.path().join("Desktop"));
}

#[gpui::test]
fn missing_place_shows_banner(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "cmd-shift-a");
    assert_eq!(dir_of(&b, cx), t.path());
    let want = format!("The folder \u{201C}{}\u{201D} can\u{2019}t be found.", t.path().join("Applications").display());
    assert_eq!(banner_of(&b, cx), Some(want));
}

#[gpui::test]
fn shift_cmd_g_folder(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("a")).unwrap();
    fs::create_dir_all(t.path().join("b")).unwrap();
    let (b, cx) = mount_home(cx, t.path(), &t.path().join("a"));
    keys(cx, "cmd-shift-g");
    assert_eq!(goto_text(&b, cx), Some("~/a".into()));
    let sel = cx.read(|cx| b.read(cx).go_to_input().unwrap().read(cx).selected_range());
    assert_eq!(sel, 0..3);
    cx.simulate_input("../b");
    cx.run_until_parked();
    keys(cx, "enter");
    assert!(!goto_open(&b, cx));
    // relative input resolves against the current folder
    assert_eq!(dir_of(&b, cx), t.path().join("b"));
}

#[gpui::test]
fn shift_cmd_g_file_selects_in_parent(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("sub")).unwrap();
    fs::write(t.path().join("sub").join("f.txt"), "x").unwrap();
    fs::write(t.path().join("sub").join("g.txt"), "x").unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "cmd-shift-g");
    cx.simulate_input("~/sub/g.txt");
    cx.run_until_parked();
    keys(cx, "enter");
    assert_eq!(dir_of(&b, cx), t.path().join("sub"));
    assert_eq!(selected(&b, 1, cx), Some("g.txt".into()));
}

#[gpui::test]
fn shift_cmd_g_tilde(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("a")).unwrap();
    let (b, cx) = mount_home(cx, t.path(), &t.path().join("a"));
    keys(cx, "cmd-shift-g");
    cx.simulate_input("  ~  ");
    cx.run_until_parked();
    keys(cx, "enter");
    assert_eq!(dir_of(&b, cx), t.path());
}

#[gpui::test]
fn shift_cmd_g_missing_banner_exact_text(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "cmd-shift-g");
    cx.simulate_input(" ~/nope ");
    cx.run_until_parked();
    keys(cx, "enter");
    assert_eq!(dir_of(&b, cx), t.path());
    assert_eq!(banner_of(&b, cx), Some("The folder \u{201C} ~/nope \u{201D} can\u{2019}t be found.".into()));
}

#[gpui::test]
fn shift_cmd_g_empty_does_nothing(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "cmd-shift-g");
    cx.simulate_input("   ");
    cx.run_until_parked();
    keys(cx, "enter");
    assert!(!goto_open(&b, cx));
    assert_eq!(banner_of(&b, cx), None);
    assert_eq!(dir_of(&b, cx), t.path());
}

#[gpui::test]
fn go_to_escape_closes_and_toggles(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("a.txt"), "x").unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "cmd-shift-g");
    assert!(goto_open(&b, cx));
    keys(cx, "escape");
    assert!(!goto_open(&b, cx));
    // focus is back in the list
    keys(cx, "down");
    assert_eq!(selected(&b, 1, cx), Some("a.txt".into()));
    // the shortcut toggles, also from inside the field
    keys(cx, "cmd-shift-g");
    assert!(goto_open(&b, cx));
    keys(cx, "cmd-shift-g");
    assert!(!goto_open(&b, cx));
}

// ---- Phase 5: Copy Path and Reveal ---------------------------------------------------------

/// Fake opener: records reveals (never spawns `open -R`) and opens.
#[derive(Clone, Default)]
struct RevealRecorder {
    revealed: Rc<RefCell<Vec<Vec<PathBuf>>>>,
    opened: Rc<RefCell<Vec<PathBuf>>>,
}

impl Opener for RevealRecorder {
    fn open(&self, _cx: &mut gpui::App, path: &Path) {
        self.opened.borrow_mut().push(path.to_path_buf());
    }
    fn reveal(&self, paths: &[PathBuf]) {
        self.revealed.borrow_mut().push(paths.to_vec());
    }
}

fn mount_rec<'a>(
    cx: &'a mut TestAppContext,
    dir: &Path,
    rec: &RevealRecorder,
) -> (gpui::Entity<Browser>, &'a mut VisualTestContext) {
    let (start, op, home) = (dir.to_path_buf(), rec.clone(), dir.to_path_buf());
    mount_opts(cx, move || BrowserOptions {
        start: Some(start),
        opener: Rc::new(op),
        places: Places::rooted(home),
        ..Default::default()
    })
}

fn clipboard_text(cx: &mut VisualTestContext) -> Option<String> {
    cx.read(|cx| cx.read_from_clipboard().and_then(|c| c.text()))
}

#[gpui::test]
fn alt_cmd_c_copies_selection_newline_joined(cx: &mut TestAppContext) {
    let t = three_files();
    let (_b, cx) = mount(cx, t.path());
    keys(cx, "down shift-down");
    keys(cx, "alt-cmd-c");
    let want = format!("{}\n{}", t.path().join("a.txt").display(), t.path().join("b.txt").display());
    assert_eq!(clipboard_text(cx), Some(want));
}

#[gpui::test]
fn alt_cmd_c_no_selection_copies_folder(cx: &mut TestAppContext) {
    let t = three_files();
    let (_b, cx) = mount(cx, t.path());
    keys(cx, "alt-cmd-c");
    assert_eq!(clipboard_text(cx), Some(t.path().display().to_string()));
}

#[gpui::test]
fn alt_cmd_c_in_rename_types_nothing(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down enter");
    keys(cx, "alt-cmd-c");
    assert_eq!(clipboard_text(cx), None);
    assert_eq!(renaming(&b, cx), Some(("a.txt".into(), 0..1)));
}

#[gpui::test]
fn reveal_records_selection_or_folder(cx: &mut TestAppContext) {
    let t = three_files();
    let rec = RevealRecorder::default();
    let (_b, cx) = mount_rec(cx, t.path(), &rec);
    cx.dispatch_action(nanofm::ui::actions::RevealInFinder);
    cx.run_until_parked();
    keys(cx, "down");
    cx.dispatch_action(nanofm::ui::actions::RevealInFinder);
    cx.run_until_parked();
    assert_eq!(*rec.revealed.borrow(), [vec![t.path().to_path_buf()], vec![t.path().join("a.txt")]]);
}

// ---- Phase 5: sidebar toggle ---------------------------------------------------------------

fn sidebar_visible(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| b.read(cx).sidebar_visible())
}

#[gpui::test]
fn ctrl_cmd_s_toggles_and_persists(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed(state.path(), &[], &[], leaf_snap(&[t.path()], 0));
    let (dir, home) = (state.path().to_path_buf(), t.path().to_path_buf());
    let (b, cx) = mount_opts(cx, move || BrowserOptions {
        store: Some(StateStore::new(dir)),
        places: Places::rooted(home),
        ..Default::default()
    });
    assert!(sidebar_visible(&b, cx));
    keys(cx, "ctrl-cmd-s");
    assert!(!sidebar_visible(&b, cx));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(!StateStore::new(state.path()).load().sidebar_visible);
    keys(cx, "ctrl-cmd-s");
    assert!(sidebar_visible(&b, cx));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(StateStore::new(state.path()).load().sidebar_visible);
}

#[gpui::test]
fn sidebar_hidden_state_restored(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut s = StateStore::new(state.path()).load_with_home(t.path());
    s.layout = leaf_snap(&[t.path()], 0);
    s.sidebar_visible = false;
    StateStore::new(state.path()).save(&s).unwrap();
    let (dir, home) = (state.path().to_path_buf(), t.path().to_path_buf());
    let (b, cx) = mount_opts(cx, move || BrowserOptions {
        store: Some(StateStore::new(dir)),
        places: Places::rooted(home),
        ..Default::default()
    });
    assert!(!sidebar_visible(&b, cx));
}

#[gpui::test]
fn go_to_typing_ctrl_cmd_s_ignored(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let (b, cx) = mount_home(cx, t.path(), t.path());
    keys(cx, "cmd-shift-g");
    keys(cx, "ctrl-cmd-s");
    assert!(sidebar_visible(&b, cx));
    assert!(goto_open(&b, cx));
    assert_eq!(goto_text(&b, cx), Some("~".into()));
}

// ---- Phase 5: Open in New Tab / Split --------------------------------------------------------

fn tab_dirs(b: &gpui::Entity<Browser>, pane: u64, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.read(|cx| {
        b.read(cx).pane(PaneId(pane)).unwrap().read(cx).lists().iter().map(|l| l.read(cx).current_dir().to_path_buf()).collect()
    })
}

fn two_dirs_and_file() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir(t.path().join("a")).unwrap();
    fs::create_dir(t.path().join("b")).unwrap();
    fs::write(t.path().join("f.txt"), "x").unwrap();
    t
}

#[gpui::test]
fn open_in_new_tab_multi_folders(cx: &mut TestAppContext) {
    let t = two_dirs_and_file();
    let rec = RevealRecorder::default();
    let (b, cx) = mount_rec(cx, t.path(), &rec);
    keys(cx, "cmd-a");
    cx.dispatch_action(nanofm::ui::actions::OpenInNewTab);
    cx.run_until_parked();
    assert_eq!(tab_state(&b, 1, cx), (3, 1));
    assert_eq!(tab_dirs(&b, 1, cx), [t.path().to_path_buf(), t.path().join("a"), t.path().join("b")]);
    assert_eq!(dir_of(&b, cx), t.path().join("a"));
    assert_eq!(*rec.opened.borrow(), [t.path().join("f.txt")]);
}

#[gpui::test]
fn open_in_new_tab_no_selection_opens_current_folder(cx: &mut TestAppContext) {
    let t = two_dirs_and_file();
    let rec = RevealRecorder::default();
    let (b, cx) = mount_rec(cx, t.path(), &rec);
    cx.dispatch_action(nanofm::ui::actions::OpenInNewTab);
    cx.run_until_parked();
    assert_eq!(tab_state(&b, 1, cx), (2, 1));
    assert_eq!(tab_dirs(&b, 1, cx), [t.path().to_path_buf(), t.path().to_path_buf()]);
    assert!(rec.opened.borrow().is_empty());
}

#[gpui::test]
fn open_in_split_rest_as_tabs(cx: &mut TestAppContext) {
    let t = two_dirs_and_file();
    let rec = RevealRecorder::default();
    let (b, cx) = mount_rec(cx, t.path(), &rec);
    keys(cx, "cmd-a");
    cx.dispatch_action(nanofm::ui::actions::OpenInSplit);
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| b.read(cx).layout().clone()), LayoutTree::split(Axis::SideBySide, vec![leaf(1), leaf(2)]));
    assert_eq!(focused(&b, cx), 2);
    assert_eq!(tab_state(&b, 2, cx), (2, 0));
    assert_eq!(tab_dirs(&b, 2, cx), [t.path().join("a"), t.path().join("b")]);
    assert_eq!(tab_dirs(&b, 1, cx), [t.path().to_path_buf()]);
    assert_eq!(*rec.opened.borrow(), [t.path().join("f.txt")]);
}

// ---- Phase 5: path bar and status ------------------------------------------------------------

fn crumb_titles(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).crumbs(cx))
}

fn status_of(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> String {
    cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).status_text(cx))
}

#[gpui::test]
fn crumb_click_navigates(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("a").join("b")).unwrap();
    fs::write(t.path().join("a").join("f.txt"), "x").unwrap();
    let (b, cx) = mount_home(cx, t.path(), &t.path().join("a").join("b"));
    assert_eq!(crumb_titles(&b, cx), ["Home", "a", "b"]);
    let r = cx.debug_bounds("crumb-1-1").expect("crumb rendered");
    cx.simulate_click(r.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(dir_of(&b, cx), t.path().join("a"));
    assert_eq!(crumb_titles(&b, cx), ["Home", "a"]);
    assert_eq!(entry_names(&b, cx), ["b", "f.txt"]);
    // the home crumb goes home; focus is back in the list for keyboard use
    let r = cx.debug_bounds("crumb-1-0").expect("home crumb");
    cx.simulate_click(r.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(dir_of(&b, cx), t.path());
    keys(cx, "down");
    assert_eq!(selected(&b, 1, cx), Some("a".into()));
}

#[gpui::test]
fn crumbs_outside_home_start_at_root(cx: &mut TestAppContext) {
    let home = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let (b, cx) = mount_home(cx, home.path(), other.path());
    let titles = crumb_titles(&b, cx);
    assert_eq!(titles[0], "Macintosh HD");
    assert_eq!(titles.last().map(String::as_str), other.path().file_name().and_then(|n| n.to_str()));
}

#[gpui::test]
fn status_counts_and_selection(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    assert_eq!(status_of(&b, cx), "3 items");
    keys(cx, "down");
    assert_eq!(status_of(&b, cx), "1 of 3 items selected");
    keys(cx, "cmd-a");
    assert_eq!(status_of(&b, cx), "3 of 3 items selected");
}

#[gpui::test]
fn status_singular(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("x"), "x").unwrap();
    let (b, cx) = mount(cx, t.path());
    assert_eq!(status_of(&b, cx), "1 item");
}

#[gpui::test]
fn corrupt_state_renamed_and_tempdir_defaults(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    fs::write(state.path().join("state.json"), "{ definitely not json").unwrap();
    let (dir, home) = (state.path().to_path_buf(), t.path().to_path_buf());
    let (b, cx) = mount_opts(cx, move || BrowserOptions {
        store: Some(StateStore::new(dir)),
        places: Places::rooted(home),
        ..Default::default()
    });
    // defaults come from the rooted home (2x2 grid on its Desktop/Documents/Downloads/Home), never real folders
    assert_eq!(cx.read(|cx| b.read(cx).favorites().len()), 4);
    assert!(cx.read(|cx| b.read(cx).favorites().iter().all(|f| f.path.starts_with(t.path()))));
    let files: Vec<String> =
        fs::read_dir(state.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with("state.json.corrupt-"));
    assert_eq!(fs::read(state.path().join(&files[0])).unwrap(), b"{ definitely not json");
}

// ---- Phase 5: asynchronous column view -------------------------------------------------------

fn preview_names(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).columns().expect("column view").read(cx).preview_names())
}

#[gpui::test]
fn column_view_async_preview(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    nested(t.path());
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-2");
    // Selecting a folder shows its columns at once, but the entries come from a background read.
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::SelectNext), cx));
    assert_eq!(preview_names(&b, cx), None);
    cx.run_until_parked();
    assert_eq!(preview_names(&b, cx), Some(vec!["deep".to_string(), "f.txt".to_string()]));
}

#[gpui::test]
fn column_drill_before_preview_loaded_selects_first(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    nested(t.path());
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-2");
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::SelectNext), cx));
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::ColumnRight), cx));
    cx.run_until_parked();
    assert_eq!(dir_of(&b, cx), t.path().join("sub"));
    assert_eq!(selected(&b, 1, cx), Some("deep".into()));
    assert_eq!(preview_names(&b, cx), Some(vec!["x.txt".to_string()]));
}

// ---- Phase 5 stretch: row context menu -------------------------------------------------------

fn right_click(cx: &mut VisualTestContext, p: gpui::Point<gpui::Pixels>) {
    cx.simulate_mouse_down(p, gpui::MouseButton::Right, Modifiers::none());
    cx.simulate_mouse_up(p, gpui::MouseButton::Right, Modifiers::none());
    cx.run_until_parked();
}

fn menu_open(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| b.read(cx).active_list(cx).read(cx).context_menu_open())
}

fn sel_count(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> usize {
    cx.read(|cx| b.read(cx).active_list(cx).read(cx).selection_count())
}

#[gpui::test]
fn right_click_unselected_row_selects_only_it(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "down");
    let r = cx.debug_bounds("row-2").unwrap();
    right_click(cx, r.center());
    assert_eq!(selected(&b, 1, cx), Some("c.txt".into()));
    assert_eq!(sel_count(&b, cx), 1);
    assert!(menu_open(&b, cx));
}

#[gpui::test]
fn right_click_selected_row_keeps_selection(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-a");
    let r = cx.debug_bounds("row-0").unwrap();
    right_click(cx, r.center());
    assert_eq!(sel_count(&b, cx), 3);
    assert!(menu_open(&b, cx));
}

#[gpui::test]
fn right_click_empty_space_deselects(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-a");
    let r = cx.debug_bounds("row-2").unwrap();
    right_click(cx, gpui::point(r.center().x, r.bottom() + gpui::px(60.)));
    assert_eq!(sel_count(&b, cx), 0);
    assert!(!menu_open(&b, cx));
}

#[gpui::test]
fn context_menu_item_runs_action(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    let r = cx.debug_bounds("row-1").unwrap();
    right_click(cx, r.center());
    let item = cx.debug_bounds("ctx-copy-path").expect("menu item rendered");
    cx.simulate_click(item.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(!menu_open(&b, cx));
    assert_eq!(clipboard_text(cx), Some(t.path().join("b.txt").display().to_string()));
}

#[gpui::test]
fn context_menu_reveal_uses_opener(cx: &mut TestAppContext) {
    let t = three_files();
    let rec = RevealRecorder::default();
    let (_b, cx) = mount_rec(cx, t.path(), &rec);
    let r = cx.debug_bounds("row-0").unwrap();
    right_click(cx, r.center());
    let item = cx.debug_bounds("ctx-reveal").expect("menu item rendered");
    cx.simulate_click(item.center(), Modifiers::none());
    cx.run_until_parked();
    assert_eq!(*rec.revealed.borrow(), [vec![t.path().join("a.txt")]]);
}

#[gpui::test]
fn deep_path_keeps_last_crumb_visible(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    let deep = t.path().join("alpha_long_name").join("bravo_long_name").join("charlie_long_name").join("delta_long_name").join("current_folder");
    fs::create_dir_all(&deep).unwrap();
    let (b, cx) = mount_home(cx, t.path(), &deep);
    cx.simulate_resize(gpui::size(gpui::px(640.), gpui::px(300.)));
    cx.run_until_parked();
    let n = crumb_titles(&b, cx).len();
    assert_eq!(n, 6);
    let pane = cx.debug_bounds("pane-1").expect("pane");
    let last = cx.debug_bounds("crumb-1-5").expect("last crumb");
    assert!(last.left() >= pane.left() && last.right() <= pane.right(), "last crumb {last:?} outside pane {pane:?}");
}

#[gpui::test]
fn escape_closes_context_menu(cx: &mut TestAppContext) {
    let t = three_files();
    let (b, cx) = mount(cx, t.path());
    let r = cx.debug_bounds("row-1").unwrap();
    right_click(cx, r.center());
    assert!(menu_open(&b, cx));
    keys(cx, "escape");
    assert!(!menu_open(&b, cx));
    assert_eq!(selected(&b, 1, cx), Some("b.txt".into()));
}

fn select_first_pending(b: &gpui::Entity<Browser>, cx: &mut VisualTestContext) -> bool {
    cx.read(|cx| b.read(cx).pane(PaneId(1)).unwrap().read(cx).columns().expect("column view").read(cx).select_first_pending())
}

#[gpui::test]
fn column_drill_into_empty_folder_clears_pending_select(cx: &mut TestAppContext) {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join("empty")).unwrap();
    let (b, cx) = mount(cx, t.path());
    keys(cx, "cmd-2");
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::SelectNext), cx));
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::ColumnRight), cx));
    assert!(select_first_pending(&b, cx));
    cx.run_until_parked();
    assert_eq!(dir_of(&b, cx), t.path().join("empty"));
    assert!(!select_first_pending(&b, cx));
    // Left then Right again: the flag is set and cleared once more.
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::ColumnLeft), cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.dispatch_action(Box::new(nanofm::ui::actions::ColumnRight), cx));
    cx.run_until_parked();
    assert!(!select_first_pending(&b, cx));
}
