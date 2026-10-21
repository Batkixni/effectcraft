//! The protocol stand-in (`aex-mock-helper`). It speaks the same JSON as the real helper and
//! inverts pixels, so the host can be tested without a plug-in and without aexlo.

use serde_json::{Value, json};

use crate::protocol::{self, PROTOCOL};

pub enum Action {
    /// One JSON line to write back.
    Reply(String),
    /// The process should exit after this reply has been written. `None` means exit with no reply,
    /// which is how a crashing plug-in looks to the host.
    Exit(Option<String>),
}

fn err(message: &str) -> String {
    json!({"ok": false, "error": message}).to_string()
}

fn invert_effect(path: &str) -> Value {
    json!({
        "id": "aex.invert",
        "name": "Invert",
        "category": "After Effects",
        "version": "0",
        "path": path,
        "params": [{
            "index": 1,
            "aex": "float",
            "id": "mix",
            "name": "Mix",
            "group": "",
            "type": "slider",
            "default": 1.0,
            "min": 0.0,
            "max": 1.0,
            "decimals": 2,
            "integer": false
        }]
    })
}

/// Answer one request line.
pub fn reply(line: &str) -> Action {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Action::Reply(err("the request is not JSON")),
    };
    let op = value.get("op").and_then(Value::as_str).unwrap_or("");
    match op {
        "ping" => Action::Reply(json!({"ok": true, "protocol": PROTOCOL, "name": "aex-mock-helper"}).to_string()),
        "quit" => Action::Exit(Some(json!({"ok": true}).to_string())),
        "crash" => Action::Exit(None),
        "load" => {
            let path = value.get("path").and_then(Value::as_str).unwrap_or("");
            let name = std::path::Path::new(path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if name.eq_ignore_ascii_case("missing.aex") || !(name.to_ascii_lowercase().ends_with(".aex") || name.to_ascii_lowercase().ends_with(".plugin")) {
                return Action::Reply(err("cannot load this file as an After Effects plug-in"));
            }
            Action::Reply(json!({"ok": true, "effects": [invert_effect(path)]}).to_string())
        }
        "render" => {
            let path = value.get("path").and_then(Value::as_str).unwrap_or("");
            if std::path::Path::new(path).file_name().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("crash.aex")) {
                return Action::Exit(None);
            }
            let width = value.get("width").and_then(Value::as_u64).unwrap_or(0) as u32;
            let height = value.get("height").and_then(Value::as_u64).unwrap_or(0) as u32;
            let pixels = value.get("pixels").and_then(Value::as_str).unwrap_or("");
            let mix = value
                .get("params")
                .and_then(Value::as_array)
                .and_then(|params| params.iter().find(|p| p.get("index").and_then(Value::as_u64) == Some(1)))
                .and_then(|p| p.get("value").and_then(Value::as_f64))
                .unwrap_or(1.0);
            match render(width, height, pixels, mix) {
                Ok(pixels) => Action::Reply(json!({"ok": true, "width": width, "height": height, "pixels": pixels}).to_string()),
                Err(e) => Action::Reply(err(&e)),
            }
        }
        _ => Action::Reply(err("unknown request")),
    }
}

fn render(width: u32, height: u32, pixels: &str, mix: f64) -> Result<String, String> {
    let count = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or_else(|| "frame is too large".to_string())?;
    let mut bytes = protocol::b64_decode(pixels)?;
    if bytes.len() != count {
        return Err(format!("expected {count} bytes, got {}", bytes.len()));
    }
    if mix >= 0.5 {
        for b in &mut bytes {
            *b = 255 - *b;
        }
    }
    Ok(protocol::b64_encode(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_inverts_when_mix_is_high_and_refuses_a_bad_file() {
        let Action::Reply(pong) = reply(r#"{"op":"ping"}"#) else { panic!("ping") };
        assert!(pong.contains("\"protocol\":1"));
        let Action::Reply(bad) = reply(r#"{"op":"load","path":"missing.aex"}"#) else { panic!("load") };
        assert!(bad.contains("cannot load"));
        let Action::Reply(load) = reply(r#"{"op":"load","path":"invert.aex"}"#) else { panic!("load") };
        assert!(load.contains("aex.invert"));
        let pixels = protocol::b64_encode(&[255, 0, 0, 255]);
        let Action::Reply(frame) = reply(&format!(
            r#"{{"op":"render","path":"invert.aex","width":1,"height":1,"params":[{{"index":1,"aex":"float","value":1.0}}],"pixels":"{pixels}"}}"#
        )) else {
            panic!("render")
        };
        let value: Value = serde_json::from_str(&frame).unwrap();
        let out = protocol::b64_decode(value["pixels"].as_str().unwrap()).unwrap();
        assert_eq!(out, [0, 255, 255, 0]);
        let Action::Exit(None) = reply(r#"{"op":"crash"}"#) else { panic!("crash") };
    }
}
