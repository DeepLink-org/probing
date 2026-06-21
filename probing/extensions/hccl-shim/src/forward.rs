//! Lazy forward to the real CANN `libprofapi.so` (never dlopen the shim name).

#![cfg(target_os = "linux")]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

type ProfCommandHandle = Option<unsafe extern "C" fn(u32, *mut c_void, u32) -> i32>;

type FnRegisterCallback = unsafe extern "C" fn(u32, ProfCommandHandle) -> i32;
type FnRegTypeInfo = unsafe extern "C" fn(u16, u32, *const c_char) -> i32;
type FnReportApi = unsafe extern "C" fn(u32, *const c_void) -> i32;
type FnReportBlob = unsafe extern "C" fn(u32, *const c_void, u32) -> i32;
type FnGetHashId = unsafe extern "C" fn(*const c_char, u32) -> u64;
type FnSysCycleTime = unsafe extern "C" fn() -> u64;

struct RealApi {
    register_callback: FnRegisterCallback,
    reg_type_info: FnRegTypeInfo,
    report_api: FnReportApi,
    report_compact: FnReportBlob,
    report_additional: FnReportBlob,
    get_hash_id: FnGetHashId,
    sys_cycle_time: FnSysCycleTime,
}

struct RealLib {
    handle: *mut c_void,
    api: RealApi,
}

unsafe impl Send for RealLib {}

impl Drop for RealLib {
    fn drop(&mut self) {
        unsafe {
            if !self.handle.is_null() {
                libc::dlclose(self.handle);
            }
        }
    }
}

static INIT: Lazy<Mutex<Option<RealLib>>> = Lazy::new(|| Mutex::new(None));
static INIT_FAILED: AtomicBool = AtomicBool::new(false);
static LOGGED_INIT: AtomicBool = AtomicBool::new(false);

const ENV_REAL: &str = "PROBING_HCCL_PROFAPI_REAL";
const REAL_BASENAME: &str = "libprofapi.so.real";
const ENV_ASCEND_HOME: &str = "ASCEND_HOME";
const ENV_ASCEND_INSTALL: &str = "ASCEND_INSTALL_PATH";

fn log_once(msg: &str) {
    if std::env::var_os("PROBING_HCCL_SHIM_LOG").is_some()
        && LOGGED_INIT
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        eprintln!("[probing-hccl-shim] {msg}");
    }
}

fn shim_directory() -> Option<PathBuf> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    for line in maps.lines() {
        if !line.contains("libprofapi.so") {
            continue;
        }
        let path = line.split_whitespace().last()?;
        let p = Path::new(path);
        if p.is_absolute() {
            return p.parent().map(|d| d.to_path_buf());
        }
    }
    None
}

fn ascend_lib_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for key in [ENV_ASCEND_HOME, ENV_ASCEND_INSTALL] {
        if let Ok(v) = std::env::var(key) {
            let base = PathBuf::from(v);
            out.push(base.join("lib64"));
            out.push(base.join("lib"));
        }
    }
    out
}

fn candidate_real_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(p) = std::env::var(ENV_REAL) {
        out.push(PathBuf::from(p));
    }
    if let Some(dir) = shim_directory() {
        out.push(dir.join(REAL_BASENAME));
    }
    for libdir in ascend_lib_dirs() {
        out.push(libdir.join("libprofapi.so"));
    }
    out
}

unsafe fn load_sym<T>(handle: *mut c_void, name: &CStr) -> Option<T> {
    let sym = libc::dlsym(handle, name.as_ptr());
    if sym.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy(&sym))
    }
}

unsafe fn open_real() -> Option<RealLib> {
    for path in candidate_real_paths() {
        if !path.is_file() {
            continue;
        }
        let cpath = CString::new(path.as_os_str().as_bytes()).ok()?;
        let handle = libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            continue;
        }
        let api = RealApi {
            register_callback: load_sym(handle, c"MsprofRegisterCallback")?,
            reg_type_info: load_sym(handle, c"MsprofRegTypeInfo")?,
            report_api: load_sym(handle, c"MsprofReportApi")?,
            report_compact: load_sym(handle, c"MsprofReportCompactInfo")?,
            report_additional: load_sym(handle, c"MsprofReportAdditionalInfo")?,
            get_hash_id: load_sym(handle, c"MsprofGetHashId")?,
            sys_cycle_time: load_sym(handle, c"MsprofSysCycleTime")?,
        };
        log_once(&format!("forwarding to {}", path.display()));
        return Some(RealLib { handle, api });
    }
    None
}

fn real_lib() -> Option<parking_lot::MutexGuard<'static, Option<RealLib>>> {
    let mut guard = INIT.lock();
    if guard.is_none() && !INIT_FAILED.load(Ordering::Relaxed) {
        unsafe {
            *guard = open_real();
        }
        if guard.is_none() {
            INIT_FAILED.store(true, Ordering::Relaxed);
            if LOGGED_INIT
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                eprintln!(
                    "[probing-hccl-shim] real libprofapi not found; MSProf forward disabled. \
                     Set {ENV_REAL} or place {REAL_BASENAME} next to the shim."
                );
            }
        }
    }
    Some(guard)
}

unsafe extern "C" fn stub_register(_: u32, _: ProfCommandHandle) -> i32 {
    0
}
unsafe extern "C" fn stub_reg_type(_: u16, _: u32, _: *const c_char) -> i32 {
    0
}
unsafe extern "C" fn stub_report_api(_: u32, _: *const c_void) -> i32 {
    0
}
unsafe extern "C" fn stub_report_blob(_: u32, _: *const c_void, _: u32) -> i32 {
    0
}
unsafe extern "C" fn stub_hash(_: *const c_char, _: u32) -> u64 {
    0
}
unsafe extern "C" fn stub_time() -> u64 {
    0
}

pub fn forward_register(module_id: u32, handle: ProfCommandHandle) -> i32 {
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.register_callback)(module_id, handle) };
        }
    }
    unsafe { stub_register(module_id, handle) }
}

pub fn forward_reg_type_info(level: u16, type_id: u32, type_name: *const c_char) -> i32 {
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.reg_type_info)(level, type_id, type_name) };
        }
    }
    unsafe { stub_reg_type(level, type_id, type_name) }
}

pub fn forward_report_api(aging: u32, api: *const c_void) -> i32 {
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.report_api)(aging, api) };
        }
    }
    unsafe { stub_report_api(aging, api) }
}

pub fn forward_report_compact(aging: u32, data: *const c_void, len: u32) -> i32 {
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.report_compact)(aging, data, len) };
        }
    }
    unsafe { stub_report_blob(aging, data, len) }
}

pub fn forward_report_additional(aging: u32, data: *const c_void, len: u32) -> i32 {
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.report_additional)(aging, data, len) };
        }
    }
    unsafe { stub_report_blob(aging, data, len) }
}

pub fn forward_get_hash_id(hash_info: *const c_char, length: u32) -> u64 {
    if hash_info.is_null() || length == 0 {
        return 0;
    }
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.get_hash_id)(hash_info, length) };
        }
    }
    unsafe { stub_hash(hash_info, length) }
}

pub fn forward_sys_cycle_time() -> u64 {
    if let Some(guard) = real_lib() {
        if let Some(real) = guard.as_ref() {
            return unsafe { (real.api.sys_cycle_time)() };
        }
    }
    unsafe { stub_time() }
}
