//! `effectcraft-helper-aex`: load an After Effects plug-in with aexlo and render one frame.
//!
//! EffectCraft talks to this program on stdin/stdout, one JSON object per line (docs/aex.md).
//! This program is the only place a `.aex` is loaded. When a plug-in crashes, this process
//! ends and EffectCraft starts another one for the next frame.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use aexlo::{Depth8, Host, Layer, ParamKind, ParamValue, PluginInstance};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};

const PROTOCOL: u32 = 1;
const MAX_EDGE: u32 = 4096;
const MAX_BYTES: usize = 4096 * 4096 * 4;
const MAX_PLUGINS: usize = 64;
const MAX_INSTANCES: usize = 8;
const MAX_PARAMS: usize = 1024;

struct RenderHost;

impl aexlo::AppHost for RenderHost {
    fn is_render_engine(&self) -> bool {
        true
    }
    fn language(&self) -> String {
        "en_US".into()
    }
}

#[derive(Deserialize)]
struct Request {
    op: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    instance: u64,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    time: f64,
    #[serde(default = "default_fps")]
    fps: f64,
    #[serde(default)]
    params: Vec<ParamSet>,
    #[serde(default)]
    pixels: String,
}

fn default_fps() -> f64 {
    30.0
}

#[derive(Deserialize)]
struct ParamSet {
    index: usize,
    aex: String,
    #[serde(default)]
    value: f64,
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    z: f64,
    #[serde(default)]
    r: f64,
    #[serde(default)]
    g: f64,
    #[serde(default)]
    b: f64,
    #[serde(default = "one")]
    a: f64,
}

fn one() -> f64 {
    1.0
}

struct Catalog {
    described: HashMap<String, Value>,
    instances: HashMap<(String, u64), PluginInstance>,
    quit: bool,
}

fn main() {
    if Host::install(RenderHost).is_err() {
        let _ = Host::get();
    }
    if let Err(e) = serve() {
        eprintln!("effectcraft-helper-aex: {e}");
        std::process::exit(1);
    }
}

fn serve() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut incoming = stdin.lock().lines();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let mut catalog = Catalog { described: HashMap::new(), instances: HashMap::new(), quit: false };
    while let Some(line) = incoming.next() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = catch_unwind(AssertUnwindSafe(|| catalog.handle(&line))).unwrap_or_else(|_| err("the plug-in panicked"));
        writeln!(out, "{reply}").map_err(|e| e.to_string())?;
        out.flush().map_err(|e| e.to_string())?;
        if catalog.quit {
            break;
        }
    }
    Ok(())
}

fn err(message: &str) -> String {
    json!({"ok": false, "error": message}).to_string()
}

