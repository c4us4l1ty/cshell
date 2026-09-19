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

/// Full cached summary (temp + desc + hum + forecast) for the popup.
pub fn cached_summary(city: &str) -> String {
    let text = fs::read_to_string(cache_path()).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    let temp = v.get("temp").and_then(|t| t.as_str()).unwrap_or("--");
    let desc = v.get("desc").and_then(|t| t.as_str()).unwrap_or("");
    let fc = v
        .get("forecast")
        .and_then(|f| f.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    if fc.is_empty() {
        format!("{} — {} {}", city, temp, desc)
    } else {
        format!("{} — {} {}\n{}", city, temp, desc, fc)
    }
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

/// Fetch now (blocking, user-action only): runs curl argv[0] with argv[1..],
/// extracts current temp, writes cache atomically. No shell, no interpolation.
pub fn fetch_now(city: &str) -> Result<String, String> {
    let argv = curl_argv(city).ok_or_else(|| "weather disabled (empty city)".to_string())?;
    let out = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err("curl failed (offline?)".to_string());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|_| "bad weather JSON".to_string())?;
    let cur = v.pointer("/current_condition/0").ok_or("no current_condition".to_string())?;
    let get = |k: &str| cur.get(k).and_then(|x| x.as_str()).unwrap_or("--").to_string();
    let temp = format!("{}°C (feels {})", get("temp_C"), get("FeelsLikeC"));
    let desc = cur
        .get("weatherDesc")
        .and_then(|d| d.get(0))
        .and_then(|d| d.get("value"))
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();
    let hum = format!("Humidity {}%", get("humidity"));
    // 3-day highs/lows (same role as QML WeatherPopup forecast strip)
    let mut forecast = vec![];
    if let Some(days) = v.get("weather").and_then(|w| w.as_array()) {
        for d in days.iter().take(3) {
            let date = d.get("date").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let hi = d.get("maxtempC").and_then(|x| x.as_str()).unwrap_or("--").to_string();
            let lo = d.get("mintempC").and_then(|x| x.as_str()).unwrap_or("--").to_string();
            forecast.push(format!("{} {}°/{}°", date, hi, lo));
        }
    }
    let cache = serde_json::json!({"temp": temp, "desc": desc, "hum": hum, "forecast": forecast});
    let p = cache_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = p.with_extension("tmp.json");
    std::fs::write(&tmp, serde_json::to_string(&cache).unwrap_or_default())
        .map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
    Ok(format!("{} — {} — {}\n{}", temp, desc, hum, forecast.join(" · ")))
}
