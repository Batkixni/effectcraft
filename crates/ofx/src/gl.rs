//! OpenGL textures for OFX renders (Windows).
//!
//! EffectCraft's compositor is wgpu, so this module keeps a private WGL context instead of
//! borrowing the window's device. One thread owns the context. Renders are queued onto it, the
//! output texture is bound as the draw target, and the pixels are read back into the CPU buffer
//! the rest of the compositor uses. `kOfxStatGLRenderFailed` and an out-of-memory status go back
//! to the caller, which retries the same frame on the CPU when the plug-in allows it.
//!
//! The context is the compatibility profile `wglCreateContext` returns (the profile OFX plug-ins
//! written against OpenGL 2.1 / 3.x compatibility expect). There is no OpenGL path on other
//! operating systems: [`usable`] is false and renders stay on the CPU.
//!
//! `EFFECTCRAFT_OFX_NO_GL=1` disables the path before the first plug-in is described.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use crate::clips::{ClipImage, Comps, Depth};
use crate::ffi::{K_STAT_GL_OUT_OF_MEMORY, K_STAT_GL_RENDER_FAILED, K_STAT_OK, OfxRectI, OfxStatus};

/// An OpenGL texture name handed to a plug-in through an image handle.
#[derive(Clone, Copy, Debug)]
pub struct GpuInfo {
    pub index: u32,
    /// `GL_TEXTURE_2D` as an `i32` (`kOfxImageEffectPropOpenGLTextureTarget`).
    pub target: i32,
    /// The render's output texture. Releasing the handle must not delete it; the host reads it
    /// back after the plug-in returns.
    pub output: bool,
}

/// The host can offer OpenGL. Optimistic until the first real context attempt fails.
pub fn usable() -> bool {
    if !cfg!(windows) || std::env::var_os("EFFECTCRAFT_OFX_NO_GL").is_some_and(|v| v == "1") {
        return false;
    }
    !BROKEN.load(Ordering::Relaxed)
}

static BROKEN: AtomicBool = AtomicBool::new(false);

#[cfg(not(windows))]
pub fn with_current<R: Send>(f: impl FnOnce() -> R + Send) -> Result<R, String> {
    let _ = f;
    Err("OpenGL rendering is implemented on Windows".into())
}

#[cfg(not(windows))]
pub fn draw_output(_img: &ClipImage, _window: OfxRectI, _body: impl FnOnce() -> OfxStatus) -> OfxStatus {
    K_STAT_GL_RENDER_FAILED
}

#[cfg(not(windows))]
pub fn attach_texture(_img: &ClipImage, _output: bool) -> Option<GpuInfo> {
    None
}

#[cfg(not(windows))]
pub fn release_gpu(_info: GpuInfo) {}

#[cfg(windows)]
mod win {
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::mpsc::{self, Sender};

    use super::*;

    const GL_TEXTURE_2D: u32 = 0x0DE1;
    const GL_RGBA: u32 = 0x1908;
    const GL_RGB: u32 = 0x1907;
    const GL_RED: u32 = 0x1903;
    const GL_UNSIGNED_BYTE: u32 = 0x1401;
    const GL_UNSIGNED_SHORT: u32 = 0x1403;
    const GL_FLOAT: u32 = 0x1406;
    const GL_RGBA8: i32 = 0x8058;
    const GL_RGBA16: i32 = 0x805B;
    const GL_RGBA32F: i32 = 0x8814;
    const GL_RGB8: i32 = 0x8051;
    const GL_RGB16: i32 = 0x8054;
    const GL_RGB32F: i32 = 0x8815;
    const GL_R8: i32 = 0x8229;
    const GL_R16: i32 = 0x822A;
    const GL_R32F: i32 = 0x822E;
    const GL_NEAREST: i32 = 0x2600;
    const GL_CLAMP_TO_EDGE: i32 = 0x812F;
    const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
    const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
    const GL_TEXTURE_WRAP_S: u32 = 0x2802;
    const GL_TEXTURE_WRAP_T: u32 = 0x2803;
    const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;
    const GL_PACK_ALIGNMENT: u32 = 0x0D05;
    const GL_FRAMEBUFFER: u32 = 0x8D40;
    const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
    const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
    const GL_VERSION: u32 = 0x1F02;
    const GL_MAX_TEXTURE_SIZE: u32 = 0x0D33;

