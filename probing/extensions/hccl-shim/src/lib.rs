//! `libprofapi.so` shim — intercept MSProf, write `hccl.*` memtables, forward to CANN.

#![allow(clippy::missing_safety_doc)]
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(target_os = "linux")]
mod forward;
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
}

#[cfg(not(target_os = "linux"))]
mod stub {
    pub const BUILD_NOTE: &str = "probing-hccl-shim: libprofapi.so built on Linux only";
}
