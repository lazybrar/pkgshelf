// SPDX-License-Identifier: GPL-2.0-or-later
use crate::util;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq)]
pub struct Src {
    pub names: Vec<String>,
    pub version: String,
}

pub fn parse_srcinfo(s: &str) -> Option<Src> {
    let (mut names, mut ver, mut rel, mut epoch) = (Vec::new(), None, None, None);
    for l in s.lines() {
        let Some((k, v)) = l.trim().split_once(" = ") else {
            continue;
        };
        match k {
            "pkgname" => names.push(v.to_string()),
            "pkgver" => ver = Some(v.to_string()),
            "pkgrel" => rel = Some(v.to_string()),
            "epoch" => epoch = Some(v.to_string()),
            _ => {}
        }
    }
    let version = match epoch.filter(|e| e != "0") {
        Some(e) => format!("{e}:{}-{}", ver?, rel?),
        None => format!("{}-{}", ver?, rel?),
    };
    (!names.is_empty()).then_some(Src { names, version })
}

pub fn srcinfo(dir: &Path) -> Result<Src, String> {
    let s = util::out(
        Command::new("makepkg")
            .arg("--printsrcinfo")
            .current_dir(dir),
    )?;
    parse_srcinfo(&s)
        .ok_or_else(|| format!("cannot read version/name from {}/PKGBUILD", dir.display()))
}

/// Values of `.SRCINFO` keys (e.g. depends, makedepends, validpgpkeys) for a PKGBUILD dir.
pub fn srcinfo_values(dir: &Path, keys: &[&str]) -> Result<Vec<String>, String> {
    let s = util::out(
        Command::new("makepkg")
            .arg("--printsrcinfo")
            .current_dir(dir),
    )?;
    Ok(values_of(&s, keys))
}

fn values_of(srcinfo: &str, keys: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = srcinfo
        .lines()
        .filter_map(|l| l.trim().split_once(" = "))
        .filter(|(k, _)| keys.contains(k))
        .map(|(_, v)| v.trim().to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Package name part of a dependency string like `foo>=1.2` or `foo: why`.
pub fn dep_name(d: &str) -> &str {
    d.split(['<', '>', '=', ':']).next().unwrap_or(d).trim()
}

/// Built files of `name` in the repo, newest version first.
pub fn versions_of(repo: &Path, name: &str) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = fs::read_dir(repo)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let (n, ver) = e.file_name().to_str().and_then(file_parts)?;
            (n == name).then(|| (ver, e.path()))
        })
        .collect();
    v.sort_by(|a, b| util::vercmp(&b.0, &a.0));
    v
}

/// Delete all but the `keep` newest built versions of every package (kept for `rollback`).
pub fn prune_repo(repo: &Path, keep: usize) {
    let names: HashSet<String> = fs::read_dir(repo)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| Some(file_parts(e.file_name().to_str()?)?.0))
        .collect();
    for n in names {
        for (_, p) in versions_of(repo, &n).into_iter().skip(keep) {
            let _ = fs::remove_file(p);
        }
    }
}

/// Directory holding the PKGBUILD for a local project: the dir itself or its `pkg/` subdir.
pub fn local_pkgdir(path: &Path) -> Option<PathBuf> {
    if path.join("PKGBUILD").exists() {
        Some(path.to_path_buf())
    } else if path.join("pkg/PKGBUILD").exists() {
        Some(path.join("pkg"))
    } else {
        None
    }
}

/// `name-pkgver-pkgrel-arch.pkg.tar.zst` -> (name, "pkgver-pkgrel")
pub fn file_parts(file: &str) -> Option<(String, String)> {
    if file.ends_with(".sig") {
        return None;
    }
    let (stem, _) = file.split_once(".pkg.tar")?;
    let mut p = stem.rsplitn(4, '-');
    let (_arch, rel, ver, name) = (p.next()?, p.next()?, p.next()?, p.next()?);
    Some((name.to_string(), format!("{ver}-{rel}")))
}

/// Newest built version of each package in the local repo directory.
pub fn repo_versions(repo: &Path) -> HashMap<String, String> {
    let mut m: HashMap<String, String> = HashMap::new();
    for e in fs::read_dir(repo).into_iter().flatten().flatten() {
        let Some((name, ver)) = e.file_name().to_str().and_then(file_parts) else {
            continue;
        };
        match m.get(&name) {
            Some(old) if util::vercmp(old, &ver) != Ordering::Less => {}
            _ => {
                m.insert(name, ver);
            }
        }
    }
    m
}

pub fn installed() -> HashMap<String, String> {
    util::out(Command::new("pacman").arg("-Q"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srcinfo_versions() {
        let s = "pkgbase = foo\n\tpkgver = 1.2\n\tpkgrel = 3\n\nepoch = 0\npkgname = foo\n";
        assert_eq!(
            parse_srcinfo(s),
            Some(Src {
                names: vec!["foo".into()],
                version: "1.2-3".into()
            })
        );
        let s = "pkgbase = b\n\tpkgver = 1.96.60\n\tpkgrel = 1\n\tepoch = 1\npkgname = brave-bin\npkgname = brave-extra\n";
        let r = parse_srcinfo(s).unwrap();
        assert_eq!(r.version, "1:1.96.60-1");
        assert_eq!(r.names.len(), 2);
        assert_eq!(parse_srcinfo("pkgname = x\n"), None);
    }

    #[test]
    fn srcinfo_keys() {
        let s = "pkgbase = a\n\tdepends = x>=1\n\tmakedepends = y\n\tdepends = x>=1\n\tvalidpgpkeys = ABCD\n";
        assert_eq!(values_of(s, &["depends", "makedepends"]), vec!["x>=1", "y"]);
        assert_eq!(values_of(s, &["validpgpkeys"]), vec!["ABCD"]);
        assert_eq!(dep_name("foo>=1.2"), "foo");
        assert_eq!(dep_name("foo: for bar"), "foo");
    }

    #[test]
    fn filenames() {
        assert_eq!(
            file_parts("brave-bin-1:1.96.60-1-x86_64.pkg.tar.zst"),
            Some(("brave-bin".into(), "1:1.96.60-1".into()))
        );
        assert_eq!(
            file_parts("a-b-c-0.1-2-any.pkg.tar.xz"),
            Some(("a-b-c".into(), "0.1-2".into()))
        );
        assert_eq!(file_parts("x-1-1-any.pkg.tar.zst.sig"), None);
        assert_eq!(file_parts("pkgshelf.db.tar.zst"), None);
    }
}