    pub(super) struct Texel {
        pub(super) internal: i32,
        pub(super) format: u32,
        pub(super) typ: u32,
    }

    pub(super) fn texel(depth: Depth, comps: Comps) -> Texel {
        let (internal, format) = match comps {
            Comps::Rgba => (
                match depth {
                    Depth::Byte => GL_RGBA8,
                    Depth::Short => GL_RGBA16,
                    Depth::Float => GL_RGBA32F,
                },
                GL_RGBA,
            ),
            Comps::Rgb => (
                match depth {
                    Depth::Byte => GL_RGB8,
                    Depth::Short => GL_RGB16,
                    Depth::Float => GL_RGB32F,
                },
                GL_RGB,
            ),
            Comps::Alpha => (
                match depth {
                    Depth::Byte => GL_R8,
                    Depth::Short => GL_R16,
                    Depth::Float => GL_R32F,
                },
                GL_RED,
            ),
        };
        let typ = match depth {
            Depth::Byte => GL_UNSIGNED_BYTE,
            Depth::Short => GL_UNSIGNED_SHORT,
            Depth::Float => GL_FLOAT,
        };
        Texel { internal, format, typ }
    }

    type GlGenTextures = unsafe extern "system" fn(i32, *mut u32);
    type GlDeleteTextures = unsafe extern "system" fn(i32, *const u32);
    type GlBindTexture = unsafe extern "system" fn(u32, u32);
    type GlTexImage2D = unsafe extern "system" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
    type GlTexParameteri = unsafe extern "system" fn(u32, u32, i32);
    type GlGetTexImage = unsafe extern "system" fn(u32, i32, u32, u32, *mut c_void);
    type GlPixelStorei = unsafe extern "system" fn(u32, i32);
    type GlViewport = unsafe extern "system" fn(i32, i32, i32, i32);
    type GlFinish = unsafe extern "system" fn();
    type GlGetError = unsafe extern "system" fn() -> u32;
    type GlGetString = unsafe extern "system" fn(u32) -> *const u8;
    type GlGetIntegerv = unsafe extern "system" fn(u32, *mut i32);
    type GlGenFramebuffers = unsafe extern "system" fn(i32, *mut u32);
    type GlDeleteFramebuffers = unsafe extern "system" fn(i32, *const u32);
    type GlBindFramebuffer = unsafe extern "system" fn(u32, u32);
    type GlFramebufferTexture2D = unsafe extern "system" fn(u32, u32, u32, u32, i32);
    type GlCheckFramebufferStatus = unsafe extern "system" fn(u32) -> u32;

    struct Fns {
        gen_textures: GlGenTextures,
        delete_textures: GlDeleteTextures,
        bind_texture: GlBindTexture,
        tex_image: GlTexImage2D,
        tex_parameteri: GlTexParameteri,
        get_tex_image: GlGetTexImage,
        pixel_storei: GlPixelStorei,
        viewport: GlViewport,
        finish: GlFinish,
        get_error: GlGetError,
        get_string: GlGetString,
        get_integerv: GlGetIntegerv,
        gen_framebuffers: GlGenFramebuffers,
        delete_framebuffers: GlDeleteFramebuffers,
        bind_framebuffer: GlBindFramebuffer,
        framebuffer_texture: GlFramebufferTexture2D,
        check_framebuffer: GlCheckFramebufferStatus,
    }

    static FNS: OnceLock<Fns> = OnceLock::new();

