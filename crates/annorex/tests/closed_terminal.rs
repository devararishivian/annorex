//! The viewer must exit when its terminal closes. The event reader of crossterm can spin at 100% CPU
//! after the terminal input ends (crossterm issue 793). The `use-dev-tty` feature avoids it.

use std::process::Command;

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
    sys.exit(0)
os.close(master)
try:
    proc.wait(3)
except subprocess.TimeoutExpired:
    proc.kill()
    sys.exit(1)
"#;

#[test]
fn the_viewer_exits_when_the_terminal_closes() {
    let Ok(status) =
        Command::new("python3").args(["-I", "-c", SCRIPT, env!("CARGO_BIN_EXE_annorex")]).status()
    else {
        return; // No python3 on this machine: skip.
    };
    assert!(status.success(), "the viewer kept running after its terminal closed");
}
