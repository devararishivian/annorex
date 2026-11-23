//! The `annorex` binary: commands, usage errors, and exit codes.

#![allow(clippy::expect_used, clippy::panic, reason = "tests assert by panicking")]

use std::io::Write as _;
use std::process::{Command, Output, Stdio};

fn annorex(args: &[&str], env: &[(&str, &str)], stdin: &str) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_annorex"));
    command.args(args).env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("annorex starts");
    child.stdin.take().expect("stdin").write_all(stdin.as_bytes()).expect("stdin written");
    child.wait_with_output().expect("annorex ends")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_lists_probe_and_deliver() {
    let output = annorex(&["--help"], &[], "");
    assert_eq!(output.status.code(), Some(0));
    let help = text(&output.stdout);
    assert!(help.contains("probe"), "{help}");
    assert!(help.contains("deliver"), "{help}");
}

#[test]
fn missing_rex_binary_exits_4_and_names_paths() {
    let env = [("REX_BIN_DIR", "/nonexistent"), ("PATH", ""), ("REX_SESSION", "session:x")];
    let output = annorex(&["probe", "--target", "block:b1"], &env, "");
    assert_eq!(output.status.code(), Some(4));
    assert!(text(&output.stderr).contains("/nonexistent/rex"), "{}", text(&output.stderr));
}

