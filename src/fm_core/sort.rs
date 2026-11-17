use super::entry::FileEntry;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Kind,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortOrder {
    pub key: SortKey,
    pub ascending: bool,
    pub folders_first: bool,
}

impl Default for SortOrder {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            ascending: true,
            folders_first: true,
        }
    }
}

impl SortOrder {
    /// Header click: the same column flips direction, a new column starts ascending.
    pub fn header_click(self, key: SortKey) -> Self {
        if self.key == key {
            Self { ascending: !self.ascending, ..self }
        } else {
            Self { key, ascending: true, ..self }
        }
    }
}

/// Case-insensitive comparison where digit runs compare numerically.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut i, mut j) = (0, 0);
    let mut tie: Option<Ordering> = None;
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = a[si..i].iter().collect::<String>().trim_start_matches('0').into();
            let nb: String = b[sj..j].iter().collect::<String>().trim_start_matches('0').into();
            let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if ord != Ordering::Equal {
                return ord;
            }
            // Equal values: fewer leading zeros first ("file1" < "file01").
            let raw = (i - si).cmp(&(j - sj));
            if raw != Ordering::Equal {
                tie = tie.or(Some(raw));
            }
        } else {
            let ca = a[i].to_lowercase().next().unwrap_or(a[i]);
            let cb = b[j].to_lowercase().next().unwrap_or(b[j]);
            let ord = ca.cmp(&cb);
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j)).then_with(|| tie.unwrap_or(Ordering::Equal))
}

fn opt_cmp<T: Ord>(a: &Option<T>, b: &Option<T>, ascending: bool) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            if ascending {
                x.cmp(y)
            } else {
                y.cmp(x)
            }
        }
    }
}

pub fn sort_entries(v: &mut [FileEntry], o: SortOrder) {
    v.sort_by(|a, b| {
        if o.folders_first && a.is_dir != b.is_dir {
            return if a.is_dir {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let name_dir = |x: Ordering| if o.ascending { x } else { x.reverse() };
        let primary = match o.key {
            SortKey::Name => name_dir(natural_cmp(&a.name, &b.name)),
            SortKey::Kind => name_dir(natural_cmp(&a.kind, &b.kind)),
            SortKey::Size => opt_cmp(&a.size, &b.size, o.ascending),
            SortKey::Modified => opt_cmp(&a.modified, &b.modified, o.ascending),
        };
        primary.then_with(|| natural_cmp(&a.name, &b.name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime};

    fn e(name: &str, is_dir: bool, size: Option<u64>) -> FileEntry {
        FileEntry {
            path: PathBuf::from(name),
            name: name.into(),
            is_dir,
            is_package: false,
            is_hidden: false,
            is_symlink: false,
            size,
            modified: None,
            kind: if is_dir { "Folder".into() } else { "Document".into() },
        }
    }

    fn names(v: &[FileEntry]) -> Vec<&str> {
        v.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn header_click_flips_or_resets() {
        let o = SortOrder::default();
        let o = o.header_click(SortKey::Size);
        assert_eq!((o.key, o.ascending), (SortKey::Size, true));
        let o = o.header_click(SortKey::Size);
        assert!(!o.ascending);
        let o = o.header_click(SortKey::Kind);
        assert_eq!((o.key, o.ascending), (SortKey::Kind, true));
    }

    #[test]
    fn natural() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("ABC", "abc"), Ordering::Equal);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
    }

    #[test]
    fn leading_zeros() {
        assert_eq!(natural_cmp("file002", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("file1", "file01"), Ordering::Less);
        assert_eq!(natural_cmp("file01", "file1"), Ordering::Greater);
        assert_eq!(natural_cmp("file01", "file01"), Ordering::Equal);
    }

    #[test]
    fn none_last_mixed_folders_files_under_size() {
        let mut v = vec![e("f1", false, Some(5)), e("d1", true, None), e("f2", false, Some(1)), e("d2", true, None)];
        let mut o = SortOrder { key: SortKey::Size, ascending: true, folders_first: true };
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["d1", "d2", "f2", "f1"]);
        o.folders_first = false;
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["f2", "f1", "d1", "d2"]);
        o.ascending = false;
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["f1", "f2", "d1", "d2"]);
    }

    #[test]
    fn name_sort_and_folders_first() {
        let mut v = vec![e("b", false, Some(1)), e("z", true, None), e("a", false, Some(1)), e("file10", false, Some(1)), e("file2", false, Some(1))];
        sort_entries(&mut v, SortOrder::default());
        assert_eq!(names(&v), ["z", "a", "b", "file2", "file10"]);
        let o = SortOrder { folders_first: false, ..Default::default() };
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["a", "b", "file2", "file10", "z"]);
    }

    #[test]
    fn descending() {
        let mut v = vec![e("a", false, Some(1)), e("b", false, Some(5)), e("c", false, Some(3))];
        let mut o = SortOrder { key: SortKey::Name, ascending: false, folders_first: true };
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["c", "b", "a"]);
        o.key = SortKey::Size;
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["b", "c", "a"]);
        let t = SystemTime::UNIX_EPOCH;
        v[0].modified = Some(t + Duration::from_secs(1));
        v[1].modified = Some(t + Duration::from_secs(3));
        v[2].modified = Some(t + Duration::from_secs(2));
        o.key = SortKey::Modified;
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["c", "a", "b"]);
        v[0].kind = "B".into();
        v[1].kind = "A".into();
        v[2].kind = "C".into();
        o.key = SortKey::Kind;
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["b", "c", "a"]);
    }

    #[test]
    fn none_last_both_directions() {
        let mut v = vec![e("d", true, None), e("e", true, None), e("a", true, None)];
        v[0].is_dir = false;
        v[1].is_dir = false;
        v[2].is_dir = false;
        v[0].size = Some(2);
        v[1].size = None;
        v[2].size = Some(1);
        let mut o = SortOrder { key: SortKey::Size, ascending: true, folders_first: true };
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["a", "d", "e"]);
        o.ascending = false;
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["d", "a", "e"]);
    }

    #[test]
    fn stable_ties_by_name() {
        let mut v = vec![e("b", false, Some(1)), e("a", false, Some(1))];
        let o = SortOrder { key: SortKey::Size, ascending: true, folders_first: true };
        sort_entries(&mut v, o);
        assert_eq!(names(&v), ["a", "b"]);
    }
}
