//! Fuzzy matching for the command palette: the query's characters, in order, anywhere in the
//! candidate. Whitespace in the query is ignored, so "cls tab" matches "Close tab".

const MATCH: i32 = 1;
const WORD_START: i32 = 8;
const CONSECUTIVE: i32 = 5;

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
    for (i, q) in query.iter().enumerate() {
        for (j, &(_, c)) in chars.iter().enumerate() {
            if !c.to_lowercase().eq(std::iter::once(*q)) {
                continue;
            }
            let own = MATCH + if word_start(&chars, j) { WORD_START } else { 0 };
            best[i][j] = if i == 0 {
                Some((own, 0))
            } else {
                best_before(&best[i - 1], j).map(|(prev, k)| (prev + own, k))
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

/// The best place left of `j` for the previous query character, with the consecutive bonus
/// when it is right next to `j`; ties go to the nearest.
fn best_before(row: &[Option<(i32, usize)>], j: usize) -> Option<(i32, usize)> {
    row[..j]
        .iter()
        .enumerate()
        .filter_map(|(k, s)| {
            let joined = if k + 1 == j { CONSECUTIVE } else { 0 };
            s.map(|(score, _)| (score + joined, k))
        })
        .max()
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
