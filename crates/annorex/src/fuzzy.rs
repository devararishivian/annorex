//! A small fuzzy find for paths: the query characters must appear in order.

/// The most results that `rank` gives.
pub const MAX_RESULTS: usize = 200;

const MATCH: i32 = 1;
const CONSECUTIVE: i32 = 10;
const WORD_START: i32 = 8;
const IN_NAME: i32 = 6;
const NONE: i32 = i32::MIN / 2;

/// True for a character after which a new word starts.
fn is_boundary(c: char) -> bool {
    matches!(c, '/' | '-' | '_' | '.' | ' ')
}

/// The best score of `query` in `item`, or `None` when the characters are not in order. The case
/// is ignored. A higher score for consecutive characters, for a match at the start of a word, and
/// for a match in the file name (the part after the last `/`).
fn score(query: &[char], item: &str) -> Option<i32> {
    let chars: Vec<char> = item.chars().flat_map(char::to_lowercase).collect();
    let name_start = chars.iter().rposition(|&c| c == '/').map_or(0, |at| at + 1);
    let bonus = |j: usize| {
        let mut bonus = MATCH;
        if j == 0 || chars.get(j - 1).copied().is_some_and(is_boundary) {
            bonus += WORD_START;
        }
        if j >= name_start {
            bonus += IN_NAME;
        }
        bonus
    };
    // best[j]: the best score when the current query character stands at position j.
    let mut best: Vec<i32> = vec![NONE; chars.len()];
    for (i, &wanted) in query.iter().enumerate() {
        let mut next = vec![NONE; chars.len()];
        let mut before = NONE; // the best of the earlier row at positions up to j - 2
        for j in 0..chars.len() {
            if chars.get(j) == Some(&wanted) {
                let from_row = if i == 0 {
                    0
                } else {
                    let consecutive = j.checked_sub(1).and_then(|k| best.get(k)).copied().unwrap_or(NONE);
                    let consecutive = if consecutive > NONE { consecutive + CONSECUTIVE } else { NONE };
                    before.max(consecutive)
                };
                if let Some(cell) = next.get_mut(j).filter(|_| from_row > NONE) {
                    *cell = from_row + bonus(j);
                }
            }
            // Position j - 1 joins the "up to j - 1" part for the next position.
            if let Some(&previous) = j.checked_sub(1).and_then(|k| best.get(k)) {
                before = before.max(previous);
            }
        }
        best = next;
    }
    best.into_iter().max().filter(|&score| score > NONE)
}

/// The items that match `query`, best first, as index and item. Items with the same score: the
/// shorter first, then the earlier one. An empty query gives the items in their order. At most
/// [`MAX_RESULTS`] results.
pub fn rank<'a>(query: &str, items: &'a [String]) -> Vec<(usize, &'a str)> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return items
            .iter()
            .enumerate()
            .take(MAX_RESULTS)
            .map(|(index, item)| (index, item.as_str()))
            .collect();
    }
    let mut found: Vec<(i32, usize, &str)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| score(&query, item).map(|score| (score, index, item.as_str())))
        .collect();
    found.sort_by(|a, b| {
        b.0.cmp(&a.0).then(a.2.chars().count().cmp(&b.2.chars().count())).then(a.1.cmp(&b.1))
    });
    found.into_iter().take(MAX_RESULTS).map(|(_, index, item)| (index, item)).collect()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    fn items(list: &[&str]) -> Vec<String> {
        list.iter().map(|item| (*item).to_owned()).collect()
    }

    fn names<'a>(ranked: &[(usize, &'a str)]) -> Vec<&'a str> {
        ranked.iter().map(|&(_, item)| item).collect()
    }

    #[test]
    fn fuzzy_orders_by_score() {
        let list = items(&["README.md", "docs/domain.md", "src/main.rs"]);
        assert_eq!(names(&rank("main", &list)), ["src/main.rs", "docs/domain.md"]);
        assert_eq!(names(&rank("MAIN", &list)), ["src/main.rs", "docs/domain.md"], "the case is ignored");
        assert_eq!(rank("main", &list)[0].0, 2, "the index is the index in the list");
        assert_eq!(rank("zzz", &list), Vec::<(usize, &str)>::new());
        assert!(rank("nai", &list).iter().all(|&(_, item)| item != "README.md"), "order matters");
    }

    #[test]
    fn a_match_in_the_file_name_beats_one_in_a_directory() {
        let list = items(&["a/b/xfooy.txt", "foo/bar.txt", "bar/foo.txt"]);
        assert_eq!(names(&rank("foo", &list)), ["bar/foo.txt", "a/b/xfooy.txt", "foo/bar.txt"]);
    }

    #[test]
    fn empty_query_keeps_the_order_and_ties_are_stable() {
        let list = items(&["b.txt", "a.txt", "c.txt"]);
        assert_eq!(names(&rank("", &list)), ["b.txt", "a.txt", "c.txt"]);
        let tie = items(&["x/a1.txt", "y/a2.txt"]);
        assert_eq!(names(&rank("a", &tie)), ["x/a1.txt", "y/a2.txt"], "same score and length: earlier first");
    }

    #[test]
    fn results_are_limited() {
        let list: Vec<String> = (0..500).map(|n| format!("file{n}.txt")).collect();
        assert_eq!(rank("file", &list).len(), MAX_RESULTS);
        assert_eq!(rank("", &list).len(), MAX_RESULTS);
    }
}
