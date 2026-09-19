//! Theming: matugen dry-run JSON -> GTK accent CSS (same role as QML Appearance).
//! Full template pass (gtk.css/hypr colors) runs in wallpaper::apply when the
//! repo's matugen templates are present. This module only handles the live
//! in-process accent: re-read on every Refresh event (file read, no fork).

use std::path::PathBuf;

pub fn theme_json_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".config/sshell/material-theme.json")
}

/// Deep hex search under a `primary` subtree (handles real matugen shape
/// {"primary":{"default":{"hex":"#D0BCFF"}}} where leaves are objects, not strings).
fn deep_hex(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if s.starts_with('#') => Some(s.clone()),
        serde_json::Value::Object(m) => {
            // prefer hex/default leaves first (stable accent, not random nested key order)
            for k in ["hex", "default", "dark", "light"] {
                if let Some(sub) = m.get(k) {
                    if let Some(s) = deep_hex(sub) {
                        return Some(s);
                    }
                }
            }
            for (_, sub) in m {
                if let Some(s) = deep_hex(sub) {
                    return Some(s);
                }
            }
            None
        }
        serde_json::Value::Array(a) => a.iter().find_map(deep_hex),
        _ => None,
    }
}

/// Recursively find `key` subtree then deep-search hex inside it.
fn find_key(v: &serde_json::Value, key: &str) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => {
            for (k, val) in m {
                if k == key {
                    if let Some(s) = val.as_str() {
                        if s.starts_with('#') {
                            return Some(s.to_string());
                        }
                    }
                    if let Some(s) = deep_hex(val) {
                        return Some(s);
                    }
                }
                if let Some(found) = find_key(val, key) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

pub fn accent() -> Option<String> {
    let text = std::fs::read_to_string(theme_json_path()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    find_key(&v, "primary")
}

/// Reload the accent into an existing provider (called on Refresh).
/// Reuses the provider so repeated refreshes don't stack providers.
pub fn reload_into(provider: &gtk4::CssProvider, base_css: &str) {
    let css = match accent() {
        Some(a) => format!(
            "{}\n.sshell-accent {{ color: {}; }}\n.sshell-bar {{ border-color: {}; }}",
            base_css, a, a
        ),
        None => base_css.to_string(),
    };
    provider.load_from_data(&css);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_real_matugen_shape() {
        let v: serde_json::Value = serde_json::from_str(
            r##"{"colors":{"primary":{"default":{"hex":"#D0BCFF"}}}}"##,
        )
        .unwrap();
        assert_eq!(find_key(&v, "primary"), Some("#D0BCFF".into()));
    }
    #[test]
    fn finds_nested_primary() {
        let v: serde_json::Value = serde_json::from_str(
            r##"{"colors":{"primary":{"default":"#D0BCFF"}}}"##,
        )
        .unwrap();
        assert_eq!(find_key(&v, "primary"), Some("#D0BCFF".into()));
    }
    #[test]
    fn finds_flat_primary() {
        let v: serde_json::Value =
            serde_json::from_str(r##"{"primary":"#123456"}"##).unwrap();
        assert_eq!(find_key(&v, "primary"), Some("#123456".into()));
    }
    #[test]
    fn missing_is_none() {
        let v: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
        assert_eq!(find_key(&v, "primary"), None);
    }
}
