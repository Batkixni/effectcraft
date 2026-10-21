//! A long-lived `effectcraft-helper-aex` process. One request at a time. If the process
//! dies, the next request starts it again; the plug-in that killed it comes back as an error
//! on that frame and the rest of EffectCraft keeps running.

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use serde_json::{Value, json};

use crate::protocol::{self, MAX_MESSAGE, PROTOCOL, Reply};

struct Proc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The helper program. Clone the `Arc` into every effect; they share one process.
pub struct Helper {
    path: PathBuf,
    proc: Mutex<Option<Proc>>,
}

impl Helper {
    /// Start `path` and require a protocol-1 greeting. The process is spawned on the first
    /// request, so a missing greeting fails here, before any plug-in is loaded.
    pub fn spawn(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Err(format!("effectcraft-helper-aex was not found at {}", path.display()));
        }
        let helper = Helper { path: path.to_path_buf(), proc: Mutex::new(None) };
        let pong = helper.exchange(&json!({"op": "ping"}))?;
        if !pong.ok || pong.protocol != PROTOCOL {
            return Err(format!("{} did not answer as effectcraft-helper-aex protocol {PROTOCOL}", path.display()));
        }
        Ok(helper)
    }

    /// Send one request and return the helper's JSON object.
    pub fn call(&self, request: &Value) -> Result<Value, String> {
        let reply = self.exchange(request)?;
        serde_json::to_value(&reply).map_err(|e| e.to_string())
    }

    pub(crate) fn exchange(&self, request: &Value) -> Result<Reply, String> {
        let mut guard = self.proc.lock().unwrap_or_else(PoisonError::into_inner);
        match self.transact(&mut guard, request) {
            Ok(reply) => Ok(reply),
            Err(e) => {
                log::warn!("After Effects helper stopped ({e}); starting it again");
                *guard = None;
                *guard = Some(self.start()?);
                match self.transact(&mut guard, request) {
                    Ok(reply) => Ok(reply),
                    Err(e2) => {
                        *guard = None;
                        Err(e2)
                    }
                }
            }
        }
    }

    /// Ask the helper to exit, then stop the process if it is still there.
    pub fn quit(&self) {
        let mut guard = self.proc.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(proc) = guard.as_mut() {
            let _ = writeln!(proc.stdin, r#"{{"op":"quit"}}"#);
            let _ = proc.stdin.flush();
        }
        *guard = None;
    }

    fn transact(&self, slot: &mut Option<Proc>, request: &Value) -> Result<Reply, String> {
        if slot.is_none() {
            *slot = Some(self.start()?);
        }
        let line = serde_json::to_string(request).map_err(|e| e.to_string())?;
        if line.len() > MAX_MESSAGE {
            return Err("this frame is too large to send to the helper (at most 4096×4096)".into());
        }
        let failed = {
            let proc = slot.as_mut().ok_or_else(|| "the After Effects helper is not running".to_string())?;
            writeln!(proc.stdin, "{line}").and_then(|_| proc.stdin.flush()).is_err()
        };
        if failed {
            return Err("the After Effects helper stopped before it could take the frame".into());
        }
        let text = {
            let proc = slot.as_mut().ok_or_else(|| "the After Effects helper is not running".to_string())?;
            protocol::read_line_capped(&mut proc.stdout)
        };
        let text = text?;
        serde_json::from_str(&text).map_err(|e| format!("the helper sent something that is not JSON: {e}"))
    }

    fn start(&self) -> Result<Proc, String> {
        let mut cmd = Command::new(&self.path);
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        // CPU render unless the user opts in. A missing GPU driver then cannot take the helper down.
        if std::env::var_os("EFFECTCRAFT_AEX_GPU").is_none() {
            cmd.env("AEXLO_DISABLE_GPU", "1");
        }
        let mut child = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", self.path.display()))?;
        let stdin = child.stdin.take().ok_or_else(|| "the helper has no standard input".to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "the helper has no standard output".to_string())?;
        Ok(Proc { child, stdin, stdout: BufReader::new(stdout) })
    }
}

fn slot() -> &'static Mutex<Option<Arc<Helper>>> {
    static SLOT: OnceLock<Mutex<Option<Arc<Helper>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// The helper for this process, started the first time a plug-in is loaded or rendered.
pub fn shared() -> Result<Arc<Helper>, String> {
    let mut guard = slot().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(helper) = guard.as_ref() {
        return Ok(Arc::clone(helper));
    }
    let path = crate::helper_path().ok_or_else(missing)?;
    let helper = Arc::new(Helper::spawn(&path)?);
    *guard = Some(Arc::clone(&helper));
    Ok(helper)
}

pub fn shutdown() {
    let mut guard = slot().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(helper) = guard.take() {
        helper.quit();
    }
}

fn missing() -> String {
    "effectcraft-helper-aex was not found. Build helpers/effectcraft-helper-aex and put the program next to EffectCraft, or set EFFECTCRAFT_AEX_HELPER to its path. See docs/aex.md.".into()
}
