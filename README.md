# EffectCraft

Windows build of [EffectCraft](https://github.com/storytold/effectcraft) with an OpenFX host.
Compositions, layers, keyframes, effects, expressions and a render queue, in pure Rust, plus
native OpenFX 1.4 plug-ins. On Windows those plug-ins can render with OpenGL and fall back to
the CPU.

This repository is [Batkixni/effectcraft](https://github.com/Batkixni/effectcraft), branch
`ofx-gl`. The upstream project is [storytold/effectcraft](https://github.com/storytold/effectcraft).
Upstream builds do not include this OpenFX host.

<p align="center">
  <img src="docs/images/effectcraft-hero.png" alt="EffectCraft's main window: the demo composition, Project panel, Timeline and Properties" width="100%">
</p>

## Install

Windows x64 only for this release. The installer and a portable zip are on
[Releases](https://github.com/Batkixni/effectcraft/releases).

| File | What it is |
|---|---|
| `effectcraft-<ver>-windows-x64.msi` | Installer. Default folder is Program Files. |
| `effectcraft-<ver>-windows-x64-portable.zip` | `effectcraft.exe`, `effectcraft-cli.exe` and `portable.txt`. Settings stay in `EffectCraftData` next to the exe. |

Silent install:

```text
msiexec /i effectcraft-<ver>-windows-x64.msi /qn INSTALLFOLDER="D:\Apps\EffectCraft"
```

These binaries are not code-signed. Windows SmartScreen will warn the first time you open them.

The portable zip and the installer both ship the desktop app and `effectcraft-cli`. OpenFX is
compiled in. On startup the app scans the usual OpenFX folders unless `EFFECTCRAFT_NO_OFX=1`.

## OpenFX

EffectCraft loads `.ofx` binaries and `.ofx.bundle` folders and lists each plug-in as a normal
effect (Effect ▸ Load OpenFX Plug-in…). Details and the command names are in
[docs/ofx.md](docs/ofx.md).

Search order:

- `%CommonProgramFiles%\OFX\Plugins` (usually `C:\Program Files\Common Files\OFX\Plugins`)
- an `OFX` folder next to `effectcraft.exe`
- every directory in `OFX_PLUGIN_PATH`

A plug-in that sets `OfxImageEffectPropOpenGLRenderSupported` to `true` renders into an OpenGL
texture on a private WGL context. The frame is read back into the compositor. If the plug-in
returns `kOfxStatGLRenderFailed` or runs out of GPU memory, that frame is rendered again on the
CPU, as long as the plug-in does not require OpenGL (`needed`). `EFFECTCRAFT_OFX_NO_GL=1` forces
the CPU path.

This host is in-process. A plug-in that corrupts memory can still exit the app. A binary that
dies while loading or rendering is remembered; two deaths in a row put it on a blocklist and the
next launch skips it. Clear that list with `effect.plugins.scanOfx {"retry": true}`.

Not in this host: CUDA, Metal, OpenCL, overlay UI, custom interact UI, tiles, and fields.
Linux and macOS builds of this branch stay on the CPU. Filter, general, transition and generator
contexts are supported.

`EFFECTCRAFT_OFX_HOST_NAME` is the host name plug-ins see. Some bundles only turn themselves on
for hosts they recognise. The scan log is `%APPDATA%\EffectCraft\ofx_scan.txt` (or
`EffectCraftData` in a portable install).

## What the app does

You build a composition out of layers (solids, shapes, text, footage, nested compositions, nulls,
adjustment layers, cameras and lights), animate properties with keyframes, stack effects and
render.

- **Keyframes.** Linear, Bezier, hold, auto and continuous Bezier, Easy Ease, the Graph Editor.
- **Time.** Remap, stretch, reverse, freeze frame, markers, exact frame timing including 29.97.
- **Shapes, masks, text.** Trim paths, repeaters, pen-tool masks, Character and Paragraph panels,
  text animators.
- **Expressions.** JavaScript (`wiggle`, `loopOut`, `thisComp.layer`, the pick-whip).
- **306 built-in effects**, nine layer styles, and the OpenFX plug-ins you load on top.
- **3D.** Cameras, lights, depth of field, intersecting layers.
- **Render queue.** H.264, HEVC, AV1, ProRes, WebM (VP9 or AV1, Opus), PNG/JPEG/TIFF/EXR, GIF,
  WAV/AIFF. No FFmpeg inside. Lottie import and export are built in.
- **Projects are JSON** (`.ecproj`).
- **Agents.** Every menu action is a command. `effectcraft --control 9877` speaks JSON lines.
  `effectcraft-cli mcp` is an MCP server. See [docs/agents.md](docs/agents.md).

## Build

[Rust](https://rustup.rs/) 1.95 or newer, the MSVC target on Windows.

```sh
git clone https://github.com/Batkixni/effectcraft
cd effectcraft
git checkout ofx-gl
cargo build --release -p effectcraft -p effectcraft-cli
```

`openfx` is a default feature of the desktop app and the CLI. The Windows installer is
`packaging/windows/package.ps1` (WiX 5). It writes
`dist/release/effectcraft-<ver>-windows-x64.msi` and the portable zip.

## Limits

EffectCraft is young. This fork does not make it a finished After Effects replacement.

- Projects are `.ecproj`. After Effects project files are not imported here.
- OpenFX runs in-process. A hard crash in a plug-in takes the process down; the next launch
  skips a binary after two crashes.
- OpenGL is Windows-only. Plug-ins that set OpenGL to `needed` do not run without it.
- No plug-in overlay or custom UI. No CUDA, Metal or OpenCL.
- Automated tests cover the host, not a catalogue of commercial OpenFX bundles. If a plug-in
  draws the wrong frame, say which bundle and which effect.

Upstream status, the parity tables and the roadmap live in the
[storytold repository](https://github.com/storytold/effectcraft).

## License

EffectCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Copyright (c) 2026 ArtCraft Team and the EffectCraft contributors. Notices are in
[NOTICE](NOTICE).

Bundled fonts, icons and images keep their own open licenses. Each one is listed in
[ATTRIBUTION.md](ATTRIBUTION.md).

The ArtCraft name and the logos under `docs/brand/` are trademarks of the ArtCraft Team. They
are not covered by the code license. This fork does not ship those marks as its own branding.
See [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
