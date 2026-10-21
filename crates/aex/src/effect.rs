//! One After Effects plug-in as an ordinary EffectCraft effect. Rendering sends the frame to
//! the helper and copies the reply back. The helper is shared and serialised: two frames never
//! enter it at once, which is slower than an in-process effect and is the isolation the helper
//! exists for.

use std::sync::Arc;

use effectcraft_effects::plugin::{EffectPlugin, PluginFrame, PluginHost, PluginManifest, PluginParams, register_plugin};
use serde_json::{Value, json};

use crate::client::Helper;
use crate::protocol::{self, Described, WireParam};

struct Binding {
    index: u64,
    aex: String,
}

pub struct AexEffect {
    manifest: PluginManifest,
    path: String,
    bindings: Vec<Binding>,
    helper: Arc<Helper>,
}

impl AexEffect {
    fn render_with(&self, frame: &mut PluginFrame, params: &PluginParams, time: f64, fps: f64, instance: u64) -> Result<(), String> {
        if frame.width == 0 || frame.height == 0 || frame.width > protocol::MAX_EDGE || frame.height > protocol::MAX_EDGE {
            return Err(format!("After Effects plug-ins render frames up to {}×{} pixels", protocol::MAX_EDGE, protocol::MAX_EDGE));
        }
        let count = (frame.width as usize).checked_mul(frame.height as usize).ok_or_else(|| "frame is too large".to_string())?;
        if frame.pixels.len() != count {
            return Err(format!("frame is {}×{} but holds {} pixels", frame.width, frame.height, frame.pixels.len()));
        }
        let bytes = protocol::frame_to_rgba8(frame.pixels)?;
        let mut flat_at = 0usize;
        let mut wire = Vec::with_capacity(self.bindings.len());
        for (param, binding) in self.manifest.params.iter().zip(&self.bindings) {
            let width = param.kind.width();
            let slot = params.flat.get(flat_at..flat_at.saturating_add(width)).unwrap_or(&[]);
            wire.push(protocol::wire_value(&binding.aex, binding.index, slot));
            flat_at = flat_at.saturating_add(width);
        }
        let fps = if fps.is_finite() && fps > 0.0 && fps <= 1000.0 { fps } else { 30.0 };
        let time = if time.is_finite() { time } else { 0.0 };
        let reply = self.helper.exchange(&json!({
            "op": "render",
            "path": self.path,
            "instance": instance,
            "width": frame.width,
            "height": frame.height,
            "time": time,
            "fps": fps,
            "params": wire,
            "pixels": protocol::b64_encode(&bytes),
        }))?;
        if !reply.ok {
            return Err(if reply.error.is_empty() { "the After Effects plug-in failed to render".into() } else { reply.error });
        }
        if reply.width != frame.width || reply.height != frame.height {
            return Err(format!("the plug-in returned a {}×{} frame for a {}×{} layer", reply.width, reply.height, frame.width, frame.height));
        }
        let out = protocol::b64_decode(&reply.pixels)?;
        protocol::rgba8_to_frame(&out, frame.pixels)
    }
}

impl EffectPlugin for AexEffect {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    fn source(&self) -> String {
        self.path.clone()
    }
    fn render(&self, frame: &mut PluginFrame, params: &PluginParams, time: f64) -> Result<(), String> {
        self.render_with(frame, params, time, 30.0, 0)
    }
    fn render_v2(&self, frame: &mut PluginFrame, params: &PluginParams, time: f64, host: &dyn PluginHost) -> Result<(), String> {
        self.render_with(frame, params, time, host.frame_rate(), host.instance_key())
    }
}

/// Register every effect in a helper `load` reply. One bad description is recorded and skipped.
/// An id that is already registered counts as loaded, so a second scan does not fail.
pub fn register_effects(helper: Arc<Helper>, effects: &[Described]) -> (Vec<Value>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut errors = Vec::new();
    for desc in effects {
        match register_one(Arc::clone(&helper), desc) {
            Ok(value) => loaded.push(value),
            Err(e) => errors.push(e),
        }
    }
    (loaded, errors)
}

fn register_one(helper: Arc<Helper>, desc: &Described) -> Result<Value, String> {
    let mut desc = desc.clone();
    protocol::finish_described(&mut desc).map_err(|e| format!("{}: {e}", desc.path))?;
    let manifest = protocol::manifest_of(&desc).map_err(|e| format!("{}: {e}", desc.path))?;
    let bindings = desc.params.iter().map(|p: &WireParam| Binding { index: p.index, aex: p.aex.clone() }).collect();
    let summary = json!({
        "id": manifest.id,
        "name": manifest.name,
        "category": manifest.category,
        "path": desc.path,
        "params": manifest.params.len(),
    });
    let effect = Arc::new(AexEffect { manifest, path: desc.path.clone(), bindings, helper });
    match register_plugin(effect) {
        Ok(_) => Ok(summary),
        Err(e) if e.contains("already registered") => Ok(summary),
        Err(e) => Err(format!("{}: {e}", desc.path)),
    }
}
