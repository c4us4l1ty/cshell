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
