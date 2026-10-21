//! Finding `.aex` files and `.plugin` bundles, and turning a helper reply into registered effects.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::client::Helper;
use crate::effect::register_effects;
use crate::protocol::Reply;

const MAX_PLUGINS: usize = 2048;
const MAX_DIRS: u32 = 10_000;
const MAX_DEPTH: u32 = 8;

fn is_aex(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("aex")) && path.is_file()
}

fn is_bundle(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("plugin")) && path.is_dir()
}

/// Plug-in files under `root` (or `root` itself when it is one plug-in). Depth and count are
/// capped so a link loop or a huge tree cannot hang the scan.
pub fn collect_plugins(root: &Path) -> Result<Vec<PathBuf>, String> {
    if is_aex(root) || is_bundle(root) {
        return Ok(vec![root.to_path_buf()]);
    }
    if !root.is_dir() {
        return Err(format!("cannot find {}", root.display()));
    }
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0u32)];
    let mut seen = HashSet::new();
    let mut dirs = 0u32;
    while let Some((dir, depth)) = stack.pop() {
        dirs = dirs.saturating_add(1);
        if dirs > MAX_DIRS || out.len() >= MAX_PLUGINS {
            break;
        }
        if let Ok(canon) = dir.canonicalize()
            && !seen.insert(canon)
        {
            continue;
        }
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if dir == root => return Err(format!("cannot list {}: {e}", dir.display())),
            Err(_) => continue,
        };
        for entry in rd.flatten() {
            if out.len() >= MAX_PLUGINS {
                break;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if is_aex(&path) || is_bundle(&path) {
                out.push(path);
            } else if depth < MAX_DEPTH && entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((path, depth.saturating_add(1)));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn reply_error(reply: &Reply) -> Result<(), String> {
    if reply.ok {
        Ok(())
    } else if reply.error.is_empty() {
        Err("the helper could not load this plug-in".into())
    } else {
        Err(reply.error.clone())
    }
}

/// Load one plug-in, or every plug-in under a folder, and register what described cleanly.
pub fn load_path(helper: &Arc<Helper>, path: &Path) -> Result<Value, String> {
    let files = collect_plugins(path)?;
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    for file in &files {
        match helper.exchange(&json!({"op": "load", "path": file.to_string_lossy()})) {
            Ok(reply) => match reply_error(&reply) {
                Ok(()) => {
                    let mut described = Vec::new();
                    for item in &reply.effects {
                        match crate::protocol::parse_effect(item) {
                            Ok(desc) => described.push(desc),
                            Err(e) => errors.push(format!("{}: {e}", file.display())),
                        }
                    }
                    let (ok, bad) = register_effects(Arc::clone(helper), &described);
                    if ok.is_empty() && described.is_empty() && errors.is_empty() {
                        errors.push(format!("{}: the file is not an After Effects plug-in this helper can describe", file.display()));
                    }
                    loaded.extend(ok);
                    errors.extend(bad);
                }
                Err(e) => errors.push(format!("{}: {e}", file.display())),
            },
            Err(e) => errors.push(format!("{}: {e}", file.display())),
        }
    }
    if loaded.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(json!({"loaded": loaded, "errors": errors}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_finds_aex_files_and_stops_at_a_bundle() {
        let root = std::env::temp_dir().join(format!("ec-aex-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("vendor").join("nested")).unwrap();
        std::fs::write(root.join("vendor").join("Glow.aex"), b"not a real plugin").unwrap();
        std::fs::write(root.join("vendor").join("nested").join("note.txt"), b"skip").unwrap();
        std::fs::write(root.join("vendor").join("nested").join("Blur.AEX"), b"x").unwrap();
        std::fs::create_dir_all(root.join("mac").join("Thing.plugin").join("Contents")).unwrap();
        std::fs::write(root.join("mac").join("Thing.plugin").join("Contents").join("hidden.aex"), b"x").unwrap();
        let found = collect_plugins(&root).unwrap();
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        assert!(names.contains(&"Glow.aex".to_string()), "{names:?}");
        assert!(names.contains(&"Blur.AEX".to_string()), "{names:?}");
        assert!(names.contains(&"Thing.plugin".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n == "hidden.aex"), "{names:?}");
        assert!(collect_plugins(&root.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
