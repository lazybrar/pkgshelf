// SPDX-License-Identifier: GPL-2.0-or-later
use std::cmp::Ordering;
use std::io::{self, BufRead, Write};
use std::process::{Command, Stdio};

fn prog(c: &Command) -> String {
    c.get_program().to_string_lossy().into_owned()
}

/// Run a command and capture stdout; stderr goes to the terminal.
pub fn out(cmd: &mut Command) -> Result<String, String> {
    let o = cmd
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", prog(cmd)))?;
    if !o.status.success() {
        return Err(format!("{} failed ({})", prog(cmd), o.status));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Run a command with the terminal attached.
pub fn run(cmd: &mut Command) -> Result<(), String> {
    let s = cmd
        .status()
        .map_err(|e| format!("cannot run {}: {e}", prog(cmd)))?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("{} failed ({s})", prog(cmd)))
    }
}

/// Compare two package versions with pacman's own `vercmp`.
pub fn vercmp(a: &str, b: &str) -> Ordering {
    match out(Command::new("vercmp").args([a, b])) {
        Ok(s) => match s.trim() {
            "-1" => Ordering::Less,
            "1" => Ordering::Greater,
            _ => Ordering::Equal,
        },
        Err(_) => a.cmp(b),
    }
}

pub fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

pub fn confirm(q: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    eprint!("{q} [y/N] ");
    let _ = io::stderr().flush();
    let mut s = String::new();
    io::stdin().lock().read_line(&mut s).is_ok()
        && matches!(s.trim().to_lowercase().as_str(), "y" | "yes")
}
