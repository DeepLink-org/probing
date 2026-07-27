//! `libprofapi.so` shim — intercept MSProf, write `hccl.*` memtables, forward to CANN.

#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(target_os = "linux")]
mod forward;
mod log;
mod msprof;
mod names;
pub mod tables;
pub use tables::register_docs;
mod writer;

#[cfg(not(target_os = "linux"))]
mod forward {
    use std::os::raw::{c_char, c_void};
    type ProfCommandHandle = Option<unsafe extern "C" fn(u32, *mut c_void, u32) -> i32>;
    pub fn forward_register(_: u32, _: ProfCommandHandle) -> i32 {
        0
    }
    pub fn forward_reg_type_info(_: u16, _: u32, _: *const c_char) -> i32 {
        0
    }
    pub fn forward_report_api(_: u32, _: *const c_void) -> i32 {
        0
    }
    pub fn forward_report_compact(_: u32, _: *const c_void, _: u32) -> i32 {
        0
    }
    pub fn forward_report_additional(_: u32, _: *const c_void, _: u32) -> i32 {
        0
    }
    pub fn forward_get_hash_id(_: *const c_char, _: u32) -> u64 {
        0
    }
    pub fn forward_sys_cycle_time() -> u64 {
        0
    }
}

pub use tables::{
    collectives_schema, context_ids_schema, host_ops_schema, mc2_streams_schema, tasks_schema,
    COLLECTIVES_FILE, CONTEXT_IDS_FILE, HOST_OPS_FILE, MC2_STREAMS_FILE, TASKS_FILE,
};

use std::os::raw::c_void;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::msprof::{
    classify_additional, is_hccl_op_compact, read_additional_header, read_api, read_compact_header,
    read_context_id_info, read_hccl_info, read_hccl_op_info, read_mc2_comm_info, AdditionalKind,
    MSPROF_ADDITIONAL_HEADER, MSPROF_BLOB_HEADER,
};
use crate::names::{lookup_type_id, preseed_hashes};
use crate::writer::HcclWriter;

static WRITER: Lazy<Mutex<HcclWriter>> = Lazy::new(|| Mutex::new(HcclWriter::new()));

type ProfCommandHandle = Option<unsafe extern "C" fn(u32, *mut c_void, u32) -> i32>;

fn hash_fn(s: *const std::os::raw::c_char, l: u32) -> u64 {
    forward::forward_get_hash_id(s, l)
}

fn ensure_names() {
    preseed_hashes(hash_fn);
}

fn capture_api(aging: u32, ptr: *const c_void) {
    if ptr.is_null() {
        return;
    }
    ensure_names();
    if let Some(api) = read_api(ptr as *const u8, crate::msprof::MSPROF_API_SIZE as u32) {
        WRITER.lock().record_api(aging, &api);
    }
}

fn capture_compact(_aging: u32, ptr: *const c_void, len: u32) {
    if ptr.is_null() {
        return;
    }
    ensure_names();
    let Some(header) = read_compact_header(ptr as *const u8, len) else {
        return;
    };
    let data_ptr = unsafe { (ptr as *const u8).add(MSPROF_BLOB_HEADER) };
    let type_name = lookup_type_id(header.type_id);
    if is_hccl_op_compact(&type_name, header.data_len) {
        if let Some(op) = read_hccl_op_info(data_ptr, header.data_len) {
            WRITER.lock().record_compact_hccl_op(&header, &op);
        }
    }
}

fn capture_additional(_aging: u32, ptr: *const c_void, len: u32) {
    if ptr.is_null() {
        return;
    }
    ensure_names();
    let Some(header) = read_additional_header(ptr as *const u8, len) else {
        return;
    };
    let data_ptr = unsafe { (ptr as *const u8).add(MSPROF_ADDITIONAL_HEADER) };
    let type_name = lookup_type_id(header.type_id);
    match classify_additional(header.type_id, &type_name, header.data_len) {
        AdditionalKind::HcclTask => {
            if let Some(hccl) = read_hccl_info(data_ptr, header.data_len) {
                WRITER
                    .lock()
                    .record_task(&header, &hccl, header.data_len as i32);
            }
        }
        AdditionalKind::Mc2Comm => {
            if let Some(mc2) = read_mc2_comm_info(data_ptr, header.data_len) {
                WRITER.lock().record_mc2(&header, &mc2);
            }
        }
        AdditionalKind::ContextId => {
            if let Some(ctx) = read_context_id_info(data_ptr, header.data_len) {
                WRITER.lock().record_context(&header, &ctx);
            }
        }
        AdditionalKind::Unknown => {}
    }
}

#[cfg(target_os = "linux")]
mod export {
    use std::os::raw::{c_char, c_void};

    use super::*;

