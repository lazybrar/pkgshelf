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
        .env("PKGSHELF_ROOT", h.join("shared"))
        .env_remove("PKGSHELF_CACHE")
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .output()
        .unwrap()
}

fn seed(h: &Path, text: &str) -> PathBuf {
    let f = h.join("shared/packages");
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

#[test]
fn migrate_copies_per_user_data_into_the_shared_root() {
    let h = home("migrate");
    let legacy_cfg = h.join(".config/pkgshelf");
    let legacy_data = h.join(".local/share/pkgshelf");
    fs::create_dir_all(&legacy_cfg).unwrap();
    fs::create_dir_all(&legacy_data).unwrap();
    fs::write(
        legacy_cfg.join("packages"),
        "aur brave-bin\naur claude-code hold\n",
    )
    .unwrap();
    fs::write(legacy_data.join("reviewed"), "brave-bin abc123\n").unwrap();

    let o = run(&h, &["migrate"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout(&o);
    assert!(
        out.contains("tracked entries: 2 copied") && out.contains("reviewed commits: 1 copied"),
        "{out}"
    );
    let list = stdout(&run(&h, &["list"]));
    assert!(
        list.contains("brave-bin") && list.contains("claude-code  (hold)"),
        "{list}"
    );
    assert!(
        fs::read_to_string(h.join("shared/reviewed"))
            .unwrap()
            .contains("brave-bin abc123")
    );

    // idempotent, and the old data is left in place
    assert!(stdout(&run(&h, &["migrate"])).contains("tracked entries: 0 copied"));
    assert!(legacy_cfg.join("packages").exists());
}

#[test]
fn shared_files_are_group_writable() {
    use std::os::unix::fs::PermissionsExt;
    let h = home("perms");
    seed(&h, "aur brave-bin\n");
    assert!(run(&h, &["hold", "brave-bin"]).status.success());
    let mode = fs::metadata(h.join("shared/packages"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o664);
}
