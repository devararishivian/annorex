//! Calls of the `rex` CLI.

use std::ffi::OsString;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
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

/// How long [`Rex::reload_config_later`] waits before the reload. The action that started Annorex
/// must have ended; a user action that starts within about 1 s after an upgrade can be cancelled once.
pub(crate) const RELOAD_DELAY: Duration = Duration::from_secs(1);

/// How long one Rex call can take. A call that takes longer fails with [`RexError::Timeout`].
pub(crate) const REX_TIMEOUT: Duration = Duration::from_secs(10);

/// The `--timeout` that every block and API call passes to `rex`. It is shorter than
/// [`REX_TIMEOUT`], so `rex` reports its own timeout first. That error names the server, and it
/// comes before Annorex kills `rex`.
pub(crate) const REX_CALL_TIMEOUT: &str = "8s";

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

/// Find the `rex` binary. See [`find_binary_with`]. The parent is the process that started Annorex.
pub fn find_binary(env: impl Fn(&str) -> Option<OsString>) -> Result<PathBuf, RexError> {
    find_binary_with(env, || executable_of(std::os::unix::process::parent_id()))
}

/// Find the `rex` binary, in this order:
///
/// 1. `$REX_BIN_DIR/rex`. Rex sets `REX_BIN_DIR` in its terminals.
/// 2. The executable of the parent process, when its file name is `rex`. A command that a Rex action
///    starts with `rex.exec` has the server as its parent, and gets neither `REX_BIN_DIR` nor the
///    `PATH` of the login shell.
/// 3. `rex` in each `PATH` directory.
pub(crate) fn find_binary_with(
    env: impl Fn(&str) -> Option<OsString>,
    parent: impl FnOnce() -> Option<PathBuf>,
) -> Result<PathBuf, RexError> {
    let mut tried = Vec::new();
    if let Some(dir) = env("REX_BIN_DIR").filter(|dir| !dir.is_empty()) {
        let candidate = PathBuf::from(dir).join("rex");
        if candidate.is_file() {
            return Ok(candidate);
        }
        tried.push(candidate);
    }
    // Ask for the parent only now: it starts a process, and a Rex terminal does not need it.
    if let Some(candidate) = parent().filter(|exe| exe.file_name().is_some_and(|name| name == "rex")) {
        if candidate.is_file() {
            return Ok(candidate);
        }
        tried.push(candidate);
    }
    let path_dirs =
        env("PATH").map(|path| std::env::split_paths(&path).collect::<Vec<_>>()).unwrap_or_default();
    for dir in path_dirs.into_iter().filter(|dir| !dir.as_os_str().is_empty()) {
        let candidate = dir.join("rex");
        if candidate.is_file() {
            return Ok(candidate);
        }
        tried.push(candidate);
    }
    Err(RexError::BinaryNotFound { tried })
}

/// The path of the executable of process `pid`. `lsof` gives the text file of the process, which is
/// the real executable. `ps -o comm=` is the fallback: it gives `argv[0]`, which is a full path only
/// when the process was started by its full path.
fn executable_of(pid: u32) -> Option<PathBuf> {
    let pid = pid.to_string();
    let run = |program: &str, args: &[&str]| -> Option<String> {
        let output =
            Command::new(program).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        output.status.success().then(|| String::from_utf8(output.stdout).ok()).flatten()
    };
    if let Some(path) =
        run("/usr/sbin/lsof", &["-a", "-p", &pid, "-d", "txt", "-Fn"]).as_deref().and_then(lsof_path)
    {
        return Some(path);
    }
    let text = run("/bin/ps", &["-o", "comm=", "-p", &pid])?;
    let path = text.trim_end_matches(['\n', '\r']);
    path.starts_with('/').then(|| PathBuf::from(path))
}

/// The first `n` (name) line of `lsof -Fn` output, when it is an absolute path.
pub fn lsof_path(output: &str) -> Option<PathBuf> {
    let name = output.lines().find_map(|line| line.strip_prefix('n'))?;
    name.starts_with('/').then(|| PathBuf::from(name))
}

