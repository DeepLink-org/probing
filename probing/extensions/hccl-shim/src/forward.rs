//! Lazy forward to the real CANN `libprofapi.so` (never dlopen the shim name).

#![cfg(target_os = "linux")]

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use once_cell::sync::OnceCell;

type ProfCommandHandle = Option<unsafe extern "C" fn(u32, *mut c_void, u32) -> i32>;

type FnRegisterCallback = unsafe extern "C" fn(u32, ProfCommandHandle) -> i32;
type FnRegTypeInfo = unsafe extern "C" fn(u16, u32, *const c_char) -> i32;
type FnReportApi = unsafe extern "C" fn(u32, *const c_void) -> i32;
type FnReportBlob = unsafe extern "C" fn(u32, *const c_void, u32) -> i32;
type FnGetHashId = unsafe extern "C" fn(*const c_char, u32) -> u64;
type FnSysCycleTime = unsafe extern "C" fn() -> u64;
type FnI32 = unsafe extern "C" fn() -> i32;
type FnBool = unsafe extern "C" fn() -> bool;
type FnInit = unsafe extern "C" fn(u32, *mut c_void, u32) -> i32;
type FnStartStop = unsafe extern "C" fn(u32, *const c_void, u32) -> i32;
type FnReportData = unsafe extern "C" fn(u32, u32, *mut c_void, u32) -> i32;
type FnNotifySetDevice = unsafe extern "C" fn(u32, u32, bool) -> i32;
type FnProfileCallback = unsafe extern "C" fn(i32, *mut c_void, u32) -> i32;
type FnSetConfig = unsafe extern "C" fn(u32, *const c_char, usize) -> i32;
type FnReportEvent = unsafe extern "C" fn(u32, *const c_void) -> i32;
type FnBatchMax = unsafe extern "C" fn(u32) -> usize;
type FnRegFormat = unsafe extern "C" fn(u16, u32, *const c_char) -> i32;
type FnStr2Id = unsafe extern "C" fn(*const c_char, usize) -> u64;
type FnGeModel = unsafe extern "C" fn(u32, u32) -> i32;
type FnRawCb = Option<unsafe extern "C" fn(*mut c_void) -> i32>;
type FnSubscribe = unsafe extern "C" fn(FnRawCb) -> i32;

struct RealApi {
    register_callback: FnRegisterCallback,
    reg_type_info: FnRegTypeInfo,
    report_api: FnReportApi,
    report_compact: FnReportBlob,
    report_additional: FnReportBlob,
    get_hash_id: FnGetHashId,
    sys_cycle_time: FnSysCycleTime,
    // Required when LD_LIBRARY_PATH points at this shim (Ascend runtime links libprofapi).
    report_data: Option<FnReportData>,
    init: Option<FnInit>,
    finalize: Option<FnI32>,
    start: Option<FnStartStop>,
    stop: Option<FnStartStop>,
    notify_set_device: Option<FnNotifySetDevice>,
    register_profile_callback: Option<FnProfileCallback>,
    set_config: Option<FnSetConfig>,
    report_event: Option<FnReportEvent>,
    report_batch_additional: Option<FnReportBlob>,
    get_batch_max: Option<FnBatchMax>,
    reg_data_format: Option<FnRegFormat>,
    str2id: Option<FnStr2Id>,
    set_device_by_ge: Option<FnGeModel>,
    unset_device_by_ge: Option<FnGeModel>,
    host_freq_is_enable: Option<FnBool>,
    subscribe_raw: Option<FnSubscribe>,
    unsubscribe_raw: Option<FnI32>,
    // Atlas / ACL registration entry points (needed for acl.prof.init).
    prof_reg_reporter: Option<
        unsafe extern "C" fn(
            Option<unsafe extern "C" fn(u32, u32, *mut c_void, u32) -> i32>,
        ) -> i32,
    >,
    prof_reg_ctrl: Option<
        unsafe extern "C" fn(Option<unsafe extern "C" fn(u32, *mut c_void, u32) -> i32>) -> i32,
    >,
    prof_reg_device_state:
        Option<unsafe extern "C" fn(Option<unsafe extern "C" fn(*mut c_void, u32) -> i32>) -> i32>,
    prof_get_device_by_ge: Option<unsafe extern "C" fn(u32, *mut u32) -> i32>,
    prof_set_command: Option<unsafe extern "C" fn(*mut c_void, u32) -> i32>,
    prof_set_step_info: Option<unsafe extern "C" fn(u64, u16, *mut c_void) -> i32>,
}

struct RealLib {
    // Kept for the process lifetime; never dlclosed, so it is intentionally unread.
    #[allow(dead_code)]
    handle: *mut c_void,
    api: RealApi,
}

