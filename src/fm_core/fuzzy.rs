//! Fuzzy subsequence scoring (port of the Swift FuzzyMatcher).

use unicode_segmentation::UnicodeSegmentation;

const NEG_INF: i64 = i64::MIN / 4;
const GAP: i64 = 1;
const CONSECUTIVE: i64 = 15;

fn lower(g: &str) -> String {
    g.to_lowercase()
}

fn is_boundary(c: char) -> bool {
    matches!(c, '/' | '-' | '_' | '.' | ' ') || !(c.is_alphabetic() || c.is_numeric())
}

fn char_score(c: &[&str], j: usize, first_query_char: bool) -> i64 {
    let mut s = 10;
    if j == 0 {
        s += 15;
        if first_query_char {
            s += 25;
        }
    } else {
        let p = c[j - 1].chars().next().unwrap_or(' ');
        let cur = c[j].chars().next().unwrap_or(' ');
        if is_boundary(p) {
            s += 15;
        } else if p.is_lowercase() && cur.is_uppercase() {
            s += 12;
        }
    }
    s
}

/// None = no match. Case-insensitive subsequence; higher is better. Bonuses: prefix,
/// word boundary (after / - _ . space or any non-alphanumeric, camelCase hump), consecutive
/// runs; shorter candidates win ties.
pub fn score(query: &str, candidate: &str) -> Option<i64> {
    let q: Vec<String> = query
        .graphemes(true)
        .filter(|g| !g.chars().all(char::is_whitespace))
        .map(lower)
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let c: Vec<&str> = candidate.graphemes(true).collect();
    let (n, m) = (c.len(), q.len());
    if m > n {
        return None;
    }
    let cl: Vec<String> = c.iter().map(|g| lower(g)).collect();
    let mut qi = 0;
    for ch in &cl {
        if qi < m && *ch == q[qi] {
            qi += 1;
        }
    }
    if qi < m {
        return None;
    }

    let mut prev = vec![NEG_INF; n];
    let mut cur = vec![NEG_INF; n];
    for i in 0..m {
        if i == 0 {
            for j in 0..n {
                cur[j] = if cl[j] == q[0] { char_score(&c, j, true) - j.min(5) as i64 } else { NEG_INF };
            }
        } else {
            let mut running = NEG_INF; // max over k <= j-2 of prev[k] + k*gap
            for j in 0..n {
                cur[j] = NEG_INF;
                if j >= 2 && prev[j - 2] > NEG_INF {
                    running = running.max(prev[j - 2] + (j as i64 - 2) * GAP);
                }
                if cl[j] != q[i] || j < i {
                    continue;
                }
                let mut best = NEG_INF;
                if j >= 1 && prev[j - 1] > NEG_INF {
                    best = prev[j - 1] + CONSECUTIVE;
                }
                if running > NEG_INF {
                    best = best.max(running - (j as i64 - 1) * GAP);
                }
                if best > NEG_INF {
                    cur[j] = best + char_score(&c, j, false);
                }
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let total = prev.iter().copied().max()?;
    if total <= NEG_INF {
        return None;
    }
    Some(total * 100 - n.min(99) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(q: &str, c: &str) -> Option<i64> {
        score(q, c)
    }

    #[test]
    fn no_match_is_none() {
        assert!(s("xyz", "Documents").is_none());
        assert!(s("doc", "cod").is_none());
        assert!(s("abcd", "abc").is_none());
        assert!(s("aa", "a").is_none());
    }

    #[test]
    fn non_ascii_and_case() {
        assert!(s("rsm", "R\u{e9}sum\u{e9}.txt").is_some());
        assert!(s("resume", "R\u{e9}sum\u{e9}.txt").is_none());
        assert!(s("\u{65e5}\u{672c}", "\u{65e5}\u{672c}\u{8a9e}.txt").is_some());
        assert!(s("ABC", "xaXbXc").is_some());
        assert!(s("abc", "ACB").is_none());
    }

    #[test]
    fn case_insensitive_subsequence() {
        assert!(s("DOC", "documents").is_some());
        assert!(s("dcs", "Documents").is_some());
        assert!(s("", "anything").is_some());
    }

    #[test]
    fn prefix_beats_mid_word() {
        assert!(s("doc", "Documents").unwrap() > s("doc", "mydocs").unwrap());
    }

    #[test]
    fn boundary_bonuses() {
        assert!(s("rp", "annual_report.pdf").unwrap() > s("rp", "carpet.txt").unwrap());
        assert!(s("fm", "FileManager").unwrap() > s("fm", "formal.txt").unwrap());
        assert!(s("nt", "my-notes").unwrap() > s("nt", "monument").unwrap());
    }

    #[test]
    fn consecutive_beats_gaps() {
        assert!(s("note", "a_notes").unwrap() > s("note", "a_nxoxtxe").unwrap());
    }

    #[test]
    fn shorter_wins_ties() {
        assert!(s("doc", "docs").unwrap() > s("doc", "docsandmore").unwrap());
        assert!(s("readme", "readme").unwrap() > s("readme", "readme.md").unwrap());
    }

    #[test]
    fn ranking_order() {
        let cands = ["Downloads", "Documents", "doc.txt", "my_doc_notes.txt", "undocumented", "d-o-c"];
        let mut ranked: Vec<(&str, i64)> = cands.iter().filter_map(|c| s("doc", c).map(|v| (*c, v))).collect();
        ranked.sort_by_key(|a| std::cmp::Reverse(a.1));
        let names: Vec<&str> = ranked.iter().map(|r| r.0).collect();
        assert_eq!(names[0], "doc.txt");
        let pos = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(pos("Documents") < pos("my_doc_notes.txt"));
        assert!(pos("my_doc_notes.txt") < pos("undocumented"));
        assert!(!names.contains(&"Downloads"));
    }
}
