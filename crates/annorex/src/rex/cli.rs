//! Calls of the `rex` CLI.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde::de::DeserializeOwned;

use super::types::{Modes, ProcessInfo, StatusList};

/// Runs one command. Tests replace it with a fake.
pub trait Runner {
    fn run(&self, argv: &[OsString], stdin: Option<&[u8]>) -> std::io::Result<Output>;
}

/// Runs the command as a child process.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, argv: &[OsString], stdin: Option<&[u8]>) -> std::io::Result<Output> {
        let Some((program, rest)) = argv.split_first() else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command"));
        };
        let mut child = Command::new(program)
            .args(rest)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Write standard input on its own thread, so a child that writes much output before it
        // reads all of its input cannot block both sides.
        let writer = match (stdin, child.stdin.take()) {
            (Some(data), Some(mut pipe)) => {
                let data = data.to_vec();
                Some(std::thread::spawn(move || pipe.write_all(&data)))
            }
            _ => None,
        };
        let output = child.wait_with_output()?;
        if let Some(writer) = writer {
            writer.join().map_err(|_| std::io::Error::other("stdin writer thread failed"))??;
        }
        Ok(output)
    }
}

/// An error from a Rex call.
#[derive(Debug, thiserror::Error)]
pub enum RexError {
    #[error("the rex binary was not found; tried: {}", display_paths(.tried))]
    BinaryNotFound { tried: Vec<PathBuf> },
    #[error("no Rex session: give --session or run inside Rex (REX_SESSION)")]
    NoSession,
    #[error("rex block call {method} failed (exit {}): {stderr}", code.map_or_else(|| "signal".to_owned(), |c| c.to_string()))]
    Failed { method: String, code: Option<i32>, stderr: String },
    #[error("rex block call {method} gave JSON that Annorex cannot read: {source}")]
    BadJson { method: String, source: serde_json::Error },
    #[error("cannot run rex: {0}")]
    Io(#[from] std::io::Error),
}

fn display_paths(paths: &[PathBuf]) -> String {
    paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
}

/// A connection to one Rex session through the `rex` CLI.
#[derive(Debug)]
pub struct Rex<R: Runner> {
    pub(crate) bin: PathBuf,
    pub(crate) session: String,
    pub(crate) runner: R,
}

impl Rex<SystemRunner> {
    /// Find the `rex` binary and the session. The binary is `$REX_BIN_DIR/rex` if that file
    /// exists, else `rex` in a `PATH` directory. The session is `session`, else `REX_SESSION`.
    pub fn discover_with(
        env: impl Fn(&str) -> Option<OsString>,
        session: Option<String>,
    ) -> Result<Self, RexError> {
        let session = session
            .filter(|s| !s.is_empty())
            .or_else(|| env("REX_SESSION").and_then(|s| s.into_string().ok()).filter(|s| !s.is_empty()))
            .ok_or(RexError::NoSession)?;
        let mut tried = Vec::new();
        let bin_dir = env("REX_BIN_DIR").filter(|dir| !dir.is_empty()).map(PathBuf::from);
        let path_dirs =
            env("PATH").map(|path| std::env::split_paths(&path).collect::<Vec<_>>()).unwrap_or_default();
        for dir in bin_dir.into_iter().chain(path_dirs.into_iter().filter(|dir| !dir.as_os_str().is_empty()))
        {
            let candidate = dir.join("rex");
            if candidate.is_file() {
                return Ok(Self::new(candidate, session, SystemRunner));
            }
            tried.push(candidate);
        }
        Err(RexError::BinaryNotFound { tried })
    }
}

impl<R: Runner> Rex<R> {
    pub fn new(bin: PathBuf, session: String, runner: R) -> Self {
        Self { bin, session, runner }
    }

    /// The session that every block call names with `-s`.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// Call a block method. `args` goes on standard input.
    pub fn block_call(
        &self,
        block: &str,
        method: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, RexError> {
        let command: Vec<OsString> = [
            self.bin.as_os_str(),
            "block".as_ref(),
            "call".as_ref(),
            "-s".as_ref(),
            self.session.as_ref(),
            "-b".as_ref(),
            block.as_ref(),
            method.as_ref(),
            "-".as_ref(),
        ]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
        let input = serde_json::to_vec(args)
            .map_err(|source| RexError::BadJson { method: method.to_owned(), source })?;
        let output = self.runner.run(&command, Some(&input))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            return Err(RexError::Failed {
                method: method.to_owned(),
                code: output.status.code(),
                stderr: stderr.strip_prefix("Error: ").unwrap_or(stderr).to_owned(),
            });
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|source| RexError::BadJson { method: method.to_owned(), source })
    }

    fn typed<T: DeserializeOwned>(&self, block: &str, method: &str) -> Result<T, RexError> {
        let value = self.block_call(block, method, &serde_json::json!({}))?;
        serde_json::from_value(value)
            .map_err(|source| RexError::BadJson { method: method.to_owned(), source })
    }

    pub fn process(&self, block: &str) -> Result<ProcessInfo, RexError> {
        self.typed(block, "process")
    }

    pub fn program_status(&self, block: &str) -> Result<StatusList, RexError> {
        self.typed(block, "program_status")
    }

    pub fn modes(&self, block: &str) -> Result<Modes, RexError> {
        self.typed(block, "modes")
    }

    /// Write `data` to the terminal input of `block`, as if the user typed it.
    pub fn write(&self, block: &str, data: &str) -> Result<(), RexError> {
        self.block_call(block, "write", &serde_json::json!({ "data": data })).map(drop)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, reason = "tests assert by panicking")]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::os::unix::process::ExitStatusExt;
    use std::path::PathBuf;
    use std::process::{ExitStatus, Output};

