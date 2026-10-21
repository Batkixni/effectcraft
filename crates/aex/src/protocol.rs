//! JSON-lines protocol shared with `effectcraft-helper-aex` (docs/aex.md).
//!
//! One JSON object per line. Pixels are standard base64 of straight 8-bit RGBA,
//! row-major, row 0 at the top. EffectCraft's frames are premultiplied float;
//! the bridge quantizes them to 8-bit and back. That is the whole pixel path.

use std::collections::HashSet;
use std::path::Path;

use effectcraft_effects::plugin::{PluginManifest, PluginParam};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Protocol version the host and `effectcraft-helper-aex` both speak.
pub const PROTOCOL: u32 = 1;

/// Largest frame edge, in pixels. A larger frame is refused before any byte is sent.
pub const MAX_EDGE: u32 = 4096;

/// Largest JSON line accepted from the helper (a 4096² RGBA frame in base64, plus a margin).
pub const MAX_MESSAGE: usize = 96 << 20;

/// One parameter as the helper described it, plus the EffectCraft parameter it becomes.
#[derive(Clone, Debug)]
pub struct WireParam {
    pub index: u64,
    /// `float`, `fixed`, `int`, `checkbox`, `popup`, `angle`, `point`, `color` or `point3`.
    pub aex: String,
    pub param: PluginParam,
}

/// One effect the helper described. `id` and parameter ids may be empty: the host fills them
/// in so every plug-in gets the same ids no matter which helper build described it.
#[derive(Clone, Debug)]
pub struct Described {
    pub id: String,
    pub name: String,
    pub category: String,
    pub version: String,
    pub path: String,
    pub params: Vec<WireParam>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Reply {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub protocol: u32,
    /// Effect objects, parsed by [`parse_effect`] (they carry both aexlo's index and an
    /// EffectCraft parameter, which serde cannot flatten in one step).
    #[serde(default)]
    pub effects: Vec<Value>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub pixels: String,
}

fn json_str(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// Read one effect object from a helper reply and fill any ids the helper left blank.
pub fn parse_effect(value: &Value) -> Result<Described, String> {
    let path = json_str(value, "path");
    if path.is_empty() {
        return Err("the helper described a plug-in without a path".into());
    }
    let mut params = Vec::new();
    if let Some(list) = value.get("params").and_then(Value::as_array) {
        for item in list {
            params.push(parse_param(item)?);
        }
    }
    let mut desc = Described {
        id: json_str(value, "id"),
        name: json_str(value, "name"),
        category: json_str(value, "category"),
        version: json_str(value, "version"),
        path,
        params,
    };
    finish_described(&mut desc)?;
    Ok(desc)
}

fn parse_param(value: &Value) -> Result<WireParam, String> {
    let index = value.get("index").and_then(Value::as_u64).ok_or_else(|| "a parameter is missing its index".to_string())?;
    let aex = json_str(value, "aex");
    // `index` and `aex` are not part of an EffectCraft parameter. serde ignores them.
    let param: PluginParam = serde_json::from_value(value.clone()).map_err(|e| format!("parameter {index}: {e}"))?;
    Ok(WireParam { index, aex, param })
}

/// Fold a display name into an effect-id component (`Deep Glow` → `deep-glow`).
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn generic_parent(s: &str) -> bool {
    matches!(s, "aex" | "plugins" | "plug-ins" | "plugin" | "effects" | "win64" | "win32" | "macos" | "contents" | "support-files")
}

/// Stable effect id for a plug-in file. The parent folder becomes part of the id when it is
/// a vendor folder, so two `Glow.aex` files from different vendors do not collide. The id does
/// not include the drive or the install location, so a project still finds the effect after
/// the plug-in folder moves.
pub fn effect_id(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = slug(&stem);
    let parent = slug(&parent);
    let stem = if stem.is_empty() { "plugin".to_string() } else { stem };
    let id = if parent.is_empty() || generic_parent(&parent) { format!("aex.{stem}") } else { format!("aex.{parent}.{stem}") };
    let mut id: String = id.chars().take(120).collect();
    while id.ends_with('.') || id.ends_with('-') {
        id.pop();
    }
    if id.len() < 5 {
        id = format!("aex.{stem}");
    }
    id
}

/// Stable parameter id. `used` disambiguates two parameters with the same name.
pub fn param_id(name: &str, index: usize, used: &mut HashSet<String>) -> String {
    let mut raw = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            raw.push(c.to_ascii_lowercase());
        } else if (c.is_ascii_whitespace() || c == '_' || c == '-') && !raw.ends_with('_') {
            raw.push('_');
        }
    }
    while raw.ends_with('_') {
        raw.pop();
    }
    let base = if raw.is_empty() { format!("p{index}") } else { raw };
    let mut id = if used.contains(&base) { format!("{base}_{index}") } else { base.clone() };
    let mut n = 2u32;
    while used.contains(&id) && n < 10_000 {
        id = format!("{base}_{index}_{n}");
        n += 1;
    }
    used.insert(id.clone());
    id
}

