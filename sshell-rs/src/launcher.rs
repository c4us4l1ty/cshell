//! Launcher: scan .desktop once + inotify, fuzzy in-memory.
//! Clipboard: read cliphist ONLY on Super+V open. No background poll.

use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub exec: String,
    pub icon: String,
    pub desktop_id: String,
}

pub fn app_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    vec![
        PathBuf::from(&home).join(".local/share/applications"),
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from(&home).join(".local/share/flatpak/exports/share/applications"),
    ]
}

/// Minimal .desktop parse (Name/Exec/Icon, skip NoDisplay). No shell interpolation.
pub fn scan() -> Vec<Entry> {
    let mut out = Vec::new();
    for dir in app_dirs() {
        let rd = match fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }
            let text = match fs::read_to_string(&p) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let mut name = String::new();
            let mut exec = String::new();
            let mut icon = String::new();
            let mut nodisplay = false;
            for line in text.lines() {
                let line = line.trim();
                if let Some(v) = line.strip_prefix("Name=") {
                    if name.is_empty() {
                        name = v.to_string();
                    }
                } else if let Some(v) = line.strip_prefix("Exec=") {
                    if exec.is_empty() {
                        // strip %U/%F field codes (no shell)
                        exec = v.split_whitespace().next().unwrap_or("").to_string();
                    }
                } else if let Some(v) = line.strip_prefix("Icon=") {
                    if icon.is_empty() {
                        icon = v.to_string();
                    }
                } else if line == "NoDisplay=true" {
                    nodisplay = true;
                }
            }
            if nodisplay || name.is_empty() || exec.is_empty() {
                continue;
            }
            out.push(Entry {
                name,
                exec,
                icon,
                desktop_id: p.file_name().unwrap_or_default().to_string_lossy().to_string(),
            });
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.dedup_by(|a, b| a.desktop_id == b.desktop_id);
    out
}

/// Subsequence fuzzy match (case-insensitive). Fast enough for 300+ apps <100ms.
pub fn fuzzy<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return vec![];
    }
    let mut scored: Vec<(i32, &Entry)> = entries
        .iter()
        .filter_map(|e| {
            let n = e.name.to_lowercase();
            if n.contains(&q) {
                let score = if n.starts_with(&q) { 0 } else { 1 };
                Some((score, e))
            } else {
                // subsequence fallback
                let mut qi = q.chars();
                let mut cur = qi.next()?;
                for c in n.chars() {
                    if c == cur {
                        if let Some(nc) = qi.next() {
                            cur = nc;
                        } else {
                            cur = '\0';
                            break;
                        }
                    }
                }
                if cur == '\0' {
                    Some((2, e))
                } else {
                    None
                }
            }
        })
        .collect();
    scored.sort_by_key(|(s, e)| (*s, e.name.len()));
    scored.into_iter().map(|(_, e)| e).take(20).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fuzzy_prefix_first() {
        let v = vec![
            Entry { name: "Firefox".into(), exec: "firefox".into(), icon: "".into(), desktop_id: "a".into() },
            Entry { name: "Terminal".into(), exec: "foot".into(), icon: "".into(), desktop_id: "b".into() },
        ];
        let r = fuzzy(&v, "fire");
        assert_eq!(r[0].name, "Firefox");
    }
}