#[test]
fn missing_session_exits_64() {
    let output = annorex(&["probe", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn deliver_with_empty_stdin_exits_64() {
    let env = [("REX_BIN_DIR", "/nonexistent"), ("PATH", ""), ("REX_SESSION", "session:x")];
    let output = annorex(&["deliver", "--target", "block:b1"], &env, "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("empty"), "{}", text(&output.stderr));
}

#[test]
fn unknown_argument_exits_64() {
    let output = annorex(&["probe", "--nope"], &[], "");
    assert_eq!(output.status.code(), Some(64));
}

#[test]
fn missing_target_exits_64() {
    let output = annorex(&["deliver"], &[("REX_SESSION", "session:x")], "note");
    assert_eq!(output.status.code(), Some(64));
}

#[test]
fn rex_install_without_rex_writes_init_lua_and_skips_the_check_and_the_reload() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = annorex(&["rex", "install", "--dir", path], &[("PATH", "/usr/bin")], "");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert!(out.contains(&format!("Wrote {path}/annorex.lua")), "{out}");
    assert!(out.contains(&format!("Created {path}/init.lua with these lines:")), "{out}");
    assert!(out.contains("Skipped the check of init.lua: rex is not available."), "{out}");
    assert!(out.contains("Skipped rex config reload: rex is not available."), "{out}");
    // The module starts the binary that installed it, so no bin option is needed.
    let bin = env!("CARGO_BIN_EXE_annorex");
    assert!(out.contains(&format!("Rex starts: {bin}")), "{out}");
    assert!(!out.contains("setup{ bin = \""), "{out}");
    let module = std::fs::read_to_string(dir.path().join("annorex.lua")).expect("module");
    assert!(module.contains(&format!("local INSTALLED_BIN = \"{bin}\"\n")), "{module}");
    let init = std::fs::read_to_string(dir.path().join("init.lua")).expect("init.lua");
    assert_eq!(init, SETUP_LINES.join("\n") + "\n");
}

/// The lines of the setup block, as `rex install` writes them into a new `init.lua`.
const SETUP_LINES: [&str; 4] = [
    "-- Added by `annorex rex install`. Remove these lines to remove Annorex.",
    "require(\"annorex\").setup{}",
    "rex.bind(\"cmd+shift+a\", \"annorex_last\")",
    "rex.bind(\"cmd+shift+f\", \"annorex_files\")",
];

/// A fake `rex` in a new folder, for `rex config check` and `rex config reload`. It logs each
/// call to `log`. The file `check-fails` makes the check fail; `reload-fails` makes the reload
/// fail. Put the folder first on `PATH`.
fn fake_rex_config() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("temp dir");
    let script = "#!/bin/sh\nhere=$(dirname \"$0\")\necho \"$* REX_CONFIG=$REX_CONFIG\" >> \"$here/log\"\n\
         case \"$1 $2\" in\n  \
         'config check') if [ -e \"$here/check-fails\" ]; then echo \"error $(basename \"$3\"):1: boom\"; \
         echo 'Error: the configuration has errors' >&2; exit 1; fi; echo \"path: $3\" ;;\n  \
         'config reload') if [ -e \"$here/reload-fails\" ]; then echo 'Error: cannot reload' >&2; exit 1; fi ;;\n  \
         *) exit 2 ;;\nesac\n";
    let rex = dir.path().join("rex");
    std::fs::write(&rex, script).expect("fake rex");
    std::fs::set_permissions(&rex, std::fs::Permissions::from_mode(0o755)).expect("mode");
    dir
}

/// Run `annorex rex install --dir DIR` with the fake `rex` first on `PATH`.
fn install_with_fake_rex(rex: &tempfile::TempDir, dir: &std::path::Path, extra: &[&str]) -> Output {
    let path = format!("{}:/usr/bin:/bin", rex.path().display());
    let mut args = vec!["rex", "install", "--dir", dir.to_str().expect("utf-8 path")];
    args.extend_from_slice(extra);
    annorex(&args, &[("PATH", path.as_str())], "")
}

fn rex_log(rex: &tempfile::TempDir) -> String {
    std::fs::read_to_string(rex.path().join("log")).unwrap_or_default()
}

/// Assert that `needles` are in `out`, in this order.
fn assert_in_order(out: &str, needles: &[&str]) {
    let mut from = 0;
    for needle in needles {
        let at = out.get(from..).and_then(|rest| rest.find(needle)).unwrap_or_else(|| {
            panic!("{needle:?} is missing after byte {from} in:\n{out}");
        });
        from += at + needle.len();
    }
}

#[test]
fn rex_install_creates_init_lua_checks_it_and_reloads_rex() {
    let rex = fake_rex_config();
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    let wrote = format!("Wrote {path}/annorex.lua");
    let created = format!("Created {path}/init.lua with these lines:");
    let mut order = vec![wrote.as_str(), created.as_str()];
    order.extend(SETUP_LINES);
    order.extend([
        "Rex starts: ",
        "Checked init.lua with rex config check: no errors.",
        "Ran rex config reload: Rex uses the new configuration.",
    ]);
    assert_in_order(&out, &order);
    assert!(!out.contains("Add these lines"), "{out}");
    let init = std::fs::read_to_string(dir.path().join("init.lua")).expect("init.lua");
    assert_eq!(init, SETUP_LINES.join("\n") + "\n");
    let log = rex_log(&rex);
    // The check ran on a file beside init.lua, so `require("annorex")` finds the module.
    assert!(log.contains(&format!("config check {path}/.init.lua.annorex-check.")), "{log}");
    assert!(log.contains("config reload"), "{log}");
    let names: Vec<_> =
        std::fs::read_dir(dir.path()).expect("dir").map(|e| e.expect("entry").file_name()).collect();
    assert_eq!(names.len(), 2, "only annorex.lua and init.lua stay: {names:?}");
}

#[test]
fn rex_install_appends_to_init_lua_and_names_a_bound_key() {
    let rex = fake_rex_config();
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    std::fs::write(dir.path().join("init.lua"), "rex.bind(\"shift+cmd+f\", \"find\")\n").expect("init");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    let added = format!("Added these lines to {path}/init.lua:");
    assert_in_order(
        &out,
        &[
            &added,
            SETUP_LINES[0],
            SETUP_LINES[1],
            SETUP_LINES[2],
            "init.lua already binds cmd+shift+f, so Annorex left out its line. Choose a free key and add:",
            "rex.bind(\"KEY\", \"annorex_files\")",
            "Checked init.lua with rex config check: no errors.",
            "Ran rex config reload",
        ],
    );
    let init = std::fs::read_to_string(dir.path().join("init.lua")).expect("init.lua");
    assert!(!init.contains("\"cmd+shift+f\""), "{init}");
    assert!(init.starts_with("rex.bind(\"shift+cmd+f\", \"find\")\n\n-- Added by"), "{init}");
}

#[test]
fn rex_install_leaves_init_lua_that_loads_annorex_and_still_reloads() {
    let rex = fake_rex_config();
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    let mine = "require('annorex').setup{ update_check = false }\n";
    std::fs::write(dir.path().join("init.lua"), mine).expect("init");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert_in_order(
        &out,
        &[
            "init.lua already loads Annorex. No change.",
            "Checked init.lua with rex config check: no errors.",
            "Ran rex config reload: Rex uses the new configuration.",
        ],
    );
    assert_eq!(std::fs::read_to_string(dir.path().join("init.lua")).expect("init"), mine);
    // The real init.lua is checked in place.
    let log = rex_log(&rex);
    assert!(log.contains(&format!("config check {path}/init.lua REX_CONFIG={path}/init.lua")), "{log}");
}

#[test]
fn rex_install_of_a_loaded_init_lua_with_errors_exits_1() {
    let rex = fake_rex_config();
    std::fs::write(rex.path().join("check-fails"), "").expect("flag");
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("init.lua"), "require(\"annorex\").setup{}\n").expect("init");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert_in_order(
        &out,
        &[
            "init.lua already loads Annorex. No change.",
            "rex config check found errors in init.lua:",
            "error init.lua:1: boom",
        ],
    );
    assert!(!rex_log(&rex).contains("config reload"), "{}", rex_log(&rex));
}

#[test]
fn rex_install_reports_a_failed_reload_and_exits_0() {
    let rex = fake_rex_config();
    std::fs::write(rex.path().join("reload-fails"), "").expect("flag");
    let dir = tempfile::tempdir().expect("temp dir");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert!(out.contains("Could not reload Rex. Run: rex config reload"), "{out}");
    assert!(!out.contains("Ran rex config reload"), "{out}");
    assert!(dir.path().join("init.lua").exists());
}

#[test]
fn rex_install_hand_list_leaves_out_a_bound_key_and_names_it() {
    let rex = fake_rex_config();
    std::fs::write(rex.path().join("check-fails"), "").expect("flag");
    let dir = tempfile::tempdir().expect("temp dir");
    let mine = "rex.bind(\"shift+cmd+f\", \"find\")\n";
    std::fs::write(dir.path().join("init.lua"), mine).expect("init");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert!(out.contains("annorex_last"), "{out}");
    let list = out.split("error init.lua").next().expect("list");
    assert!(!list.contains("rex.bind(\"cmd+shift+f\", \"annorex_files\")"), "{out}");
    assert!(
        out.contains(
            "init.lua already binds cmd+shift+f, so Annorex left out its line. Choose a free key and add:"
        ),
        "{out}"
    );
    assert_eq!(std::fs::read_to_string(dir.path().join("init.lua")).expect("init"), mine);
}

#[test]
fn rex_install_hand_list_of_a_write_error_leaves_out_a_bound_key_and_names_it() {
    use std::os::unix::fs::PermissionsExt as _;
    let rex = fake_rex_config();
    let dir = tempfile::tempdir().expect("temp dir");
    // init.lua is a link to a file in a read-only folder, so the final write fails.
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).expect("locked dir");
    std::fs::write(locked.join("init.lua"), "rex.bind(\"shift+cmd+f\", \"find\")\n").expect("init");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).expect("mode");
    std::os::unix::fs::symlink(locked.join("init.lua"), dir.path().join("init.lua")).expect("link");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).expect("mode");
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    assert!(out.contains("Add these lines by hand:"), "{out}");
    assert!(out.contains("annorex_last"), "{out}");
    assert!(!out.contains("rex.bind(\"cmd+shift+f\", \"annorex_files\")"), "{out}");
    assert!(
        out.contains(
            "init.lua already binds cmd+shift+f, so Annorex left out its line. Choose a free key and add:"
        ),
        "{out}"
    );
}