    use super::*;

    /// One recorded call: argv and standard input.
    type Call = (Vec<OsString>, Option<Vec<u8>>);

    #[derive(Default)]
    struct FakeRunner {
        calls: RefCell<Vec<Call>>,
        code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    impl Runner for FakeRunner {
        fn run(&self, argv: &[OsString], stdin: Option<&[u8]>) -> std::io::Result<Output> {
            self.calls.borrow_mut().push((argv.to_vec(), stdin.map(<[u8]>::to_vec)));
            Ok(Output {
                status: ExitStatus::from_raw(self.code << 8),
                stdout: self.stdout.clone(),
                stderr: self.stderr.clone(),
            })
        }
    }

    fn rex(runner: FakeRunner) -> Rex<FakeRunner> {
        Rex::new(PathBuf::from("/opt/rex/bin/rex"), "session:s1".to_owned(), runner)
    }

    fn strings(argv: &[OsString]) -> Vec<String> {
        argv.iter().map(|arg| arg.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn block_call_puts_args_on_stdin() {
        let rex = rex(FakeRunner { stdout: b"{\"ok\":true}".to_vec(), ..FakeRunner::default() });
        let args = serde_json::json!({"x": 1});
        let value = rex.block_call("block:b1", "modes", &args).expect("call");
        assert_eq!(value, serde_json::json!({"ok": true}));
        let calls = rex.runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            strings(&calls[0].0),
            ["/opt/rex/bin/rex", "block", "call", "-s", "session:s1", "-b", "block:b1", "modes", "-"]
        );
        let stdin: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(stdin, args);
    }

    #[test]
    fn write_sends_payload_on_stdin_not_argv() {
        let rex = rex(FakeRunner { stdout: b"{\"bytes_written\":300000}".to_vec(), ..FakeRunner::default() });
        let payload = "x".repeat(300_000);
        rex.write("block:b1", &payload).expect("write");
        let calls = rex.runner.calls.borrow();
        let argv_bytes: usize = calls[0].0.iter().map(|arg| arg.len()).sum();
        assert!(argv_bytes < 200, "the payload must not be in argv");
        assert_eq!(strings(&calls[0].0)[7], "write");
        let stdin: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(stdin["data"].as_str().map(str::len), Some(300_000));
    }

    #[test]
    fn nonzero_exit_is_failed_with_stderr() {
        let rex = rex(FakeRunner {
            code: 1,
            stderr: b"Error: block target \"block:b1\" not found\n".to_vec(),
            ..FakeRunner::default()
        });
        match rex.block_call("block:b1", "process", &serde_json::json!({})) {
            Err(RexError::Failed { method, code, stderr }) => {
                assert_eq!(method, "process");
                assert_eq!(code, Some(1));
                assert_eq!(stderr, "block target \"block:b1\" not found");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn bad_json_is_reported_with_the_method() {
        let rex = rex(FakeRunner { stdout: b"not json".to_vec(), ..FakeRunner::default() });
        assert!(
            matches!(rex.process("block:b1"), Err(RexError::BadJson { method, .. }) if method == "process")
        );
    }

    #[test]
    fn discover_reports_every_path_tried() {
        let bin_dir = tempfile::tempdir().expect("temp dir");
        let path_dir = tempfile::tempdir().expect("temp dir");
        let env = |key: &str| match key {
            "REX_BIN_DIR" => Some(bin_dir.path().as_os_str().to_owned()),
            "PATH" => Some(path_dir.path().as_os_str().to_owned()),
            _ => None,
        };
        match Rex::discover_with(env, Some("session:s1".to_owned())) {
            Err(RexError::BinaryNotFound { tried }) => {
                assert_eq!(tried, [bin_dir.path().join("rex"), path_dir.path().join("rex")]);
            }
            other => panic!("expected BinaryNotFound, got {other:?}"),
        }
    }

    #[test]
    fn discover_prefers_rex_bin_dir() {
        let bin_dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(bin_dir.path().join("rex"), "").expect("rex file");
        let env = |key: &str| (key == "REX_BIN_DIR").then(|| bin_dir.path().as_os_str().to_owned());
        let rex = Rex::discover_with(env, Some("session:s1".to_owned())).expect("found");
        assert_eq!(rex.bin, bin_dir.path().join("rex"));
        assert_eq!(rex.session, "session:s1");
    }

    #[test]
    fn discover_takes_the_session_from_the_environment() {
        let bin_dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(bin_dir.path().join("rex"), "").expect("rex file");
        let env = |key: &str| match key {
            "REX_BIN_DIR" => Some(bin_dir.path().as_os_str().to_owned()),
            "REX_SESSION" => Some(OsString::from("session:env")),
            _ => None,
        };
        let rex = Rex::discover_with(env, None).expect("found");
        assert_eq!(rex.session, "session:env");
    }

    #[test]
    fn discover_without_session_is_no_session() {
        let env = |key: &str| (key == "REX_SESSION").then(|| OsString::from(""));
        assert!(matches!(Rex::discover_with(env, None), Err(RexError::NoSession)));
    }

    #[test]
    fn system_runner_passes_a_large_stdin_through() {
        let data = "line of text\n".repeat(25_000);
        let output =
            SystemRunner.run(&[OsString::from("/bin/cat")], Some(data.as_bytes())).expect("cat runs");
        assert!(output.status.success());
        assert_eq!(output.stdout, data.as_bytes());
    }
}
