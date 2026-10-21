# After Effects plug-ins (`.aex`)

This fork can run After Effects effect plug-ins as ordinary effects. They show up in Effects &
Presets, their settings show up in Effect Controls, and they take keyframes like any other effect.

The plug-in does not run inside EffectCraft. A separate program, `effectcraft-helper-aex`, loads
the `.aex` with [aexlo](https://github.com/potistudio/aexlo) and renders frames. EffectCraft finds
that program, sends the layer and the parameter values as JSON, and copies the pixels back. If a
plug-in crashes, the helper exits. EffectCraft shows the error on that frame and starts a new
helper for the next one. The project stays open.

This is experimental. aexlo is still changing, and it does not implement every After Effects
suite. A plug-in can load and still fail because a suite is missing, because it checks a licence
against After Effects, or because it draws its own UI. Expect to test plug-ins one by one.

## What you need

* Windows x64 or macOS arm64. After Effects plug-ins are not built for Linux, so the helper is not
  either. The rest of EffectCraft does not need the helper and runs without it.
* Rust 1.99 or newer to **build the helper**. EffectCraft itself still builds with 1.95. The
  helper is a separate program (`helpers/effectcraft-helper-aex`) and is not part of
  `cargo xtask ci`.
* The plug-in's own licence. EffectCraft does not include or unlock any plug-in.

```sh
cargo build --release --manifest-path helpers/effectcraft-helper-aex/Cargo.toml
```

Put `effectcraft-helper-aex` (`.exe` on Windows) next to `effectcraft`, or set
`EFFECTCRAFT_AEX_HELPER` to its full path.

aexlo is pinned in the helper's `Cargo.toml` (`28e9f9a4a371d0ecae0d2b183e167755b3a7cac3`). It is MIT.
Bump that revision on purpose and retest the plug-ins you care about; the API moves.

On Windows, that revision does not compile: `PrPixelFormat` is a signed int, and aexlo stores
formats as `u32`. `rustc-wrapper.cmd` applies those two casts to the cargo checkout once, then
runs the real compiler. The pin stays the same.

## Where plug-ins are found

At start-up, when the helper is installed and `EFFECTCRAFT_NO_AEX` is not `1`, EffectCraft scans:

| Where | Notes |
|---|---|
| Every folder in `AEX_PLUGIN_PATH` | `;` between folders on Windows, `:` elsewhere |
| `AEX` next to `effectcraft` | Copy `.aex` files here |

A plug-in is an `.aex` file on Windows, or a `.plugin` bundle on macOS. The scan is written to
`aex_scan.txt` in the config folder (on Windows, `%APPDATA%\EffectCraft\aex_scan.txt`).

Adobe's own application folder is not scanned. Point `AEX_PLUGIN_PATH` at a folder of plug-ins
you want to use. Effect ▸ Load After Effects Plug-in… loads one file for this session.
`effect.plugins.loadAex {"path": "..."}` and `effect.plugins.scanAex {}` do the same from the
control channel.

## Limits

These are limits of this integration and of aexlo `28e9f9a`, not of the plug-in's own licence.

### Disclaimer: serial numbers and trial builds

EffectCraft does not unlock paid plug-ins. It does not bypass, patch, hook, or replace a vendor's
licence check, and it does not accept or store a serial on the vendor's behalf. If a plug-in stays
on its trial build here, that is the plug-in's own check running without the window it uses inside
After Effects.

* **No registration window.** aexlo does not send `PF_Cmd_EVENT`, so a plug-in has nowhere to draw
  its own UI. Buttons (the usual "License" or "Register" control) are not shown in Effect
  Controls. The helper also tells the plug-in it is a headless renderer. An aescripts plug-in
  that unlocks only after you type a serial in its dialog stays on the trial build. EffectCraft
  does not provide a serial field and does not bypass the vendor's check.
* A licence file the plug-in already wrote, after you registered it in After Effects, is read
  only if that plug-in looks the file up itself. Many do not.
* **8-bit RGBA only**, at most 4096×4096. EffectCraft's frames are float; the bridge quantizes
  both ways. A plug-in that changes the frame size fails that frame.
* One helper, one frame at a time. Slower than the same effect inside the process.
* No second layer input, no paths, no arbitrary data, no custom UI.
* Missing After Effects suites, a check against the host serial, or anything that needs After
  Effects' own process state can fail even when the `.aex` loads.
* GPU rendering inside the helper is off unless `EFFECTCRAFT_AEX_GPU=1`.
* Linux has no `.aex` plug-ins. The helper is Windows x64 and macOS arm64.
* Adobe's application folder is not scanned. Copy the `.aex` into `AEX` next to `effectcraft`,
  or set `AEX_PLUGIN_PATH`.

## Libraries

The desktop app does not link these. Only `effectcraft-helper-aex` does. Versions are pinned in
`helpers/effectcraft-helper-aex/Cargo.lock`.

| Package | Version | Licence | Role |
|---|---|---|---|
| [aexlo](https://github.com/potistudio/aexlo) | git `28e9f9a4a371d0ecae0d2b183e167755b3a7cac3` | MIT | Loads the `.aex` and runs its render. Not part of EffectCraft. |
| aexlo-macros | same git revision | MIT | Proc macros used by aexlo. |
| [after-effects](https://crates.io/crates/after-effects) | 0.4.0 | Apache-2.0 OR BSD-3-Clause OR MIT OR Zlib | Safe Rust API over the After Effects plug-in types. |
| [after-effects-sys](https://crates.io/crates/after-effects-sys) | 0.4.0 | Apache-2.0 OR BSD-3-Clause OR MIT OR Zlib | Raw bindings. On Windows `PrPixelFormat` is `c_int`; aexlo stores `u32`, so the Windows build casts those two call sites. |
| [cudarc](https://crates.io/crates/cudarc) | 0.19.10 (requested 0.19.8) | MIT OR Apache-2.0 | Optional CUDA path. Not used unless `EFFECTCRAFT_AEX_GPU=1`. Loaded at runtime, not shipped. |
| base64 | 0.22 | MIT OR Apache-2.0 | Pixel bytes on the helper's JSON line. |
| serde / serde_json | 1 | MIT OR Apache-2.0 | The JSON-lines protocol. |

EffectCraft's own side (`crates/aex`) is MIT OR Apache-2.0 and has no aexlo dependency. It only
talks to the helper. Tests use `aex-mock-helper`, which is not aexlo and does not load a plug-in.

## What the effect can and cannot do

* Sliders, angles, checkboxes, popups, points, colours and 3D points become normal parameters.
  Groups become the parameter's group path. Buttons, arbitrary data, paths and extra layer
  inputs are skipped; the plug-in keeps its own default for those.
* The input is the layer's pixels. There is no second layer input yet, and no custom UI, so a
  plug-in whose controls are drawn by the plug-in itself has nothing to draw.
* Frames are 8-bit RGBA, at most 4096×4096. EffectCraft's buffers are float; the bridge
  quantizes on the way out and on the way back. A frame that the plug-in resizes is an error
  on that frame.
* Rendering is one frame at a time, in one helper, so it is slower than the same effect inside
  the process. GPU render is off unless `EFFECTCRAFT_AEX_GPU=1` (aexlo's own GPU path; a missing
  driver would otherwise risk the helper).
* The effect id is `aex.` plus the file name, and the vendor folder when that folder is not a
  generic `AEX` or `Plug-ins` directory. The same file name in the same vendor folder keeps the
  same id, so keyframes still find the effect after you move the folder.

## The protocol

The helper reads one JSON object per line on stdin and writes one JSON object per line on
stdout. Stdout is only for this protocol. A line larger than 96 MiB is refused.

```json
{"op":"ping"}
{"op":"load","path":"C:/AEX/Glow.aex"}
{"op":"render","path":"C:/AEX/Glow.aex","instance":1,"width":2,"height":2,"time":0.0,"fps":30.0,"params":[{"index":1,"aex":"float","value":0.5}],"pixels":"<base64 of RGBA8>"}
{"op":"quit"}
```

`ping` answers `{"ok":true,"protocol":1,"name":"effectcraft-helper-aex"}`. `load` answers
`{"ok":true,"effects":[...]}`. Each effect has `name`, `path` and `params`. A parameter has
`index` (the plug-in's index; 0 is the input layer and is never sent), `aex` (`float`, `fixed`,
`int`, `checkbox`, `popup`, `angle`, `point`, `color`, `point3`) and the same fields as an
EffectCraft plug-in parameter (`type`, `default`, …). Empty `id`s are filled in by EffectCraft.
Popup values on the wire are 0-based; the helper adds one, because aexlo's popup values are
1-based. `instance` is the applied effect instance, so two layers do not share one plug-in
instance. `pixels` is standard base64 of 8-bit RGBA, row-major, row 0 at the top.

A failure is `{"ok":false,"error":"..."}`. No reply, or the process exiting, means the plug-in
took the helper down. EffectCraft reports that and starts a new helper.

`crates/aex` tests this with `aex-mock-helper`, which inverts a frame and does not load a
plug-in. The real helper is `helpers/effectcraft-helper-aex`.
