// SPDX-License-Identifier: GPL-2.0-or-later
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn home(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn run(h: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pkgshelf"))
        .args(args)
        .env("HOME", h)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .output()
        .unwrap()
}

fn seed(h: &Path, text: &str) -> PathBuf {
    let f = h.join(".config/pkgshelf/packages");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    fs::write(&f, text).unwrap();
    f
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn list_hold_unhold_rm() {
    let h = home("lifecycle");
    let f = seed(&h, "aur brave-bin\naur claude-code\n");
    assert_eq!(stdout(&run(&h, &["list"])).lines().count(), 2);

    assert!(run(&h, &["hold", "brave-bin"]).status.success());
    assert!(
        fs::read_to_string(&f)
            .unwrap()
            .contains("aur brave-bin hold")
    );
    assert!(stdout(&run(&h, &["list"])).contains("(hold)"));

    assert!(run(&h, &["unhold", "brave-bin"]).status.success());
    assert!(!fs::read_to_string(&f).unwrap().contains("hold\n"));

    assert!(run(&h, &["rm", "claude-code"]).status.success());
    let left = fs::read_to_string(&f).unwrap();
    assert!(left.contains("brave-bin") && !left.contains("claude-code"));
}

#[test]
fn untracked_name_fails() {
    let h = home("untracked");
    seed(&h, "aur brave-bin\n");
    let o = run(&h, &["hold", "nope"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("not tracked"));
}

#[test]
fn empty_list_and_usage_errors() {
    let h = home("empty");
    assert!(stdout(&run(&h, &["list"])).contains("nothing tracked"));
    assert_eq!(run(&h, &["bogus"]).status.code(), Some(2));
    assert_eq!(run(&h, &["add", "aur"]).status.code(), Some(2));
    assert!(stdout(&run(&h, &["--version"])).starts_with("pkgshelf 0."));
}

#[test]
fn invalid_aur_names_are_rejected_before_any_network_call() {
    let h = home("invalid");
    let o = run(&h, &["add", "aur", "bad name&x=1"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("invalid package name"));
}
