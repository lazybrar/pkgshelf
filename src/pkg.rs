// SPDX-License-Identifier: GPL-2.0-or-later
use crate::util;
use std::cmp::Ordering;
use std::collections::HashMap;
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
