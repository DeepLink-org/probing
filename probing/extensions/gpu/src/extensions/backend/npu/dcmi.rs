//! Ascend DCMI (`libdcmi.so`) — in-process device management API (NVML analogue).
//!
//! Loaded dynamically so non-Ascend hosts still build/link. Prefer this over
//! `npu-smi` subprocesses for utilization / HBM / temperature / power.

use std::collections::HashMap;
use std::os::raw::{c_int, c_uint};
use std::path::Path;
use std::sync::Mutex;

use libloading::Library;
use once_cell::sync::Lazy;

use super::npu_smi::NpuDeviceStats;

const DCMI_UTILIZATION_RATE_AICORE: c_int = 2;
const DCMI_UTILIZATION_RATE_HBM: c_int = 6;
const DCMI_UTILIZATION_RATE_NPU: c_int = 13;

/// Empirical: `dcmi_get_device_power_info` returns tenths of a watt (1640 → 164.0 W).
const POWER_SCALE: f32 = 0.1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct DcmiHbmInfo {
    memory_size: u64,
    freq: u32,
    memory_usage: u64,
    temp: c_int,
    bandwith_util_rate: u32,
}

type FnInit = unsafe extern "C" fn() -> c_int;
type FnGetCardList = unsafe extern "C" fn(*mut c_int, *mut c_int, c_int) -> c_int;
type FnGetDeviceNumInCard = unsafe extern "C" fn(c_int, *mut c_int) -> c_int;
type FnGetDeviceUtilizationRate = unsafe extern "C" fn(c_int, c_int, c_int, *mut c_uint) -> c_int;
type FnGetDeviceTemperature = unsafe extern "C" fn(c_int, c_int, *mut c_int) -> c_int;
type FnGetDevicePowerInfo = unsafe extern "C" fn(c_int, c_int, *mut c_int) -> c_int;
type FnGetDeviceHbmInfo = unsafe extern "C" fn(c_int, c_int, *mut DcmiHbmInfo) -> c_int;

struct DcmiApi {
    _lib: Library,
    init: FnInit,
    get_card_list: FnGetCardList,
    get_device_num_in_card: FnGetDeviceNumInCard,
    get_device_utilization_rate: FnGetDeviceUtilizationRate,
    get_device_temperature: FnGetDeviceTemperature,
    get_device_power_info: FnGetDevicePowerInfo,
    get_device_hbm_info: FnGetDeviceHbmInfo,
}

static DCMI: Lazy<Mutex<Option<DcmiApi>>> = Lazy::new(|| Mutex::new(load_dcmi()));

fn candidate_paths() -> Vec<String> {
    let mut paths = Vec::new();
    if let Ok(p) = std::env::var("PROBING_DCMI_LIB") {
        if !p.is_empty() {
            paths.push(p);
        }
    }
    paths.extend([
        "/usr/local/Ascend/driver/lib64/driver/libdcmi.so".to_string(),
        "/usr/local/Ascend/driver/lib64/libdcmi.so".to_string(),
        "libdcmi.so".to_string(),
    ]);
    paths
}

fn load_dcmi() -> Option<DcmiApi> {
    if matches!(
        std::env::var("PROBING_NPU_SOURCE").ok().as_deref(),
        Some(v) if matches!(v.trim().to_ascii_lowercase().as_str(), "smi" | "npu-smi" | "cli")
    ) {
        log::info!("DCMI disabled via PROBING_NPU_SOURCE");
        return None;
    }

    for path in candidate_paths() {
        if path != "libdcmi.so" && !Path::new(&path).exists() {
            continue;
        }
        // SAFETY: we only call Ascend DCMI C ABI symbols after successful load.
        let result = unsafe { try_load_path(&path) };
        match result {
            Ok(api) => {
                let rc = unsafe { (api.init)() };
                if rc != 0 {
                    log::warn!("dcmi_init failed on {path}: rc={rc}");
                    continue;
                }
                log::info!("DCMI loaded from {path}");
                return Some(api);
            }
            Err(e) => log::debug!("DCMI load skip {path}: {e}"),
        }
    }
    None
}