unsafe impl Send for RealLib {}
// SAFETY: after one-time init the handle/function pointers are only ever read.
// The real library is never dlclosed (process-lifetime), so the raw handle stays valid.
unsafe impl Sync for RealLib {}

// One-time init WITHOUT holding a lock across real-library calls. HCCL/MSProf calls
// re-enter the shim's exported `Msprof*` symbols; a held mutex here would deadlock
// (parking_lot::Mutex is not reentrant), which manifested as aclprofStart hanging.
static REAL: OnceCell<Option<RealLib>> = OnceCell::new();
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
        crate::log::info(msg);
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
            report_data: load_sym(handle, c"MsprofReportData"),
            init: load_sym(handle, c"MsprofInit"),
            finalize: load_sym(handle, c"MsprofFinalize"),
            start: load_sym(handle, c"MsprofStart"),
            stop: load_sym(handle, c"MsprofStop"),
            notify_set_device: load_sym(handle, c"MsprofNotifySetDevice"),
            register_profile_callback: load_sym(handle, c"MsprofRegisterProfileCallback"),
            set_config: load_sym(handle, c"MsprofSetConfig"),
            report_event: load_sym(handle, c"MsprofReportEvent"),
            report_batch_additional: load_sym(handle, c"MsprofReportBatchAdditionalInfo"),
            get_batch_max: load_sym(handle, c"MsprofGetBatchReportMaxSize"),
            reg_data_format: load_sym(handle, c"MsprofRegDataFormat"),
            str2id: load_sym(handle, c"MsprofStr2Id"),
            set_device_by_ge: load_sym(handle, c"MsprofSetDeviceIdByGeModelIdx"),
            unset_device_by_ge: load_sym(handle, c"MsprofUnsetDeviceIdByGeModelIdx"),
            host_freq_is_enable: load_sym(handle, c"MsprofHostFreqIsEnable"),
            subscribe_raw: load_sym(handle, c"MsprofSubscribeRawData"),
            unsubscribe_raw: load_sym(handle, c"MsprofUnSubscribeRawData"),
            prof_reg_reporter: load_sym(handle, c"profRegReporterCallback"),
            prof_reg_ctrl: load_sym(handle, c"profRegCtrlCallback"),
            prof_reg_device_state: load_sym(handle, c"profRegDeviceStateCallback"),
            prof_get_device_by_ge: load_sym(handle, c"profGetDeviceIdByGeModelIdx"),
            prof_set_command: load_sym(handle, c"profSetProfCommand"),
            prof_set_step_info: load_sym(handle, c"profSetStepInfo"),
        };
        log_once(&format!("forwarding to {}", path.display()));
        return Some(RealLib { handle, api });
    }
    None
}

fn real_api() -> Option<&'static RealApi> {
    let slot = REAL.get_or_init(|| {
        let lib = unsafe { open_real() };
        if lib.is_none()
            && LOGGED_INIT
                .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            crate::log::warn(format!(
                "real libprofapi not found; MSProf forward disabled. \
                 Set {ENV_REAL} or place {REAL_BASENAME} next to the shim."
            ));
        }
        lib
    });
    slot.as_ref().map(|lib| &lib.api)
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
    if let Some(api) = real_api() {
        return unsafe { (api.register_callback)(module_id, handle) };
    }
    unsafe { stub_register(module_id, handle) }
}

pub fn forward_reg_type_info(level: u16, type_id: u32, type_name: *const c_char) -> i32 {
    if let Some(api) = real_api() {
        return unsafe { (api.reg_type_info)(level, type_id, type_name) };
    }
    unsafe { stub_reg_type(level, type_id, type_name) }
}

pub fn forward_report_api(aging: u32, api_ptr: *const c_void) -> i32 {
    if let Some(api) = real_api() {
        return unsafe { (api.report_api)(aging, api_ptr) };
    }
    unsafe { stub_report_api(aging, api_ptr) }
}

pub fn forward_report_compact(aging: u32, data: *const c_void, len: u32) -> i32 {
    if let Some(api) = real_api() {
        return unsafe { (api.report_compact)(aging, data, len) };
    }
    unsafe { stub_report_blob(aging, data, len) }
}

pub fn forward_report_additional(aging: u32, data: *const c_void, len: u32) -> i32 {
    if let Some(api) = real_api() {
        return unsafe { (api.report_additional)(aging, data, len) };
    }
    unsafe { stub_report_blob(aging, data, len) }
}

pub fn forward_get_hash_id(hash_info: *const c_char, length: u32) -> u64 {
    if hash_info.is_null() || length == 0 {
        return 0;
    }
    if let Some(api) = real_api() {
        return unsafe { (api.get_hash_id)(hash_info, length) };
    }
    unsafe { stub_hash(hash_info, length) }
}

