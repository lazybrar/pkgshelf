// SPDX-License-Identifier: GPL-2.0-or-later
//! pkgshelf: track the few AUR and local packages you choose in one local pacman repo,
//! so plain `pacman -Syu` keeps them updated. It builds; pacman installs.

mod aur;
mod pkg;
mod store;
mod util;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use store::{Dirs, Entry, Kind, REPO};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_REVIEW_LINES: usize = 300;

const USAGE: &str = "pkgshelf: track the few AUR and local packages you choose in one pacman repo

usage:
  pkgshelf add aur <name>...          track AUR packages
  pkgshelf add local <path>           track a local project (dir with PKGBUILD or pkg/PKGBUILD;
                                      a Rust project without one gets it from `cratepkg init`)
  pkgshelf rm <name> [--purge]        stop tracking (--purge also deletes built files from the repo)
  pkgshelf list                       show tracked entries
  pkgshelf check [--quiet]            installed / built / latest versions; exit 100 if updates pending
  pkgshelf diff <name>                PKGBUILD changes of an AUR package since you last reviewed it
  pkgshelf update [name...] [--yes] [--force]
                                      build outdated packages into the local repo
  pkgshelf hold <name> | unhold <name>
  pkgshelf doctor                     check tools, pacman.conf and untracked foreign packages

pkgshelf only builds into a local repo; install with `sudo pacman -Syu`.
AUR PKGBUILDs are shown for review before they are built.";

// ---------- cli ----------

#[derive(Debug, PartialEq)]
enum Cmd {
    Add {
        kind: Kind,
        targets: Vec<String>,
    },
    Rm {
        name: String,
        purge: bool,
    },
    List,
    Check {
        quiet: bool,
    },
    Diff {
        name: String,
    },
    Update {
        names: Vec<String>,
        yes: bool,
        force: bool,
    },
    Hold {
        name: String,
        on: bool,
    },
    Doctor,
    Help,
    Version,
}

fn parse_args(args: &[String]) -> Result<Cmd, String> {
    let Some(first) = args.first() else {
        return Ok(Cmd::Help);
    };
    let rest = &args[1..];
    let flag = |f: &str| rest.iter().any(|a| a == f);
    let names: Vec<String> = rest
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let allowed = |ok: &[&str]| match rest
        .iter()
        .find(|a| a.starts_with("--") && !ok.contains(&a.as_str()))
    {
        Some(bad) => Err(format!("unknown option '{bad}' for {first}")),
        None => Ok(()),
    };
    let one = |what: &str| match names.as_slice() {
        [n] => Ok(n.clone()),
        _ => Err(format!("{first} needs exactly one {what}")),
    };
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Cmd::Help),
        "-V" | "--version" => Ok(Cmd::Version),
        "add" => {
            allowed(&[])?;
            let kind = match names.first().map(String::as_str) {
                Some("aur") => Kind::Aur,
                Some("local") => Kind::Local,
                _ => return Err("add needs `aur <name>...` or `local <path>`".into()),
            };
            let targets = names[1..].to_vec();
            if targets.is_empty() || (kind == Kind::Local && targets.len() != 1) {
                return Err("add aur <name>... | add local <path>".into());
            }
            Ok(Cmd::Add { kind, targets })
        }
        "rm" => {
            allowed(&["--purge"])?;
            Ok(Cmd::Rm {
                name: one("name")?,
                purge: flag("--purge"),
            })
        }
        "list" => allowed(&[]).map(|_| Cmd::List),
        "check" => allowed(&["--quiet"]).map(|_| Cmd::Check {
            quiet: flag("--quiet"),
        }),
        "diff" => allowed(&[]).and_then(|_| Ok(Cmd::Diff { name: one("name")? })),
        "update" => allowed(&["--yes", "--force"]).map(|_| Cmd::Update {
            names,
            yes: flag("--yes"),
            force: flag("--force"),
        }),
        "hold" | "unhold" => allowed(&[]).and_then(|_| {
            Ok(Cmd::Hold {
                name: one("name")?,
                on: first == "hold",
            })
        }),
        "doctor" => allowed(&[]).map(|_| Cmd::Doctor),
        other => Err(format!("unknown command '{other}'")),
    }
}

