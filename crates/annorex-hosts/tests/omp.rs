//! `OhMyPi` sessions: bucket names, the title slot, and the active leaf.
//! The shapes come from can1357/oh-my-pi @ v19.1.1: `packages/coding-agent/src/session/session-paths.ts`
//! (bucket names), `session-title-slot.ts` (the 256-byte title line), and `session-manager.ts`
//! (`activeLeafId`).

#![allow(clippy::expect_used, clippy::indexing_slicing, reason = "tests assert by panicking")]

use std::path::{Path, PathBuf};

use annorex_hosts::omp::{bucket_names, parse_messages, sessions_for_cwd};
use annorex_hosts::{Host, Role};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/omp")).join(name))
        .expect("fixture")
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("annorex-omp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn put(root: &Path, bucket: &str, name: &str, text: &str, seconds: u64) -> PathBuf {
    let dir = root.join(bucket);
    std::fs::create_dir_all(&dir).expect("bucket");
    let path = dir.join(name);
    std::fs::write(&path, text).expect("session");
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    std::fs::File::options().write(true).open(&path).expect("open").set_modified(time).expect("mtime");
    path
}

#[test]
fn the_label_and_the_name_are_omp_and_ohmypi() {
    assert_eq!(Host::OhMyPi.label(), "omp");
    assert_eq!(Host::OhMyPi.display_name(), "OhMyPi");
}

#[test]
fn bucket_names_for_home_temp_and_other_folders() {
    let home = Path::new("/Users/dev");
    let tmp = Path::new("/var/folders/ab/T");
    assert_eq!(
        bucket_names(Path::new("/Users/dev/Development/project/annorex"), home, tmp),
        ["-Development-project-annorex", "--Users-dev-Development-project-annorex--"]
    );
    assert_eq!(
        bucket_names(Path::new("/var/folders/ab/T/x/y"), home, tmp),
        ["-tmp-x-y", "--var-folders-ab-T-x-y--"]
    );
    assert_eq!(bucket_names(Path::new("/private/tmp/x"), home, tmp), ["--private-tmp-x--"]);
    assert_eq!(bucket_names(Path::new("/Users/dev"), home, tmp), ["-", "--Users-dev--"]);
    assert_eq!(bucket_names(tmp, home, tmp), ["-tmp", "--var-folders-ab-T--"]);
}

#[test]
fn a_temp_root_inside_home_gives_both_names() {
    let home = Path::new("/Users/dev");
    let tmp = Path::new("/Users/dev/T");
    assert_eq!(bucket_names(Path::new("/Users/dev/T/x"), home, tmp), ["-tmp-x", "-T-x", "--Users-dev-T-x--"]);
}

#[test]
fn the_title_slot_line_is_skipped_and_the_active_leaf_decides_the_branch() {
    let text = fixture("active-leaf.jsonl");
    assert_eq!(text.lines().next().expect("slot").len() + 1, 256, "the slot is 256 bytes");
    let messages = parse_messages(&text, 1);
    assert_eq!(messages[0].role, Role::Assistant);
    assert_eq!(messages[0].text, "second reply");
}

#[test]
fn sessions_of_all_three_folder_names_rank_together_by_modification_time() {
    let root = temp("buckets");
    let text = fixture("active-leaf.jsonl");
    let home = Path::new("/Users/dev");
    let tmp = Path::new("/var/folders/ab/T");
    let cwd = Path::new("/Users/dev/work");
    let old = put(&root, "-work", "a.jsonl", &text, 1_000);
    let newest = put(&root, "--Users-dev-work--", "b.jsonl", &text, 3_000);
    put(&root, "-other", "c.jsonl", &text, 9_000);
    put(&root, "-work", "empty.jsonl", text.lines().take(2).collect::<Vec<_>>().join("\n").as_str(), 8_000);
    let found = sessions_for_cwd(&root, cwd, home, tmp, usize::MAX);
    let paths: Vec<_> = found.into_iter().map(|(path, _)| path).collect();
    assert_eq!(paths, [newest, old]);
    assert_eq!(sessions_for_cwd(&root, cwd, home, tmp, 1).len(), 1);
    std::fs::remove_dir_all(&root).expect("cleanup");
}
