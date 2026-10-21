//! Stop a native plug-in fault from killing the process (Windows).
//!
//! `catch_unwind` only sees a Rust panic. An access violation inside an OpenFX plug-in is a
//! structured exception: nothing returns, and the process exits. While a plug-in action is on
//! the stack, a vectored handler catches that class of fault and resumes just after the context
//! was captured, so the action comes back as an error. The OpenGL path then renders the frame
//! on the CPU and stops using the display for the rest of the session.
//!
//! This does not undo damage the plug-in already did to the heap. It keeps the window open when
//! the fault is a bad pointer in the plug-in, which is what the OpenGL path was hitting.

use std::sync::atomic::{AtomicU64, Ordering};

static FAULTS: AtomicU64 = AtomicU64::new(0);

/// How many native faults [`protect`] has caught in this process.
pub fn faults() -> u64 {
    FAULTS.load(Ordering::Relaxed)
}

/// A plug-in action stopped because of a native CPU fault. The process is still running.
#[derive(Debug)]
pub struct Fault;

/// Run `f`. On Windows, an access violation (and the other CPU faults the handler lists) becomes
/// `Err(Fault)` instead of exiting the process. Elsewhere this only runs `f`.
pub fn protect<R>(f: impl FnOnce() -> R) -> Result<R, Fault> {
    #[cfg(windows)]
    {
        win::protect(f)
    }
    #[cfg(not(windows))]
    {
        Ok(f())
    }
}

#[cfg(windows)]
mod win {
    use std::cell::Cell;
    use std::sync::Once;
    use std::sync::atomic::Ordering;

    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_POINTERS, RtlCaptureContext, CONTEXT,
    };

    use super::FAULTS;

    const CONTINUE_SEARCH: i32 = 0;
    /// Access violation, in-page error, illegal instruction, integer divide by zero, privileged
    /// instruction, stack overflow.
    const CAUGHT: [u32; 6] = [0xC000_0005, 0xC000_0006, 0xC000_001D, 0xC000_0094, 0xC000_0096, 0xC000_00FD];

    #[repr(C, align(16))]
    struct Snap {
        ctx: CONTEXT,
    }

    thread_local! {
        static ARMED: Cell<*mut Snap> = const { Cell::new(std::ptr::null_mut()) };
        /// Set by the handler. It lives outside the restored stack, so the resumed function can see it.
        static HIT: Cell<bool> = const { Cell::new(false) };
    }

    pub fn protect<R>(f: impl FnOnce() -> R) -> Result<R, super::Fault> {
        install();
        // SAFETY: `CONTEXT` is a plain C record. `RtlCaptureContext` writes it before it is read.
        let mut snap = Snap { ctx: unsafe { std::mem::zeroed() } };
        let prev = ARMED.with(|c| c.replace(&mut snap));
        // A fault while `f` runs resumes at this return, with `HIT` set.
        unsafe { RtlCaptureContext(&mut snap.ctx) };
        if HIT.with(|c| c.replace(false)) {
            ARMED.with(|c| c.set(prev));
            return Err(super::Fault);
        }
        let out = f();
        ARMED.with(|c| c.set(prev));
        Ok(out)
    }

    fn install() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            // SAFETY: the handler only reads the exception record and, when this thread armed a
            // snapshot, copies that snapshot over the CPU context the OS will resume.
            let handle = unsafe { AddVectoredExceptionHandler(1, Some(handler)) };
            if handle.is_null() {
                log::warn!("ofx: could not install the native-fault handler");
            }
        });
    }

    unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
        if info.is_null() {
            return CONTINUE_SEARCH;
        }
        // SAFETY: the OS passes a live exception record for the fault on this thread.
        let info = unsafe { &*info };
        if info.ExceptionRecord.is_null() || info.ContextRecord.is_null() {
            return CONTINUE_SEARCH;
        }
        let rec = unsafe { &*info.ExceptionRecord };
        if rec.ExceptionFlags & 1 != 0 {
            return CONTINUE_SEARCH;
        }
        let code = rec.ExceptionCode as u32;
        if !CAUGHT.contains(&code) {
            return CONTINUE_SEARCH;
        }
        let snap = ARMED.with(|c| c.get());
        if snap.is_null() {
            return CONTINUE_SEARCH;
        }
        HIT.with(|c| c.set(true));
        FAULTS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: `snap` addresses the `protect` frame still on this thread. Resuming it leaves
        // the plug-in call without running the rest of its stack.
        unsafe { *info.ContextRecord = (*snap).ctx };
        EXCEPTION_CONTINUE_EXECUTION
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::protect;
    use windows_sys::Win32::System::Diagnostics::Debug::RaiseException;

    #[test]
    fn an_access_violation_returns_instead_of_exiting() {
        let err = protect(|| unsafe { RaiseException(0xC000_0005, 0, 0, std::ptr::null()) });
        assert!(err.is_err(), "the access violation should come back as a fault");
    }
}
