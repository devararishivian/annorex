//! Calls of the `rex` CLI.

use std::ffi::OsString;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;

use super::types::{LayerState, Modes, NewLayer, ProcessInfo, SessionList, SessionView, Size, StatusList};

/// Runs one command. Tests replace it with a fake.
pub trait Runner {
    fn run(&self, argv: &[OsString], stdin: Option<&[u8]>) -> std::io::Result<Output>;
}

/// Runs the command as a child process.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunner;

impl<R: Runner + ?Sized> Runner for &R {
    fn run(&self, argv: &[OsString], stdin: Option<&[u8]>) -> std::io::Result<Output> {
        (**self).run(argv, stdin)
    }
}

/// How long one Rex call can take. A call that takes longer fails with [`RexError::Timeout`].
pub(crate) const REX_TIMEOUT: Duration = Duration::from_secs(10);

impl SystemRunner {
    /// Run the command. Kill it and give an error of kind `TimedOut` when it runs longer than
    /// `timeout`.
    pub fn run_with_timeout(
        &self,
        argv: &[OsString],
        stdin: Option<&[u8]>,
        timeout: Duration,
    ) -> std::io::Result<Output> {
        let Some((program, rest)) = argv.split_first() else {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty command"));
        };
        let mut child = Command::new(program)
            .args(rest)
            .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Each pipe has its own thread, so a child that writes much output before it reads all of
        // its input cannot block both sides, and the wait below can time out.
        let writer = match (stdin, child.stdin.take()) {
            (Some(data), Some(mut pipe)) => {
                let data = data.to_vec();
                Some(std::thread::spawn(move || pipe.write_all(&data)))
            }
            _ => None,
        };
        let stdout = child.stdout.take().map(read_all);
        let stderr = child.stderr.take().map(read_all);
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the command timed out"));
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let collect = |handle: Option<std::thread::JoinHandle<Vec<u8>>>| {
            handle.map(|handle| handle.join().unwrap_or_default()).unwrap_or_default()
        };
        let output = Output { status, stdout: collect(stdout), stderr: collect(stderr) };
        if let Some(writer) = writer {
            let written = writer.join().map_err(|_| std::io::Error::other("stdin writer thread failed"))?;
            // A child that fails can exit before it reads all of its input. Then its exit status and
            // its stderr explain the failure better than the broken pipe.
            match written {
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe && !output.status.success() => {}
                other => other?,
            }
        }
        Ok(output)
    }
}

/// Read a pipe to its end on its own thread.
fn read_all(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = pipe.read_to_end(&mut data);
        data
    })
}

impl Runner for SystemRunner {
    fn run(&self, argv: &[OsString], stdin: Option<&[u8]>) -> std::io::Result<Output> {
        self.run_with_timeout(argv, stdin, REX_TIMEOUT)
    }
}