/// Fill empty ids. Returns an error when a parameter kind is not one this host can drive.
pub fn finish_described(desc: &mut Described) -> Result<(), String> {
    if desc.name.trim().is_empty() {
        desc.name = Path::new(&desc.path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Plug-in".into());
    }
    if desc.category.trim().is_empty() {
        desc.category = "After Effects".into();
    }
    if desc.id.trim().is_empty() {
        desc.id = effect_id(Path::new(&desc.path));
    }
    let mut used = HashSet::new();
    for p in &mut desc.params {
        if !known_kind(&p.aex) {
            return Err(format!("plug-in `{}`: parameter `{}` has an unknown kind `{}`", desc.name, p.param.name, p.aex));
        }
        if p.index == 0 {
            return Err(format!("plug-in `{}`: parameter `{}` uses the input-layer index", desc.name, p.param.name));
        }
        if p.param.id.trim().is_empty() {
            p.param.id = param_id(&p.param.name, p.index as usize, &mut used);
        } else if !used.insert(p.param.id.clone()) {
            return Err(format!("plug-in `{}`: duplicate parameter id `{}`", desc.name, p.param.id));
        }
        if p.param.name.trim().is_empty() {
            p.param.name = p.param.id.clone();
        }
    }
    Ok(())
}

pub fn known_kind(kind: &str) -> bool {
    matches!(kind, "float" | "fixed" | "int" | "checkbox" | "popup" | "angle" | "point" | "color" | "point3")
}

pub fn manifest_of(desc: &Described) -> Result<PluginManifest, String> {
    let manifest = PluginManifest {
        api: 2,
        id: desc.id.clone(),
        name: desc.name.clone(),
        category: desc.category.clone(),
        version: desc.version.clone(),
        author: String::new(),
        description: format!("After Effects plug-in, rendered out of process ({})", desc.path),
        params: desc.params.iter().map(|p| p.param.clone()).collect(),
    };
    manifest.validate()?;
    Ok(manifest)
}

/// The value the helper should apply for one parameter, in the helper's wire shape.
pub fn wire_value(kind: &str, index: u64, flat: &[f64]) -> Value {
    let num = |i: usize| flat.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
    match kind {
        "point" => json!({"index": index, "aex": kind, "x": num(0), "y": num(1)}),
        "point3" => json!({"index": index, "aex": kind, "x": num(0), "y": num(1), "z": num(2)}),
        "color" => json!({"index": index, "aex": kind, "r": num(0), "g": num(1), "b": num(2), "a": num(3)}),
        "checkbox" => json!({"index": index, "aex": kind, "value": if num(0) != 0.0 { 1.0 } else { 0.0 }}),
        "popup" | "int" => json!({"index": index, "aex": kind, "value": num(0).round()}),
        _ => json!({"index": index, "aex": kind, "value": num(0)}),
    }
}

/// Quantize premultiplied float pixels to 8-bit RGBA. Non-finite channels become 0.
pub fn frame_to_rgba8(pixels: &[[f32; 4]]) -> Result<Vec<u8>, String> {
    let n = pixels.len().checked_mul(4).ok_or_else(|| "frame is too large".to_string())?;
    let mut out = Vec::new();
    out.try_reserve(n).map_err(|_| "frame is too large".to_string())?;
    for px in pixels {
        for c in px {
            let v = if c.is_finite() { c.clamp(0.0, 1.0) } else { 0.0 };
            out.push((v * 255.0).round() as u8);
        }
    }
    Ok(out)
}

/// Write 8-bit RGBA back into a float frame. The lengths must match.
pub fn rgba8_to_frame(bytes: &[u8], pixels: &mut [[f32; 4]]) -> Result<(), String> {
    let need = pixels.len().checked_mul(4).ok_or_else(|| "frame is too large".to_string())?;
    if bytes.len() != need {
        return Err(format!("the helper returned {} bytes for {} pixels", bytes.len(), pixels.len()));
    }
    for (i, px) in pixels.iter_mut().enumerate() {
        let at = i.saturating_mul(4);
        for c in 0..4 {
            let b = bytes.get(at + c).copied().unwrap_or(0);
            px[c] = f32::from(b) / 255.0;
        }
    }
    Ok(())
}

pub fn b64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let groups = data.len() / 3;
    let extra = usize::from(!data.len().is_multiple_of(3));
    let _ = out.try_reserve(groups.saturating_mul(4).saturating_add(extra.saturating_mul(4)));
    let mut i = 0;
    while i + 3 <= data.len() {
        let (Some(&b0), Some(&b1), Some(&b2)) = (data.get(i), data.get(i + 1), data.get(i + 2)) else { break };
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        for shift in [18, 12, 6, 0] {
            let idx = ((n >> shift) & 63) as usize;
            out.push(T.get(idx).copied().unwrap_or(b'A') as char);
        }
        i += 3;
    }
    if let Some(rest) = data.get(i..) {
        if rest.len() == 1 {
            let n = u32::from(rest[0]) << 16;
            out.push(T.get(((n >> 18) & 63) as usize).copied().unwrap_or(b'A') as char);
            out.push(T.get(((n >> 12) & 63) as usize).copied().unwrap_or(b'A') as char);
            out.push('=');
            out.push('=');
        } else if rest.len() == 2 {
            let n = (u32::from(rest[0]) << 16) | (u32::from(rest[1]) << 8);
            out.push(T.get(((n >> 18) & 63) as usize).copied().unwrap_or(b'A') as char);
            out.push(T.get(((n >> 12) & 63) as usize).copied().unwrap_or(b'A') as char);
            out.push(T.get(((n >> 6) & 63) as usize).copied().unwrap_or(b'A') as char);
            out.push('=');
        }
    }
    out
}

