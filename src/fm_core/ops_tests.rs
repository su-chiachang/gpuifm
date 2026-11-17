//! Ports of the Swift FileOperationServiceTests. Everything happens inside tempdirs; items that get
//! trashed carry a unique `nanofm-test-` name.

use super::ops::*;
use super::undo::UndoStack;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn uid() -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("nanofm-test-{}-{nanos}-{n}", std::process::id())
}

fn mkdir(base: &Path, rel: &str) -> PathBuf {
    let p = base.join(rel);
    fs::create_dir_all(&p).unwrap();
    p
}

fn mkfile(base: &Path, rel: &str, contents: &str) -> PathBuf {
    let p = base.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, contents).unwrap();
    p
}

fn read(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok()
}

fn exists(p: &Path) -> bool {
    lexists(p)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn leafs(v: &[PathBuf]) -> Vec<String> {
    v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
}

/// Restores permissions on drop so the tempdir can be removed.
struct Restore(PathBuf);
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
    }
}

fn run(st: &mut UndoStack, req: OpRequest) -> Result<Vec<PathBuf>, OpError> {
    let out = perform(req)?;
    if let Some(r) = out.record {
        st.push(r);
    }
    Ok(out.results)
}

fn mv(st: &mut UndoStack, srcs: &[&Path], dest: &Path) -> Result<Vec<PathBuf>, OpError> {
    run(st, OpRequest::Move { srcs: srcs.iter().map(|p| p.to_path_buf()).collect(), dest: dest.to_path_buf() })
}

fn cp(st: &mut UndoStack, srcs: &[&Path], dest: &Path) -> Result<Vec<PathBuf>, OpError> {
    run(st, OpRequest::Copy { srcs: srcs.iter().map(|p| p.to_path_buf()).collect(), dest: dest.to_path_buf() })
}

fn dup(st: &mut UndoStack, srcs: &[&Path]) -> Result<Vec<PathBuf>, OpError> {
    run(st, OpRequest::Duplicate(srcs.iter().map(|p| p.to_path_buf()).collect()))
}

fn ren(st: &mut UndoStack, src: &Path, name: &str) -> Result<Vec<PathBuf>, OpError> {
    run(st, OpRequest::Rename { src: src.to_path_buf(), name: name.into() })
}

fn trash(st: &mut UndoStack, srcs: &[&Path]) -> Result<Vec<PathBuf>, OpError> {
    run(st, OpRequest::Trash(srcs.iter().map(|p| p.to_path_buf()).collect()))
}

/// Creates a folder with a unique name through the real create path (so undo trashes only that).
fn new_folder_named(st: &mut UndoStack, dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    let rec = execute(Plan::CreateDir(p.clone())).unwrap().unwrap();
    st.push(rec);
    p
}

#[test]
fn move_and_undo_redo() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, "a.txt", "A");
    let b = mkfile(dir, "b.txt", "B");
    let dest = mkdir(dir, "dest");
    let mut st = UndoStack::default();
    assert!(!st.can_undo() && !st.can_redo());
    assert!(st.undo_title().is_none() && st.redo_title().is_none());

    let moved = mv(&mut st, &[&a, &b], &dest).unwrap();
    assert_eq!(moved, vec![dest.join("a.txt"), dest.join("b.txt")]);
    assert!(!exists(&a) && !exists(&b));
    assert_eq!(read(&moved[0]).as_deref(), Some("A"));
    assert_eq!(st.undo_title(), Some("Move of 2 Items"));

    st.undo().unwrap();
    assert!(exists(&a) && exists(&b));
    assert!(names(&dest).is_empty());
    assert!(!st.can_undo() && st.can_redo());
    assert_eq!(st.redo_title(), Some("Move of 2 Items"));

    st.redo().unwrap();
    assert!(!exists(&a));
    assert_eq!(names(&dest), ["a.txt", "b.txt"]);
    assert!(st.can_undo() && !st.can_redo());
}