// ---------- tracked rows ----------

#[derive(Debug, PartialEq)]
enum Status {
    Build,
    Update,
    Install,
    Ok,
    Hold,
    Missing,
}

fn status(
    hold: bool,
    latest: Option<&str>,
    repo: Option<&str>,
    inst: Option<&str>,
    cmp: &dyn Fn(&str, &str) -> Ordering,
) -> Status {
    let Some(latest) = latest else {
        return Status::Missing;
    };
    if hold {
        return Status::Hold;
    }
    match (repo, inst) {
        (None, _) => Status::Build,
        (Some(r), _) if cmp(r, latest) == Ordering::Less => Status::Update,
        (Some(r), Some(i)) if cmp(i, r) == Ordering::Less => Status::Install,
        _ => Status::Ok,
    }
}

struct Row {
    entry: Entry,
    name: String,
    names: Vec<String>,
    base: String,
    latest: Option<String>,
    flagged: bool,
    dir: Option<PathBuf>,
    note: Option<String>,
}

fn resolve_rows(es: &[Entry]) -> Result<Vec<Row>, String> {
    let aur_names: Vec<String> = es
        .iter()
        .filter(|e| e.kind == Kind::Aur)
        .map(|e| e.target.clone())
        .collect();
    let infos: HashMap<String, aur::Info> = if aur_names.is_empty() {
        HashMap::new()
    } else {
        aur::info(&aur_names)?
            .into_iter()
            .map(|i| (i.name.clone(), i))
            .collect()
    };
    Ok(es
        .iter()
        .map(|e| match e.kind {
            Kind::Aur => {
                let i = infos.get(&e.target);
                Row {
                    entry: e.clone(),
                    name: e.target.clone(),
                    names: vec![e.target.clone()],
                    base: i
                        .map(|i| i.base.clone())
                        .unwrap_or_else(|| e.target.clone()),
                    latest: i.map(|i| i.version.clone()),
                    flagged: i.is_some_and(|i| i.flagged),
                    dir: None,
                    note: i.is_none().then(|| "not found on AUR".to_string()),
                }
            }
            Kind::Local => {
                let path = PathBuf::from(&e.target);
                let src = pkg::local_pkgdir(&path)
                    .ok_or_else(|| "no PKGBUILD (or pkg/PKGBUILD) found".to_string())
                    .and_then(|d| pkg::srcinfo(&d).map(|s| (d, s)));
                match src {
                    Ok((dir, s)) => Row {
                        entry: e.clone(),
                        name: s.names[0].clone(),
                        names: s.names,
                        base: String::new(),
                        latest: Some(s.version),
                        flagged: false,
                        dir: Some(dir),
                        note: None,
                    },
                    Err(msg) => Row {
                        entry: e.clone(),
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| e.target.clone()),
                        names: Vec::new(),
                        base: String::new(),
                        latest: None,
                        flagged: false,
                        dir: None,
                        note: Some(msg),
                    },
                }
            }
        })
        .collect())
}

struct Ctx {
    d: Dirs,
    entries: Vec<Entry>,
}

impl Ctx {
    fn new() -> Result<Ctx, String> {
        let d = store::dirs()?;
        let entries = store::load(&d)?;
        Ok(Ctx { d, entries })
    }

    fn statuses(&self, rows: &[Row]) -> Vec<Status> {
        let repo = pkg::repo_versions(&self.d.repo);
        let inst = pkg::installed();
        rows.iter()
            .map(|r| {
                status(
                    r.entry.hold,
                    r.latest.as_deref(),
                    repo.get(&r.name).map(String::as_str),
                    inst.get(&r.name).map(String::as_str),
                    &util::vercmp,
                )
            })
            .collect()
    }

