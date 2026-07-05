//! Logging for the NCCL profiler plugin (dlopen'd without the main probing module).
//!
//! Prefer NCCL's `ncclDebugLogger_t` when v4 `init` provides one; otherwise fall
//! back to the Rust `log` crate with a one-time `env_logger` init (`PROBING_LOGLEVEL`,
//! default `warn`).

use std::ffi::{c_char, CString};
use std::os::raw::c_int;
use std::sync::Once;

use once_cell::sync::OnceCell;

/// NCCL `ncclDebugLogLevel` values we use.
pub const NCCL_LOG_WARN: c_int = 2;
pub const NCCL_LOG_INFO: c_int = 3;
pub const NCCL_LOG_TRACE: c_int = 5;

/// NCCL subsystem flag for profiler plugins (`NCCL_PROFILE`).
pub const NCCL_PROFILE: u64 = 0x4000;

/// C variadic logger invoked with a single `%s` argument (stable Rust calling convention).
pub type NcclDebugLoggerFn = Option<
    unsafe extern "C" fn(
        level: c_int,
        flags: u64,
        file: *const c_char,
        line: c_int,
        fmt: *const c_char,
        msg: *const c_char,
    ),
>;

static NCCL_LOGGER: OnceCell<NcclDebugLoggerFn> = OnceCell::new();
static RUST_LOG: Once = Once::new();

const LOG_TARGET: &str = "probing-nccl-profiler";

pub fn set_nccl_logger(logfn: NcclDebugLoggerFn) {
    if logfn.is_some() {
        let _ = NCCL_LOGGER.set(logfn);
    }
    ensure_rust_logger();
}

pub fn ensure_rust_logger() {
    RUST_LOG.call_once(|| {
        let _ = env_logger::Builder::from_env(
            env_logger::Env::new()
                .default_filter_or("warn")
                .filter("PROBING_LOGLEVEL"),
        )
        .format_target(true)
        .try_init();
    });
}

fn nccl_emit(level: c_int, msg: &str) {
    if let Some(f) = NCCL_LOGGER.get().and_then(|f| *f) {
        let Ok(cmsg) = CString::new(msg) else {
            return;
        };
        unsafe {
            f(
                level,
                NCCL_PROFILE,
                c"probing-nccl-profiler".as_ptr(),
                0,
                c"%s".as_ptr(),
                cmsg.as_ptr(),
            );
        }
        return;
    }
    match level {
        NCCL_LOG_WARN => log::warn!(target: LOG_TARGET, "{msg}"),
        NCCL_LOG_INFO => log::info!(target: LOG_TARGET, "{msg}"),
        _ => log::debug!(target: LOG_TARGET, "{msg}"),
    }
}

pub fn info(msg: impl AsRef<str>) {
    ensure_rust_logger();
    nccl_emit(NCCL_LOG_INFO, msg.as_ref());
}

pub fn warn(msg: impl AsRef<str>) {
    ensure_rust_logger();
    nccl_emit(NCCL_LOG_WARN, msg.as_ref());
}

pub fn debug(msg: impl AsRef<str>) {
    ensure_rust_logger();
    nccl_emit(NCCL_LOG_TRACE, msg.as_ref());
}
