// SPDX-License-Identifier: GPL-2.0-or-later
//! Shared (multi-user) setup: permission checks, the pacman.conf check and migration from per-user dirs.
use crate::store::{self, Dirs, GROUP, REPO};
use crate::{pkg, util};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn can_write(dir: &Path) -> bool {
    let probe = dir.join(".pkgshelf-write-test");
    let ok = fs::File::create(&probe).is_ok();
    let _ = fs::remove_file(probe);
    ok
}

/// Print one line per shared directory; false if the setup blocks normal use.
pub fn check_dirs(d: &Dirs) -> bool {
    let mut ok = true;
    let mut to_create: Vec<PathBuf> = vec![d.root.clone(), d.repo.clone()];
    if let Some(p) = d.cache.parent() {
        to_create.push(p.to_path_buf());
    }
    to_create.push(d.cache.clone());
    to_create.dedup();
    for (what, p) in [("data ", &d.root), ("repo ", &d.repo), ("cache", &d.cache)] {
        let Ok(m) = fs::metadata(p) else {
            ok = false;
            println!("MISSING {what} dir {}", p.display());
            continue;
        };
        let mode = m.permissions().mode();
        let mut notes: Vec<String> = Vec::new();
        if mode & 0o002 != 0 {
            ok = false;
            notes.push(
                "WORLD-WRITABLE: any user could plant packages that pacman installs as root".into(),
            );
        }
        if mode & 0o020 == 0 {
            notes.push(format!(
                "not group-writable, other {GROUP} members cannot update"
            ));
        }
        if mode & 0o2000 == 0 {
            notes.push("no setgid bit, new files will not inherit the group".into());
        }
        if !can_write(p) {
            ok = false;
            notes.push(format!(
                "you cannot write here (join the `{GROUP}` group, then log in again)"
            ));
        }
        let state = if notes.is_empty() {
            "ok     "
        } else {
            "WARN   "
        };
        let tail = if notes.is_empty() {
            String::new()
        } else {
            format!(": {}", notes.join("; "))
        };
        println!(
            "{state} {what} dir {} (mode {:o}){tail}",
            p.display(),
            mode & 0o7777
        );
    }
    if !ok {
        let dirs: Vec<String> = to_create.iter().map(|p| p.display().to_string()).collect();
        println!(
            "\nthe pkgshelf package creates these through sysusers.d/tmpfiles.d; by hand, as root:\n  \
             sudo groupadd -r {GROUP}\n  sudo install -d -m 2775 -o root -g {GROUP} {}\n  \
             sudo usermod -aG {GROUP} <user>      # then log in again\n",
            dirs.join(" ")
        );
    }
    ok
}

/// None: no [pkgshelf] section. Some(None): section without a Server line. Some(Some(url)): its Server.
pub fn parse_server(conf: &str) -> Option<Option<String>> {
    let (mut inside, mut found, mut server) = (false, false, None);
    for l in conf.lines().map(str::trim) {
        if l.starts_with('[') {
            inside = l == format!("[{REPO}]");
            found |= inside;
        } else if inside {
            if let Some(v) = l.strip_prefix("Server") {
                if let Some((_, v)) = v.split_once('=') {
                    server = Some(v.trim().to_string());
                }
            }
        }
    }
    found.then_some(server)
}

pub fn check_pacman_conf(d: &Dirs) -> bool {
    let conf = fs::read_to_string("/etc/pacman.conf").unwrap_or_default();
    let want = format!("file://{}", d.repo.display());
    match parse_server(&conf) {
        Some(Some(s)) if s.trim_end_matches('/') == want => {
            println!("ok      /etc/pacman.conf [{REPO}] -> {s}");
            true
        }
        Some(other) => {
            println!(
                "WARN    /etc/pacman.conf [{REPO}] Server is {}, expected {want}; fix it, then run `sudo pacman -Sy`",
                other.unwrap_or_else(|| "missing".into())
            );
            false
        }
        None => {
            println!(
                "MISSING [{REPO}] in /etc/pacman.conf; add this at the end, then run `sudo pacman -Sy`:\n\n\
                 [{REPO}]\nSigLevel = Optional TrustAll\nServer = {want}\n"
            );
            false
        }
    }
}

/// makepkg and repo-add create 0644 files whatever the umask; make the repo group-writable so any
/// `pkgshelf` member can replace them. Files owned by someone else fail the chmod and are skipped.
pub fn share_repo(repo: &Path) {
    for e in fs::read_dir(repo).into_iter().flatten().flatten() {
        if e.metadata().is_ok_and(|m| m.is_file()) {
            let _ = fs::set_permissions(e.path(), fs::Permissions::from_mode(0o664));
        }
    }
}

/// Copy tracked entries, reviewed commits and built packages from the per-user dirs (pre 0.2.0)
/// into the shared location. Nothing is deleted.
pub fn migrate(d: &Dirs) -> Result<Vec<String>, String> {
    let old = store::legacy_dirs().ok_or("HOME is not set")?;
    if old.root == d.root {
        return Err("the per-user and shared locations are the same".into());
    }
    let mut lines = Vec::new();

    let mut entries = store::load(d)?;
    let mut added = 0;
    for e in store::load(&old)? {
        if !entries
            .iter()
            .any(|x| x.kind == e.kind && x.target == e.target)
        {
            entries.push(e);
            added += 1;
        }
    }
    if added > 0 {
        store::save(d, &entries)?;
    }
    lines.push(format!("tracked entries: {added} copied"));

    let have = store::load_reviewed(d);
    let mut reviewed = 0;
    for (name, commit) in store::load_reviewed(&old) {
        if !have.contains_key(&name) {
            store::set_reviewed(d, &name, &commit)?;
            reviewed += 1;
        }
    }
    lines.push(format!("reviewed commits: {reviewed} copied"));

    let mut copied: Vec<PathBuf> = Vec::new();
    for e in fs::read_dir(&old.repo).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if pkg::file_parts(&name).is_none() {
            continue;
        }
        let dest = d.repo.join(&name);
        if !dest.exists() {
            fs::create_dir_all(&d.repo)
                .map_err(|e| format!("cannot create {}: {e}", d.repo.display()))?;
            fs::copy(e.path(), &dest).map_err(|e| format!("cannot copy {name}: {e}"))?;
            let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(0o664));
            copied.push(dest);
        }
    }
    if !copied.is_empty() {
        util::run(
            util::grp("repo-add")
                .arg("-R")
                .arg(d.repo.join(format!("{REPO}.db.tar.zst")))
                .args(&copied),
        )?;
    }
    share_repo(&d.repo);
    lines.push(format!("built packages: {} copied", copied.len()));
    lines.push(format!(
        "next: point [{REPO}] in /etc/pacman.conf at file://{}, run `sudo pacman -Sy`; the old data in {} can then be deleted",
        d.repo.display(),
        old.root.display()
    ));
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacman_conf_server() {
        let conf = "[options]\nHoldPkg = pacman\n[core]\nInclude = x\n[pkgshelf]\n# Server = file:///old\nSigLevel = Optional TrustAll\nServer = file:///var/lib/pkgshelf/repo\n[extra]\nServer = y\n";
        assert_eq!(
            parse_server(conf),
            Some(Some("file:///var/lib/pkgshelf/repo".into()))
        );
        assert_eq!(parse_server("[pkgshelf]\nSigLevel = Never\n"), Some(None));
        assert_eq!(parse_server("[core]\nServer = z\n"), None);
    }
}