    /// index of the entry matching `name` (AUR name, local package name or directory name)
    fn find(&self, name: &str) -> Result<usize, String> {
        for (i, e) in self.entries.iter().enumerate() {
            let hit = match e.kind {
                Kind::Aur => e.target == name,
                Kind::Local => {
                    let p = PathBuf::from(&e.target);
                    p.file_name().is_some_and(|n| n == name)
                        || pkg::local_pkgdir(&p)
                            .and_then(|d| pkg::srcinfo(&d).ok())
                            .is_some_and(|s| s.names.iter().any(|n| n == name))
                }
            };
            if hit {
                return Ok(i);
            }
        }
        Err(format!("'{name}' is not tracked (see `pkgshelf list`)"))
    }
}

// ---------- commands ----------

fn add(c: &mut Ctx, kind: Kind, targets: Vec<String>) -> Result<(), String> {
    match kind {
        Kind::Aur => {
            if let Some(bad) = targets.iter().find(|t| !store::valid_aur_name(t)) {
                return Err(format!("invalid package name '{bad}'"));
            }
            let found: HashSet<String> = aur::info(&targets)?.into_iter().map(|i| i.name).collect();
            for t in &targets {
                if !found.contains(t) {
                    return Err(format!("'{t}' not found on the AUR"));
                }
            }
            for t in targets {
                if c.entries
                    .iter()
                    .any(|e| e.kind == Kind::Aur && e.target == t)
                {
                    println!("{t}: already tracked");
                } else {
                    println!("tracking {t} (AUR)");
                    c.entries.push(Entry {
                        kind,
                        target: t,
                        hold: false,
                    });
                }
            }
        }
        Kind::Local => {
            let path = fs::canonicalize(&targets[0]).map_err(|e| format!("{}: {e}", targets[0]))?;
            if pkg::local_pkgdir(&path).is_none() {
                if !path.join("Cargo.toml").exists() {
                    return Err(format!(
                        "{} has no PKGBUILD or pkg/PKGBUILD",
                        path.display()
                    ));
                }
                if !util::have("cratepkg") {
                    return Err("no PKGBUILD found and `cratepkg` is not installed (see github.com/lazybrar/cratepkg)".into());
                }
                util::run(Command::new("cratepkg").arg("init").arg(&path))?;
            }
            let dir = pkg::local_pkgdir(&path).ok_or("no PKGBUILD found")?;
            let src = pkg::srcinfo(&dir)?;
            let target = path.to_string_lossy().into_owned();
            if c.entries
                .iter()
                .any(|e| e.kind == Kind::Local && e.target == target)
            {
                println!("{}: already tracked", src.names[0]);
            } else {
                println!(
                    "tracking {} {} (local: {target})",
                    src.names[0], src.version
                );
                c.entries.push(Entry {
                    kind,
                    target,
                    hold: false,
                });
            }
        }
    }
    store::save(&c.d, &c.entries)?;
    println!("next: pkgshelf update   (then: sudo pacman -Syu)");
    Ok(())
}

fn rm(c: &mut Ctx, name: &str, purge: bool) -> Result<(), String> {
    let i = c.find(name)?;
    let e = c.entries.remove(i);
    let names = match e.kind {
        Kind::Aur => vec![e.target.clone()],
        Kind::Local => pkg::local_pkgdir(Path::new(&e.target))
            .and_then(|d| pkg::srcinfo(&d).ok())
            .map(|s| s.names)
            .unwrap_or_default(),
    };
    store::save(&c.d, &c.entries)?;
    println!("no longer tracking {name}");
    if purge {
        let db = c.d.repo.join(format!("{REPO}.db.tar.zst"));
        if db.exists() && !names.is_empty() {
            let _ = util::run(Command::new("repo-remove").arg(&db).args(&names));
        }
        for f in fs::read_dir(&c.d.repo).into_iter().flatten().flatten() {
            if f.file_name()
                .to_str()
                .and_then(pkg::file_parts)
                .is_some_and(|(n, _)| names.contains(&n))
            {
                let _ = fs::remove_file(f.path());
            }
        }
        println!(
            "purged {name} from the repo; it stays installed until you run `sudo pacman -R {name}`"
        );
    } else {
        println!("it stays in the repo and installed; use --purge to remove built files");
    }
    Ok(())
}

