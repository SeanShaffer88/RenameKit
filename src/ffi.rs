//! C ABI surface — everything declared in include/renamekit.h.
//! Conventions (house standard, from WinRecovery):
//! - `*mut c_char` results are caller-owned; free with `rk_free_string`.
//! - Failures return null / an ERR_* code + a thread-local last-error message.
//! - Nothing unwinds across the FFI boundary (catch_unwind everywhere;
//!   release profile also sets panic = "abort").

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::catch_unwind;

use crate::{engine, preview_json, ERR_INTERNAL, ERR_INVALID_ARG};

thread_local! {
    static LAST_ERROR: RefCell<(i32, CString)> =
        RefCell::new((0, CString::new("").unwrap()));
}

pub fn set_error(code: i32, msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = (
            code,
            CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap()),
        );
    });
}

fn clear_error() {
    set_error(0, "");
}

#[no_mangle]
pub extern "C" fn rk_version() -> *const c_char {
    static VERSION_C: &[u8] = b"0.1.0\0";
    VERSION_C.as_ptr() as *const c_char
}

#[no_mangle]
pub extern "C" fn rk_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().1.as_ptr())
}

#[no_mangle]
pub extern "C" fn rk_last_error_code() -> c_int {
    LAST_ERROR.with(|e| e.borrow().0) as c_int
}

/// # Safety (caller contract): `p` is null or a string this library
/// returned and has not been freed yet. Documented in the header.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn rk_free_string(p: *mut c_char) {
    if !p.is_null() {
        unsafe {
            drop(CString::from_raw(p));
        }
    }
}

/// Shared body for the three JSON-in / JSON-out exports. A null or
/// non-UTF-8 input is RK_ERR_INVALID_ARG; a spec/input problem the
/// core reports keeps its own code; a panic becomes
/// RK_ERR_INTERNAL. Success clears the last error and returns a
/// caller-owned string.
fn run_json(
    input: *const c_char,
    what: &str,
    f: fn(&str) -> Result<String, (i32, String)>,
) -> *mut c_char {
    if input.is_null() {
        set_error(ERR_INVALID_ARG, &format!("{what}: input pointer is null"));
        return std::ptr::null_mut();
    }
    // SAFETY: the caller contract (header) is that `input` is a
    // valid NUL-terminated UTF-8 string for the duration of the call.
    let text = match unsafe { CStr::from_ptr(input) }.to_str() {
        Ok(t) => t.to_string(),
        Err(_) => {
            set_error(
                ERR_INVALID_ARG,
                &format!("{what}: input is not valid UTF-8"),
            );
            return std::ptr::null_mut();
        }
    };
    match catch_unwind(|| f(&text)) {
        Ok(Ok(json)) => match CString::new(json) {
            Ok(c) => {
                clear_error();
                c.into_raw()
            }
            Err(_) => {
                set_error(
                    ERR_INTERNAL,
                    &format!("{what}: result contained a NUL byte"),
                );
                std::ptr::null_mut()
            }
        },
        Ok(Err((code, msg))) => {
            set_error(code, &msg);
            std::ptr::null_mut()
        }
        Err(_) => {
            set_error(ERR_INTERNAL, &format!("{what}: internal panic"));
            std::ptr::null_mut()
        }
    }
}

/// # Safety (caller contract): `spec_json` is null or a valid
/// NUL-terminated UTF-8 string. Documented in the header.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn rk_preview(spec_json: *const c_char) -> *mut c_char {
    run_json(spec_json, "rk_preview", preview_json)
}

/// # Safety (caller contract): `spec_json` is null or a valid
/// NUL-terminated UTF-8 string. Documented in the header.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn rk_apply(spec_json: *const c_char) -> *mut c_char {
    run_json(spec_json, "rk_apply", engine::apply_json)
}

/// # Safety (caller contract): `log_json` is null or a valid
/// NUL-terminated UTF-8 string. Documented in the header.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
#[no_mangle]
pub extern "C" fn rk_rollback(log_json: *const c_char) -> *mut c_char {
    run_json(log_json, "rk_rollback", engine::rollback_json)
}
