//! Fuzzy matching for the command palette: the query's characters, in order, anywhere in the
//! candidate. Whitespace in the query is ignored, so "cls tab" matches "Close tab".

const MATCH: i32 = 1;
const WORD_START: i32 = 8;
const CONSECUTIVE: i32 = 5;
/// Per character skipped between two matches: tighter matches win, and a far word start only wins
/// when its bonus outweighs the skip (without it "usr" ranks `user_settings_archive` over `users`).
const GAP: i32 = 1;

/// The best-scoring way `query` is a case-insensitive subsequence of `candidate`, and the byte
/// offsets of the matched characters for highlighting; `None` when it isn't one.
pub fn score(query: &str, candidate: &str) -> Option<(i32, Vec<usize>)> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let chars: Vec<(usize, char)> = candidate.char_indices().collect();
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    // best[i][j]: the best score with query[..=i] matched and query[i] at chars[j], and where
    // query[i - 1] went.
    let mut best: Vec<Vec<Option<(i32, usize)>>> = vec![vec![None; chars.len()]; query.len()];
    let lower: Vec<Option<char>> = chars.iter().map(|&(_, c)| single_lower(c)).collect();
    let starts: Vec<bool> = (0..chars.len()).map(|j| word_start(&chars, j)).collect();
    for (i, q) in query.iter().enumerate() {
        // The best `score + GAP * k` over k <= j - 2, so a gapped jump costs O(1) per `j`.
        let mut far: Option<(i32, usize)> = None;
        for j in 0..chars.len() {
            if i > 0
                && j >= 2
                && let Some((score, _)) = best[i - 1][j - 2]
            {
                let k = j - 2;
                let lifted = score + GAP * k as i32;
                // `>=`: ties go to the nearest.
                if far.is_none_or(|(f, _)| lifted >= f) {
                    far = Some((lifted, k));
                }
            }
            if lower[j] != Some(*q) {
                continue;
            }
            let own = MATCH + if starts[j] { WORD_START } else { 0 };
            best[i][j] = if i == 0 {
                Some((own, 0))
            } else {
                let next_to = j
                    .checked_sub(1)
                    .and_then(|k| best[i - 1][k].map(|(score, _)| (score + CONSECUTIVE, k)));
                let gapped = far.map(|(f, k)| (f - GAP * (j as i32 - 1), k));
                next_to.max(gapped).map(|(prev, k)| (prev + own, k))
            };
        }
    }
    let last = query.len() - 1;
    let (end, &(total, _)) = best[last]
        .iter()
        .enumerate()
        .filter_map(|(j, s)| s.as_ref().map(|s| (j, s)))
        .max_by_key(|(j, s)| (s.0, std::cmp::Reverse(*j)))?;
    let mut at = vec![0; query.len()];
    at[last] = end;
    for i in (1..query.len()).rev() {
        at[i - 1] = best[i][at[i]].expect("on the best path").1;
    }
    Some((total, at.into_iter().map(|j| chars[j].0).collect()))
}

fn word_start(chars: &[(usize, char)], j: usize) -> bool {
    let Some(&(_, prev)) = j.checked_sub(1).and_then(|p| chars.get(p)) else {
        return true;
    };
    let c = chars[j].1;
    !prev.is_alphanumeric() || (prev.is_lowercase() && c.is_uppercase())
}

/// `c` lowercased when that is a single character, the only way it can match a query character.
fn single_lower(c: char) -> Option<char> {
    let mut lower = c.to_lowercase();
    lower.next().filter(|_| lower.next().is_none())
}

#[cfg(test)]
mod tests {
    use super::score;

    fn s(query: &str, candidate: &str) -> i32 {
        score(query, candidate).map(|(s, _)| s).unwrap_or(i32::MIN)
    }

    #[test]
    fn ranks_word_starts_and_consecutive_runs_higher_and_rejects_non_subsequences() {
        // (query, better, worse)
        for (query, better, worse) in [
            ("ct", "Close tab", "Connect"),
            ("tab", "Close tab", "Stable"),
            ("nt", "Next tab", "Paint"),
            ("cls", "Close", "Columns"),
            ("rt", "Reopen tab", "Restart"),
            ("ab", "axb", "axxxxb"),
        ] {
            assert!(
                s(query, better) > s(query, worse),
                "{query:?}: {better:?} ({}) should beat {worse:?} ({})",
                s(query, better),
                s(query, worse)
            );
        }
        for (query, candidate) in [
            ("xyz", "Close tab"),
            ("bat", "Close tab"),
            ("tabs", "Close tab"),
        ] {
            assert_eq!(score(query, candidate), None, "{query:?} in {candidate:?}");
        }
    }

    #[test]
    fn is_case_insensitive_and_returns_the_matched_offsets() {
        assert_eq!(score("CT", "close tab").map(|(_, at)| at), Some(vec![0, 6]));
        assert_eq!(
            score("tab", "Close tab").map(|(_, at)| at),
            Some(vec![6, 7, 8])
        );
        assert_eq!(score("", "Close tab"), Some((0, vec![])));
    }
}