    #[no_mangle]
    pub unsafe extern "C" fn MsprofRegisterCallback(
        module_id: u32,
        handle: ProfCommandHandle,
    ) -> i32 {
        forward::forward_register(module_id, handle)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofRegTypeInfo(
        level: u16,
        type_id: u32,
        type_name: *const c_char,
    ) -> i32 {
        ensure_names();
        crate::names::register_type_info(type_id, type_name, hash_fn);
        forward::forward_reg_type_info(level, type_id, type_name)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofReportApi(aging_flag: u32, api: *const c_void) -> i32 {
        capture_api(aging_flag, api);
        forward::forward_report_api(aging_flag, api)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofReportCompactInfo(
        aging_flag: u32,
        data: *const c_void,
        length: u32,
    ) -> i32 {
        capture_compact(aging_flag, data, length);
        forward::forward_report_compact(aging_flag, data, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofReportAdditionalInfo(
        aging_flag: u32,
        data: *const c_void,
        length: u32,
    ) -> i32 {
        capture_additional(aging_flag, data, length);
        forward::forward_report_additional(aging_flag, data, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofGetHashId(hash_info: *const c_char, length: u32) -> u64 {
        ensure_names();
        let hash = forward::forward_get_hash_id(hash_info, length);
        crate::names::register_hash_string(hash_info, length, hash);
        hash
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofSysCycleTime() -> u64 {
        forward::forward_sys_cycle_time()
    }

    // Passthrough exports so Ascend runtime can resolve libprofapi when this shim
    // is first on LD_LIBRARY_PATH (e.g. MsprofReportData from libruntime_common).
    #[no_mangle]
    pub unsafe extern "C" fn MsprofReportData(
        module_id: u32,
        ty: u32,
        data: *mut c_void,
        len: u32,
    ) -> i32 {
        forward::forward_report_data(module_id, ty, data, len)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofInit(data_type: u32, data: *mut c_void, data_len: u32) -> i32 {
        forward::forward_init(data_type, data, data_len)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofFinalize() -> i32 {
        forward::forward_finalize()
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofStart(
        data_type: u32,
        data: *const c_void,
        length: u32,
    ) -> i32 {
        forward::forward_start(data_type, data, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofStop(
        data_type: u32,
        data: *const c_void,
        length: u32,
    ) -> i32 {
        forward::forward_stop(data_type, data, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofNotifySetDevice(
        chip_id: u32,
        device_id: u32,
        is_open: bool,
    ) -> i32 {
        forward::forward_notify_set_device(chip_id, device_id, is_open)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofRegisterProfileCallback(
        callback_type: i32,
        callback: *mut c_void,
        len: u32,
    ) -> i32 {
        forward::forward_register_profile_callback(callback_type, callback, len)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofSetConfig(
        config_type: u32,
        config: *const c_char,
        config_length: usize,
    ) -> i32 {
        forward::forward_set_config(config_type, config, config_length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofReportEvent(aging_flag: u32, event: *const c_void) -> i32 {
        forward::forward_report_event(aging_flag, event)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofReportBatchAdditionalInfo(
        aging_flag: u32,
        data: *const c_void,
        length: u32,
    ) -> i32 {
        forward::forward_report_batch_additional(aging_flag, data, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofGetBatchReportMaxSize(ty: u32) -> usize {
        forward::forward_get_batch_max(ty)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofRegDataFormat(
        level: u16,
        type_id: u32,
        data_format: *const c_char,
    ) -> i32 {
        forward::forward_reg_data_format(level, type_id, data_format)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofStr2Id(hash_info: *const c_char, length: usize) -> u64 {
        forward::forward_str2id(hash_info, length)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofSetDeviceIdByGeModelIdx(
        ge_model_idx: u32,
        device_id: u32,
    ) -> i32 {
        forward::forward_set_device_by_ge(ge_model_idx, device_id)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofUnsetDeviceIdByGeModelIdx(
        ge_model_idx: u32,
        device_id: u32,
    ) -> i32 {
        forward::forward_unset_device_by_ge(ge_model_idx, device_id)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofHostFreqIsEnable() -> bool {
        forward::forward_host_freq_is_enable()
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofSubscribeRawData(
        cb: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    ) -> i32 {
        forward::forward_subscribe_raw(cb)
    }

    #[no_mangle]
    pub unsafe extern "C" fn MsprofUnSubscribeRawData() -> i32 {
        forward::forward_unsubscribe_raw()
    }

    #[no_mangle]
    pub unsafe extern "C" fn profRegReporterCallback(
        reporter: forward::ReportHandle,
    ) -> i32 {
        forward::forward_prof_reg_reporter(reporter)
    }

    #[no_mangle]
    pub unsafe extern "C" fn profRegCtrlCallback(handle: forward::CtrlHandle) -> i32 {
        forward::forward_prof_reg_ctrl(handle)
    }

    #[no_mangle]
    pub unsafe extern "C" fn profRegDeviceStateCallback(
        handle: forward::DeviceStateHandle,
    ) -> i32 {
        forward::forward_prof_reg_device_state(handle)
    }

    #[no_mangle]
    pub unsafe extern "C" fn profGetDeviceIdByGeModelIdx(
        model_idx: u32,
        device_id: *mut u32,
    ) -> i32 {
        forward::forward_prof_get_device_by_ge(model_idx, device_id)
    }

    #[no_mangle]
    pub unsafe extern "C" fn profSetProfCommand(command: *mut c_void, len: u32) -> i32 {
        forward::forward_prof_set_command(command, len)
    }

    #[no_mangle]
    pub unsafe extern "C" fn profSetStepInfo(
        index_id: u64,
        tag_id: u16,
        stream: *mut c_void,
    ) -> i32 {
        forward::forward_prof_set_step_info(index_id, tag_id, stream)
    }
}

#[cfg(not(target_os = "linux"))]
mod stub {
    pub const BUILD_NOTE: &str = "probing-hccl-shim: libprofapi.so built on Linux only";
}
