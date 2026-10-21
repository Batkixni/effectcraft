//! `OfxImageEffectOpenGLRenderSuiteV1` (`ofxGPURender.h`).
//!
//! `clipLoadTexture` uploads a clip image as a `GL_TEXTURE_2D` and returns a property set whose
//! `kOfxImageEffectPropOpenGLTextureIndex` is the texture name. The output clip reuses the
//! texture the host already bound as the draw target. `clipFreeTexture` deletes input textures
//! and only releases the handle for the output (the host reads that texture back itself).

use std::ffi::c_char;
use std::sync::atomic::Ordering;

use crate::clips::{ClipObj, Depth, new_image_handle, release_image};
use crate::ffi::*;
use crate::instance::{ClipRole, EffectObj, RenderCtx};

#[repr(C)]
pub struct OfxImageEffectOpenGLRenderSuiteV1 {
    pub clip_load_texture: extern "C" fn(Handle, f64, *const c_char, *const OfxRectD, *mut Handle) -> OfxStatus,
    pub clip_free_texture: extern "C" fn(Handle) -> OfxStatus,
    pub flush_resources: extern "C" fn() -> OfxStatus,
}

fn ctx_of(c: &ClipObj) -> Option<std::sync::Arc<RenderCtx>> {
    // SAFETY: an instance outlives its clips.
    unsafe { c.owner.as_ref() }.and_then(EffectObj::current_render)
}

extern "C" fn clip_load_texture(clip: Handle, time: f64, format: *const c_char, _region: *const OfxRectD, out: *mut Handle) -> OfxStatus {
    guard(|| {
        // SAFETY: handles and the format string come from the plug-in.
        let Some(c) = (unsafe { ClipObj::from_handle(clip) }) else { return K_STAT_ERR_BAD_HANDLE };
        if out.is_null() {
            return K_STAT_ERR_BAD_HANDLE;
        }
        let Some(ctx) = ctx_of(c) else { return K_STAT_FAILED };
        if !ctx.opengl.load(Ordering::Relaxed) {
            return K_STAT_FAILED;
        }
        let Some(img) = ctx.image(&c.name, time) else { return K_STAT_FAILED };
        if let Some(asked) = unsafe { str_arg(format) }
            && !asked.is_empty()
            && asked != DEPTH_NONE
            && Depth::from_name(asked) != Some(img.depth)
        {
            return K_STAT_ERR_FORMAT;
        }
        let output = matches!(ctx.cfg.clip(&c.name).map(|k| &k.role), Some(ClipRole::Output));
        let Some(gpu) = crate::gl::attach_texture(&img, output) else { return K_STAT_ERR_MEMORY };
        let h = new_image_handle(&img, ctx.geom.scale, RenderCtx::next_image_id(), Some(gpu));
        unsafe { *out = h };
        K_STAT_OK
    })
}

extern "C" fn clip_free_texture(handle: Handle) -> OfxStatus {
    guard(|| release_image(handle))
}

extern "C" fn flush_resources() -> OfxStatus {
    // The host does not keep a cache of textures past the current frame.
    K_STAT_REPLY_DEFAULT
}

pub static SUITE: OfxImageEffectOpenGLRenderSuiteV1 = OfxImageEffectOpenGLRenderSuiteV1 {
    clip_load_texture,
    clip_free_texture,
    flush_resources,
};
