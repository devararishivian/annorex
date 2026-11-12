//! The files that an agent changed in its last turns, read from a transcript.
//!
//! Each host maps its transcript lines to [`Event`]s. [`collect`] keeps the calls that
//! succeeded, in the last turns, newest first. The result is a hint for ordering: a change
//! made by a shell command has no path in the transcript and is not listed.

use std::collections::HashSet;

/// A file that a successful tool call wrote or edited. The path is as the agent wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub path: String,
}

/// One fact of a transcript, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    /// A prompt of the human: a new turn starts.
    Prompt,
    /// A tool call that changes `path`.
    Call { id: String, path: String },
    /// The result of the call `id`.
    Result { id: String, ok: bool },
}

/// The paths of the calls that succeeded in the last `turns` turns, newest first, once each.
/// A call with no result, or a failed result, does not count. `turns` of 0 counts as 1.
pub(crate) fn collect(events: impl IntoIterator<Item = Event>, turns: usize) -> Vec<Changed> {
    let turns = turns.max(1);
    let mut prompts = 0usize;
    let mut calls: Vec<(usize, String, String)> = Vec::new();
    let mut ok: HashSet<String> = HashSet::new();
    for event in events {
        match event {
            Event::Prompt => prompts += 1,
            Event::Call { id, path } => calls.push((prompts, id, path)),
            Event::Result { id, ok: true } => {
                ok.insert(id);
            }
            Event::Result { .. } => {}
        }
    }
    let mut seen = HashSet::new();
    calls
        .into_iter()
        .rev()
        .filter(|(turn, id, _)| turn + turns > prompts && ok.contains(id))
        .filter(|(_, _, path)| seen.insert(path.clone()))
        .map(|(_, _, path)| Changed { path })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, path: &str) -> Event {
        Event::Call { id: id.to_owned(), path: path.to_owned() }
    }
    fn done(id: &str, ok: bool) -> Event {
        Event::Result { id: id.to_owned(), ok }
    }
    fn paths(changed: &[Changed]) -> Vec<&str> {
        changed.iter().map(|c| c.path.as_str()).collect()
    }

    #[test]
    fn keeps_a_call_with_a_good_result() {
        let got = collect([Event::Prompt, call("1", "a.rs"), done("1", true)], 1);
        assert_eq!(paths(&got), ["a.rs"]);
    }

    #[test]
    fn drops_a_failed_call_and_a_call_without_a_result() {
        let events = [Event::Prompt, call("1", "a.rs"), done("1", false), call("2", "b.rs")];
        assert_eq!(collect(events, 1), Vec::<Changed>::new());
    }

    #[test]
    fn limits_to_the_last_turns() {
        let events = || {
            [
                Event::Prompt,
                call("1", "old.rs"),
                done("1", true),
                Event::Prompt,
                call("2", "mid.rs"),
                done("2", true),
                Event::Prompt,
                call("3", "new.rs"),
                done("3", true),
            ]
        };
        assert_eq!(paths(&collect(events(), 1)), ["new.rs"]);
        assert_eq!(paths(&collect(events(), 2)), ["new.rs", "mid.rs"]);
        assert_eq!(paths(&collect(events(), 0)), ["new.rs"]);
    }

    #[test]
    fn lists_a_file_once_newest_first() {
        let events = [
            Event::Prompt,
            call("1", "a.rs"),
            done("1", true),
            call("2", "b.rs"),
            done("2", true),
            call("3", "a.rs"),
            done("3", true),
        ];
        assert_eq!(paths(&collect(events, 1)), ["a.rs", "b.rs"]);
    }

    #[test]
    fn a_transcript_with_no_prompt_is_one_turn() {
        assert_eq!(paths(&collect([call("1", "a.rs"), done("1", true)], 1)), ["a.rs"]);
    }
}
