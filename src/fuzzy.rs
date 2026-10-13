//! Shared fuzzy subsequence matching.
//!
//! One small matcher serves two callers that must agree on what "fuzzy" means:
//! the macOS terminal-emulator resolver (picking a `.app` from a typed name) and
//! the user-management window's search bar (filtering the user database).

/// Fuzzy match a query against a candidate: `query` must appear as a
/// case-insensitive subsequence of `candidate`. E.g. "iterm" matches
/// "iTerm.app", "wez" matches "WezTerm.app". Returns the match score
/// (0 = no match, higher = better), biased toward fewer skipped characters.
pub(crate) fn fuzzy_subsequence(query: &str, candidate: &str) -> usize {
    let q: Vec<char> = query
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let c: Vec<char> = candidate.chars().collect();
    if q.is_empty() {
        return 0;
    }
    let mut qi = 0;
    let mut skipped = 0usize;
    for ch in &c {
        if qi < q.len() && ch.eq_ignore_ascii_case(&q[qi]) {
            qi += 1;
        } else if qi > 0 {
            skipped += 1;
        }
    }
    if qi != q.len() {
        return 0;
    }
    // Fewer trailing skipped chars = a better match. Exact (case-insensitive)
    // prefix matches score highest.
    let exact_prefix = c
        .iter()
        .take(q.len())
        .zip(q.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b));
    if exact_prefix {
        1000 - skipped
    } else {
        500 - skipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_subsequence_matches_names_partially() {
        // Exact-ish matches score highest.
        assert!(
            fuzzy_subsequence("iterm", "iTerm") > fuzzy_subsequence("iterm", "iTermSomethingElse")
        );
        // Case-insensitive subsequence.
        assert_eq!(fuzzy_subsequence("iterm", "iTerm"), 1000);
        assert!(fuzzy_subsequence("wez", "WezTerm") > 0);
        assert!(fuzzy_subsequence("alacr", "Alacritty") > 0);
        // No match -> 0.
        assert_eq!(fuzzy_subsequence("zzz", "Terminal"), 0);
        // Empty query -> 0.
        assert_eq!(fuzzy_subsequence("", "Terminal"), 0);
    }
}