fn list(c: &Ctx) {
    if c.entries.is_empty() {
        println!("nothing tracked yet: pkgshelf add aur <name>  |  pkgshelf add local <path>");
    }
    for e in &c.entries {
        let k = if e.kind == Kind::Aur {
            "aur  "
        } else {
            "local"
        };
        println!("{k} {}{}", e.target, if e.hold { "  (hold)" } else { "" });
    }
}

fn label(s: &Status) -> &'static str {
    match s {
        Status::Build => "build (not built yet)",
        Status::Update => "update (run pkgshelf update)",
        Status::Install => "install (run sudo pacman -Syu)",
        Status::Ok => "ok",
        Status::Hold => "hold",
        Status::Missing => "missing",
    }
}

/// returns true if something needs `pkgshelf update`
fn check(c: &Ctx, quiet: bool) -> Result<bool, String> {
    let rows = resolve_rows(&c.entries)?;
    let sts = c.statuses(&rows);
    let repo = pkg::repo_versions(&c.d.repo);
    let inst = pkg::installed();
    if !quiet {
        let dash = |v: Option<&String>| v.cloned().unwrap_or_else(|| "-".into());
        let mut t = vec![[
            "NAME".to_string(),
            "INSTALLED".into(),
            "BUILT".into(),
            "LATEST".into(),
            "STATUS".into(),
        ]];
        for (r, s) in rows.iter().zip(&sts) {
            let mut st = label(s).to_string();
            if r.flagged {
                st += " [flagged out-of-date on AUR]";
            }
            if let Some(n) = &r.note {
                st += &format!(" ({n})");
            }
            t.push([
                r.name.clone(),
                dash(inst.get(&r.name)),
                dash(repo.get(&r.name)),
                dash(r.latest.as_ref()),
                st,
            ]);
        }
        let w: Vec<usize> = (0..5)
            .map(|i| t.iter().map(|r| r[i].chars().count()).max().unwrap_or(0))
            .collect();
        for r in &t {
            let cells: Vec<String> = (0..4).map(|i| format!("{:<1$}", r[i], w[i])).collect();
            println!("{}  {}", cells.join("  "), r[4]);
        }
        if rows.is_empty() {
            println!("nothing tracked yet");
        }
    }
    Ok(sts
        .iter()
        .any(|s| matches!(s, Status::Build | Status::Update)))
}

fn git(dir: &Path) -> Command {
    let mut g = Command::new("git");
    g.arg("-C").arg(dir).arg("--no-pager");
    g
}

fn sync_clone(c: &Ctx, base: &str) -> Result<PathBuf, String> {
    let dir = c.d.cache.join(base);
    if dir.join(".git").exists() {
        util::run(git(&dir).args(["pull", "--quiet", "--ff-only"]))?;
    } else {
        fs::create_dir_all(&c.d.cache)
            .map_err(|e| format!("cannot create {}: {e}", c.d.cache.display()))?;
        util::run(
            Command::new("git")
                .args([
                    "clone",
                    "--quiet",
                    &format!("https://aur.archlinux.org/{base}.git"),
                ])
                .arg(&dir),
        )?;
    }
    Ok(dir)
}