impl Catalog {
    fn handle(&mut self, line: &str) -> String {
        let request: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => return err(&format!("the request is not JSON: {e}")),
        };
        match request.op.as_str() {
            "ping" => json!({"ok": true, "protocol": PROTOCOL, "name": "effectcraft-helper-aex"}).to_string(),
            "quit" => {
                self.quit = true;
                json!({"ok": true}).to_string()
            }
            "load" => match self.load(&request.path) {
                Ok(effect) => json!({"ok": true, "effects": [effect]}).to_string(),
                Err(e) => err(&e),
            },
            "render" => match self.render(&request) {
                Ok(pixels) => json!({"ok": true, "width": request.width, "height": request.height, "pixels": pixels}).to_string(),
                Err(e) => err(&e),
            },
            _ => err("unknown request"),
        }
    }

    fn load(&mut self, path: &str) -> Result<Value, String> {
        if path.is_empty() {
            return Err("missing plug-in path".into());
        }
        if let Some(desc) = self.described.get(path) {
            return Ok(desc.clone());
        }
        if self.described.len() >= MAX_PLUGINS {
            return Err(format!("too many plug-ins are loaded (at most {MAX_PLUGINS}); restart EffectCraft"));
        }
        let file = PathBuf::from(path);
        if !file.exists() {
            return Err(format!("cannot find {}", file.display()));
        }
        let instance = load_instance(&file)?;
        let desc = describe(&instance, &file)?;
        self.described.insert(path.to_string(), desc.clone());
        Ok(desc)
    }

    fn render(&mut self, request: &Request) -> Result<String, String> {
        if request.path.is_empty() {
            return Err("missing plug-in path".into());
        }
        if request.width == 0 || request.height == 0 || request.width > MAX_EDGE || request.height > MAX_EDGE {
            return Err(format!("frames must be between 1 and {MAX_EDGE} pixels on each side"));
        }
        let count = (request.width as usize).checked_mul(request.height as usize).and_then(|n| n.checked_mul(4)).ok_or_else(|| "frame is too large".to_string())?;
        if count > MAX_BYTES {
            return Err("frame is too large".into());
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(request.pixels.trim()).map_err(|_| "pixel data is not valid base64".to_string())?;
        if bytes.len() != count {
            return Err(format!("expected {count} bytes of RGBA, got {}", bytes.len()));
        }
        let key = (request.path.clone(), request.instance);
        if !self.instances.contains_key(&key) {
            self.make_room(&key);
            let instance = load_instance(Path::new(&request.path))?;
            self.instances.insert(key.clone(), instance);
        }
        let Some(mut instance) = self.instances.remove(&key) else {
            return Err("the plug-in could not be kept loaded".into());
        };
        // The instance is out of the map, so a panic drops it instead of leaving a broken one behind.
        let rendered = catch_unwind(AssertUnwindSafe(|| render_instance(&mut instance, request, bytes)));
        match rendered {
            Ok(Ok(pixels)) => {
                self.instances.insert(key, instance);
                Ok(pixels)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the plug-in panicked while rendering".into()),
        }
    }

    fn make_room(&mut self, keep: &(String, u64)) {
        while self.instances.len() >= MAX_INSTANCES {
            let Some(drop_key) = self.instances.keys().find(|k| *k != keep).cloned() else { break };
            self.instances.remove(&drop_key);
        }
    }
}

fn load_instance(path: &Path) -> Result<PluginInstance, String> {
    Host::get().try_load(path).map_err(|e| format!("{e}"))
}

fn render_instance(instance: &mut PluginInstance, request: &Request, bytes: Vec<u8>) -> Result<String, String> {
    let layer = Layer::<Depth8>::from_raw(bytes, request.width, request.height).map_err(|e| format!("input layer: {e}"))?;
    instance.set_render_size(request.width, request.height);
    instance.set_input_layer(layer);
    apply_time(instance, request.time, request.fps);
    for param in &request.params {
        apply_param(instance, param)?;
    }
    instance.render_frame().map_err(|e| format!("{e}"))?;
    let (width, height) = instance.output_size();
    if width != request.width || height != request.height {
        return Err(format!("the plug-in rendered {width}×{height} for a {}×{} layer", request.width, request.height));
    }
    let mut out = vec![0u8; bytes_len(width, height)?];
    instance.write_rendered_pixels(&mut out).map_err(|e| format!("{e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(out))
}

fn bytes_len(width: u32, height: u32) -> Result<usize, String> {
    (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4)).ok_or_else(|| "frame is too large".to_string())
}

fn apply_time(instance: &mut PluginInstance, time: f64, fps: f64) {
    let fps = if fps.is_finite() && (1.0..1000.0).contains(&fps) { fps } else { 30.0 };
    let time = if time.is_finite() { time } else { 0.0 };
    // current_time / time_scale seconds. Both divisors stay positive, which is what aexlo requires.
    let scale: u32 = 60_000;
    let step = ((f64::from(scale) / fps).round() as i32).clamp(1, i32::MAX);
    let current = (time * f64::from(scale)).round().clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    instance.set_time(current, step, scale);
}

fn apply_param(instance: &mut PluginInstance, param: &ParamSet) -> Result<(), String> {
    let value = match param.aex.as_str() {
        "float" => ParamValue::Float(finite(param.value)),
        "fixed" => ParamValue::Fixed(f32_of(param.value)),
        "int" => ParamValue::Slider(i32_of(param.value)),
        "checkbox" => ParamValue::Checkbox(param.value != 0.0),
        "popup" => {
            let mut choice = i32_of(param.value).saturating_add(1);
            if let Some(options) = instance.param_choices(param.index) {
                if !options.is_empty() {
                    choice = choice.clamp(1, options.len() as i32);
                }
            }
            ParamValue::Popup(choice)
        }
        "angle" => ParamValue::Angle(f32_of(param.value)),
        "point" => ParamValue::Point { x: f32_of(param.x), y: f32_of(param.y) },
        "color" => ParamValue::Color { red: u8_of(param.r), green: u8_of(param.g), blue: u8_of(param.b), alpha: u8_of(param.a) },
        "point3" => ParamValue::Point3D { x: finite(param.x), y: finite(param.y), z: finite(param.z) },
        other => return Err(format!("parameter {} has an unknown kind `{other}`", param.index)),
    };
    instance.set_param(param.index, value).map_err(|e| format!("parameter {}: {e}", param.index))
}

fn describe(instance: &PluginInstance, path: &Path) -> Result<Value, String> {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Plug-in".into());
    let (setup_w, setup_h) = instance.input_size();
    let mut group: Vec<String> = Vec::new();
    let mut params = Vec::new();
    for index in 1..instance.param_count() {
        if params.len() >= MAX_PARAMS {
            break;
        }
        let pname = instance.param_name(index).unwrap_or_default();
        match instance.param_kind(index) {
            Some(ParamKind::GroupStart) => {
                group.push(pname);
                continue;
            }
            Some(ParamKind::GroupEnd) => {
                group.pop();
                continue;
            }
            _ => {}
        }
        if instance.param_hidden(index) {
            continue;
        }
        if let Some(param) = map_param(instance, index, &pname, &group, setup_w, setup_h) {
            params.push(param);
        }
    }
    Ok(json!({
        "id": "",
        "name": name,
        "category": "After Effects",
        "version": "",
        "path": path.to_string_lossy(),
        "params": params,
    }))
}

fn map_param(instance: &PluginInstance, index: usize, name: &str, group: &[String], setup_w: u32, setup_h: u32) -> Option<Value> {
    let kind = instance.param_kind(index)?;
    let (type_name, aex, extra) = match kind {
        ParamKind::FloatSlider => {
            let default = match instance.get_param(index) {
                Some(ParamValue::Float(v)) => finite(v),
                _ => 0.0,
            };
            let (min, max) = slider_range(instance, index, default);
            ("slider", "float", json!({"default": default, "min": min, "max": max, "decimals": 2, "integer": false}))
        }
        ParamKind::FixedSlider => {
            let default = match instance.get_param(index) {
                Some(ParamValue::Fixed(v)) => finite(f64::from(v)),
                _ => 0.0,
            };
            let (min, max) = slider_range(instance, index, default);
            ("slider", "fixed", json!({"default": default, "min": min, "max": max, "decimals": 2, "integer": false}))
        }
        ParamKind::Slider => {
            let default = match instance.get_param(index) {
                Some(ParamValue::Slider(v)) => f64::from(v),
                _ => 0.0,
            };
            let (min, max) = slider_range(instance, index, default);
            ("slider", "int", json!({"default": default, "min": min, "max": max, "decimals": 0, "integer": true}))
        }
        ParamKind::Checkbox => {
            let default = matches!(instance.get_param(index), Some(ParamValue::Checkbox(true)));
            ("checkbox", "checkbox", json!({"default": default}))
        }
        ParamKind::Popup => {
            let options = instance.param_choices(index).unwrap_or_default();
            if options.is_empty() {
                return None;
            }
            let current = match instance.get_param(index) {
                Some(ParamValue::Popup(v)) => v.max(1) as usize,
                _ => 1,
            };
            let default = current.saturating_sub(1).min(options.len().saturating_sub(1));
            ("popup", "popup", json!({"options": options, "default": default}))
        }
        ParamKind::Angle => {
            let default = match instance.get_param(index) {
                Some(ParamValue::Angle(v)) => finite(f64::from(v)),
                _ => 0.0,
            };
            ("angle", "angle", json!({"default": default}))
        }
        ParamKind::Point => {
            let (x, y) = match instance.get_param(index) {
                Some(ParamValue::Point { x, y }) => (fraction(x, setup_w), fraction(y, setup_h)),
                _ => (0.5, 0.5),
            };
            ("point", "point", json!({"default": [x, y]}))
        }
        ParamKind::Color => {
            let (r, g, b, a) = match instance.get_param(index) {
                Some(ParamValue::Color { red, green, blue, alpha }) => (unit(red), unit(green), unit(blue), unit(alpha)),
                _ => (1.0, 1.0, 1.0, 1.0),
            };
            ("color", "color", json!({"default": [r, g, b, a]}))
        }
        ParamKind::Point3D => {
            let (x, y, z) = match instance.get_param(index) {
                Some(ParamValue::Point3D { x, y, z }) => (finite(x), finite(y), finite(z)),
                _ => (0.0, 0.0, 0.0),
            };
            ("point3", "point3", json!({"default": [x, y, z]}))
        }
        _ => return None,
    };
    let mut obj = serde_json::Map::new();
    obj.insert("index".into(), json!(index));
    obj.insert("aex".into(), json!(aex));
    obj.insert("id".into(), json!(""));
    obj.insert("name".into(), json!(if name.trim().is_empty() { format!("Parameter {index}") } else { name.to_string() }));
    obj.insert("group".into(), json!(group.join("/")));
    obj.insert("type".into(), json!(type_name));
    if let Value::Object(extra) = extra {
        obj.extend(extra);
    }
    Some(Value::Object(obj))
}

fn slider_range(instance: &PluginInstance, index: usize, default: f64) -> (f64, f64) {
    let default = finite(default);
    match instance.param_slider_range(index) {
        Some((min, max)) if min.is_finite() && max.is_finite() => {
            let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
            (lo.min(default), hi.max(default))
        }
        _ => {
            let span = default.abs().max(1.0);
            (default - span, default + span)
        }
    }
}

fn fraction(pixel: f32, size: u32) -> f64 {
    if size == 0 {
        return 0.5;
    }
    let v = f64::from(pixel) / f64::from(size);
    if v.is_finite() { v } else { 0.5 }
}

fn unit(channel: u8) -> f64 {
    f64::from(channel) / 255.0
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn f32_of(v: f64) -> f32 {
    let v = finite(v) as f32;
    if v.is_finite() { v } else { 0.0 }
}

fn i32_of(v: f64) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    v.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

fn u8_of(v: f64) -> u8 {
    if !v.is_finite() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
