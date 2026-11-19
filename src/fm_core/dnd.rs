use super::ops::OpRequest;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropOp {
    Move,
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSource {
    Internal,
    External,
}

/// Whether `a` and `b` live on the same device. Any error counts as "different" (copy is the safe choice).
pub fn same_volume(a: &Path, b: &Path) -> bool {
    match (std::fs::symlink_metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev(),
        _ => false,
    }
}

/// What a drop of `srcs` onto folder `dest` should do, or `None` to refuse. Lexical apart from `same_volume`.
/// `src_parents[i]` is the parent folder of `srcs[i]`.
pub fn drop_op(
    srcs: &[PathBuf],
    src_parents: &[PathBuf],
    dest: &Path,
    alt: bool,
    src: DragSource,
    same_volume: &dyn Fn(&Path, &Path) -> bool,
) -> Option<DropOp> {
    if srcs.is_empty() {
        return None;
    }
    if srcs.iter().any(|s| s == dest || dest.starts_with(s)) {
        return None;
    }
    if alt || src == DragSource::External || srcs.iter().any(|s| !same_volume(s, dest)) {
        return Some(DropOp::Copy);
    }
    if src_parents.len() == srcs.len() && src_parents.iter().all(|p| p == dest) {
        return None;
    }
    Some(DropOp::Move)
}

pub fn request(op: DropOp, srcs: Vec<PathBuf>, dest: PathBuf) -> OpRequest {
    match op {
        DropOp::Move => OpRequest::Move { srcs, dest },
        DropOp::Copy => OpRequest::Copy { srcs, dest },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }
    fn same(_: &Path, _: &Path) -> bool {
        true
    }
    fn diff(_: &Path, _: &Path) -> bool {
        false
    }
    fn op(srcs: &[&str], dest: &str, alt: bool, src: DragSource, vol: &dyn Fn(&Path, &Path) -> bool) -> Option<DropOp> {
        let srcs: Vec<PathBuf> = srcs.iter().map(|s| p(s)).collect();
        let parents: Vec<PathBuf> = srcs.iter().map(|s| s.parent().unwrap().to_path_buf()).collect();
        drop_op(&srcs, &parents, Path::new(dest), alt, src, vol)
    }

    #[test]
    fn none_when_empty() {
        assert_eq!(drop_op(&[], &[], Path::new("/d"), false, DragSource::Internal, &same), None);
    }

    #[test]
    fn none_into_self() {
        assert_eq!(op(&["/a/x"], "/a/x", false, DragSource::Internal, &same), None);
        assert_eq!(op(&["/a/x", "/a/y"], "/a/y", true, DragSource::Internal, &same), None);
    }

    #[test]
    fn none_into_descendant() {
        assert_eq!(op(&["/a/x"], "/a/x/sub/deep", false, DragSource::Internal, &same), None);
        assert_eq!(op(&["/a/x"], "/a/xy", false, DragSource::Internal, &same), Some(DropOp::Move));
    }

    #[test]
    fn same_parent_refused_but_alt_copies() {
        assert_eq!(op(&["/a/x", "/a/y"], "/a", false, DragSource::Internal, &same), None);
        assert_eq!(op(&["/a/x"], "/a", true, DragSource::Internal, &same), Some(DropOp::Copy));
        // mixed parents still move
        assert_eq!(op(&["/a/x", "/b/y"], "/a", false, DragSource::Internal, &same), Some(DropOp::Move));
    }

    #[test]
    fn cross_volume_copies() {
        assert_eq!(op(&["/a/x"], "/b", false, DragSource::Internal, &diff), Some(DropOp::Copy));
    }

    #[test]
    fn external_always_copies() {
        assert_eq!(op(&["/a/x"], "/b", false, DragSource::External, &same), Some(DropOp::Copy));
        assert_eq!(op(&["/a/x"], "/a", false, DragSource::External, &same), Some(DropOp::Copy));
    }

    #[test]
    fn default_moves() {
        assert_eq!(op(&["/a/x"], "/b", false, DragSource::Internal, &same), Some(DropOp::Move));
        match request(DropOp::Move, vec![p("/a/x")], p("/b")) {
            OpRequest::Move { srcs, dest } => assert_eq!((srcs, dest), (vec![p("/a/x")], p("/b"))),
            _ => panic!("expected Move"),
        }
        match request(DropOp::Copy, vec![p("/a/x")], p("/b")) {
            OpRequest::Copy { srcs, dest } => assert_eq!((srcs, dest), (vec![p("/a/x")], p("/b"))),
            _ => panic!("expected Copy"),
        }
    }

    #[test]
    fn same_volume_tempdir() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f");
        std::fs::write(&f, "x").unwrap();
        assert!(same_volume(&f, t.path()));
        assert!(!same_volume(&f, &t.path().join("missing")));
        assert!(!same_volume(&t.path().join("missing"), t.path()));
    }
}