#[test]
fn rex_install_does_not_write_init_lua_when_the_check_fails() {
    let rex = fake_rex_config();
    std::fs::write(rex.path().join("check-fails"), "").expect("flag");
    let dir = tempfile::tempdir().expect("temp dir");
    let mine = "-- mine\n";
    std::fs::write(dir.path().join("init.lua"), mine).expect("init");
    let output = install_with_fake_rex(&rex, dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    let mut order = vec!["rex config check found errors. init.lua was not changed. Add these lines by hand:"];
    // The hand-written lines are the three lines, without the comment that names `rex install`.
    order.extend(&SETUP_LINES[1..]);
    order.push("error init.lua:1: boom");
    assert_in_order(&out, &order);
    assert_eq!(std::fs::read_to_string(dir.path().join("init.lua")).expect("init"), mine);
    assert!(dir.path().join("annorex.lua").exists(), "the module stays written");
    assert!(!rex_log(&rex).contains("config reload"), "{}", rex_log(&rex));
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 2, "no check file stays");
}

#[test]
fn rex_install_no_init_keeps_the_old_output() {
    let rex = fake_rex_config();
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = install_with_fake_rex(&rex, dir.path(), &["--no-init"]);
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let out = text(&output.stdout);
    let wrote = format!("Wrote {path}/annorex.lua");
    assert_in_order(
        &out,
        &[
            &wrote,
            "Add these lines to init.lua:",
            "  require(\"annorex\").setup{}",
            "  rex.bind(\"cmd+shift+a\", \"annorex_last\")",
            "  rex.bind(\"cmd+shift+f\", \"annorex_files\")",
            "Rex starts: ",
            "Checked the module with rex config check: no errors.",
            "Then run: rex config reload",
        ],
    );
    assert!(!dir.path().join("init.lua").exists());
    assert!(!rex_log(&rex).contains("config reload"), "{}", rex_log(&rex));
}

