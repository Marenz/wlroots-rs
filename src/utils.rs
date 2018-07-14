//! Utility functions for use within wlroots-rs

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::process::exit;
use std::time::Duration;

use libc::{clock_gettime, CLOCK_MONOTONIC, timespec};

use wlroots_sys::{__va_list_tag, wlr_log_init, wlr_edges};
pub use wlroots_sys::wlr_log_importance::{self, *};

static mut RUST_LOGGING_FN: LogCallback = dummy_callback;

/// The signature for the callback function you can hook into the logging
/// functionality of wlroots.
pub type LogCallback = fn(LogVerbosity, String);

/// How verbose you want the logging. Lower levels prints more.
pub type LogVerbosity = wlr_log_importance;

/// Initialize wlroots logging at a certain level of verbosity with
/// an optional callback that will be called for every log.
///
/// To log using this system, use the `wlr_log!` macro.
// TODO Wrap the callback function type
pub fn init_logging<T>(verbosity: LogVerbosity, callback: T)
    where T: Into<Option<LogCallback>>
{
    unsafe {
        match callback.into() {
            None => wlr_log_init(verbosity, None),
            Some(callback) => {
                RUST_LOGGING_FN = callback;
                wlr_log_init(verbosity, Some(log_callback));
            }
        }
    }
}

/// Dummy callback to fill in RUST_LOGGING_FN when it's not in use.
fn dummy_callback(_: LogVerbosity, _: String) {}

/// Real hook into the logging callback, calls the real user-supplied callback
/// with nice Rust inputs.
unsafe extern "C" fn log_callback(importance: wlr_log_importance,
                                  fmt: *const c_char,
                                  _va_list: *mut __va_list_tag) {
    RUST_LOGGING_FN(importance,
                    c_to_rust_string(fmt).unwrap_or_else(|| "".into()))
}

/// Trait to convert something to mili seconds.
///
/// Used primarily to convert a `std::time::Duration` into
/// something usable by wlroots
pub trait ToMS {
    fn to_ms(self) -> u32;
}

impl ToMS for Duration {
    fn to_ms(self) -> u32 {
        let seconds_delta = self.as_secs() as u32;
        let nano_delta = self.subsec_nanos();
        (seconds_delta * 1000) + nano_delta / 1000000
    }
}

/// Converts a Rust string into C string without error handling.
/// If any error occurs, it is logged and then the program is immediantly
/// aborted.
pub fn safe_as_cstring<S>(string: S) -> CString
    where S: Into<Vec<u8>>
{
    match CString::new(string) {
        Ok(string) => string,
        Err(err) => {
            wlr_log!(WLR_ERROR,
                     "Error occured while trying to convert a Rust string to a C string {:?}",
                     err);
            exit(1)
        }
    }
}

/// Converts a C string into a Rust string without error handling.
/// The pointer passed to this function _must_ be valid.
pub unsafe fn c_to_rust_string(c_str: *const c_char) -> Option<String> {
    if c_str.is_null() {
        None
    } else {
        Some(CStr::from_ptr(c_str).to_string_lossy().into_owned())
    }
}

/// Handle unwinding from a panic, used in conjunction with
/// `::std::panic::catch_unwind`.
///
/// When a panic occurs, we terminate the compositor and let the rest
/// of the code run.
pub(crate) unsafe fn handle_unwind<T>(res: ::std::thread::Result<T>) {
    match res {
        Ok(_) => {}
        Err(err) => {
            if ::compositor::COMPOSITOR_PTR == 0 as *mut _ {
                ::std::process::abort();
            }
            (&mut *::compositor::COMPOSITOR_PTR).save_panic_error(err);
            ::compositor::terminate()
        }
    }
}

/// Get the current time as a duration suitable for `surface.send_frame_done()` and synthetic seat
/// events.
pub fn current_time() -> Duration {
    unsafe {
        let mut ts = timespec{tv_sec: 0, tv_nsec: 0};
        clock_gettime(CLOCK_MONOTONIC, &mut ts);
        Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
    }
}

bitflags! {
    pub struct Edges: u32 {
        const WLR_EDGE_NONE = wlr_edges::WLR_EDGE_NONE as u32;
        const WLR_EDGE_TOP = wlr_edges::WLR_EDGE_TOP as u32;
        const WLR_EDGE_BOTTOM = wlr_edges::WLR_EDGE_BOTTOM as u32;
        const WLR_EDGE_LEFT = wlr_edges::WLR_EDGE_LEFT as u32;
        const WLR_EDGE_RIGHT = wlr_edges::WLR_EDGE_RIGHT as u32;
    }
}