#[test]
fn move_conflict_changes_nothing() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, "a.txt", "A");
    let b = mkfile(dir, "b.txt", "B");
    let dest = mkdir(dir, "dest");
    mkfile(&dest, "b.txt", "OTHER");
    let mut st = UndoStack::default();
    match mv(&mut st, &[&a, &b], &dest) {
        Err(OpError::DestinationExists(p)) => assert_eq!(p.file_name().unwrap(), "b.txt"),
        other => panic!("{other:?}"),
    }
    assert!(exists(&a) && exists(&b));
    assert_eq!(names(&dest), ["b.txt"]);
    assert!(!st.can_undo());
}

#[test]
fn move_missing_source() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dest = mkdir(t.path(), "dest");
    let ghost = t.path().join("ghost.txt");
    let mut st = UndoStack::default();
    match mv(&mut st, &[&ghost], &dest) {
        Err(OpError::SourceMissing(p)) => assert_eq!(p, ghost),
        other => panic!("{other:?}"),
    }
}

#[test]
fn into_itself_or_descendant() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let folder = mkdir(t.path(), "folder");
    let child = mkdir(&folder, "child");
    let mut st = UndoStack::default();
    for target in [&folder, &child] {
        match mv(&mut st, &[&folder], target) {
            Err(OpError::IntoItself(p)) => assert_eq!(p, folder),
            other => panic!("{other:?}"),
        }
    }
    assert!(matches!(cp(&mut st, &[&folder], &child), Err(OpError::IntoItself(_))));
    assert!(exists(&child));
    assert!(!st.can_undo());
}

#[test]
fn symlink_is_not_its_target() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let folder = mkdir(t.path(), "folder");
    let link = t.path().join("link");
    std::os::unix::fs::symlink(&folder, &link).unwrap();
    let mut st = UndoStack::default();
    // moving the symlink into the folder it points at is fine
    let out = mv(&mut st, &[&link], &folder).unwrap();
    assert_eq!(out, vec![folder.join("link")]);
}

#[test]
fn move_already_there() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "a.txt", "A");
    let b = mkfile(t.path(), "b.txt", "B");
    let mut st = UndoStack::default();
    assert!(matches!(mv(&mut st, &[&a, &b], t.path()), Err(OpError::AlreadyThere(_))));
    // some already there, some not: destination conflict
    let sub = mkdir(t.path(), "sub");
    let c = mkfile(&sub, "c.txt", "C");
    assert!(matches!(mv(&mut st, &[&a, &c], t.path()), Err(OpError::DestinationExists(_))));
    assert!(exists(&a) && exists(&c));
    assert!(!st.can_undo());
}

#[test]
fn move_rolls_back_on_partial_failure() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, "a.txt", "A");
    let locked = mkdir(dir, "locked");
    let b = mkfile(&locked, "b.txt", "B");
    let dest = mkdir(dir, "dest");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let _g = Restore(locked.clone());
    let mut st = UndoStack::default();
    assert!(matches!(mv(&mut st, &[&a, &b], &dest), Err(OpError::Io(_))));
    assert_eq!(read(&a).as_deref(), Some("A"));
    assert!(exists(&b));
    assert!(names(&dest).is_empty());
    assert!(!st.can_undo());
}

#[test]
fn copy_uniquifies_and_undo_redo() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let src = mkdir(dir, "src");
    let dest = mkdir(dir, "dest");
    let p = uid();
    let a = mkfile(&src, &format!("{p}-name.txt"), "A");
    let no_ext = mkfile(&src, &format!("{p}-README"), "R");
    let mut st = UndoStack::default();

    let first = cp(&mut st, &[&a, &no_ext], &dest).unwrap();
    assert_eq!(leafs(&first), [format!("{p}-name.txt"), format!("{p}-README")]);
    assert!(exists(&a));
    assert_eq!(st.undo_title(), Some("Copy of 2 Items"));

    let second = cp(&mut st, &[&a, &no_ext], &dest).unwrap();
    assert_eq!(leafs(&second), [format!("{p}-name 2.txt"), format!("{p}-README 2")]);
    let third = cp(&mut st, &[&a], &dest).unwrap();
    assert_eq!(leafs(&third), [format!("{p}-name 3.txt")]);
    assert_eq!(read(&third[0]).as_deref(), Some("A"));

    st.undo().unwrap();
    assert!(!exists(&third[0]));
    assert_eq!(
        names(&dest),
        [format!("{p}-README"), format!("{p}-README 2"), format!("{p}-name 2.txt"), format!("{p}-name.txt")]
    );
    st.redo().unwrap();
    assert_eq!(read(&third[0]).as_deref(), Some("A"));
    assert_eq!(names(&dest).len(), 5);
    st.undo().unwrap();
    assert!(!exists(&third[0]));
    assert!(exists(&a));
}