    thread_local! {
        static ON_GL: Cell<bool> = const { Cell::new(false) };
        static OUTPUT: Cell<u32> = const { Cell::new(0) };
        static MAX_EDGE: Cell<i32> = const { Cell::new(4096) };
    }

    pub(super) fn on_gl() -> bool {
        ON_GL.with(Cell::get)
    }

    struct Job {
        thunk: usize,
        f: usize,
        out: usize,
        done: Sender<bool>,
    }

    type Thunk = unsafe fn(usize, usize);

    fn jobs() -> &'static Mutex<Option<Sender<Job>>> {
        static JOBS: OnceLock<Mutex<Option<Sender<Job>>>> = OnceLock::new();
        JOBS.get_or_init(|| Mutex::new(None))
    }

    /// Run `f` on the thread that owns the context. The caller waits, so `f` may borrow the
    /// caller's stack. A nested call from that thread runs `f` directly.
    pub fn with_current<R: Send, F: FnOnce() -> R + Send>(f: F) -> Result<R, String> {
        if on_gl() {
            return std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| "OpenGL job panicked".into());
        }
        if !ensure() {
            return Err("OpenGL is unavailable".into());
        }
        unsafe fn run<R, F: FnOnce() -> R>(f_ptr: usize, out_ptr: usize) {
            // SAFETY: the caller waits on `done` and keeps both slots alive for this call.
            let slot = unsafe { &mut *(f_ptr as *mut Option<F>) };
            let out = unsafe { &mut *(out_ptr as *mut Option<R>) };
            if let Some(f) = slot.take() {
                *out = Some(f());
            }
        }
        let mut slot = Some(f);
        let mut out: Option<R> = None;
        let (done_tx, done_rx) = mpsc::channel();
        let job = Job { thunk: run::<R, F> as Thunk as usize, f: (&mut slot) as *mut Option<F> as usize, out: (&mut out) as *mut Option<R> as usize, done: done_tx };
        {
            let guard = jobs().lock().unwrap_or_else(PoisonError::into_inner);
            let tx = guard.as_ref().ok_or("OpenGL is unavailable")?;
            tx.send(job).map_err(|_| "OpenGL thread stopped".to_string())?;
        }
        if !done_rx.recv().unwrap_or(false) {
            return Err("OpenGL job panicked".into());
        }
        out.ok_or_else(|| "OpenGL job returned nothing".into())
    }

    fn ensure() -> bool {
        static ONCE: OnceLock<bool> = OnceLock::new();
        *ONCE.get_or_init(|| {
            if std::env::var_os("EFFECTCRAFT_OFX_NO_GL").is_some_and(|v| v == "1") {
                return false;
            }
            let (tx, rx) = mpsc::channel::<Job>();
            let (ready_tx, ready_rx) = mpsc::channel::<bool>();
            let spawned = std::thread::Builder::new().name("ofx-gl".into()).stack_size(16 << 20).spawn(move || {
                let ok = init_gl();
                let _ = ready_tx.send(ok);
                if !ok {
                    return;
                }
                ON_GL.with(|c| c.set(true));
                while let Ok(job) = rx.recv() {
                    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        // SAFETY: `thunk` is `run::<R, F>` and the two addresses point at the
                        // caller's stack, which stays alive until `done` is signalled below.
                        unsafe {
                            let thunk: Thunk = std::mem::transmute(job.thunk);
                            thunk(job.f, job.out);
                        }
                    }))
                    .is_err();
                    let _ = job.done.send(!panicked);
                }
            });
            if spawned.is_err() {
                BROKEN.store(true, Ordering::Relaxed);
                return false;
            }
            let ok = ready_rx.recv().unwrap_or(false);
            if ok {
                *jobs().lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
            } else {
                BROKEN.store(true, Ordering::Relaxed);
            }
            ok
        })
    }

    fn proc<T>(module: windows_sys::Win32::Foundation::HMODULE, name: &std::ffi::CStr) -> Option<T> {
        let ptr = unsafe { windows_sys::Win32::System::LibraryLoader::GetProcAddress(module, name.as_ptr().cast()) }?;
        Some(unsafe { std::mem::transmute_copy(&ptr) })
    }

    fn wgl_proc<T>(name: &std::ffi::CStr) -> Option<T> {
        let ptr = unsafe { windows_sys::Win32::Graphics::OpenGL::wglGetProcAddress(name.as_ptr().cast()) }?;
        Some(unsafe { std::mem::transmute_copy(&ptr) })
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn init_gl() -> bool {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::Graphics::Gdi::GetDC;
        use windows_sys::Win32::Graphics::OpenGL::{
            ChoosePixelFormat, PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA, PIXELFORMATDESCRIPTOR, SetPixelFormat,
            wglCreateContext, wglMakeCurrent,
        };
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryA};
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, CS_OWNDC, WNDCLASSW, WS_OVERLAPPEDWINDOW};

        let class_name: &'static [u16] = Box::leak(wide("EffectCraftOfxGl").into_boxed_slice());
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        if instance.is_null() {
            return false;
        }
        let class = WNDCLASSW {
            style: CS_OWNDC,
            lpfnWndProc: Some(DefWindowProcW),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return false;
        }
        let hwnd: HWND = unsafe { CreateWindowExW(0, class_name.as_ptr(), class_name.as_ptr(), WS_OVERLAPPEDWINDOW, 0, 0, 64, 64, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null()) };
        if hwnd.is_null() {
            return false;
        }
        let dc = unsafe { GetDC(hwnd) };
        if dc.is_null() {
            unsafe { DestroyWindow(hwnd) };
            return false;
        }
        let pfd = PIXELFORMATDESCRIPTOR {
            nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
            nVersion: 1,
            dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            iPixelType: PFD_TYPE_RGBA,
            cColorBits: 32,
            cRedBits: 0,
            cRedShift: 0,
            cGreenBits: 0,
            cGreenShift: 0,
            cBlueBits: 0,
            cBlueShift: 0,
            cAlphaBits: 8,
            cAlphaShift: 0,
            cAccumBits: 0,
            cAccumRedBits: 0,
            cAccumGreenBits: 0,
            cAccumBlueBits: 0,
            cAccumAlphaBits: 0,
            cDepthBits: 24,
            cStencilBits: 8,
            cAuxBuffers: 0,
            iLayerType: 0,
            bReserved: 0,
            dwLayerMask: 0,
            dwVisibleMask: 0,
            dwDamageMask: 0,
        };
        let format = unsafe { ChoosePixelFormat(dc, &pfd) };
        if format == 0 || unsafe { SetPixelFormat(dc, format, &pfd) } == 0 {
            unsafe { DestroyWindow(hwnd) };
            return false;
        }
        let rc = unsafe { wglCreateContext(dc) };
        if rc.is_null() || unsafe { wglMakeCurrent(dc, rc) } == 0 {
            unsafe { DestroyWindow(hwnd) };
            return false;
        }
        let module = unsafe { LoadLibraryA(c"opengl32.dll".as_ptr().cast()) };
        if module.is_null() {
            return false;
        }
        let Some(fns) = load_fns(module) else { return false };
        if FNS.set(fns).is_err() {
            return false;
        }
        let Some(fns) = FNS.get() else { return false };
        unsafe { (fns.pixel_storei)(GL_UNPACK_ALIGNMENT, 1) };
        unsafe { (fns.pixel_storei)(GL_PACK_ALIGNMENT, 1) };
        let mut max_edge = 0i32;
        unsafe { (fns.get_integerv)(GL_MAX_TEXTURE_SIZE, &mut max_edge) };
        if max_edge > 0 {
            MAX_EDGE.with(|c| c.set(max_edge.min(16384)));
        }
        let version = unsafe { (fns.get_string)(GL_VERSION) };
        if !version.is_null() {
            let bytes = unsafe { std::ffi::CStr::from_ptr(version as *const std::ffi::c_char) };
            log::info!("openfx: OpenGL context ready ({})", bytes.to_string_lossy());
        } else {
            log::info!("openfx: OpenGL context ready");
        }
        let _ = (hwnd, dc, rc);
        true
    }

    fn load_fns(module: windows_sys::Win32::Foundation::HMODULE) -> Option<Fns> {
        let gen_framebuffers = wgl_proc(c"glGenFramebuffers").or_else(|| wgl_proc(c"glGenFramebuffersEXT"))?;
        let delete_framebuffers = wgl_proc(c"glDeleteFramebuffers").or_else(|| wgl_proc(c"glDeleteFramebuffersEXT"))?;
        let bind_framebuffer = wgl_proc(c"glBindFramebuffer").or_else(|| wgl_proc(c"glBindFramebufferEXT"))?;
        let framebuffer_texture = wgl_proc(c"glFramebufferTexture2D").or_else(|| wgl_proc(c"glFramebufferTexture2DEXT"))?;
        let check_framebuffer = wgl_proc(c"glCheckFramebufferStatus").or_else(|| wgl_proc(c"glCheckFramebufferStatusEXT"))?;
        Some(Fns {
            gen_textures: proc(module, c"glGenTextures")?,
            delete_textures: proc(module, c"glDeleteTextures")?,
            bind_texture: proc(module, c"glBindTexture")?,
            tex_image: proc(module, c"glTexImage2D")?,
            tex_parameteri: proc(module, c"glTexParameteri")?,
            get_tex_image: proc(module, c"glGetTexImage")?,
            pixel_storei: proc(module, c"glPixelStorei")?,
            viewport: proc(module, c"glViewport")?,
            finish: proc(module, c"glFinish")?,
            get_error: proc(module, c"glGetError")?,
            get_string: proc(module, c"glGetString")?,
            get_integerv: proc(module, c"glGetIntegerv")?,
            gen_framebuffers,
            delete_framebuffers,
            bind_framebuffer,
            framebuffer_texture,
            check_framebuffer,
        })
    }

    fn clear_error(fns: &Fns) {
        for _ in 0..8 {
            if unsafe { (fns.get_error)() } == 0 {
                break;
            }
        }
    }

    fn upload(img: &ClipImage) -> Option<u32> {
        let fns = FNS.get()?;
        let w = i32::try_from(img.width()).ok()?;
        let h = i32::try_from(img.height()).ok()?;
        let max_edge = MAX_EDGE.with(Cell::get);
        if w < 1 || h < 1 || w > max_edge || h > max_edge {
            return None;
        }
        let texel = texel(img.depth, img.comps);
        let bytes = img.width().checked_mul(img.height())?.checked_mul(img.pixel_bytes())?;
        // SAFETY: the plug-in is not writing this buffer; the caller is the host, on the GL thread.
        let pixels = unsafe { img.buf.bytes_ref() };
        if pixels.len() < bytes {
            return None;
        }
        let mut id = 0u32;
        clear_error(fns);
        unsafe {
            (fns.gen_textures)(1, &mut id);
            if id == 0 {
                return None;
            }
            (fns.bind_texture)(GL_TEXTURE_2D, id);
            (fns.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (fns.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (fns.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (fns.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (fns.tex_image)(GL_TEXTURE_2D, 0, texel.internal, w, h, 0, texel.format, texel.typ, pixels.as_ptr().cast());
            (fns.bind_texture)(GL_TEXTURE_2D, 0);
        }
        if unsafe { (fns.get_error)() } != 0 {
            unsafe { (fns.delete_textures)(1, &id) };
            return None;
        }
        Some(id)
    }

    fn delete_texture(id: u32) {
        let Some(fns) = FNS.get() else { return };
        if id != 0 {
            unsafe { (fns.delete_textures)(1, &id) };
        }
    }

    pub fn attach_texture(img: &ClipImage, output: bool) -> Option<GpuInfo> {
        if !on_gl() {
            return None;
        }
        if output {
            let id = OUTPUT.with(Cell::get);
            if id != 0 {
                return Some(GpuInfo { index: id, target: GL_TEXTURE_2D as i32, output: true });
            }
        }
        let index = upload(img)?;
        Some(GpuInfo { index, target: GL_TEXTURE_2D as i32, output: false })
    }

    pub fn release_gpu(info: GpuInfo) {
        if info.output || info.index == 0 {
            return;
        }
        if on_gl() {
            delete_texture(info.index);
        } else {
            let _ = with_current(move || delete_texture(info.index));
        }
    }

    struct OutputGuard {
        tex: u32,
        fbo: u32,
    }

    impl Drop for OutputGuard {
        fn drop(&mut self) {
            if let Some(fns) = FNS.get() {
                unsafe {
                    (fns.bind_framebuffer)(GL_FRAMEBUFFER, 0);
                    if self.fbo != 0 {
                        (fns.delete_framebuffers)(1, &self.fbo);
                    }
                    if self.tex != 0 {
                        (fns.delete_textures)(1, &self.tex);
                    }
                }
            }
            OUTPUT.with(|c| c.set(0));
        }
    }

    pub fn draw_output(img: &ClipImage, window: OfxRectI, body: impl FnOnce() -> OfxStatus) -> OfxStatus {
        if !on_gl() {
            return K_STAT_GL_RENDER_FAILED;
        }
        let Some(fns) = FNS.get() else { return K_STAT_GL_RENDER_FAILED };
        let Some(tex) = upload(img) else { return K_STAT_GL_OUT_OF_MEMORY };
        let mut fbo = 0u32;
        unsafe {
            (fns.gen_framebuffers)(1, &mut fbo);
            (fns.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
            (fns.framebuffer_texture)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        }
        let guard = OutputGuard { tex, fbo };
        if fbo == 0 || unsafe { (fns.check_framebuffer)(GL_FRAMEBUFFER) } != GL_FRAMEBUFFER_COMPLETE {
            return K_STAT_GL_OUT_OF_MEMORY;
        }
        let (vw, vh) = ((window.x2 - window.x1).max(1), (window.y2 - window.y1).max(1));
        unsafe { (fns.viewport)(0, 0, vw, vh) };
        OUTPUT.with(|c| c.set(tex));
        clear_error(fns);
        let st = body();
        unsafe { (fns.finish)() };
        let err = unsafe { (fns.get_error)() };
        if err != 0 {
            log::warn!("openfx: OpenGL error 0x{err:x} after render");
        }
        let texel = texel(img.depth, img.comps);
        let bytes = img.width().saturating_mul(img.height()).saturating_mul(img.pixel_bytes());
        // SAFETY: the plug-in's render has returned; this is the host reading the texture back.
        let pixels = unsafe { img.buf.bytes_mut() };
        if pixels.len() >= bytes && bytes > 0 {
            unsafe {
                (fns.bind_texture)(GL_TEXTURE_2D, tex);
                (fns.get_tex_image)(GL_TEXTURE_2D, 0, texel.format, texel.typ, pixels.as_mut_ptr().cast());
                (fns.bind_texture)(GL_TEXTURE_2D, 0);
            }
        }
        drop(guard);
        if st == K_STAT_OK { K_STAT_OK } else { st }
    }
}

#[cfg(windows)]
pub use win::{attach_texture, draw_output, release_gpu, with_current};

#[cfg(all(test, windows))]
mod tests {
    use super::win::texel;
    use crate::clips::{Comps, Depth};

    #[test]
    fn float_rgba_is_a_32_bit_float_texture() {
        let t = texel(Depth::Float, Comps::Rgba);
        assert_eq!(t.internal, 0x8814);
        assert_eq!(t.format, 0x1908);
        assert_eq!(t.typ, 0x1406);
    }
}