#[test]
fn rex_install_help_names_init_lua_and_no_init() {
    let help = text(&annorex(&["rex", "install", "--help"], &[], "").stdout);
    assert!(help.contains("Write the Lua module and add the setup lines to `init.lua`"), "{help}");
    assert!(help.contains("--no-init"), "{help}");
}

#[test]
fn rex_install_refuses_foreign_file_with_exit_3() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("annorex.lua"), "-- mine\n").expect("foreign");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = annorex(&["rex", "install", "--dir", path], &[], "");
    assert_eq!(output.status.code(), Some(3));
    assert!(text(&output.stderr).contains("annorex.lua"), "{}", text(&output.stderr));
}

#[test]
fn rex_open_without_session_exits_64() {
    let output = annorex(&["rex", "open", "last", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn rex_open_has_no_placement_flag() {
    let env = [("REX_SESSION", "session:x")];
    let output = annorex(&["rex", "open", "last", "--target", "block:b1", "--placement", "right"], &env, "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("--placement"), "{}", text(&output.stderr));
}

#[test]
fn a_config_file_with_the_removed_placement_key_exits_64_and_names_the_key() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(dir.path().join("annorex")).expect("config dir");
    std::fs::write(dir.path().join("annorex/config.toml"), "placement = \"right\"\n").expect("config");
    let home = dir.path().to_string_lossy().into_owned();
    let env = [("XDG_CONFIG_HOME", home.as_str()), ("REX_SESSION", "session:x"), ("PATH", "")];
    let output = annorex(&["rex", "open", "last", "--target", "block:b1"], &env, "");
    assert_eq!(output.status.code(), Some(64), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("placement"), "{}", text(&output.stderr));
}

#[test]
fn last_without_session_exits_64() {
    let output = annorex(&["last", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn help_lists_last_and_rex() {
    let output = annorex(&["--help"], &[], "");
    assert_eq!(output.status.code(), Some(0));
    let help = text(&output.stdout);
    assert!(help.contains("last"), "{help}");
    assert!(help.contains("rex"), "{help}");
}

#[test]
fn bad_config_exits_64() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(dir.path().join("annorex")).expect("config dir");
    std::fs::write(dir.path().join("annorex/config.toml"), "colour = 1\n").expect("config");
    let home = dir.path().to_string_lossy().into_owned();
    let env = [("XDG_CONFIG_HOME", home.as_str()), ("REX_SESSION", "session:x"), ("PATH", "")];
    for args in
        [&["deliver", "--target", "block:b1"][..], &["rex", "open", "last", "--target", "block:b1"][..]]
    {
        let output = annorex(args, &env, "hello");
        assert_eq!(output.status.code(), Some(64), "{args:?}: {}", text(&output.stderr));
        assert!(text(&output.stderr).contains("colour"), "{}", text(&output.stderr));
    }
}

#[test]
fn files_command_parses_paths_and_target() {
    // The command line is valid, so the run goes on and fails at the Rex discovery (exit 4).
    let env = [("REX_BIN_DIR", "/nonexistent"), ("PATH", ""), ("REX_SESSION", "session:x")];
    let output = annorex(&["files", "--target", "block:b1", "README.md", "docs/a.md"], &env, "");
    assert_eq!(output.status.code(), Some(4), "{}", text(&output.stderr));
    let output = annorex(&["files", "--target", "block:b1", "--session", "session:x"], &env, "");
    assert_eq!(output.status.code(), Some(4), "{}", text(&output.stderr));
}

#[test]
fn files_without_target_or_session_exits_64() {
    let output = annorex(&["files"], &[("REX_SESSION", "session:x")], "");
    assert_eq!(output.status.code(), Some(64));
    let output = annorex(&["files", "--target", "block:b1"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn rex_open_files_without_session_exits_64() {
    let output = annorex(&["rex", "open", "files", "--target", "block:b1", "a.md"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn help_lists_files() {
    let help = text(&annorex(&["--help"], &[], "").stdout);
    assert!(help.contains("files"), "{help}");
}

#[test]
fn update_help_lists_check() {
    let help = text(&annorex(&["update", "--help"], &[], "").stdout);
    assert!(help.contains("--check"), "{help}");
}

#[test]
fn the_viewer_commands_list_the_update_opt_out() {
    for args in [
        &["last", "--help"][..],
        &["files", "--help"][..],
        &["rex", "open", "last", "--help"][..],
        &["rex", "open", "files", "--help"][..],
    ] {
        let help = text(&annorex(args, &[], "").stdout);
        assert!(help.contains("--no-update-check"), "{args:?}: {help}");
    }
}

#[test]
fn the_module_has_the_update_check_option() {
    assert!(annorex::install::MODULE_SOURCE.contains("update_check"));
    assert!(annorex::install::MODULE_SOURCE.contains("--no-update-check"));
}

/// A fake `rex` in a new folder: every block is a shell pane. Give the folder as `REX_BIN_DIR`.
fn fake_rex_with_a_shell_pane() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("temp dir");
    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rex");
    // The method is the argument before the last one (`-`).
    let script = format!(
        "#!/bin/sh\nfor arg in \"$@\"; do method=\"$last\"; last=\"$arg\"; done\ncase \"$method\" in\n  \
         process) cat '{fixtures}/process-shell.json' ;;\n  \
         program_status) cat '{fixtures}/status-empty.json' ;;\n  \
         *) echo \"Error: unknown method $method\" >&2; exit 1 ;;\nesac\n"
    );
    let rex = dir.path().join("rex");
    std::fs::write(&rex, script).expect("fake rex");
    std::fs::set_permissions(&rex, std::fs::Permissions::from_mode(0o755)).expect("mode");
    dir
}

#[test]
fn rex_open_of_a_pane_without_an_agent_exits_5_and_deliver_still_exits_3() {
    let rex = fake_rex_with_a_shell_pane();
    let bin_dir = rex.path().to_str().expect("utf-8 path");
    let env = [("REX_BIN_DIR", bin_dir), ("PATH", "/usr/bin:/bin"), ("REX_SESSION", "session:x")];
    let not_agent = "zsh is not supported";
    for view in ["last", "files"] {
        let output = annorex(&["rex", "open", view, "--target", "block:b1"], &env, "");
        assert_eq!(output.status.code(), Some(5), "{view}: {}", text(&output.stderr));
        assert!(text(&output.stderr).contains(not_agent), "{}", text(&output.stderr));
    }
    let output = annorex(&["deliver", "--target", "block:b1"], &env, "a note");
    assert_eq!(output.status.code(), Some(3), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("no supported agent"), "{}", text(&output.stderr));
}

#[test]
fn rex_agents_help_lists_the_session() {
    let output = annorex(&["rex", "agents", "--help"], &[], "");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let help = text(&output.stdout);
    assert!(help.contains("--session"), "{help}");
}

#[test]
fn rex_agents_without_session_exits_64() {
    let output = annorex(&["rex", "agents"], &[("PATH", "")], "");
    assert_eq!(output.status.code(), Some(64));
    assert!(text(&output.stderr).contains("session"), "{}", text(&output.stderr));
}

#[test]
fn the_module_asks_for_the_agent_on_exit_5() {
    let module = annorex::install::MODULE_SOURCE;
    assert!(module.contains("\"rex\", \"agents\""), "the module runs annorex rex agents");
    assert!(module.contains("rex.ui.pick"));
    assert!(module.contains("local NOT_AGENT_CODE = 5"));
    assert_eq!(annorex::cli::EXIT_NOT_AGENT, 5);
}

#[test]
fn the_installed_module_always_asks_which_agent_to_open() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().to_str().expect("utf-8 path");
    let output = annorex(&["rex", "install", "--dir", path], &[("PATH", "/usr/bin")], "");
    assert_eq!(output.status.code(), Some(0), "{}", text(&output.stderr));
    let module = std::fs::read_to_string(dir.path().join("annorex.lua")).expect("module");
    assert!(module.contains("Pick an agent"), "{module}");
    assert!(!module.contains("#agents == 1"), "{module}");
}
