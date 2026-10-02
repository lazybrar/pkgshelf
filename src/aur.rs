// SPDX-License-Identifier: GPL-2.0-or-later
use crate::util;
use std::process::Command;

#[derive(Debug, PartialEq)]
pub struct Info {
    pub name: String,
    pub version: String,
    pub base: String,
    pub flagged: bool,
}

fn field<'a>(chunk: &'a str, key: &str) -> Option<&'a str> {
    let start = chunk.find(&format!("\"{key}\":\""))? + key.len() + 4;
    chunk[start..].split('"').next()
}

/// Minimal reader for the AUR RPC v5 `info` response.
pub fn parse_info(json: &str) -> Vec<Info> {
    json.split("\"Name\":\"")
        .skip(1)
        .filter_map(|chunk| {
            let name = chunk.split('"').next()?.to_string();
            let flagged = chunk
                .find("\"OutOfDate\":")
                .is_some_and(|i| !chunk[i + 12..].starts_with("null"));
            Some(Info {
                version: field(chunk, "Version")?.to_string(),
                base: field(chunk, "PackageBase").unwrap_or(&name).to_string(),
                name,
                flagged,
            })
        })
        .collect()
}

pub fn info(names: &[String]) -> Result<Vec<Info>, String> {
    let mut all = Vec::new();
    for chunk in names.chunks(100) {
        let url = format!(
            "https://aur.archlinux.org/rpc/v5/info?{}",
            chunk
                .iter()
                .map(|n| format!("arg[]={n}"))
                .collect::<Vec<_>>()
                .join("&")
        );
        let body = util::out(Command::new("curl").args([
            "-fsSg",
            "--max-time",
            "30",
            "--proto",
            "=https",
            url.as_str(),
        ]))?;
        all.extend(parse_info(&body));
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rpc() {
        let j = r#"{"resultcount":2,"results":[{"ID":1,"Name":"a","OutOfDate":null,"PackageBase":"a-base","Version":"1:2.0-1"},{"ID":2,"Name":"b","OutOfDate":1700000000,"PackageBase":"b","Version":"3-2"}],"type":"multiinfo","version":5}"#;
        let v = parse_info(j);
        assert_eq!(v.len(), 2);
        assert_eq!(
            v[0],
            Info {
                name: "a".into(),
                version: "1:2.0-1".into(),
                base: "a-base".into(),
                flagged: false
            }
        );
        assert!(v[1].flagged && v[1].version == "3-2");
        assert!(parse_info(r#"{"resultcount":0,"results":[]}"#).is_empty());
    }
}
