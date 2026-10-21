# effectcraft-helper-aex

Loads After Effects effect plug-ins (`.aex` on Windows, `.plugin` bundles on macOS) with
[aexlo](https://github.com/potistudio/aexlo) and answers the JSON-lines protocol in
[docs/aex.md](../../docs/aex.md).

EffectCraft never loads the plug-in itself. If this program crashes, EffectCraft keeps the
project and starts a new helper for the next frame.

## Limits

aexlo `28e9f9a` does not open a plug-in's own windows. aescripts (and any other vendor) that
asks for a serial in a dialog cannot be registered here. Those plug-ins stay on their trial
unless they already find a licence file on their own. There is no serial box in EffectCraft,
and this program does not remove or skip a vendor's licence check.

Full list, including the 8-bit frame limit and the libraries below: [docs/aex.md](../../docs/aex.md).

## Libraries

Pinned in `Cargo.lock`. Only this program links them.

| Package | Pin | Licence |
|---|---|---|
| aexlo, aexlo-macros | git `28e9f9a4a371d0ecae0d2b183e167755b3a7cac3` | MIT |
| after-effects, after-effects-sys | 0.4.0 | Apache-2.0 OR BSD-3-Clause OR MIT OR Zlib |
| cudarc | 0.19.10 (dynamic CUDA, off unless `EFFECTCRAFT_AEX_GPU=1`) | MIT OR Apache-2.0 |
| base64 | 0.22 | MIT OR Apache-2.0 |
| serde, serde_json | 1 | MIT OR Apache-2.0 |

On Windows, aexlo `28e9f9a` does not compile as published: `PrPixelFormat` is `c_int` and aexlo
stores formats as `u32`. `rustc-wrapper.cmd` casts those two call sites in the cargo git
checkout, then runs the real compiler. The git revision is unchanged. That wrapper is configured
only for this package (`.cargo/config.toml` here), not for the EffectCraft workspace.

## Build

Rust 1.99 or newer. Windows x64 or macOS arm64. From this directory, so the package config is
picked up:

```sh
cd helpers/effectcraft-helper-aex
cargo build --release
```

Put `effectcraft-helper-aex` (`.exe` on Windows) next to `effectcraft`, or set
`EFFECTCRAFT_AEX_HELPER` to its path.