fn show_review(dir: &Path, rev: Option<&str>) -> Result<(), String> {
    let known = rev.is_some_and(|r| {
        git(dir)
            .args(["cat-file", "-e", r])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    if let (true, Some(r)) = (known, rev) {
        println!(
            "--- changes since your last review ({}) ---",
            &r[..r.len().min(10)]
        );
        return util::run(git(dir).args(["diff", r, "HEAD", "--", ".", ":(exclude).SRCINFO"]));
    }
    println!("--- first review: full contents ---");
    let listing = util::out(git(dir).arg("ls-files"))?;
    let (hidden, mut shown): (Vec<&str>, Vec<&str>) =
        listing.lines().partition(|f| f.starts_with('.'));
    shown.sort_by_key(|f| (*f != "PKGBUILD", !f.ends_with(".install"), *f));
    for f in shown {
        let text = util::out(git(dir).arg("show").arg(format!("HEAD:{f}")))?;
        println!("\n--- {f} ---");
        text.lines()
            .take(MAX_REVIEW_LINES)
            .for_each(|l| println!("{l}"));
        let more = text.lines().count().saturating_sub(MAX_REVIEW_LINES);
        if more > 0 {
            println!(
                "... {more} more lines (git -C {} show HEAD:{f})",
                dir.display()
            );
        }
    }
    if !hidden.is_empty() {
        println!("\n(dotfiles not shown: {})", hidden.join(", "));
    }
    Ok(())
}

fn build_into_repo(c: &Ctx, dir: &Path, names: &[String]) -> Result<(), String> {
    fs::create_dir_all(&c.d.repo)
        .map_err(|e| format!("cannot create {}: {e}", c.d.repo.display()))?;
    util::run(
        Command::new("makepkg")
            .arg("-sfc")
            .current_dir(dir)
            .env("PKGDEST", &c.d.repo),
    )?;
    let list = util::out(
        Command::new("makepkg")
            .arg("--packagelist")
            .current_dir(dir)
            .env("PKGDEST", &c.d.repo),
    )?;
    let files: Vec<&str> = list
        .lines()
        .filter(|p| Path::new(p).exists())
        .filter(|p| {
            Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(pkg::file_parts)
                .is_some_and(|(n, _)| names.contains(&n))
        })
        .collect();
    if files.is_empty() {
        return Err("makepkg produced no matching package files".into());
    }
    util::run(
        Command::new("repo-add")
            .arg("-R")
            .arg(c.d.repo.join(format!("{REPO}.db.tar.zst")))
            .args(files),
    )
}

fn build_aur(c: &Ctx, r: &Row, yes: bool) -> Result<(), String> {
    let dir = sync_clone(c, &r.base)?;
    let head = util::out(git(&dir).args(["rev-parse", "HEAD"]))?
        .trim()
        .to_string();
    let reviewed = store::load_reviewed(&c.d);
    if reviewed.get(&r.name) != Some(&head) {
        show_review(&dir, reviewed.get(&r.name).map(String::as_str))?;
        if !util::confirm(&format!("Build {} from this PKGBUILD?", r.name), yes) {
            return Err("declined".into());
        }
    }
    build_into_repo(c, &dir, &r.names).inspect_err(|_| {
        eprintln!("hint: if a dependency is missing from the repos, it may be on the AUR: pkgshelf add aur <dep>, then update again");
    })?;
    store::set_reviewed(&c.d, &r.name, &head)
}

fn update(c: &Ctx, names: &[String], yes: bool, force: bool) -> Result<bool, String> {
    let rows = resolve_rows(&c.entries)?;
    let sts = c.statuses(&rows);
    let mut targets: Vec<&Row> = Vec::new();
    if names.is_empty() {
        targets.extend(
            rows.iter()
                .zip(&sts)
                .filter(|(_, s)| matches!(s, Status::Build | Status::Update))
                .map(|(r, _)| r),
        );
    } else {
        for n in names {
            let r = &rows[c.find(n)?];
            let st = &sts[rows.iter().position(|x| std::ptr::eq(x, r)).unwrap_or(0)];
            if *st == Status::Ok && !force {
                println!("{}: already up to date (use --force to rebuild)", r.name);
            } else {
                targets.push(r);
            }
        }
    }
    if targets.is_empty() {
        println!("nothing to build");
        return Ok(true);
    }
    let (mut built, mut failed) = (0, Vec::new());
    for r in targets {
        let Some(latest) = &r.latest else {
            eprintln!(
                "{}: skipped ({})",
                r.name,
                r.note.as_deref().unwrap_or("unknown version")
            );
            failed.push(r.name.clone());
            continue;
        };
        println!("==> {} -> {latest}", r.name);
        let res = match (&r.entry.kind, &r.dir) {
            (Kind::Aur, _) => build_aur(c, r, yes),
            (Kind::Local, Some(dir)) => build_into_repo(c, dir, &r.names),
            (Kind::Local, None) => Err("no PKGBUILD".into()),
        };
        match res {
            Ok(()) => built += 1,
            Err(e) => {
                eprintln!("{}: {e}", r.name);
                failed.push(r.name.clone());
            }
        }
    }
    println!(
        "\nbuilt {built} package(s){}",
        if failed.is_empty() {
            String::new()
        } else {
            format!(", failed: {}", failed.join(", "))
        }
    );
    if built > 0 {
        println!("install with: sudo pacman -Syu");
    }
    Ok(failed.is_empty())
}

fn diff(c: &Ctx, name: &str) -> Result<(), String> {
    let e = &c.entries[c.find(name)?];
    if e.kind != Kind::Aur {
        return Err(format!(
            "{name} is a local package; there is nothing to review"
        ));
    }
    let rows = resolve_rows(std::slice::from_ref(e))?;
    let dir = sync_clone(c, &rows[0].base)?;
    show_review(
        &dir,
        store::load_reviewed(&c.d).get(name).map(String::as_str),
    )
}

fn hold(c: &mut Ctx, name: &str, on: bool) -> Result<(), String> {
    let i = c.find(name)?;
    c.entries[i].hold = on;
    store::save(&c.d, &c.entries)?;
    println!(
        "{name}: {}",
        if on {
            "held (skipped by update)"
        } else {
            "released"
        }
    );
    Ok(())
}

fn doctor(c: &Ctx) -> Result<bool, String> {
    let mut ok = true;
    for t in [
        "pacman",
        "makepkg",
        "repo-add",
        "repo-remove",
        "vercmp",
        "git",
        "curl",
    ] {
        let have = util::have(t);
        ok &= have;
        println!("{} {t}", if have { "ok     " } else { "MISSING" });
    }
    let conf = fs::read_to_string("/etc/pacman.conf").unwrap_or_default();
    if conf.lines().any(|l| l.trim() == format!("[{REPO}]")) {
        println!("ok      /etc/pacman.conf has [{REPO}]");
    } else {
        ok = false;
        println!(
            "MISSING [{REPO}] in /etc/pacman.conf; add this at the end (then run `sudo pacman -Sy`):\n"
        );
        println!(
            "[{REPO}]\nSigLevel = Optional TrustAll\nServer = file://{}\n",
            c.d.repo.display()
        );
    }
    println!(
        "{} repo db {}",
        if c.d.repo.join(format!("{REPO}.db")).exists() {
            "ok     "
        } else {
            "(none)"
        },
        c.d.repo.display()
    );

    let tracked: HashSet<String> = c
        .entries
        .iter()
        .map(|e| {
            if e.kind == Kind::Aur {
                e.target.clone()
            } else {
                Path::new(&e.target)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }
        })
        .collect();
    let foreign = Command::new("pacman")
        .arg("-Qm")
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let untracked: Vec<String> = foreign
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| !tracked.contains(*n))
        .map(String::from)
        .collect();
    if !untracked.is_empty() {
        let on_aur: HashSet<String> = aur::info(&untracked)
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.name)
            .collect();
        println!("\nforeign packages not tracked by pkgshelf (not updated by pacman):");
        for n in &untracked {
            println!(
                "  {n}{}",
                if on_aur.contains(n) {
                    "   -> pkgshelf add aur ".to_string() + n
                } else {
                    String::new()
                }
            );
        }
    }
    Ok(ok)
}

