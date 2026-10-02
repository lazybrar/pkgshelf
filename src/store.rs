// SPDX-License-Identifier: GPL-2.0-or-later
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub const REPO: &str = "pkgshelf";

pub struct Dirs {
    pub packages: PathBuf,
    pub reviewed: PathBuf,
    pub repo: PathBuf,
    pub cache: PathBuf,
}

pub fn dirs() -> Result<Dirs, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let xdg = |var: &str, fallback: &str| {
        std::env::var(var)
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&home).join(fallback))
    };
    let config = xdg("XDG_CONFIG_HOME", ".config").join("pkgshelf");
    let data = xdg("XDG_DATA_HOME", ".local/share").join("pkgshelf");
    let cache = xdg("XDG_CACHE_HOME", ".cache").join("pkgshelf");
    Ok(Dirs {
        packages: config.join("packages"),
        reviewed: data.join("reviewed"),
        repo: data.join("repo"),
        cache: cache.join("aur"),
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
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| format!("cannot create {}: {e}", p.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))
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
