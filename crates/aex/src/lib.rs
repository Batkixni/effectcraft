//! After Effects plug-ins (`.aex`, and macOS `.plugin` bundles) as ordinary EffectCraft effects.
//!
//! This crate does not load a plug-in. It finds the separate program `effectcraft-helper-aex`
//! (see `helpers/effectcraft-helper-aex` and [docs/aex.md](../../../docs/aex.md)), sends it the
//! layer and the parameter values as JSON, and registers each described effect through plug-in
//! API 2. A plug-in that crashes takes the helper down. The next frame starts a new one, and
//! the document stays open.
//!
//! The helper is built on [aexlo](https://github.com/potistudio/aexlo), which is still changing.
//! A plug-in can load and still fail because a suite, a licence check or a piece of custom UI
//! is missing. Pixels are exchanged as 8-bit RGBA, so a frame is slower than the same effect
//! running inside the process and it is quantized to 8 bits.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod client;
mod effect;
mod mock;
mod protocol;
mod scan;

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

pub use client::Helper;
pub use mock::{Action as MockAction, reply as mock_reply};

/// Where a plug-in scan looks, in order: every folder in `AEX_PLUGIN_PATH` (`;` on Windows, `:`
/// elsewhere), then an `AEX` folder next to the program. Adobe's own install folders are not
/// on this list. Point `AEX_PLUGIN_PATH` at a folder of plug-ins you want EffectCraft to see.
pub fn default_search_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(raw) = std::env::var_os("AEX_PLUGIN_PATH") {
        let raw = raw.to_string_lossy();
        let sep = if cfg!(windows) { ';' } else { ':' };
        for part in raw.split(sep) {
            let part = part.trim();
            if !part.is_empty() {
                out.push(PathBuf::from(part));
            }
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        out.push(dir.join("AEX"));
    }
    out
}

/// The helper program: `EFFECTCRAFT_AEX_HELPER` when that file exists, otherwise
/// `effectcraft-helper-aex` next to this program.
pub fn helper_path() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("EFFECTCRAFT_AEX_HELPER") {
        let path = PathBuf::from(raw);
        if path.is_file() {
            return Some(path);
        }
    }
    let name = if cfg!(windows) { "effectcraft-helper-aex.exe" } else { "effectcraft-helper-aex" };
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(name);
    candidate.is_file().then_some(candidate)
}

/// Whether start-up should scan. False when the helper program is not installed, or when
/// `EFFECTCRAFT_NO_AEX=1`.
pub fn configured() -> bool {
    if std::env::var_os("EFFECTCRAFT_NO_AEX").is_some_and(|v| v == "1") {
        return false;
    }
    helper_path().is_some()
}

/// Engine hook for `effect.plugins.loadAex`: load one `.aex`, one `.plugin` bundle, or every
/// plug-in under a folder. `{loaded, errors}`. `Err` when nothing loaded and something failed.
pub fn loader(path: &str) -> Result<Value, String> {
    let path = std::path::Path::new(path);
    // A missing file fails before the helper is started, so a typo does not launch a process.
    if !path.exists() {
        return Err(format!("cannot find {}", path.display()));
    }
    let helper = client::shared()?;
    scan::load_path(&helper, path)
}

/// Stop the helper process, if this process started one.
pub fn shutdown() {
    client::shutdown();
}

/// Register the effects in a helper `load` reply.
pub fn register_reply(helper: Arc<Helper>, reply: &Value) -> Result<Value, String> {
    let reply: protocol::Reply = serde_json::from_value(reply.clone()).map_err(|e| e.to_string())?;
    if !reply.ok {
        return Err(if reply.error.is_empty() { "the helper could not load this plug-in".into() } else { reply.error });
    }
    let mut described = Vec::new();
    let mut errors = Vec::new();
    for item in &reply.effects {
        match protocol::parse_effect(item) {
            Ok(desc) => described.push(desc),
            Err(e) => errors.push(e),
        }
    }
    let (loaded, bad) = effect::register_effects(helper, &described);
    errors.extend(bad);
    if loaded.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(serde_json::json!({"loaded": loaded, "errors": errors}))
}
