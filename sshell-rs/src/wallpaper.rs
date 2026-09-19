//! Wallpaper: scan once, thumbnails via `image` crate (no convert fork per file).
//! matugen runs ONLY on setWallpaper to a different file. Atomic writes.

use std::fs;
use std::path::{Path, PathBuf};

pub fn thumb_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".cache/sshell/thumbnails")
}

fn is_img(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()).as_deref(),
        Some("jpg") | Some("jpeg") | Some("png") | Some("webp") | Some("gif")
    )
}

pub fn scan(paths: &[String]) -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    let mut out = Vec::new();
    for raw in paths {
        let expanded = raw.replace('~', &home);
        let root = PathBuf::from(&expanded);
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let rd = match fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if is_img(&p) {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// 256px thumbnail path for source; generates with `image` crate on demand.
pub fn thumb_for(src: &Path) -> PathBuf {
    // stable hash from path (fnv1a, no md5 fork)
    let mut h: u64 = 0xcbf29ce484222325;
    for b in src.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    thumb_dir().join(format!("{:016x}.jpg", h))
}

pub fn ensure_thumb(src: &Path) -> Option<PathBuf> {
    let dst = thumb_for(src);
    if dst.exists() {
        return Some(dst);
    }
    let _ = fs::create_dir_all(thumb_dir());
    let img = image::open(src).ok()?;
    // first frame for gif handled by image crate (static); animated kept as source
    let t = img.resize_to_fill(256, 256, image::imageops::FilterType::Triangle);
    // atomic: write tmp + rename
    let tmp = dst.with_extension("tmp.jpg");
    t.save(&tmp).ok()?;
    fs::rename(&tmp, &dst).ok()?;
    Some(dst)
}

fn state_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
    PathBuf::from(home).join(".local/state/sshell/wallpaper/current")
}

/// Toggle wallpaper visibility (same as QML backgroundToggle): unload from
/// hyprpaper to black, or re-apply persisted wallpaper. User action only.
pub fn toggle_visible() -> bool {
    static VISIBLE: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(true);
    let now_visible = !VISIBLE.load(std::sync::atomic::Ordering::Relaxed);
    VISIBLE.store(now_visible, std::sync::atomic::Ordering::Relaxed);
    if now_visible {
        if let Ok(cur) = fs::read_to_string(state_path()) {
            let cur = cur.trim().to_string();
            if !cur.is_empty() {
                let _ = std::process::Command::new("hyprctl")
                    .args(["hyprpaper", "preload", &cur])
                    .output();
                let _ = std::process::Command::new("hyprctl")
                    .args(["hyprpaper", "wallpaper", &format!(",{}", cur)])
                    .output();
            }
        }
    } else {
        let _ = std::process::Command::new("hyprctl")
            .args(["hyprpaper", "unload", "all"])
            .output();
    }
    now_visible
}

/// Apply wallpaper: persist state atomically, run matugen ONCE (only if changed),
/// reload hyprpaper. Called on user click only — never background.
pub fn apply(src: &Path) -> Result<String, String> {
    let srcs = src.to_string_lossy().to_string();
    // skip theme regen if unchanged (hash compare with state file)
    let prev = fs::read_to_string(state_path()).unwrap_or_default();
    let changed = prev.trim() != srcs;
    if let Some(dir) = state_path().parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tmp = state_path().with_extension("tmp");
    fs::write(&tmp, format!("{}\n", srcs)).map_err(|e| e.to_string())?;
    fs::rename(&tmp, state_path()).map_err(|e| e.to_string())?;

    // hyprpaper reload (one fork per user click; ignored if hyprpaper unused)
    let _ = std::process::Command::new("hyprctl")
        .args(["hyprpaper", "preload", &srcs])
        .output();
    let _ = std::process::Command::new("hyprctl")
        .args(["hyprpaper", "wallpaper", &format!(",{}", srcs)])
        .output();

    if !changed {
        return Ok("unchanged (theme kept)".into());
    }
    // matugen once per new wallpaper (user action; skipped if binary absent):
    // 1) dry-run JSON for our live accent (same as QML generate_colors.sh path),
    // 2) full pass so repo templates (gtk.css/hypr colors) regenerate if configured.
    if which("matugen") {
        let out = std::process::Command::new("matugen")
            .args(["image", &srcs, "-t", "scheme-vibrant", "--json", "hex", "--dry-run"])
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err("matugen failed".into());
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        let dest = PathBuf::from(&home).join(".config/sshell/material-theme.json");
        let tmpd = dest.with_extension("tmp.json");
        fs::write(&tmpd, &out.stdout).map_err(|e| e.to_string())?;
        fs::rename(&tmpd, &dest).map_err(|e| e.to_string())?;
        let templates = PathBuf::from(&home).join(".config/matugen/templates");
        if templates.is_dir() {
            // best-effort full template pass (slow ~s, still one user click)
            let _ = std::process::Command::new("matugen")
                .args(["image", &srcs, "-t", "scheme-vibrant"])
                .output();
        }
        return Ok("theme regenerated".into());
    }
    Ok("matugen not installed (wallpaper set)".into())
}

fn which(bin: &str) -> bool {
    // PATH scan without forks (called on user click only, still keep it clean)
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into());
    path.split(':').any(|d| {
        let p = PathBuf::from(d).join(bin);
        std::fs::metadata(&p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}