#[test]
fn copy_batch_same_names() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let d1 = mkdir(dir, "d1");
    let d2 = mkdir(dir, "d2");
    let a1 = mkfile(&d1, "same.txt", "1");
    let a2 = mkfile(&d2, "same.txt", "2");
    let dest = mkdir(dir, "dest");
    let mut st = UndoStack::default();
    let out = cp(&mut st, &[&a1, &a2], &dest).unwrap();
    assert_eq!(leafs(&out), ["same.txt", "same 2.txt"]);
    assert_eq!(read(&out[0]).as_deref(), Some("1"));
    assert_eq!(read(&out[1]).as_deref(), Some("2"));
}

#[test]
fn copy_folder_recursively() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let folder = mkdir(dir, "folder.v1");
    mkfile(&folder, "inner/deep.txt", "deep");
    let mut st = UndoStack::default();
    let out = cp(&mut st, &[&folder], dir).unwrap();
    assert_eq!(leafs(&out), ["folder.v1 2"]);
    assert_eq!(read(&out[0].join("inner/deep.txt")).as_deref(), Some("deep"));
}

#[test]
fn copy_rolls_back_on_partial_failure() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let ok = mkfile(dir, &format!("{}-ok.txt", uid()), "ok");
    let secret = mkfile(dir, &format!("{}-secret.txt", uid()), "s");
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    let _g = Restore(secret.clone());
    let dest = mkdir(dir, "dest");
    let mut st = UndoStack::default();
    assert!(matches!(cp(&mut st, &[&ok, &secret], &dest), Err(OpError::Io(_))));
    assert!(names(&dest).is_empty(), "the copy that succeeded must be rolled back");
    assert!(exists(&ok));
    assert!(!st.can_undo());
}

#[test]
fn duplicate_naming_and_undo() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let p = uid();
    let a = mkfile(dir, &format!("{p}-photo.jpg"), "P");
    let folder = mkdir(dir, &format!("{p}-stuff"));
    let mut st = UndoStack::default();
    let d1 = dup(&mut st, &[&a]).unwrap();
    assert_eq!(leafs(&d1), [format!("{p}-photo copy.jpg")]);
    let d2 = dup(&mut st, &[&a]).unwrap();
    assert_eq!(leafs(&d2), [format!("{p}-photo copy 2.jpg")]);
    let d3 = dup(&mut st, &[&a, &folder]).unwrap();
    assert_eq!(leafs(&d3), [format!("{p}-photo copy 3.jpg"), format!("{p}-stuff copy")]);
    assert_eq!(st.undo_title(), Some("Duplicate of 2 Items"));
    assert_eq!(read(&d1[0]).as_deref(), Some("P"));

    st.undo().unwrap();
    assert!(!exists(&d3[0]) && !exists(&d3[1]));
    assert!(exists(&d2[0]));
    st.redo().unwrap();
    assert!(exists(&d3[0]) && exists(&d3[1]));
    st.undo().unwrap(); // leave nothing important behind
}

#[test]
fn duplicate_same_name_in_batch() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "a.txt", "");
    let mut st = UndoStack::default();
    let out = dup(&mut st, &[&a, &a]).unwrap();
    assert_eq!(leafs(&out), ["a copy.txt", "a copy 2.txt"]);
}

#[test]
fn rename_undo_redo() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "old.txt", "O");
    let mut st = UndoStack::default();
    let out = ren(&mut st, &a, "new.txt").unwrap();
    assert_eq!(out, vec![t.path().join("new.txt")]);
    assert!(!exists(&a));
    assert_eq!(read(&out[0]).as_deref(), Some("O"));
    assert_eq!(st.undo_title(), Some("Rename"));
    st.undo().unwrap();
    assert!(exists(&a) && !exists(&out[0]));
    st.redo().unwrap();
    assert!(!exists(&a) && exists(&out[0]));
}

