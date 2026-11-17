//! File operations (move / copy / duplicate / rename / new folder / trash) with undo records.
//! Port of the Swift `FileOperationService`.

use super::kind::is_package_ext;
use super::sys;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum OpError {
    DestinationExists(PathBuf),
    InvalidName(String),
    SourceMissing(PathBuf),
    IntoItself(PathBuf),
    AlreadyThere(PathBuf),
    NotADirectory(PathBuf),
    Io(io::Error),
}

fn leaf(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpError::DestinationExists(p) => write!(
                f,
                "An item named \u{201C}{}\u{201D} already exists in the destination.",
                leaf(p)
            ),
            OpError::InvalidName(n) => {
                if n.trim_matches([' ', '\t']).is_empty() {
                    write!(f, "The name can\u{2019}t be empty.")
                } else {
                    write!(
                        f,
                        "\u{201C}{n}\u{201D} is not a valid name. Names can\u{2019}t contain \u{201C}/\u{201D} or \u{201C}:\u{201D}."
                    )
                }
            }
            OpError::SourceMissing(p) => write!(f, "\u{201C}{}\u{201D} no longer exists.", leaf(p)),
            OpError::IntoItself(p) => write!(
                f,
                "\u{201C}{}\u{201D} can\u{2019}t be placed inside itself.",
                leaf(p)
            ),
            OpError::AlreadyThere(p) => {
                write!(f, "The items are already in \u{201C}{}\u{201D}.", leaf(p))
            }
            OpError::NotADirectory(p) => write!(f, "\u{201C}{}\u{201D} is not a folder.", leaf(p)),
            OpError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for OpError {}

impl From<io::Error> for OpError {
    fn from(e: io::Error) -> Self {
        OpError::Io(e)
    }
}

pub enum OpRequest {
    Copy { srcs: Vec<PathBuf>, dest: PathBuf },
    Move { srcs: Vec<PathBuf>, dest: PathBuf },
    Duplicate(Vec<PathBuf>),
    Rename { src: PathBuf, name: String },
    NewFolder { dir: PathBuf },
    Trash(Vec<PathBuf>),
}

#[derive(Debug)]
pub enum Plan {
    Noop,
    Move { title: String, pairs: Vec<(PathBuf, PathBuf)> },
    Copy { title: String, pairs: Vec<(PathBuf, PathBuf)> },
    CreateDir(PathBuf),
    Trash(Vec<PathBuf>),
}

#[derive(Clone, Debug)]
pub struct Created {
    pub path: PathBuf,
    pub trashed: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum Change {
    Moved(Vec<(PathBuf, PathBuf)>),
    Created(Vec<Created>),
    Trashed(Vec<(PathBuf, PathBuf)>),
}

#[derive(Clone, Debug)]
pub struct UndoRecord {
    pub title: String,
    pub change: Change,
}

#[derive(Debug)]
pub struct Outcome {
    pub record: Option<UndoRecord>,
    pub results: Vec<PathBuf>,
    pub touched: Vec<PathBuf>,
}

fn items(n: usize) -> String {
    if n == 1 { "1 Item".into() } else { format!("{n} Items") }
}

pub fn lexists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

pub fn validate_name(name: &str) -> Result<(), OpError> {
    if name.trim().is_empty()
        || name.contains('/')
        || name.contains(':')
        || name == "."
        || name == ".."
        || name.len() > 255
        || name.contains('\0')
    {
        return Err(OpError::InvalidName(name.to_string()));
    }
    Ok(())
}

/// Splits `leaf` into (stem, extension-without-dot). Real directories never split;
/// packages and files do when the extension is non-empty and a stem remains.
pub fn split_ext(leaf: &str, real_dir: bool) -> (&str, &str) {
    if real_dir {
        return (leaf, "");
    }
    if let Some((stem, ext)) = leaf.rsplit_once('.')
        && !ext.is_empty()
        && leaf.len() > ext.len() + 1
    {
        return (stem, ext);
    }
    (leaf, "")
}

/// "name 2.ext" (copy) or "name copy.ext" / "name copy 2.ext" (duplicate).
pub fn unique_name(leaf: &str, real_dir: bool, duplicate: bool, taken: &dyn Fn(&str) -> bool) -> String {
    let (stem, ext) = split_ext(leaf, real_dir);
    let dot_ext = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let mut n = 1usize;
    loop {
        let base = match (duplicate, n) {
            (false, 1) => stem.to_string(),
            (false, _) => format!("{stem} {n}"),
            (true, 1) => format!("{stem} copy"),
            (true, _) => format!("{stem} copy {n}"),
        };
        let cand = format!("{base}{dot_ext}");
        if !taken(&cand) {
            return cand;
        }
        n += 1;
    }
}

fn is_real_dir(p: &Path) -> bool {
    let Ok(m) = fs::symlink_metadata(p) else { return false };
    if !m.is_dir() {
        return false;
    }
    !p.extension().and_then(|e| e.to_str()).is_some_and(is_package_ext)
}

fn require_directory(p: &Path) -> Result<(), OpError> {
    match fs::metadata(p) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(OpError::NotADirectory(p.to_path_buf())),
        Err(e) => Err(OpError::Io(e)),
    }
}

/// Components of `p` with the parent's symlinks resolved (the leaf is kept as is).
fn item_components(p: &Path) -> Vec<OsString> {
    let parent = p.parent().unwrap_or(Path::new("/"));
    let parent = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
    let mut v: Vec<OsString> = parent.components().map(|c| c.as_os_str().to_owned()).collect();
    if let Some(n) = p.file_name() {
        v.push(n.to_owned());
    }
    v
}

/// True when `dir` is `item` itself or lives underneath it.
fn is_inside(dir: &Path, item: &Path) -> bool {
    let d = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let d: Vec<OsString> = d.components().map(|c| c.as_os_str().to_owned()).collect();
    let s = item_components(item);
    d.len() >= s.len() && d[..s.len()] == s[..]
}

fn drop_descendants(urls: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen: HashSet<Vec<OsString>> = HashSet::new();
    let mut unique: Vec<(&PathBuf, Vec<OsString>)> = Vec::new();
    for u in urls {
        let c = item_components(u);
        if seen.insert(c.clone()) {
            unique.push((u, c));
        }
    }
    unique
        .iter()
        .filter(|(_, c)| {
            !unique
                .iter()
                .any(|(_, o)| o.len() < c.len() && c[..o.len()] == o[..])
        })
        .map(|(u, _)| (*u).clone())
        .collect()
}

fn parent_of(p: &Path) -> PathBuf {
    p.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"))
}

pub fn plan(req: &OpRequest, exists: &dyn Fn(&Path) -> bool) -> Result<Plan, OpError> {
    match req {
        OpRequest::Move { srcs, dest } => {
            if srcs.is_empty() {
                return Ok(Plan::Noop);
            }
            require_directory(dest)?;
            let mut pairs = Vec::new();
            let mut reserved = HashSet::new();
            for src in srcs {
                if !exists(src) {
                    return Err(OpError::SourceMissing(src.clone()));
                }
                if is_inside(dest, src) {
                    return Err(OpError::IntoItself(src.clone()));
                }
                let dst = dest.join(src.file_name().unwrap_or_default());
                if !reserved.insert(leaf(&dst).to_lowercase()) {
                    return Err(OpError::DestinationExists(dst));
                }
                pairs.push((src.clone(), dst));
            }
            let dest_c = fs::canonicalize(dest).unwrap_or_else(|_| dest.clone());
            let already = |s: &PathBuf| {
                s.parent()
                    .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
                    .is_some_and(|p| p == dest_c)
            };
            if pairs.iter().all(|(s, _)| already(s)) {
                return Err(OpError::AlreadyThere(dest.clone()));
            }
            Ok(Plan::Move { title: format!("Move of {}", items(srcs.len())), pairs })
        }
        OpRequest::Copy { srcs, dest } => {
            if srcs.is_empty() {
                return Ok(Plan::Noop);
            }
            require_directory(dest)?;
            let mut reserved = HashSet::new();
            let mut pairs = Vec::new();
            for src in srcs {
                if !exists(src) {
                    return Err(OpError::SourceMissing(src.clone()));
                }
                if is_inside(dest, src) {
                    return Err(OpError::IntoItself(src.clone()));
                }
                let dst = unique_dest(dest, src, false, &mut reserved, exists);
                pairs.push((src.clone(), dst));
            }
            Ok(Plan::Copy { title: format!("Copy of {}", items(srcs.len())), pairs })
        }
        OpRequest::Duplicate(srcs) => {
            if srcs.is_empty() {
                return Ok(Plan::Noop);
            }
            let mut reserved: std::collections::HashMap<PathBuf, HashSet<String>> = Default::default();
            let mut pairs = Vec::new();
            for src in srcs {
                if !exists(src) {
                    return Err(OpError::SourceMissing(src.clone()));
                }
                let dir = parent_of(src);
                let set = reserved.entry(dir.clone()).or_default();
                let dst = unique_dest(&dir, src, true, set, exists);
                pairs.push((src.clone(), dst));
            }
            Ok(Plan::Copy { title: format!("Duplicate of {}", items(srcs.len())), pairs })
        }
        OpRequest::Rename { src, name } => {
            validate_name(name)?;
            if !exists(src) {
                return Err(OpError::SourceMissing(src.clone()));
            }
            if leaf(src) == *name {
                return Ok(Plan::Noop);
            }
            let dst = parent_of(src).join(name);
            Ok(Plan::Move { title: "Rename".into(), pairs: vec![(src.clone(), dst)] })
        }
        OpRequest::NewFolder { dir } => {
            require_directory(dir)?;
            let base = "untitled folder";
            let mut n = 1;
            let mut dst = dir.join(base);
            while exists(&dst) {
                n += 1;
                dst = dir.join(format!("{base} {n}"));
            }
            Ok(Plan::CreateDir(dst))
        }
        OpRequest::Trash(urls) => {
            if urls.is_empty() {
                return Ok(Plan::Noop);
            }
            let targets = drop_descendants(urls);
            for t in &targets {
                if !exists(t) {
                    return Err(OpError::SourceMissing(t.clone()));
                }
            }
            Ok(Plan::Trash(targets))
        }
    }
}

fn unique_dest(
    dir: &Path,
    src: &Path,
    duplicate: bool,
    reserved: &mut HashSet<String>,
    exists: &dyn Fn(&Path) -> bool,
) -> PathBuf {
    let l = leaf(src);
    let name = unique_name(&l, is_real_dir(src), duplicate, &|c| {
        reserved.contains(&c.to_lowercase()) || exists(&dir.join(c))
    });
    reserved.insert(name.to_lowercase());
    dir.join(name)
}

// ---- execution -----------------------------------------------------------------------------

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(x), Ok(y)) => x.ino() == y.ino() && x.dev() == y.dev(),
        _ => false,
    }
}

