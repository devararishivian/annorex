// Derived from plannotator-tui (https://github.com/plannotator/plannotator-tui) @ 83b0cd9, Copyright (c) 2026 Michael Ramos, MIT License. Changes for Annorex: the header line; the database path is percent-encoded in the SQLite URI.
//! Read-only `SQLite` access shared by the store-backed hosts (Hermes, `OpenCode`).

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::HostError;

/// Open `db` read-only, never writing. A plain read-only open sees the live WAL; when the
/// shared-memory index cannot be opened, fall back to `immutable=1`, which reads the main
/// file only and may lag an active writer.
pub(crate) fn open_read_only(db: &Path, what: &str) -> Result<Connection, HostError> {
    if !db.is_file() {
        return Err(HostError::NoTranscript(format!("no {what} database at {}", db.display())));
    }
    let flags =
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI;
    let uri = |query: &str| format!("file:{}?{query}", uri_path(db));
    match Connection::open_with_flags(uri("mode=ro"), flags) {
        Ok(connection) if probe(&connection) => Ok(connection),
        _ => Connection::open_with_flags(uri("mode=ro&immutable=1"), flags).map_err(|e| sql_error(what, &e)),
    }
}

/// Percent-encode every byte of `path` outside the unreserved set and `/`, so `?`, `#`, and `%`
/// stay part of the file name in a `file:` URI.
fn uri_path(path: &Path) -> String {
    use std::fmt::Write as _;
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    let mut out = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// A read-only WAL open can succeed and still fail on first read when the `-shm` cannot be
/// mapped; probe once so the fallback happens before any query.
fn probe(connection: &Connection) -> bool {
    connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0)).is_ok()
}

pub(crate) fn sql_error(what: &str, err: &rusqlite::Error) -> HostError {
    HostError::NoTranscript(format!("reading the {what} database: {err}"))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use super::uri_path;

    #[test]
    fn uri_path_encodes_the_raw_bytes_of_a_non_utf8_path() {
        let path = Path::new(OsStr::from_bytes(b"/x/a\xFFb/opencode.db"));
        let uri = uri_path(path);
        assert!(uri.contains("%FF"), "{uri}");
        assert!(!uri.contains("%EF%BF%BD"), "{uri}");
    }
}