#[test]
fn rename_rejects_invalid_names() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "a.txt", "");
    mkfile(t.path(), "taken.txt", "");
    let mut st = UndoStack::default();
    let long = "x".repeat(256);
    for bad in ["", "   ", "x/y", "x:y", ".", "..", "a\0b", long.as_str()] {
        match ren(&mut st, &a, bad) {
            Err(OpError::InvalidName(n)) => assert_eq!(n, bad),
            other => panic!("{bad:?}: {other:?}"),
        }
    }
    match ren(&mut st, &a, "taken.txt") {
        Err(OpError::DestinationExists(p)) => assert_eq!(p.file_name().unwrap(), "taken.txt"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(ren(&mut st, &t.path().join("ghost"), "z"), Err(OpError::SourceMissing(_))));
    assert!(exists(&a));
    assert!(!st.can_undo());
    assert!(!OpError::InvalidName("x/y".into()).to_string().is_empty());
}

#[test]
fn rename_same_name_is_noop() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "a.txt", "");
    let mut st = UndoStack::default();
    assert_eq!(ren(&mut st, &a, "a.txt").unwrap(), vec![a]);
    assert!(!st.can_undo());
}

#[test]
fn case_only_rename() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "readme.txt", "R");
    let mut st = UndoStack::default();
    let out = ren(&mut st, &a, "README.txt").unwrap();
    assert_eq!(names(t.path()), ["README.txt"]);
    assert_eq!(read(&out[0]).as_deref(), Some("R"));
    st.undo().unwrap();
    assert_eq!(names(t.path()), ["readme.txt"]);
}

#[test]
fn new_folder_uniquifies() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let mut st = UndoStack::default();
    let f1 = run(&mut st, OpRequest::NewFolder { dir: t.path().into() }).unwrap();
    let f2 = run(&mut st, OpRequest::NewFolder { dir: t.path().into() }).unwrap();
    assert_eq!(leafs(&[f1[0].clone(), f2[0].clone()]), ["untitled folder", "untitled folder 2"]);
    assert_eq!(st.undo_title(), Some("New Folder"));
    assert!(fs::metadata(&f2[0]).unwrap().is_dir());
}

#[test]
fn new_folder_undo_redo() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let mut st = UndoStack::default();
    let f3 = new_folder_named(&mut st, t.path(), &uid());
    let f4 = new_folder_named(&mut st, t.path(), &uid());
    st.undo().unwrap();
    assert!(!exists(&f4) && exists(&f3));
    st.redo().unwrap();
    assert!(fs::metadata(&f4).unwrap().is_dir());
    st.undo().unwrap();
    st.undo().unwrap();
}

#[test]
fn trash_undo_redo() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, &format!("{}.txt", uid()), "A");
    let b = mkdir(dir, &uid());
    mkfile(&b, "inner.txt", "I");
    let mut st = UndoStack::default();
    trash(&mut st, &[&a, &b]).unwrap();
    assert!(!exists(&a) && !exists(&b));
    assert_eq!(st.undo_title(), Some("Move to Trash"));
    st.undo().unwrap();
    assert_eq!(read(&a).as_deref(), Some("A"));
    assert_eq!(read(&b.join("inner.txt")).as_deref(), Some("I"));
    assert!(st.can_redo());
    st.redo().unwrap();
    assert!(!exists(&a) && !exists(&b));
    st.undo().unwrap();
    assert!(exists(&a) && exists(&b));
}

#[test]
fn trash_skips_descendants() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let folder = mkdir(t.path(), &uid());
    let child = mkfile(&folder, "c.txt", "C");
    let mut st = UndoStack::default();
    trash(&mut st, &[&folder, &child]).unwrap();
    assert!(!exists(&folder));
    st.undo().unwrap();
    assert_eq!(read(&child).as_deref(), Some("C"));
}

#[test]
fn trash_missing_source() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), &format!("{}.txt", uid()), "");
    let mut st = UndoStack::default();
    let ghost = t.path().join("ghost");
    assert!(matches!(trash(&mut st, &[&a, &ghost]), Err(OpError::SourceMissing(_))));
    assert!(exists(&a), "nothing trashed when validation fails");
    assert!(!st.can_undo());
}