fn run(cmd: Cmd) -> Result<ExitCode, String> {
    let done = |ok: bool| {
        Ok(if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        })
    };
    match cmd {
        Cmd::Help => println!("{USAGE}"),
        Cmd::Version => println!("pkgshelf {VERSION}"),
        Cmd::Add { kind, targets } => add(&mut Ctx::new()?, kind, targets)?,
        Cmd::Rm { name, purge } => rm(&mut Ctx::new()?, &name, purge)?,
        Cmd::List => list(&Ctx::new()?),
        Cmd::Check { quiet } => {
            return Ok(if check(&Ctx::new()?, quiet)? {
                ExitCode::from(100)
            } else {
                ExitCode::SUCCESS
            });
        }
        Cmd::Diff { name } => diff(&Ctx::new()?, &name)?,
        Cmd::Update { names, yes, force } => {
            return done(update(&Ctx::new()?, &names, yes, force)?);
        }
        Cmd::Hold { name, on } => hold(&mut Ctx::new()?, &name, on)?,
        Cmd::Doctor => return done(doctor(&Ctx::new()?)?),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Err(e) => {
            eprintln!("pkgshelf: {e}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Ok(cmd) => run(cmd).unwrap_or_else(|e| {
            eprintln!("pkgshelf: {e}");
            ExitCode::from(1)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Result<Cmd, String> {
        parse_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn args() {
        assert_eq!(a(&[]), Ok(Cmd::Help));
        assert_eq!(
            a(&["add", "aur", "x", "y"]),
            Ok(Cmd::Add {
                kind: Kind::Aur,
                targets: vec!["x".into(), "y".into()]
            })
        );
        assert_eq!(
            a(&["add", "local", "/p"]),
            Ok(Cmd::Add {
                kind: Kind::Local,
                targets: vec!["/p".into()]
            })
        );
        assert!(
            a(&["add", "local", "a", "b"]).is_err()
                && a(&["add", "aur"]).is_err()
                && a(&["add", "x"]).is_err()
        );
        assert_eq!(
            a(&["rm", "x", "--purge"]),
            Ok(Cmd::Rm {
                name: "x".into(),
                purge: true
            })
        );
        assert_eq!(
            a(&["update", "--yes", "a"]),
            Ok(Cmd::Update {
                names: vec!["a".into()],
                yes: true,
                force: false
            })
        );
        assert_eq!(a(&["check", "--quiet"]), Ok(Cmd::Check { quiet: true }));
        assert_eq!(
            a(&["unhold", "x"]),
            Ok(Cmd::Hold {
                name: "x".into(),
                on: false
            })
        );
        assert!(a(&["list", "--x"]).is_err() && a(&["rm"]).is_err() && a(&["nope"]).is_err());
    }

    #[test]
    fn status_rules() {
        let cmp = |a: &str, b: &str| a.cmp(b);
        let s = |h, l, r, i| status(h, l, r, i, &cmp);
        assert_eq!(s(false, None, None, None), Status::Missing);
        assert_eq!(s(true, Some("2"), Some("1"), None), Status::Hold);
        assert_eq!(s(false, Some("1"), None, None), Status::Build);
        assert_eq!(s(false, Some("2"), Some("1"), Some("1")), Status::Update);
        assert_eq!(s(false, Some("2"), Some("2"), Some("1")), Status::Install);
        assert_eq!(s(false, Some("2"), Some("2"), Some("2")), Status::Ok);
        assert_eq!(s(false, Some("2"), Some("2"), None), Status::Ok);
    }
}
