//! The viewer must exit when its terminal closes. The event reader of crossterm can spin at 100% CPU
//! after the terminal input ends (crossterm issue 793). The `use-dev-tty` feature avoids it.
//!
//! The test needs a working `python3`. It skips when `python3` is missing or is the macOS stub that
//! asks for the command line tools. It also skips when the viewer ends before the close, because
//! then the close tests nothing.

#![allow(
    clippy::print_stderr,
    clippy::expect_used,
    reason = "the test reports a skip by printing and asserts by panicking"
)]

use std::process::{Command, Stdio};

/// The exit code of the script when the viewer ended before the close.
const ENDED_BEFORE_CLOSE: i32 = 3;

const SCRIPT: &str = r#"
import fcntl, os, pty, subprocess, sys, termios, time
master, slave = pty.openpty()
env = dict(os.environ, REX_SESSION="x")
proc = subprocess.Popen(
    [sys.argv[1], "files", "--target", "block:none", "--session", "x"],
    stdin=slave, stdout=slave, stderr=subprocess.DEVNULL, env=env, start_new_session=True,
    preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0))
os.close(slave)
time.sleep(1.5)
if proc.poll() is not None:
    sys.exit(3)
os.close(master)
try:
    proc.wait(3)
except subprocess.TimeoutExpired:
    proc.kill()
    sys.exit(1)
"#;

#[test]
fn the_viewer_exits_when_the_terminal_closes() {
    let python_works = Command::new("python3")
        .args(["-I", "-c", "import pty, termios"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !python_works {
        eprintln!("skipped: no working python3 on this machine");
        return;
    }
    let status = Command::new("python3")
        .args(["-I", "-c", SCRIPT, env!("CARGO_BIN_EXE_annorex")])
        .status()
        .expect("python3 runs");
    if status.code() == Some(ENDED_BEFORE_CLOSE) {
        eprintln!("skipped: the viewer ended before its terminal closed, so the close tested nothing");
        return;
    }
    assert!(status.success(), "the viewer kept running after its terminal closed");
}
