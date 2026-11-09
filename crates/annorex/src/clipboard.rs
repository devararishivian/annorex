//! Copy the notes: an OSC 52 sequence for the terminal, and `pbcopy` for the Mac.

use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};

use crate::ui::App;

/// A place that takes text.
pub trait Clipboard {
    /// # Errors
    /// Gives a [`CopyError`] when a way to copy failed.
    fn copy(&mut self, text: &str) -> Result<(), CopyError>;
}

/// A failed copy. When the terminal got the OSC 52 sequence, the copy can still work through it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyError {
    pub message: String,
    pub terminal_sent: bool,
}

impl CopyError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self { message: message.into(), terminal_sent: false }
    }

    pub fn terminal_only(message: impl Into<String>) -> Self {
        Self { message: message.into(), terminal_sent: true }
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 with padding (RFC 4648 §4).
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|index| chunk.get(index).copied().unwrap_or(0));
        let value = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);
        for (index, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            let symbol =
                ALPHABET.get(((value >> shift) & 63) as usize).copied().filter(|_| index <= chunk.len());
            out.push(symbol.map_or('=', char::from));
        }
    }
    out
}

/// The sequence that asks the terminal to set the clipboard: `ESC ] 52 ; c ; <base64> BEL`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Writes OSC 52 to the terminal and runs `pbcopy`. Annorex cannot know if the terminal applied
/// the sequence, because there is no query for it.
#[derive(Debug)]
pub struct SystemClipboard<W: Write> {
    out: W,
    program: OsString,
    args: Vec<OsString>,
}

impl SystemClipboard<std::io::Stdout> {
    pub fn new() -> Self {
        Self::with(std::io::stdout(), "pbcopy", &[])
    }
}

impl Default for SystemClipboard<std::io::Stdout> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Write> SystemClipboard<W> {
    /// A clipboard with another output and another program, for tests.
    pub fn with(out: W, program: &str, args: &[&str]) -> Self {
        Self { out, program: program.into(), args: args.iter().map(OsString::from).collect() }
    }
}

impl<W: Write> Clipboard for SystemClipboard<W> {
    fn copy(&mut self, text: &str) -> Result<(), CopyError> {
        self.out
            .write_all(osc52(text).as_bytes())
            .and_then(|()| self.out.flush())
            .map_err(|error| CopyError::failed(format!("cannot write to the terminal: {error}")))?;
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                CopyError::terminal_only(format!("cannot run {}: {error}", self.program.to_string_lossy()))
            })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|error| CopyError::terminal_only(format!("cannot write to pbcopy: {error}")))?;
        }
        let status =
            child.wait().map_err(|error| CopyError::terminal_only(format!("pbcopy failed: {error}")))?;
        if status.success() {
            Ok(())
        } else {
            Err(CopyError::terminal_only(format!("pbcopy failed: {status}")))
        }
    }
}

/// Copy the notes of the reply on the screen. The footer tells the result.
pub fn copy_review(app: &mut App, clipboard: &mut impl Clipboard) {
    let count = app.annotation_count();
    let text = app.export_all();
    if text.is_empty() {
        app.set_status("Nothing to copy: no annotations yet.".to_owned());
        return;
    }
    let message = match clipboard.copy(&text) {
        Ok(()) => format!("Copied {count} annotation{}.", if count == 1 { "" } else { "s" }),
        Err(error) if error.terminal_sent => {
            format!("Copied through the terminal only ({}). The terminal may ignore it.", error.message)
        }
        Err(error) => format!("Copy failed: {}", error.message),
    };
    app.set_status(message);
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests assert by panicking")]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        let vectors = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (plain, encoded) in vectors {
            assert_eq!(base64(plain.as_bytes()), encoded, "{plain:?}");
        }
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn osc52_wraps_the_base64() {
        assert_eq!(osc52("foo"), "\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn copy_writes_osc52_and_runs_pbcopy() {
        let dir = tempfile::tempdir().expect("dir");
        let file = dir.path().join("clip.txt");
        let script = format!("cat > '{}'", file.display());
        let mut out = Vec::new();
        let mut clipboard = SystemClipboard::with(&mut out, "sh", &["-c", &script]);
        clipboard.copy("héllo\nworld").expect("copy");
        assert_eq!(String::from_utf8(out).expect("utf8"), osc52("héllo\nworld"));
        assert_eq!(std::fs::read_to_string(file).expect("file"), "héllo\nworld");
    }

    #[test]
    fn a_missing_program_is_an_error_but_osc52_was_written() {
        let mut out = Vec::new();
        let mut clipboard = SystemClipboard::with(&mut out, "/nonexistent/pbcopy", &[]);
        let error = clipboard.copy("x").expect_err("missing program");
        assert!(error.message.contains("pbcopy") && error.terminal_sent, "{error:?}");
        assert_eq!(String::from_utf8(out).expect("utf8"), osc52("x"));
    }

    #[test]
    fn a_failing_program_is_an_error() {
        let mut clipboard = SystemClipboard::with(Vec::new(), "sh", &["-c", "cat >/dev/null; exit 3"]);
        assert!(clipboard.copy("x").expect_err("exit 3").message.contains("failed"));
    }
}