unsafe fn try_load_path(path: &str) -> Result<DcmiApi, String> {
    let lib = Library::new(path).map_err(|e| e.to_string())?;
    Ok(DcmiApi {
        init: *lib
            .get::<FnInit>(b"dcmi_init\0")
            .map_err(|e| e.to_string())?,
        get_card_list: *lib
            .get::<FnGetCardList>(b"dcmi_get_card_list\0")
            .map_err(|e| e.to_string())?,
        get_device_num_in_card: *lib
            .get::<FnGetDeviceNumInCard>(b"dcmi_get_device_num_in_card\0")
            .map_err(|e| e.to_string())?,
        get_device_utilization_rate: *lib
            .get::<FnGetDeviceUtilizationRate>(b"dcmi_get_device_utilization_rate\0")
            .map_err(|e| e.to_string())?,
        get_device_temperature: *lib
            .get::<FnGetDeviceTemperature>(b"dcmi_get_device_temperature\0")
            .map_err(|e| e.to_string())?,
        get_device_power_info: *lib
            .get::<FnGetDevicePowerInfo>(b"dcmi_get_device_power_info\0")
            .map_err(|e| e.to_string())?,
        get_device_hbm_info: *lib
            .get::<FnGetDeviceHbmInfo>(b"dcmi_get_device_hbm_info\0")
            .map_err(|e| e.to_string())?,
        _lib: lib,
    })
}

fn util_or_neg1(api: &DcmiApi, card: c_int, device: c_int, kind: c_int) -> f32 {
    let mut rate: c_uint = 0;
    let rc = unsafe { (api.get_device_utilization_rate)(card, device, kind, &mut rate) };
    if rc == 0 {
        (rate as f32).clamp(0.0, 100.0)
    } else {
        -1.0
    }
}

/// Batch-read via DCMI. Returns `None` if library unavailable or no devices.
pub fn read_utilization_by_index() -> Option<HashMap<i32, NpuDeviceStats>> {
    let guard = DCMI.lock().ok()?;
    let api = guard.as_ref()?;

    let mut card_num: c_int = 0;
    let mut cards = [0_i32; 64];
    let rc =
        unsafe { (api.get_card_list)(&mut card_num, cards.as_mut_ptr(), cards.len() as c_int) };
    if rc != 0 || card_num <= 0 {
        return None;
    }

    let mut map = HashMap::new();
    for &card_id in cards.iter().take(card_num as usize) {
        let mut device_num: c_int = 0;
        if unsafe { (api.get_device_num_in_card)(card_id, &mut device_num) } != 0 || device_num <= 0
        {
            continue;
        }
        let chip_count = device_num.max(1);
        for device_id in 0..device_num {
            let mut aicore = util_or_neg1(api, card_id, device_id, DCMI_UTILIZATION_RATE_AICORE);
            let npu_util = util_or_neg1(api, card_id, device_id, DCMI_UTILIZATION_RATE_NPU);
            let hbm_util = util_or_neg1(api, card_id, device_id, DCMI_UTILIZATION_RATE_HBM);
            if npu_util >= 0.0 {
                aicore = npu_util;
            } else if aicore < 0.0 {
                aicore = 0.0;
            }

            let mut temp: c_int = 0;
            let temp_c =
                if unsafe { (api.get_device_temperature)(card_id, device_id, &mut temp) } == 0 {
                    Some(temp as f32)
                } else {
                    None
                };

            let mut power_raw: c_int = 0;
            let power_w = if unsafe {
                (api.get_device_power_info)(card_id, device_id, &mut power_raw)
            } == 0
            {
                Some((power_raw as f32) * POWER_SCALE)
            } else {
                None
            };

            let mut hbm = DcmiHbmInfo::default();
            let (hbm_total_bytes, hbm_used_bytes, hbm_util_final) =
                if unsafe { (api.get_device_hbm_info)(card_id, device_id, &mut hbm) } == 0 {
                    // Empirically MB on A3 (65536 → 64 GiB), matching npu-smi.
                    let total = hbm.memory_size.saturating_mul(1024 * 1024);
                    let used = hbm.memory_usage.saturating_mul(1024 * 1024);
                    let util = if hbm_util >= 0.0 {
                        hbm_util
                    } else if total > 0 {
                        (used as f64 / total as f64 * 100.0) as f32
                    } else {
                        0.0
                    };
                    (total, used, util)
                } else {
                    (0, 0, hbm_util.max(0.0))
                };

            // DCMI field is historically misspelled `bandwith_util_rate` in the API.
            let hbm_bw = if hbm.bandwith_util_rate > 0 || hbm.memory_size > 0 {
                Some(hbm.bandwith_util_rate as f32)
            } else {
                None
            };

            let phy_id = card_id * chip_count + device_id;
            map.insert(
                phy_id,
                NpuDeviceStats {
                    ai_core_util_pct: aicore,
                    hbm_util_pct: hbm_util_final,
                    hbm_total_bytes,
                    hbm_used_bytes,
                    aivector_util_pct: None, // DCMI has no Aivector util kind today
                    hbm_bw_util_pct: hbm_bw,
                    temp_c,
                    power_w,
                    source: "dcmi",
                },
            );
        }
    }

    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// Keep the Library last-to-drop by declaring it first (Rust drops fields in reverse order).
#[allow(dead_code)]
fn _ensure_drop_order_docs() {}