#[test]
fn trash_rolls_back_on_partial_failure() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, &format!("{}-ok.txt", uid()), "A");
    let locked = mkdir(dir, "locked");
    let b = mkfile(&locked, &format!("{}-locked.txt", uid()), "B");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
    let _g = Restore(locked.clone());
    let mut st = UndoStack::default();
    assert!(matches!(trash(&mut st, &[&a, &b]), Err(OpError::Io(_))));
    assert_eq!(read(&a).as_deref(), Some("A"), "first trashed item must be put back");
    assert!(exists(&b));
    assert!(!st.can_undo());
}

#[test]
fn new_op_clears_redo_failed_does_not() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let mut st = UndoStack::default();
    let f1 = new_folder_named(&mut st, t.path(), &uid());
    st.undo().unwrap();
    assert!(st.can_redo());
    // failed op: redo survives
    assert!(ren(&mut st, &t.path().join("ghost"), "x").is_err());
    assert!(st.can_redo());
    let f2 = new_folder_named(&mut st, t.path(), &uid());
    assert!(!st.can_redo());
    assert!(st.redo_title().is_none());
    st.redo().unwrap(); // no-op
    assert!(!exists(&f1));
    assert_eq!(st.undo_title(), Some("New Folder"));
    st.undo().unwrap();
    assert!(!exists(&f2));
}

#[test]
fn multi_level_order_and_titles() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let mut st = UndoStack::default();
    let f = new_folder_named(&mut st, dir, &uid());
    let g_name = uid();
    let renamed = ren(&mut st, &f, &g_name).unwrap().remove(0);
    let file = mkfile(dir, "m.txt", "");
    let moved = mv(&mut st, &[&file], &renamed).unwrap();
    assert_eq!(st.undo_title(), Some("Move of 1 Item"));

    st.undo().unwrap();
    assert!(exists(&file));
    assert_eq!(st.undo_title(), Some("Rename"));
    st.undo().unwrap();
    assert!(exists(&f) && !exists(&renamed));
    assert_eq!(st.undo_title(), Some("New Folder"));
    assert_eq!(st.redo_title(), Some("Rename"));
    st.undo().unwrap();
    assert!(!exists(&f));
    assert!(!st.can_undo());

    st.redo().unwrap();
    st.redo().unwrap();
    st.redo().unwrap();
    assert!(exists(&moved[0]));
    assert!(!st.can_redo());
    st.undo().unwrap();
    st.undo().unwrap();
    st.undo().unwrap();
}

#[test]
fn failed_undo_keeps_entry() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, "a.txt", "");
    let dest = mkdir(dir, "dest");
    let mut st = UndoStack::default();
    mv(&mut st, &[&a], &dest).unwrap();
    mkfile(dir, "a.txt", "squatter");
    assert!(matches!(st.undo(), Err(OpError::DestinationExists(_))));
    assert!(st.can_undo());
    assert!(!st.can_redo());
    assert!(exists(&dest.join("a.txt")));
}

#[test]
fn vanished_item_undo_drops_entry() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let dir = t.path();
    let a = mkfile(dir, "a.txt", "");
    let mut st = UndoStack::default();
    let b = ren(&mut st, &a, "b.txt").unwrap().remove(0);
    let folder = new_folder_named(&mut st, dir, &uid());
    fs::rename(&folder, dir.join("moved-away")).unwrap();
    assert!(matches!(st.undo(), Err(OpError::SourceMissing(_))));
    assert_eq!(st.undo_title(), Some("Rename"));
    assert!(!st.can_redo());
    st.undo().unwrap();
    assert!(exists(&a) && !exists(&b));
}

#[test]
fn touched_dirs_cover_both_sides() {
    let _trash = super::sys::TrashCleanup;
    let t = tempfile::tempdir().unwrap();
    let a = mkfile(t.path(), "a.txt", "");
    let dest = mkdir(t.path(), "dest");
    let out = perform(OpRequest::Move { srcs: vec![a], dest: dest.clone() }).unwrap();
    assert!(out.touched.contains(&dest) && out.touched.contains(&t.path().to_path_buf()));
    let dirs = out.record.unwrap().touched_dirs();
    assert!(dirs.contains(&dest) && dirs.contains(&t.path().to_path_buf()));
}