impl Rex<SystemRunner> {
    /// Find the `rex` binary and the session. The binary is found by [`find_binary`]. The session is
    /// `session`, else `REX_SESSION`.
    pub fn discover_with(
        env: impl Fn(&str) -> Option<OsString>,
        session: Option<String>,
    ) -> Result<Self, RexError> {
        let session = session
            .filter(|s| !s.is_empty())
            .or_else(|| env("REX_SESSION").and_then(|s| s.into_string().ok()).filter(|s| !s.is_empty()))
            .ok_or(RexError::NoSession)?;
        Ok(Self::new(find_binary(env)?, session, SystemRunner))
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
            "--timeout".as_ref(),
            REX_CALL_TIMEOUT.as_ref(),
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
            "--timeout".as_ref(),
            REX_CALL_TIMEOUT.as_ref(),
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

    /// Run `rex config reload` in a child that starts after `RELOAD_DELAY` and that Annorex does
    /// not wait for. Rex cancels an action when its config file reloads while the action runs, so
    /// the reload must come after the action that started Annorex ends. The child has its own
    /// process group, so the end of the action does not stop it. When the reload fails, the child
    /// creates the file `failed`, so the next open can tell the user.
    pub fn reload_config_later(&self, failed: &Path) -> std::io::Result<()> {
        use std::os::unix::process::CommandExt as _;
        // The paths are the arguments `$0` and `$1`, so the shell never reads them as code.
        let script = format!("sleep {}; \"$0\" config reload || : > \"$1\"", RELOAD_DELAY.as_secs());
        Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .arg(&self.bin)
            .arg(failed)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map(drop)
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
    use std::path::{Path, PathBuf};
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
            [
                "/opt/rex/bin/rex",
                "--timeout",
                "8s",
                "block",
                "call",
                "-s",
                "session:s1",
                "-b",
                "block:b1",
                "modes",
                "-"
            ]
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
        assert_eq!(strings(&calls[0].0)[9], "write");
        let stdin: serde_json::Value =
            serde_json::from_slice(calls[0].1.as_deref().expect("stdin")).expect("json");
        assert_eq!(stdin["data"].as_str().map(str::len), Some(300_000));
    }

    #[test]
    fn reload_config_later_runs_rex_config_reload_after_the_delay() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("dir");
        let log = dir.path().join("args");
        let bin = dir.path().join("rex");
        std::fs::write(&bin, format!("#!/bin/sh\necho \"$@\" > '{}'\n", log.display())).expect("script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode");
        let rex = Rex::new(bin, "session:s1".to_owned(), FakeRunner::default());
        let failed = dir.path().join("reload-failed");
        let start = Instant::now();
        rex.reload_config_later(&failed).expect("spawn");
        assert!(start.elapsed() < RELOAD_DELAY, "the call must not wait: {:?}", start.elapsed());
        assert!(!log.exists(), "the reload must wait for the delay");
        while !log.exists() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(start.elapsed() >= RELOAD_DELAY, "{:?}", start.elapsed());
        assert_eq!(std::fs::read_to_string(&log).expect("args"), "config reload\n");
        assert!(rex.runner.calls.borrow().is_empty(), "the reload does not use the runner");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!failed.exists(), "a reload that works writes no marker");
    }

    #[test]
    fn reload_config_later_marks_a_failed_reload() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().expect("dir");
        let bin = dir.path().join("rex");
        std::fs::write(&bin, "#!/bin/sh\nexit 1\n").expect("script");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("mode");
        let rex = Rex::new(bin, "session:s1".to_owned(), FakeRunner::default());
        // A path with a space and a quote must not break the script.
        let failed = dir.path().join("state 'x'/reload-failed");
        std::fs::create_dir_all(failed.parent().expect("parent")).expect("state");
        rex.reload_config_later(&failed).expect("spawn");
        let start = Instant::now();
        while !failed.exists() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(failed.exists(), "a failed reload writes the marker");
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
            [
                "/opt/rex/bin/rex",
                "--timeout",
                "8s",
                "api",
                "call",
                "-s",
                "session:s1",
                "session.describe_block",
                "-"
            ]
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
        assert_eq!(strings(&calls[0].0)[7], "session.new_layer");
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
        assert_eq!(strings(&calls[0].0)[7], "session.set_layer_state");
        assert_eq!(
            bodies[0],
            serde_json::json!({ "layer_id": "layer:l1", "state": "normal", "focus": true })
        );
        assert_eq!(
            bodies[1],
            serde_json::json!({ "layer_id": "layer:l1", "state": "minimized", "focus": false })
        );
        assert_eq!(strings(&calls[2].0)[7], "session.close_layer");
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
            ["/opt/rex/bin/rex", "--timeout", "8s", "api", "call", "-s", "session:s1", "session.view", "-"]
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
        assert_eq!(strings(&calls[0].0)[6], "session:s2");
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
        assert_eq!(strings(&calls[0].0)[7], "session.focus_block");
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
        assert_eq!(strings(&rex.runner.calls.borrow()[0].0)[9], "size");
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
    #[test]
    fn find_binary_prefers_rex_bin_dir_over_path() {
        let dir = std::env::temp_dir().join(format!("annorex-find-binary-{}", std::process::id()));
        let (first, second) = (dir.join("first"), dir.join("second"));
        std::fs::create_dir_all(&first).expect("dir");
        std::fs::create_dir_all(&second).expect("dir");
        std::fs::write(first.join("rex"), "").expect("file");
        std::fs::write(second.join("rex"), "").expect("file");
        let env = |name: &str| match name {
            "REX_BIN_DIR" => Some(first.clone().into_os_string()),
            "PATH" => Some(second.clone().into_os_string()),
            _ => None,
        };
        assert_eq!(find_binary(env).expect("binary"), first.join("rex"));
        let only_path = |name: &str| (name == "PATH").then(|| second.clone().into_os_string());
        assert_eq!(find_binary(only_path).expect("binary"), second.join("rex"));
        let none = |_: &str| None;
        assert!(matches!(find_binary_with(none, || None), Err(RexError::BinaryNotFound { .. })));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_binary_uses_the_parent_rex_after_rex_bin_dir_and_before_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (bin_dir, parent_dir, path_dir) =
            (dir.path().join("bin"), dir.path().join("Rex Server.app"), dir.path().join("path"));
        for sub in [&bin_dir, &parent_dir, &path_dir] {
            std::fs::create_dir_all(sub).expect("dir");
            std::fs::write(sub.join("rex"), "").expect("file");
        }
        let parent = || Some(parent_dir.join("rex"));
        let path_only = |name: &str| (name == "PATH").then(|| path_dir.clone().into_os_string());
        assert_eq!(find_binary_with(path_only, parent).expect("binary"), parent_dir.join("rex"));
        let both = |name: &str| match name {
            "REX_BIN_DIR" => Some(bin_dir.clone().into_os_string()),
            "PATH" => Some(path_dir.clone().into_os_string()),
            _ => None,
        };
        assert_eq!(find_binary_with(both, parent).expect("binary"), bin_dir.join("rex"));
    }

    #[test]
    fn find_binary_skips_a_parent_that_is_not_rex_or_is_missing() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("zsh"), "").expect("file");
        let none = |_: &str| None;
        let shell = dir.path().join("zsh");
        match find_binary_with(none, || Some(shell.clone())) {
            Err(RexError::BinaryNotFound { tried }) => assert!(tried.is_empty(), "{tried:?}"),
            other => panic!("expected BinaryNotFound, got {other:?}"),
        }
        let gone = dir.path().join("gone").join("rex");
        match find_binary_with(none, || Some(gone.clone())) {
            Err(RexError::BinaryNotFound { tried }) => assert_eq!(tried, std::slice::from_ref(&gone)),
            other => panic!("expected BinaryNotFound, got {other:?}"),
        }
    }

    #[test]
    fn lsof_path_is_the_first_name_line() {
        let output = "p2416\nftxt\nn/Apps/Rex Server.app/Contents/MacOS/rex\nftxt\nn/usr/lib/dyld\n";
        assert_eq!(lsof_path(output), Some(PathBuf::from("/Apps/Rex Server.app/Contents/MacOS/rex")));
        assert_eq!(lsof_path("p1\nftxt\n"), None);
        assert_eq!(lsof_path("p1\nftxt\nnrelative\n"), None);
    }

    #[test]
    fn find_binary_asks_for_the_parent_only_after_rex_bin_dir() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("rex"), "").expect("file");
        let env = |name: &str| (name == "REX_BIN_DIR").then(|| dir.path().as_os_str().to_owned());
        let found = find_binary_with(env, || panic!("the parent is not needed")).expect("binary");
        assert_eq!(found, dir.path().join("rex"));
    }

    #[test]
    fn executable_of_reads_the_comm_of_a_process() {
        // This test process is a real process; ps gives the path of its executable.
        let me = executable_of(std::process::id()).expect("ps gives a path");
        let exe = std::env::current_exe().expect("exe");
        let canonical = |path: &Path| std::fs::canonicalize(path).expect("canonical");
        assert_eq!(canonical(&me), canonical(&exe));
    }
}
