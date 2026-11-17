//! Pane filter: case-insensitive substring match (not fuzzy).

/// True when `filter` (trimmed, lowercased) is empty or a substring of the lowercased `name`.
pub fn matches(name: &str, filter: &str) -> bool {
    let f = filter.trim().to_lowercase();
    f.is_empty() || name.to_lowercase().contains(&f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substring_case_insensitive() {
        assert!(matches("Report.PDF", "port"));
        assert!(matches("Report.PDF", "  PDF "));
        assert!(!matches("Report.PDF", "rpt"));
    }

    #[test]
    fn empty_matches_all() {
        assert!(matches("x", ""));
        assert!(matches("x", "   "));
    }
}
