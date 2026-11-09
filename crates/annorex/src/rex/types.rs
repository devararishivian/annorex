//! JSON types of the Rex block methods `process`, `program_status`, and `modes`.
//!
//! The fields follow the method schemas of `rex block inspect --json`. Unknown fields are
//! ignored, so a newer Rex that adds fields still parses.

use std::path::PathBuf;

use serde::Deserialize;

/// The result of the `process` method.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProcessInfo {
    /// The process group in the foreground of the terminal.
    pub foreground: Option<Proc>,
    /// The process that the terminal started.
    pub child: Option<Proc>,
}

/// One process in a `process` result.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Proc {
    pub pid: u32,
    /// The name of the executable. Do not use it to identify an agent: Pi runs as `node`.
    pub name: String,
    pub argv0: Option<String>,
    pub cwd: Option<PathBuf>,
    pub path: Option<PathBuf>,
}

/// The result of the `program_status` method.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusList {
    pub records: Vec<StatusRecord>,
}

impl StatusList {
    /// The root record: the record without an `id`.
    pub fn root(&self) -> Option<&StatusRecord> {
        self.records.iter().find(|record| record.id.is_none())
    }
}

/// One status record that a program reported.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusRecord {
    pub id: Option<String>,
    pub state: String,
    pub kind: Option<String>,
    pub app: Option<String>,
    pub msg: Option<String>,
    pub updated_at: String,
    pub owner: Option<Owner>,
}

/// The process that owns a status record.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Owner {
    pub pid: u32,
    pub name: String,
}

/// The result of the `modes` method.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Modes {
    pub screen: String,
    pub modes: Option<Vec<Mode>>,
}

/// One terminal mode.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Mode {
    pub name: String,
    pub number: u32,
    /// `Some(true)` for an ANSI mode. A DEC private mode has no `ansi` field.
    pub ansi: Option<bool>,
    pub set: bool,
}

/// The DEC private mode number of bracketed paste.
const BRACKETED_PASTE: u32 = 2004;

impl Modes {
    /// Is the DEC private mode 2004 (bracketed paste) set?
    pub fn bracketed_paste(&self) -> bool {
        self.modes.iter().flatten().any(|mode| {
            mode.name == "bracketed_paste"
                && mode.number == BRACKETED_PASTE
                && mode.ansi != Some(true)
                && mode.set
        })
    }
}
