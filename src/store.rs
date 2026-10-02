// SPDX-License-Identifier: GPL-2.0-or-later
use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub const REPO: &str = "pkgshelf";
pub const GROUP: &str = "pkgshelf";
pub const DEFAULT_ROOT: &str = "/var/lib/pkgshelf";
pub const DEFAULT_CACHE: &str = "/var/cache/pkgshelf/aur";

pub struct Dirs {
    pub root: PathBuf,
    pub packages: PathBuf,
    pub reviewed: PathBuf,
    pub repo: PathBuf,
    pub cache: PathBuf,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// One shared location for every user of the machine: `/var/lib/pkgshelf` (+ `/var/cache/pkgshelf`).
/// `$PKGSHELF_ROOT` / `$PKGSHELF_CACHE` override it (a custom root keeps its cache inside it).
pub fn dirs() -> Result<Dirs, String> {
    let custom = env_path("PKGSHELF_ROOT");
    let root = custom
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));
    let cache = env_path("PKGSHELF_CACHE").unwrap_or_else(|| match &custom {
        Some(r) => r.join("cache"),
        None => PathBuf::from(DEFAULT_CACHE),
    });
    Ok(Dirs {
        packages: root.join("packages"),
        reviewed: root.join("reviewed"),
        repo: root.join("repo"),
        cache,
        root,
    })
}

/// Per-user locations used before 0.2.0, for `pkgshelf migrate`.
pub fn legacy_dirs() -> Option<Dirs> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let xdg = |var: &str, fallback: &str| env_path(var).unwrap_or_else(|| home.join(fallback));
    let config = xdg("XDG_CONFIG_HOME", ".config").join("pkgshelf");
    let data = xdg("XDG_DATA_HOME", ".local/share").join("pkgshelf");
    Some(Dirs {
        packages: config.join("packages"),
        reviewed: data.join("reviewed"),
        repo: data.join("repo"),
        cache: xdg("XDG_CACHE_HOME", ".cache").join("pkgshelf/aur"),
        root: data,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Aur,
    Local,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    pub target: String,
    pub hold: bool,
}

pub fn valid_aur_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || "@._+-".contains(c))
        && !n.starts_with(['-', '.'])
}

pub fn parse_entries(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                return None;
            }
            let (k, rest) = l.split_once(char::is_whitespace)?;
            let rest = rest.trim();
            let (target, hold) = match rest.strip_suffix(" hold") {
                Some(t) => (t.trim(), true),
                None => (rest, false),
            };
            let kind = match k {
                "aur" if valid_aur_name(target) => Kind::Aur,
                "local" if !target.is_empty() => Kind::Local,
                _ => return None,
            };
            Some(Entry {
                kind,
                target: target.to_string(),
                hold,
            })
        })
        .collect()
}

pub fn render_entries(es: &[Entry]) -> String {
    let mut o =
        String::from("# pkgshelf tracked packages: `aur NAME [hold]` or `local PATH [hold]`\n");
    for e in es {
        let k = if e.kind == Kind::Aur { "aur" } else { "local" };
        o += &format!("{k} {}{}\n", e.target, if e.hold { " hold" } else { "" });
    }
    o
}

pub fn load(d: &Dirs) -> Result<Vec<Entry>, String> {
    match fs::read_to_string(&d.packages) {
        Ok(t) => Ok(parse_entries(&t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(format!("cannot read {}: {e}", d.packages.display())),
    }
}

fn write(path: &PathBuf, text: &str) -> Result<(), String> {
    let hint = |e: &std::io::Error| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            format!(" (are you in the `{GROUP}` group? see `pkgshelf doctor`)")
        } else {
            String::new()
        }
    };
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)
            .map_err(|e| format!("cannot create {}: {e}{}", p.display(), hint(&e)))?;
    }
    fs::write(path, text)
        .map_err(|e| format!("cannot write {}: {e}{}", path.display(), hint(&e)))?;
    // shared between users: keep it group-writable regardless of the umask
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o664));
    Ok(())
}

pub fn save(d: &Dirs, es: &[Entry]) -> Result<(), String> {
    write(&d.packages, &render_entries(es))
}

/// name -> git commit whose PKGBUILD the user last approved
pub fn load_reviewed(d: &Dirs) -> HashMap<String, String> {
    let t = fs::read_to_string(&d.reviewed).unwrap_or_default();
    t.lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(n, c)| (n.to_string(), c.to_string()))
        .collect()
}

pub fn set_reviewed(d: &Dirs, name: &str, commit: &str) -> Result<(), String> {
    let mut m = load_reviewed(d);
    m.insert(name.to_string(), commit.to_string());
    let mut keys: Vec<_> = m.iter().collect();
    keys.sort();
    write(
        &d.reviewed,
        &keys
            .iter()
            .map(|(n, c)| format!("{n} {c}\n"))
            .collect::<String>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_hold() {
        let t = "# c\naur brave-bin\naur claude-code hold\nlocal /home/u/my proj hold\nlocal /x\nbogus y\naur -bad\n";
        let es = parse_entries(t);
        assert_eq!(es.len(), 4);
        assert_eq!(
            es[1],
            Entry {
                kind: Kind::Aur,
                target: "claude-code".into(),
                hold: true
            }
        );
        assert_eq!(es[2].target, "/home/u/my proj");
        assert_eq!(parse_entries(&render_entries(&es)), es);
    }

    #[test]
    fn names() {
        assert!(valid_aur_name("brave-bin") && valid_aur_name("a+b_c.d@e"));
        assert!(
            !valid_aur_name("")
                && !valid_aur_name("-x")
                && !valid_aur_name("a b")
                && !valid_aur_name("a&b=c")
        );
    }
}
