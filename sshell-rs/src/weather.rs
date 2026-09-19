//! Weather: 6h file cache, HTTPS only, fetch on explicit signal.
//! No Timer polling. Empty city = disabled (no location leak).

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

pub fn cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".cache/sshell/weather.json")
}

pub fn cached_valid(max_age: Duration) -> bool {
    let p = cache_path();
    let meta = fs::metadata(&p).ok();
    let mtime = meta.and_then(|m| m.modified().ok());
    match mtime {
        Some(t) => SystemTime::now().duration_since(t).map(|d| d < max_age).unwrap_or(false),
        None => false,
    }
}

pub fn cached_temp() -> Option<String> {
    let text = fs::read_to_string(cache_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("temp").and_then(|t| t.as_str()).map(|s| s.to_string())
}

/// Build the fetch command (executed ONLY on user refresh or NM-connected signal).
/// Kept as data so app layer can spawn it without string interpolation RCE.
pub fn curl_argv(city: &str) -> Option<Vec<String>> {
    let city = city.trim();
    if city.is_empty() {
        return None;
    }
    let loc = city.split_whitespace().collect::<Vec<_>>().join("+");
    Some(vec![
        "curl".into(),
        "-m".into(),
        "10".into(),
        "-s".into(),
        format!("https://wttr.in/{}?format=j1", loc),
    ])
}
