# OpenFX

EffectCraft can host OpenFX 1.4 image-effect plug-ins (`.ofx` binaries and `.ofx.bundle`
folders) and register each one as an ordinary effect. The desktop app and the CLI turn this on
with the `openfx` feature, which is the default for those two packages on this fork.

This runs native code in-process. A plug-in that corrupts the heap can still take the process
down later. What the host does about a fault in the plug-in itself:

- every host callback and every action is wrapped so a Rust panic becomes an OFX error instead
  of an abort;
- on Windows, an access violation inside an action is caught the same way. That frame is rendered
  on the CPU when the plug-in allows it, and OpenGL stays off for the rest of the session;
- a binary that dies while it is being loaded, or while it is rendering, leaves a marker in the
  config directory. Two deaths in a row put it on a blocklist and the next launch skips it.
  `effect.plugins.scanOfx {"retry": true}` clears the list;
- plug-ins that say they are not thread-safe are rendered one at a time;
- on Windows, OpenGL renders share one context on one thread.

## Loading

On startup the app scans the standard folders unless `EFFECTCRAFT_NO_OFX=1`:

- Windows: `%CommonProgramFiles%\OFX\Plugins` (usually `C:\Program Files\Common Files\OFX\Plugins`)
- macOS: `/Library/OFX/Plugins`
- Linux: `/usr/OFX/Plugins`
- `OFX` next to the executable
- every path in `OFX_PLUGIN_PATH`

Effect ▸ Load OpenFX Plug-in… loads one binary, bundle or folder. The same things from an agent
or the CLI:

```text
effect.plugins.scanOfx {}
effect.plugins.loadOfx {"path": "C:\\Program Files\\Common Files\\OFX\\Plugins"}
```

A scan writes `%APPDATA%\EffectCraft\ofx_scan.txt` (or the platform config directory).

`EFFECTCRAFT_OFX_HOST_NAME` is the name plug-ins see. Some bundles only enable themselves for
hosts they recognise.

## OpenGL

The host tells plug-ins that OpenGL rendering is supported on Windows. A plug-in that sets
`OfxImageEffectPropOpenGLRenderSupported` to `true` is rendered into a `GL_TEXTURE_2D` (float,
short or byte, matching the clip). The output texture is bound as the framebuffer before
`kOfxImageEffectActionRender`. Pixels are read back into EffectCraft's premultiplied buffer.

`OfxImageEffectOpenGLRenderSuiteV1` (`clipLoadTexture`, `clipFreeTexture`, `flushResources`) is
provided. `kOfxStatGLRenderFailed`, `kOfxStatGLOutOfMemory`, an out-of-memory status, or a native
fault inside the plug-in retries that frame on the CPU when the plug-in also supports CPU
(`true`, not `needed`). A native fault also turns OpenGL off for every plug-in after that.

`EFFECTCRAFT_OFX_NO_GL=1` forces the CPU path and tells plug-ins OpenGL is not supported.
Linux and macOS builds stay on the CPU. CUDA, Metal, OpenCL, overlays and custom interact UI
are not implemented.

## What a plug-in sees

Filter, general, transition and generator contexts. One input (the layer), extra clips as layer
parameters, and the output. Temporal access and other layers go through the plug-in API. Regions
of definition larger than the layer grow the buffer, up to 4096 pixels a side. Byte, short and
float, RGBA, RGB and alpha. No tiles, no multi-resolution, no field rendering.
