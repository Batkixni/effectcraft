//! The host talks to a real child process: the mock helper, which speaks the same JSON as
//! `effectcraft-helper-aex` and does not load a plug-in.

use std::path::PathBuf;
use std::sync::Arc;

use effectcraft_effects::plugin::{PluginFrame, PluginParams, plugin};
use serde_json::json;

/// `aex-mock-helper` built next to this test's `deps` folder (`target/<profile>/`).
fn mock_helper() -> PathBuf {
    let exe = std::env::current_exe().expect("test executable");
    let profile = exe.parent().and_then(|deps| deps.parent()).expect("profile directory");
    let name = if cfg!(windows) { "aex-mock-helper.exe" } else { "aex-mock-helper" };
    profile.join(name)
}

#[test]
fn helper_process_inverts_a_frame_and_restarts_after_a_crash() {
    let path = mock_helper();
    assert!(path.is_file(), "build the aex-mock-helper binary first; looked at {}", path.display());
    let helper = Arc::new(effectcraft_aex::Helper::spawn(&path).unwrap());
    let missing = helper.call(&json!({"op": "load", "path": "missing.aex"})).unwrap();
    assert_eq!(missing["ok"], false, "{missing}");

    let reply = helper.call(&json!({"op": "load", "path": "invert.aex"})).unwrap();
    let registered = effectcraft_aex::register_reply(Arc::clone(&helper), &reply).unwrap();
    assert_eq!(registered["loaded"][0]["id"], "aex.invert", "{registered}");
    assert_eq!(registered["errors"].as_array().map(Vec::len), Some(0));

    let mut session = effectcraft_engine::Session::default();
    session.execute("comp.new", json!({"name": "P", "width": 8, "height": 8, "duration": 1})).unwrap();
    let layer = session.execute("layer.newSolid", json!({"color": [1.0, 0.0, 0.0], "width": 8, "height": 8})).unwrap()["layer"].clone();
    let applied = session.execute("effect.apply", json!({"layer": layer, "effect": "aex.invert"})).unwrap();
    assert_eq!(applied["effect"], "aex.invert", "{applied}");

    let effect = plugin("aex.invert").expect("invert registered");
    let manifest = effect.manifest().clone();
    assert_eq!(manifest.params.len(), 1);
    assert_eq!(manifest.params[0].id, "mix");

    let mut pixels = [[1.0_f32, 0.0, 0.0, 1.0]];
    {
        let flat = [1.0];
        let params = PluginParams { manifest: &manifest, flat: &flat, texts: &[] };
        let mut frame = PluginFrame { width: 1, height: 1, pixels: &mut pixels, scale: 1.0, origin: [0.0; 2], layer_size: [1.0; 2] };
        effect.render(&mut frame, &params, 0.0).unwrap();
    }
    assert_eq!(pixels, [[0.0, 1.0, 1.0, 0.0]]);

    pixels = [[1.0, 0.0, 0.0, 1.0]];
    {
        let flat = [0.0];
        let params = PluginParams { manifest: &manifest, flat: &flat, texts: &[] };
        let mut frame = PluginFrame { width: 1, height: 1, pixels: &mut pixels, scale: 1.0, origin: [0.0; 2], layer_size: [1.0; 2] };
        effect.render(&mut frame, &params, 0.0).unwrap();
    }
    assert_eq!(pixels, [[1.0, 0.0, 0.0, 1.0]]);

    // A plug-in that kills the helper fails this frame. The next request starts a new process.
    assert!(helper.call(&json!({"op": "crash"})).is_err());
    let pong = helper.call(&json!({"op": "ping"})).unwrap();
    assert_eq!(pong["ok"], true, "{pong}");
    assert_eq!(pong["protocol"], 1, "{pong}");

    pixels = [[0.0, 0.0, 1.0, 1.0]];
    {
        let flat = [1.0];
        let params = PluginParams { manifest: &manifest, flat: &flat, texts: &[] };
        let mut frame = PluginFrame { width: 1, height: 1, pixels: &mut pixels, scale: 1.0, origin: [0.0; 2], layer_size: [1.0; 2] };
        effect.render(&mut frame, &params, 0.5).unwrap();
    }
    assert_eq!(pixels, [[1.0, 1.0, 0.0, 0.0]]);
}