pub fn forward_sys_cycle_time() -> u64 {
    if let Some(api) = real_api() {
        return unsafe { (api.sys_cycle_time)() };
    }
    unsafe { stub_time() }
}

macro_rules! forward_opt {
    ($field:ident, $stub:expr, $($arg:expr),* $(,)?) => {{
        if let Some(api) = real_api() {
            if let Some(f) = api.$field {
                return unsafe { f($($arg),*) };
            }
        }
        $stub
    }};
}

pub fn forward_report_data(module_id: u32, ty: u32, data: *mut c_void, len: u32) -> i32 {
    forward_opt!(report_data, 0, module_id, ty, data, len)
}

pub fn forward_init(data_type: u32, data: *mut c_void, data_len: u32) -> i32 {
    forward_opt!(init, 0, data_type, data, data_len)
}

pub fn forward_finalize() -> i32 {
    forward_opt!(finalize, 0,)
}

pub fn forward_start(data_type: u32, data: *const c_void, length: u32) -> i32 {
    forward_opt!(start, 0, data_type, data, length)
}

pub fn forward_stop(data_type: u32, data: *const c_void, length: u32) -> i32 {
    forward_opt!(stop, 0, data_type, data, length)
}

pub fn forward_notify_set_device(chip_id: u32, device_id: u32, is_open: bool) -> i32 {
    forward_opt!(notify_set_device, 0, chip_id, device_id, is_open)
}

pub fn forward_register_profile_callback(cb_type: i32, callback: *mut c_void, len: u32) -> i32 {
    forward_opt!(register_profile_callback, 0, cb_type, callback, len)
}

pub fn forward_set_config(config_type: u32, config: *const c_char, config_length: usize) -> i32 {
    forward_opt!(set_config, 0, config_type, config, config_length)
}

pub fn forward_report_event(aging: u32, event: *const c_void) -> i32 {
    forward_opt!(report_event, 0, aging, event)
}

pub fn forward_report_batch_additional(aging: u32, data: *const c_void, len: u32) -> i32 {
    forward_opt!(report_batch_additional, 0, aging, data, len)
}

pub fn forward_get_batch_max(ty: u32) -> usize {
    forward_opt!(get_batch_max, 0, ty)
}

pub fn forward_reg_data_format(level: u16, type_id: u32, data_format: *const c_char) -> i32 {
    forward_opt!(reg_data_format, 0, level, type_id, data_format)
}

pub fn forward_str2id(hash_info: *const c_char, length: usize) -> u64 {
    forward_opt!(str2id, 0, hash_info, length)
}

pub fn forward_set_device_by_ge(ge_model_idx: u32, device_id: u32) -> i32 {
    forward_opt!(set_device_by_ge, 0, ge_model_idx, device_id)
}

pub fn forward_unset_device_by_ge(ge_model_idx: u32, device_id: u32) -> i32 {
    forward_opt!(unset_device_by_ge, 0, ge_model_idx, device_id)
}

pub fn forward_host_freq_is_enable() -> bool {
    forward_opt!(host_freq_is_enable, false,)
}

pub fn forward_subscribe_raw(cb: FnRawCb) -> i32 {
    forward_opt!(subscribe_raw, 0, cb)
}

pub fn forward_unsubscribe_raw() -> i32 {
    forward_opt!(unsubscribe_raw, 0,)
}

pub type ReportHandle = Option<unsafe extern "C" fn(u32, u32, *mut c_void, u32) -> i32>;
pub type CtrlHandle = Option<unsafe extern "C" fn(u32, *mut c_void, u32) -> i32>;
pub type DeviceStateHandle = Option<unsafe extern "C" fn(*mut c_void, u32) -> i32>;

pub fn forward_prof_reg_reporter(reporter: ReportHandle) -> i32 {
    forward_opt!(prof_reg_reporter, -1, reporter)
}

pub fn forward_prof_reg_ctrl(handle: CtrlHandle) -> i32 {
    forward_opt!(prof_reg_ctrl, -1, handle)
}

pub fn forward_prof_reg_device_state(handle: DeviceStateHandle) -> i32 {
    forward_opt!(prof_reg_device_state, -1, handle)
}

pub fn forward_prof_get_device_by_ge(model_idx: u32, device_id: *mut u32) -> i32 {
    forward_opt!(prof_get_device_by_ge, -1, model_idx, device_id)
}

pub fn forward_prof_set_command(command: *mut c_void, len: u32) -> i32 {
    forward_opt!(prof_set_command, -1, command, len)
}

pub fn forward_prof_set_step_info(index_id: u64, tag_id: u16, stream: *mut c_void) -> i32 {
    forward_opt!(prof_set_step_info, -1, index_id, tag_id, stream)
}