/// An error from a Rex call.
#[derive(Debug, thiserror::Error)]
pub enum RexError {
    #[error("the rex binary was not found; tried: {}", display_paths(.tried))]
    BinaryNotFound { tried: Vec<PathBuf> },
    #[error("no Rex session: give --session or run inside Rex (REX_SESSION)")]
    NoSession,
    #[error("{} {method} failed (exit {}): {stderr}", call_of(method), code.map_or_else(|| "signal".to_owned(), |c| c.to_string()))]
    Failed { method: String, code: Option<i32>, stderr: String },
    #[error("{} {method} gave JSON that Annorex cannot read: {source}", call_of(method))]
    BadJson { method: String, source: serde_json::Error },
    #[error("{} {method} did not answer in {} s", call_of(method), REX_TIMEOUT.as_secs())]
    Timeout { method: String },
    #[error("block {block} has no window in the session")]
    NotInWindow { block: String },
    #[error("cannot run rex: {0}")]
    Io(#[from] std::io::Error),
}

/// The `rex` command of a method. API methods have a dot (`session.view`). Block methods do not
/// (`process`).
fn call_of(method: &str) -> &'static str {
    if method.contains('.') { "rex api call" } else { "rex block call" }
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
        self.run_json(&command, method, args)
    }

    /// Call an API method of the session, for example `session.view`. `args` goes on standard
    /// input.
    pub fn api_call(&self, method: &str, args: &serde_json::Value) -> Result<serde_json::Value, RexError> {
        let command: Vec<OsString> = [
            self.bin.as_os_str(),
            "api".as_ref(),
            "call".as_ref(),
            "-s".as_ref(),
            self.session.as_ref(),
            method.as_ref(),
            "-".as_ref(),
        ]
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
        self.run_json(&command, method, args)
    }

    fn run_json(
        &self,
        command: &[OsString],
        method: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, RexError> {
        let input = serde_json::to_vec(args)
            .map_err(|source| RexError::BadJson { method: method.to_owned(), source })?;
        let output = self.runner.run(command, Some(&input)).map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                RexError::Timeout { method: method.to_owned() }
            } else {
                RexError::Io(error)
            }
        })?;
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

    pub fn size(&self, block: &str) -> Result<Size, RexError> {
        self.typed(block, "size")
    }

    fn api_typed<T: DeserializeOwned>(&self, method: &str, args: &serde_json::Value) -> Result<T, RexError> {
        let value = self.api_call(method, args)?;
        serde_json::from_value(value)
            .map_err(|source| RexError::BadJson { method: method.to_owned(), source })
    }

    /// The windows and blocks of the session.
    pub fn view(&self) -> Result<SessionView, RexError> {
        self.api_typed("session.view", &serde_json::json!({}))
    }

    /// The ids of all sessions on the server.
    pub fn sessions(&self) -> Result<Vec<String>, RexError> {
        self.api_typed::<SessionList>("session.list", &serde_json::json!({})).map(|list| list.ids())
    }

    /// The same binary and runner for another session.
    pub fn with_session(&self, session: &str) -> Rex<&R> {
        Rex { bin: self.bin.clone(), session: session.to_owned(), runner: &self.runner }
    }

    /// The id of the window that holds `block`, from `session.describe_block`.
    pub fn window_of(&self, block: &str) -> Result<String, RexError> {
        #[derive(serde::Deserialize)]
        struct Described {
            block: DescribedBlock,
        }
        #[derive(serde::Deserialize)]
        struct DescribedBlock {
            #[serde(default)]
            window_id: Option<String>,
        }
        let described: Described =
            self.api_typed("session.describe_block", &serde_json::json!({ "block_id": block }))?;
        described.block.window_id.ok_or_else(|| RexError::NotInWindow { block: block.to_owned() })
    }

    /// Create a floating layer with `session.new_layer`. `request` holds `window_id`, `bounds`, `focus`,
    /// and `layout`.
    pub fn new_layer(&self, request: &serde_json::Value) -> Result<NewLayer, RexError> {
        self.api_typed("session.new_layer", request)
    }

    /// Minimize a layer or show it again. With `focus`, Rex also raises it and focuses its block.
    pub fn set_layer_state(&self, layer: &str, state: LayerState, focus: bool) -> Result<(), RexError> {
        let args = serde_json::json!({ "layer_id": layer, "state": state, "focus": focus });
        self.api_call("session.set_layer_state", &args).map(drop)
    }

    /// Close a floating layer and every block in it.
    pub fn close_layer(&self, layer: &str) -> Result<(), RexError> {
        self.api_call("session.close_layer", &serde_json::json!({ "layer_id": layer })).map(drop)
    }

    pub fn focus_block(&self, block: &str) -> Result<(), RexError> {
        self.api_call("session.focus_block", &serde_json::json!({ "block_id": block })).map(drop)
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
    use crate::rex::types::LayerState;

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

    /// A runner that always times out.
    struct TimeoutRunner;

    impl Runner for TimeoutRunner {
        fn run(&self, _argv: &[OsString], _stdin: Option<&[u8]>) -> std::io::Result<Output> {
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"))
        }
    }

    #[test]
    fn hung_rex_call_times_out() {
        let dir = tempfile::tempdir().expect("dir");
        let pid_file = dir.path().join("pid");
        let script = format!("echo $$ > {}; exec sleep 5", pid_file.display());
        let argv: Vec<OsString> = ["sh", "-c", script.as_str()].map(OsString::from).to_vec();
        let start = Instant::now();
        let result = SystemRunner.run_with_timeout(&argv, None, Duration::from_millis(300));
        assert!(start.elapsed() < Duration::from_secs(2), "{:?}", start.elapsed());
        assert_eq!(result.expect_err("timeout").kind(), std::io::ErrorKind::TimedOut);
        let pid = std::fs::read_to_string(&pid_file).expect("pid file");
        let alive = Command::new("kill").args(["-0", pid.trim()]).output().expect("kill");
        assert!(!alive.status.success(), "the child {} is still alive", pid.trim());
    }

    #[test]
    fn a_timed_out_call_is_a_rex_timeout() {
        let rex = Rex::new(PathBuf::from("/opt/rex/bin/rex"), "session:s1".to_owned(), TimeoutRunner);
        let error = rex.view().expect_err("timeout");
        assert!(matches!(error, RexError::Timeout { .. }), "{error:?}");
        assert_eq!(error.to_string(), "rex api call session.view did not answer in 10 s");
        let error = rex.process("block:b1").expect_err("timeout");
        assert_eq!(error.to_string(), "rex block call process did not answer in 10 s");
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

    const DESCRIBE_BLOCK: &str = include_str!("../../tests/fixtures/rex/describe-block.json");
    const NEW_LAYER_REPLY: &str = include_str!("../../tests/fixtures/rex/new-layer-reply.json");

    #[test]
    fn window_of_reads_the_window_id_of_describe_block() {
        let rex = rex(FakeRunner { stdout: DESCRIBE_BLOCK.as_bytes().to_vec(), ..FakeRunner::default() });
        let window = rex.window_of("block:b1").expect("window");
        assert!(window.starts_with("window:"), "{window}");
        let calls = rex.runner.calls.borrow();
        assert_eq!(
            strings(&calls[0].0),
            ["/opt/rex/bin/rex", "api", "call", "-s", "session:s1", "session.describe_block", "-"]
        );
        let stdin: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(stdin, serde_json::json!({ "block_id": "block:b1" }));
    }

    #[test]
    fn window_of_fails_for_a_block_that_is_in_no_window() {
        let rex = rex(FakeRunner {
            stdout: br#"{"block":{"block_id":"block:b1","placement":"detached"}}"#.to_vec(),
            ..FakeRunner::default()
        });
        let error = rex.window_of("block:b1").expect_err("no window");
        assert!(matches!(error, RexError::NotInWindow { .. }), "{error:?}");
        assert_eq!(error.to_string(), "block block:b1 has no window in the session");
    }

    #[test]
    fn new_layer_sends_the_request_and_reads_the_layer_and_block_ids() {
        let rex = rex(FakeRunner { stdout: NEW_LAYER_REPLY.as_bytes().to_vec(), ..FakeRunner::default() });
        let request = serde_json::json!({ "window_id": "window:w1", "bounds": [0, 0, 1, 1], "focus": true });
        let layer = rex.new_layer(&request).expect("layer");
        assert!(layer.layer_id.starts_with("layer:"));
        assert_eq!(layer.block_ids.len(), 1);
        let calls = rex.runner.calls.borrow();
        assert_eq!(strings(&calls[0].0)[5], "session.new_layer");
        let stdin: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(stdin, request);
    }

    #[test]
    fn layer_state_and_close_send_the_documented_fields() {
        let rex = rex(FakeRunner { stdout: b"{\"revision\":2}".to_vec(), ..FakeRunner::default() });
        rex.set_layer_state("layer:l1", LayerState::Normal, true).expect("state");
        rex.set_layer_state("layer:l1", LayerState::Minimized, false).expect("state");
        rex.close_layer("layer:l1").expect("close");
        let calls = rex.runner.calls.borrow();
        let bodies: Vec<serde_json::Value> = calls
            .iter()
            .map(|call| serde_json::from_slice(call.1.as_deref().expect("stdin")).expect("json"))
            .collect();
        assert_eq!(strings(&calls[0].0)[5], "session.set_layer_state");
        assert_eq!(
            bodies[0],
            serde_json::json!({ "layer_id": "layer:l1", "state": "normal", "focus": true })
        );
        assert_eq!(
            bodies[1],
            serde_json::json!({ "layer_id": "layer:l1", "state": "minimized", "focus": false })
        );
        assert_eq!(strings(&calls[2].0)[5], "session.close_layer");
        assert_eq!(bodies[2], serde_json::json!({ "layer_id": "layer:l1" }));
    }

    #[test]
    fn api_call_puts_args_on_stdin() {
        let rex = rex(FakeRunner { stdout: b"{\"windows\":[]}".to_vec(), ..FakeRunner::default() });
        let args = serde_json::json!({"a": 1});
        rex.api_call("session.view", &args).expect("call");
        let calls = rex.runner.calls.borrow();
        assert_eq!(
            strings(&calls[0].0),
            ["/opt/rex/bin/rex", "api", "call", "-s", "session:s1", "session.view", "-"]
        );
        let stdin: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(stdin, args);
    }

    #[test]
    fn api_failure_names_api_call() {
        let rex =
            rex(FakeRunner { code: 1, stderr: b"Error: not_found\n".to_vec(), ..FakeRunner::default() });
        let error = rex.api_call("session.view", &serde_json::json!({})).expect_err("fails");
        let text = error.to_string();
        assert!(text.contains("rex api call session.view failed"), "{text}");
        assert!(text.ends_with(": not_found"), "{text}");
        let error = rex.block_call("block:b1", "size", &serde_json::json!({})).expect_err("fails");
        assert!(error.to_string().contains("rex block call size failed"), "{error}");
    }

    #[test]
    fn with_session_changes_only_the_session() {
        let rex = rex(FakeRunner { stdout: b"{}".to_vec(), ..FakeRunner::default() });
        let other = rex.with_session("session:s2");
        assert_eq!(other.session(), "session:s2");
        assert_eq!(other.bin, rex.bin);
        other.api_call("session.view", &serde_json::json!({})).expect("call");
        let calls = rex.runner.calls.borrow();
        assert_eq!(strings(&calls[0].0)[4], "session:s2");
        assert_eq!(rex.session(), "session:s1");
    }

    #[test]
    fn focus_sends_the_block() {
        let rex = rex(FakeRunner {
            stdout: br#"{"revision":1,"block_id":"block:z","zoomed":true}"#.to_vec(),
            ..FakeRunner::default()
        });
        rex.focus_block("block:f").expect("focus");
        let calls = rex.runner.calls.borrow();
        assert_eq!(strings(&calls[0].0)[5], "session.focus_block");
        let focus: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(focus, serde_json::json!({"block_id": "block:f"}));
    }

    #[test]
    fn size_calls_the_block_method() {
        let rex =
            rex(FakeRunner { stdout: br#"{"columns":80,"rows":24}"#.to_vec(), ..FakeRunner::default() });
        let size = rex.size("block:b1").expect("size");
        assert_eq!((size.columns, size.rows), (80, 24));
        assert_eq!(strings(&rex.runner.calls.borrow()[0].0)[7], "size");
    }

    #[test]
    fn broken_pipe_reports_the_rex_error() {
        let data = vec![b'x'; 1 << 20];
        let argv = [OsString::from("/bin/sh"), OsString::from("-c"), OsString::from("echo nope >&2; exit 3")];
        let output = SystemRunner.run(&argv, Some(&data)).expect("the exit status wins over the broken pipe");
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "nope");
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