/// `dst` differs from `src` only by letter case and is the very same file.
fn is_case_only_rename(src: &Path, dst: &Path) -> bool {
    src.parent() == dst.parent()
        && leaf(src) != leaf(dst)
        && leaf(src).to_lowercase() == leaf(dst).to_lowercase()
        && lexists(dst)
        && same_file(src, dst)
}

fn move_handling_case(src: &Path, dst: &Path) -> Result<(), OpError> {
    if lexists(dst) {
        if !is_case_only_rename(src, dst) {
            return Err(OpError::DestinationExists(dst.to_path_buf()));
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let tmp = parent_of(src).join(format!(".nanofm-rename-{}-{nanos}", std::process::id()));
        sys::move_item(src, &tmp)?;
        if let Err(e) = sys::move_item(&tmp, dst) {
            let _ = sys::move_item(&tmp, src);
            return Err(e.into());
        }
        Ok(())
    } else {
        sys::move_item(src, dst)?;
        Ok(())
    }
}

fn move_batch(pairs: &[(PathBuf, PathBuf)]) -> Result<(), OpError> {
    for (src, dst) in pairs {
        if !lexists(src) {
            return Err(OpError::SourceMissing(src.clone()));
        }
        if lexists(dst) && !is_case_only_rename(src, dst) {
            return Err(OpError::DestinationExists(dst.clone()));
        }
    }
    let mut done: Vec<&(PathBuf, PathBuf)> = Vec::new();
    for pair in pairs {
        match move_handling_case(&pair.0, &pair.1) {
            Ok(()) => done.push(pair),
            Err(e) => {
                for (s, d) in done.iter().rev() {
                    let _ = move_handling_case(d, s);
                }
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Rollback of something this operation created: prefer the Trash, fall back to removal.
fn discard_created(p: &Path) {
    if sys::trash_item(p).is_err() {
        let _ = match fs::symlink_metadata(p) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(p),
            _ => fs::remove_file(p),
        };
    }
}

fn copy_batch(pairs: &[(PathBuf, PathBuf)]) -> Result<(), OpError> {
    let mut done: Vec<&PathBuf> = Vec::new();
    for (src, dst) in pairs {
        let existed_before = lexists(dst);
        match sys::copy_item(src, dst) {
            Ok(()) => done.push(dst),
            Err(e) => {
                if !existed_before && lexists(dst) {
                    discard_created(dst);
                }
                for d in done.iter().rev() {
                    discard_created(d);
                }
                return Err(e.into());
            }
        }
    }
    Ok(())
}

/// Returns the Trash paths, in input order.
fn trash_batch(urls: &[PathBuf]) -> Result<Vec<PathBuf>, OpError> {
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for url in urls {
        let res = if lexists(url) {
            sys::trash_item(url).map_err(OpError::from)
        } else {
            Err(OpError::SourceMissing(url.clone()))
        };
        match res {
            Ok(t) => done.push((url.clone(), t)),
            Err(e) => {
                for (orig, trashed) in done.iter().rev() {
                    let _ = sys::move_item(trashed, orig);
                }
                return Err(e);
            }
        }
    }
    Ok(done.into_iter().map(|(_, t)| t).collect())
}

pub fn execute(plan: Plan) -> Result<Option<UndoRecord>, OpError> {
    match plan {
        Plan::Noop => Ok(None),
        Plan::Move { title, pairs } => {
            move_batch(&pairs)?;
            Ok(Some(UndoRecord { title, change: Change::Moved(pairs) }))
        }
        Plan::Copy { title, pairs } => {
            copy_batch(&pairs)?;
            let created = pairs.into_iter().map(|(_, d)| Created { path: d, trashed: None }).collect();
            Ok(Some(UndoRecord { title, change: Change::Created(created) }))
        }
        Plan::CreateDir(p) => {
            fs::create_dir(&p)?;
            Ok(Some(UndoRecord {
                title: "New Folder".into(),
                change: Change::Created(vec![Created { path: p, trashed: None }]),
            }))
        }
        Plan::Trash(targets) => {
            let trashed = trash_batch(&targets)?;
            let pairs = targets.into_iter().zip(trashed).collect();
            Ok(Some(UndoRecord { title: "Move to Trash".into(), change: Change::Trashed(pairs) }))
        }
    }
}

fn dedup(mut v: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    v.retain(|p| seen.insert(p.clone()));
    v
}

pub fn perform(req: OpRequest) -> Result<Outcome, OpError> {
    let p = plan(&req, &lexists)?;
    let (results, touched): (Vec<PathBuf>, Vec<PathBuf>) = match (&p, &req) {
        (Plan::Noop, OpRequest::Rename { src, .. }) => (vec![src.clone()], vec![]),
        (Plan::Noop, _) => (vec![], vec![]),
        (Plan::Move { pairs, .. }, _) => (
            pairs.iter().map(|(_, d)| d.clone()).collect(),
            pairs.iter().flat_map(|(s, d)| [parent_of(s), parent_of(d)]).collect(),
        ),
        (Plan::Copy { pairs, .. }, _) => (
            pairs.iter().map(|(_, d)| d.clone()).collect(),
            pairs.iter().map(|(_, d)| parent_of(d)).collect(),
        ),
        (Plan::CreateDir(d), _) => (vec![d.clone()], vec![parent_of(d)]),
        (Plan::Trash(t), _) => (vec![], t.iter().map(|x| parent_of(x)).collect()),
    };
    let record = execute(p)?;
    Ok(Outcome { record, results, touched: dedup(touched) })
}

pub fn revert(rec: &mut UndoRecord) -> Result<(), OpError> {
    match &mut rec.change {
        Change::Moved(pairs) => {
            let rev: Vec<_> = pairs.iter().rev().map(|(f, t)| (t.clone(), f.clone())).collect();
            move_batch(&rev)
        }
        Change::Created(items) => {
            let paths: Vec<PathBuf> = items.iter().map(|c| c.path.clone()).collect();
            let trashed = trash_batch(&paths)?;
            for (c, t) in items.iter_mut().zip(trashed) {
                c.trashed = Some(t);
            }
            Ok(())
        }
        Change::Trashed(pairs) => {
            let rev: Vec<_> = pairs.iter().rev().map(|(o, t)| (t.clone(), o.clone())).collect();
            move_batch(&rev)
        }
    }
}

pub fn reapply(rec: &mut UndoRecord) -> Result<(), OpError> {
    match &mut rec.change {
        Change::Moved(pairs) => move_batch(pairs),
        Change::Created(items) => {
            let restorable: Vec<(PathBuf, PathBuf)> = items
                .iter()
                .filter_map(|c| c.trashed.clone().map(|t| (t, c.path.clone())))
                .collect();
            move_batch(&restorable)?;
            for c in items.iter_mut() {
                c.trashed = None;
            }
            Ok(())
        }
        Change::Trashed(pairs) => {
            let originals: Vec<PathBuf> = pairs.iter().map(|(o, _)| o.clone()).collect();
            let trashed = trash_batch(&originals)?;
            for (p, t) in pairs.iter_mut().zip(trashed) {
                p.1 = t;
            }
            Ok(())
        }
    }
}

impl UndoRecord {
    /// Directories whose listing may have changed by this record (either direction).
    pub fn touched_dirs(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        match &self.change {
            Change::Moved(pairs) | Change::Trashed(pairs) => {
                for (a, b) in pairs {
                    v.push(parent_of(a));
                    v.push(parent_of(b));
                }
            }
            Change::Created(items) => {
                for c in items {
                    v.push(parent_of(&c.path));
                    if let Some(t) = &c.trashed {
                        v.push(parent_of(t));
                    }
                }
            }
        }
        dedup(v)
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    fn never(_: &Path) -> bool {
        false
    }

    fn names(taken: &[&str], leaf: &str, dir: bool, dup: bool) -> String {
        let t: Vec<String> = taken.iter().map(|s| s.to_lowercase()).collect();
        unique_name(leaf, dir, dup, &|c| t.contains(&c.to_lowercase()))
    }

    #[test]
    fn copy_naming() {
        assert_eq!(names(&[], "name.txt", false, false), "name.txt");
        assert_eq!(names(&["name.txt"], "name.txt", false, false), "name 2.txt");
        assert_eq!(names(&["name.txt", "name 2.txt"], "name.txt", false, false), "name 3.txt");
        assert_eq!(names(&["README"], "README", false, false), "README 2");
    }

    #[test]
    fn duplicate_naming() {
        assert_eq!(names(&[], "photo.jpg", false, true), "photo copy.jpg");
        assert_eq!(names(&["photo copy.jpg"], "photo.jpg", false, true), "photo copy 2.jpg");
        assert_eq!(names(&[], "stuff", true, true), "stuff copy");
        assert_eq!(names(&["a copy.txt"], "a.txt", false, true), "a copy 2.txt");
    }

    #[test]
    fn dots_and_packages() {
        assert_eq!(names(&["folder.v1"], "folder.v1", true, false), "folder.v1 2");
        assert_eq!(names(&["Foo.app"], "Foo.app", false, false), "Foo 2.app");
        assert_eq!(names(&[".bashrc"], ".bashrc", false, false), ".bashrc 2");
    }

    #[test]
    fn split_ext_cases() {
        assert_eq!(split_ext(".bashrc", false), (".bashrc", ""));
        assert_eq!(split_ext("foo.", false), ("foo.", ""));
        assert_eq!(split_ext("a.tar.gz", false), ("a.tar", "gz"));
        assert_eq!(split_ext("a.b", true), ("a.b", ""));
        assert_eq!(split_ext("README", false), ("README", ""));
    }

    #[test]
    fn reserved_case_insensitive() {
        assert_eq!(names(&["NAME.TXT"], "name.txt", false, false), "name 2.txt");
    }

    #[test]
    fn plan_batch_reserved_is_case_insensitive() {
        let t = tempfile::tempdir().unwrap();
        let d1 = t.path().join("d1");
        let d2 = t.path().join("d2");
        let dest = t.path().join("dest");
        for d in [&d1, &d2, &dest] {
            fs::create_dir(d).unwrap();
        }
        let a = d1.join("Same.txt");
        let b = d2.join("same.TXT");
        fs::write(&a, "1").unwrap();
        fs::write(&b, "2").unwrap();
        let p = plan(&OpRequest::Copy { srcs: vec![a, b], dest: dest.clone() }, &lexists).unwrap();
        let Plan::Copy { pairs, .. } = p else { panic!() };
        assert_eq!(leaf(&pairs[0].1), "Same.txt");
        assert_eq!(leaf(&pairs[1].1), "same 2.TXT");
    }

    #[test]
    fn validate_name_cases() {
        for bad in ["", "  ", "a/b", "a:b", ".", "..", "\0", &"x".repeat(256)] {
            assert!(matches!(validate_name(bad), Err(OpError::InvalidName(_))), "{bad:?}");
        }
        assert!(validate_name(&"x".repeat(255)).is_ok());
        assert!(validate_name("ok name.txt").is_ok());
    }

    #[test]
    fn error_messages() {
        let e = OpError::DestinationExists(PathBuf::from("/a/b.txt"));
        assert_eq!(e.to_string(), "An item named \u{201C}b.txt\u{201D} already exists in the destination.");
        assert_eq!(OpError::InvalidName("  ".into()).to_string(), "The name can\u{2019}t be empty.");
        assert!(OpError::InvalidName("x/y".into()).to_string().contains("not a valid name"));
        assert_eq!(
            OpError::AlreadyThere(PathBuf::from("/a/dest")).to_string(),
            "The items are already in \u{201C}dest\u{201D}."
        );
    }

    #[test]
    fn empty_inputs_are_noops() {
        let d = PathBuf::from("/");
        assert!(matches!(
            plan(&OpRequest::Move { srcs: vec![], dest: d.clone() }, &never),
            Ok(Plan::Noop)
        ));
        assert!(matches!(
            plan(&OpRequest::Copy { srcs: vec![], dest: d }, &never),
            Ok(Plan::Noop)
        ));
        assert!(matches!(plan(&OpRequest::Duplicate(vec![]), &never), Ok(Plan::Noop)));
        assert!(matches!(plan(&OpRequest::Trash(vec![]), &never), Ok(Plan::Noop)));
    }

    #[test]
    fn plan_titles_and_missing() {
        let t = tempfile::tempdir().unwrap();
        let ghost = t.path().join("ghost");
        let r = plan(&OpRequest::Trash(vec![ghost.clone()]), &lexists);
        assert!(matches!(r, Err(OpError::SourceMissing(_))));
        let a = t.path().join("a");
        fs::write(&a, "").unwrap();
        let Plan::Copy { title, .. } = plan(&OpRequest::Duplicate(vec![a.clone()]), &lexists).unwrap() else {
            panic!()
        };
        assert_eq!(title, "Duplicate of 1 Item");
        let Plan::Copy { title, .. } =
            plan(&OpRequest::Duplicate(vec![a.clone(), a]), &lexists).unwrap()
        else {
            panic!()
        };
        assert_eq!(title, "Duplicate of 2 Items");
    }
}
