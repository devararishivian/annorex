//! A thread that tells when the agent has a new reply. It looks at the stamp of the transcript every 2
//! seconds and reads the replies only when the stamp changed.

use std::time::SystemTime;

use annorex_hosts::roots::Roots;

use crate::replies::{self, SessionRef, Source};

/// The size and the time of a transcript. `OpenCode` writes to the `-wal` file first, so the stamp has both.
pub type Stamp = (u64, SystemTime);

/// How many looks lie between two searches for the session of Claude Code (a new session replaces the old one).
const RESOLVE_EVERY: usize = 3;

/// The stamp of the transcript of `session`, or `None` when it cannot be read.
pub fn stamp_of(session: &SessionRef) -> Option<Stamp> {
    match session {
        SessionRef::Claude(path) | SessionRef::Pi(path) => {
            let meta = std::fs::metadata(path).ok()?;
            Some((meta.len(), meta.modified().ok()?))
        }
        SessionRef::OpenCode { db, .. } => {
            let mut wal = db.as_os_str().to_owned();
            wal.push("-wal");
            let metas = [std::fs::metadata(db).ok(), std::fs::metadata(wal).ok()];
            let mut size = 0;
            let mut newest: Option<SystemTime> = None;
            for meta in metas.into_iter().flatten() {
                size += meta.len();
                let time = meta.modified().ok()?;
                newest = Some(newest.map_or(time, |old| old.max(time)));
            }
            newest.map(|time| (size, time))
        }
    }
}

/// The id of the newest reply in `session`, or `None` when there is none or it cannot be read.
pub fn newest_reply_id(session: &SessionRef) -> Option<String> {
    replies::load_replies(session).ok()?.into_iter().next().map(|reply| reply.id)
}

/// The look of the thread for a viewer. `fixed` is the session that the user chose (Pi and `OpenCode`):
/// it never changes. Without it, the look finds the newest session of the agent.
pub fn reply_looker(
    source: Source,
    roots: Roots,
    fixed: Option<SessionRef>,
) -> impl FnMut() -> Option<String> + Send + 'static {
    reply_looker_with(
        move || {
            if let Some(session) = &fixed {
                return Some(session.clone());
            }
            let table = replies::process_table().unwrap_or_default();
            crate::changes::resolve(&source, &roots, &table).ok()
        },
        stamp_of,
        newest_reply_id,
    )
}

/// A look that finds the session with `find` (again every [`RESOLVE_EVERY`] looks), reads the replies with
/// `read` only when `stamp` changed, and gives `Some(id)` of the newest reply when that id is new. A failure
/// gives `None`: the next look tries again.
pub fn reply_looker_with(
    mut find: impl FnMut() -> Option<SessionRef> + Send + 'static,
    mut stamp: impl FnMut(&SessionRef) -> Option<Stamp> + Send + 'static,
    mut read: impl FnMut(&SessionRef) -> Option<String> + Send + 'static,
) -> impl FnMut() -> Option<String> + Send + 'static {
    let mut session: Option<SessionRef> = None;
    let mut last_stamp: Option<Stamp> = None;
    let mut last_id: Option<String> = None;
    let mut looks = 0usize;
    move || {
        if session.is_none() || looks.is_multiple_of(RESOLVE_EVERY) {
            match find() {
                Some(fresh) if session.as_ref().map(SessionRef::id) != Some(fresh.id()) => {
                    session = Some(fresh);
                    last_stamp = None;
                    last_id = None;
                }
                Some(_) | None => {}
            }
        }
        looks += 1;
        let current = session.as_ref()?;
        let now = stamp(current)?;
        if last_stamp == Some(now) {
            return None;
        }
        last_stamp = Some(now);
        let id = read(current)?;
        if last_id.as_ref() == Some(&id) {
            return None;
        }
        last_id = Some(id.clone());
        Some(id)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    fn session(name: &str) -> SessionRef {
        SessionRef::Pi(PathBuf::from(format!("/{name}.jsonl")))
    }

    #[test]
    fn the_look_reads_only_when_the_stamp_changes() {
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reads);
        let tick = Arc::new(AtomicUsize::new(1));
        let seen = Arc::clone(&tick);
        let mut look = reply_looker_with(
            || Some(session("a")),
            move |_| {
                Some((1, SystemTime::UNIX_EPOCH + Duration::from_secs(seen.load(Ordering::SeqCst) as u64)))
            },
            move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                Some("r1".to_owned())
            },
        );
        assert_eq!(look(), Some("r1".to_owned()), "the first look gives the newest id");
        assert_eq!(look(), None);
        assert_eq!(look(), None);
        assert_eq!(reads.load(Ordering::SeqCst), 1, "the same stamp reads nothing");
        tick.store(2, Ordering::SeqCst);
        assert_eq!(look(), None, "a new stamp with the same newest reply gives nothing");
        assert_eq!(reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_new_newest_reply_is_reported_once() {
        let id = Arc::new(AtomicUsize::new(1));
        let tick = Arc::new(AtomicUsize::new(1));
        let (read_id, read_tick) = (Arc::clone(&id), Arc::clone(&tick));
        let mut look = reply_looker_with(
            || Some(session("a")),
            move |_| {
                Some((
                    1,
                    SystemTime::UNIX_EPOCH + Duration::from_secs(read_tick.load(Ordering::SeqCst) as u64),
                ))
            },
            move |_| Some(format!("r{}", read_id.load(Ordering::SeqCst))),
        );
        assert_eq!(look(), Some("r1".to_owned()));
        id.store(2, Ordering::SeqCst);
        tick.store(2, Ordering::SeqCst);
        assert_eq!(look(), Some("r2".to_owned()));
        assert_eq!(look(), None);
    }

    #[test]
    fn a_new_session_starts_again() {
        let which = Arc::new(AtomicUsize::new(0));
        let find_which = Arc::clone(&which);
        let mut look = reply_looker_with(
            move || Some(session(if find_which.load(Ordering::SeqCst) == 0 { "a" } else { "b" })),
            |_| Some((1, SystemTime::UNIX_EPOCH)),
            |_| Some("r1".to_owned()),
        );
        assert_eq!(look(), Some("r1".to_owned()));
        which.store(1, Ordering::SeqCst);
        // The session is searched again at look 4 (after RESOLVE_EVERY looks). The same id in a new session counts as new.
        assert_eq!(look(), None);
        assert_eq!(look(), None);
        assert_eq!(look(), Some("r1".to_owned()));
    }

    #[test]
    fn no_session_gives_nothing() {
        let mut look = reply_looker_with(|| None, |_| None, |_| None);
        assert_eq!(look(), None);
    }
}