fn b64_val(c: u8) -> Result<u8, String> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err("pixel data is not valid base64".into()),
    }
}

pub fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.len().is_multiple_of(4) {
        return Err("pixel data is not valid base64".into());
    }
    let mut out = Vec::new();
    out.try_reserve(bytes.len() / 4 * 3).map_err(|_| "pixel data is too large".to_string())?;
    for chunk in bytes.chunks_exact(4) {
        let a = *chunk.first().ok_or("pixel data is not valid base64")?;
        let b = *chunk.get(1).ok_or("pixel data is not valid base64")?;
        let c = *chunk.get(2).ok_or("pixel data is not valid base64")?;
        let d = *chunk.get(3).ok_or("pixel data is not valid base64")?;
        let (c_pad, d_pad) = (c == b'=', d == b'=');
        if a == b'=' || b == b'=' || (c_pad && !d_pad) {
            return Err("pixel data is not valid base64".into());
        }
        let n = (u32::from(b64_val(a)?) << 18)
            | (u32::from(b64_val(b)?) << 12)
            | (u32::from(if c_pad { 0 } else { b64_val(c)? }) << 6)
            | u32::from(if d_pad { 0 } else { b64_val(d)? });
        out.push((n >> 16) as u8);
        if !c_pad {
            out.push((n >> 8) as u8);
        }
        if !d_pad {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// Read one protocol line, refusing a response large enough to exhaust memory.
pub fn read_line_capped(reader: &mut impl std::io::BufRead) -> Result<String, String> {
    let mut buf = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|e| format!("the After Effects helper stopped: {e}"))?;
        if available.is_empty() {
            return Err("the After Effects helper stopped".into());
        }
        let newline = available.iter().position(|b| *b == b'\n');
        let take = newline.map_or(available.len(), |i| i.saturating_add(1));
        if buf.len().saturating_add(take) > MAX_MESSAGE {
            return Err("the helper response is too large".into());
        }
        if let Some(part) = available.get(..take) {
            buf.extend_from_slice(part);
        }
        reader.consume(take);
        if newline.is_some() {
            break;
        }
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    String::from_utf8(buf).map_err(|_| "the helper response is not UTF-8".to_string())
}

#[cfg(test)]
mod tests {
    use effectcraft_effects::plugin::PluginParamKind;

    use super::*;

    #[test]
    fn base64_roundtrip_and_padding() {
        assert_eq!(b64_encode(b""), "");
        assert_eq!(b64_decode("").unwrap(), b"");
        assert_eq!(b64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(b64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(b64_decode("aGVs bG8=").unwrap(), b"hello");
        let data: Vec<u8> = (0..50).collect();
        assert_eq!(b64_decode(&b64_encode(&data)).unwrap(), data);
        assert!(b64_decode("@@@@").is_err());
        assert!(b64_decode("abc").is_err());
        assert!(b64_decode("=abc").is_err());
    }

    #[test]
    fn ids_are_stable_and_unique() {
        assert_eq!(effect_id(Path::new(r"C:\AEX\Deep Glow.aex")), "aex.deep-glow");
        assert_eq!(effect_id(Path::new(r"D:\Plugins\Vendor\Deep Glow.aex")), "aex.vendor.deep-glow");
        assert_eq!(effect_id(Path::new("/tmp/plug-ins/Glow.plugin")), "aex.glow");
        let mut used = HashSet::new();
        assert_eq!(param_id("Amount", 1, &mut used), "amount");
        assert_eq!(param_id("Use GPU", 2, &mut used), "use_gpu");
        assert_eq!(param_id("", 4, &mut used), "p4");
        assert_eq!(param_id("Amount", 5, &mut used), "amount_5");
    }

    #[test]
    fn pixels_survive_8_bit() {
        let src = [[1.0, 0.0, 0.5, 1.0], [0.0, 1.0, 0.0, 0.0]];
        let bytes = frame_to_rgba8(&src).unwrap();
        assert_eq!(bytes, [255, 0, 128, 255, 0, 255, 0, 0]);
        let mut dst = [[0.0; 4]; 2];
        rgba8_to_frame(&bytes, &mut dst).unwrap();
        assert_eq!(dst[0][0], 1.0);
        assert_eq!(dst[0][2], 128.0 / 255.0);
        assert!(rgba8_to_frame(&[0, 0, 0], &mut dst).is_err());
        let bad = [[f32::NAN, f32::INFINITY, -1.0, 2.0]];
        assert_eq!(frame_to_rgba8(&bad).unwrap(), [0, 0, 0, 255]);
    }

    #[test]
    fn empty_ids_are_filled_and_checked() {
        let mut desc = Described {
            id: String::new(),
            name: String::new(),
            category: String::new(),
            version: String::new(),
            path: r"C:\AEX\Glow.aex".into(),
            params: vec![WireParam {
                index: 1,
                aex: "float".into(),
                param: PluginParam {
                    id: String::new(),
                    name: "Amount".into(),
                    group: String::new(),
                    kind: PluginParamKind::Slider { default: 1.0, min: 0.0, max: 2.0, slider_min: None, slider_max: None, decimals: 2, integer: false },
                },
            }],
        };
        finish_described(&mut desc).unwrap();
        assert_eq!(desc.id, "aex.glow");
        assert_eq!(desc.name, "Glow");
        assert_eq!(desc.category, "After Effects");
        assert_eq!(desc.params[0].param.id, "amount");
        manifest_of(&desc).unwrap();
        desc.params[0].aex = "layer".into();
        assert!(finish_described(&mut desc).is_err());
    }
}
