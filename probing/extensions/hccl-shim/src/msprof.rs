//! Best-effort layouts for CANN MSProf structs (toolchain/prof_api.h).
//!
//! Field order follows open HCCL usage in task_profiling.cc / profiling_manager.cc.
//! Validate with `sizeof` checks at runtime; CANN version drift may require updates.

use std::mem::size_of;

/// HCCL passes `sizeof(MsprofHcclInfo)` bytes in AdditionalInfo.data for task reports.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MsprofHcclInfo {
    pub item_id: u64,
    pub ccl_tag: u64,
    pub group_name: u64,
    pub local_rank: u32,
    pub remote_rank: u32,
    pub rank_size: u32,
    pub workflow_mode: u32,
    pub plane_id: u32,
    pub ctx_id: u32,
    pub notify_id: u64,
    pub stage: u32,
    pub role: u32,
    pub _pad: u32,
    pub duration_estimated: f64,
    pub src_addr: u64,
    pub dst_addr: u64,
    pub data_size: u64,
    pub op_type: u32,
    pub data_type: u32,
    pub link_type: u32,
    pub transport_type: u32,
    pub rdma_type: u32,
}

pub const MSPROF_HCCL_INFO_MIN: usize = size_of::<MsprofHcclInfo>();

/// Observed HCCL MsprofApi initialization pattern.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MsprofApi {
    pub level: u16,
    pub reserve: u16,
    pub type_id: u32,
    pub thread_id: u32,
    pub reserve2: u32,
    pub begin_time: u64,
    pub end_time: u64,
    pub item_id: u64,
}

pub const MSPROF_API_SIZE: usize = size_of::<MsprofApi>();

/// Header shared by MsprofAdditionalInfo and MsprofCompactInfo.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MsprofBlobHeader {
    pub level: u32,
    pub type_id: u32,
    pub thread_id: u32,
    pub data_len: u32,
    pub time_stamp: u64,
}

pub type MsprofAdditionalInfoHeader = MsprofBlobHeader;
pub type MsprofCompactInfoHeader = MsprofBlobHeader;

pub const MSPROF_BLOB_HEADER: usize = size_of::<MsprofBlobHeader>();
pub const MSPROF_ADDITIONAL_HEADER: usize = MSPROF_BLOB_HEADER;

/// `CallMsprofReportHostHcclOpInfo` payload inside MsprofCompactInfo.data.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MsprofHCCLOPInfo {
    pub relay: u32,
    pub retry: u32,
    pub data_type: u32,
    pub _pad: u32,
    pub alg_type: u64,
    pub count: u64,
    pub group_name: u64,
}

pub const MSPROF_HCCL_OP_INFO_MIN: usize = size_of::<MsprofHCCLOPInfo>();

/// `CallMsprofReportContextIdInfo` payload.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MsprofContextIdInfo {
    pub ctx_id_num: u32,
    pub ctx_ids: [u32; 2],
}

pub const MSPROF_CONTEXT_ID_INFO: usize = size_of::<MsprofContextIdInfo>();

/// Prefix of `ProfilingDeviceCommResInfo` from hccl_communicator_host.cc.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ProfilingDeviceCommResInfoHeader {
    pub group_name: u64,
    pub rank_size: u32,
    pub rank_id: u32,
    pub usr_rank_id: u32,
    pub aicpu_kfc_stream_id: u32,
    pub reserve: u32,
}

pub const MSPROF_MC2_HEADER: usize = size_of::<ProfilingDeviceCommResInfoHeader>();

/// ProfTaskType::TASK_HCCL_INFO
pub const PROF_TASK_HCCL_INFO: u32 = 0;

pub fn decode_plane(plane_id: u32) -> (i32, i32, i32) {
    let id = plane_id as u64;
    let plane_index = ((id >> 28) & 0xF) as i32;
    let rank_size_plane = ((id >> 16) & 0xFFF) as i32;
    let rank_in_plane = (id & 0xFFFF) as i32;
    (plane_index, rank_in_plane, rank_size_plane)
}

pub fn read_api(ptr: *const u8, len: u32) -> Option<MsprofApi> {
    if ptr.is_null() || (len as usize) < MSPROF_API_SIZE {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(ptr as *const MsprofApi) })
}

pub fn read_blob_header(ptr: *const u8, len: u32) -> Option<MsprofBlobHeader> {
    if ptr.is_null() || (len as usize) < MSPROF_BLOB_HEADER {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(ptr as *const MsprofBlobHeader) })
}

pub fn read_additional_header(ptr: *const u8, len: u32) -> Option<MsprofAdditionalInfoHeader> {
    read_blob_header(ptr, len)
}

pub fn read_compact_header(ptr: *const u8, len: u32) -> Option<MsprofCompactInfoHeader> {
    read_blob_header(ptr, len)
}

pub fn read_hccl_info(data: *const u8, data_len: u32) -> Option<MsprofHcclInfo> {
    if data.is_null() || (data_len as usize) < MSPROF_HCCL_INFO_MIN {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(data as *const MsprofHcclInfo) })
}

pub fn read_hccl_op_info(data: *const u8, data_len: u32) -> Option<MsprofHCCLOPInfo> {
    if data.is_null() || (data_len as usize) < MSPROF_HCCL_OP_INFO_MIN {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(data as *const MsprofHCCLOPInfo) })
}

pub fn read_context_id_info(data: *const u8, data_len: u32) -> Option<MsprofContextIdInfo> {
    if data.is_null() || (data_len as usize) < MSPROF_CONTEXT_ID_INFO {
        return None;
    }
    Some(unsafe { std::ptr::read_unaligned(data as *const MsprofContextIdInfo) })
}

pub struct Mc2CommInfo {
    pub header: ProfilingDeviceCommResInfoHeader,
    pub comm_stream_size: u32,
    pub comm_stream_ids: Vec<u32>,
}

pub fn read_mc2_comm_info(data: *const u8, data_len: u32) -> Option<Mc2CommInfo> {
    if data.is_null() || (data_len as usize) < MSPROF_MC2_HEADER + 4 {
        return None;
    }
    let header =
        unsafe { std::ptr::read_unaligned(data as *const ProfilingDeviceCommResInfoHeader) };
    let tail = data_len as usize - MSPROF_MC2_HEADER;
    if tail < 4 {
        return None;
    }
    let tail_ptr = unsafe { data.add(MSPROF_MC2_HEADER) };
    // HCCL sets commStreamSize after filling ids; layout is header + ids[] + commStreamSize
    // or header + commStreamSize + ids[]. Accept either by checking plausibility.
    let comm_stream_size_tail =
        unsafe { std::ptr::read_unaligned(tail_ptr.add(tail - 4) as *const u32) };
    let comm_stream_size_head = unsafe { std::ptr::read_unaligned(tail_ptr as *const u32) };

    let (comm_stream_size, id_offset) = if comm_stream_size_head > 0 && comm_stream_size_head <= 512
    {
        (comm_stream_size_head, 4usize)
    } else if comm_stream_size_tail > 0 && comm_stream_size_tail <= 512 {
        (comm_stream_size_tail, 0usize)
    } else {
        (0u32, 4usize)
    };

    let ids_bytes = tail.saturating_sub(4);
    let ids_ptr = unsafe { tail_ptr.add(id_offset) };
    let max_ids = ids_bytes / 4;
    let n = if comm_stream_size > 0 {
        (comm_stream_size as usize).min(max_ids)
    } else {
        max_ids
    };
    let mut comm_stream_ids = Vec::with_capacity(n);
    for i in 0..n {
        let id = unsafe { std::ptr::read_unaligned(ids_ptr.add(i * 4) as *const u32) };
        comm_stream_ids.push(id);
    }
    let comm_stream_size = if comm_stream_size > 0 {
        comm_stream_size
    } else {
        n as u32
    };

    Some(Mc2CommInfo {
        header,
        comm_stream_size,
        comm_stream_ids,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdditionalKind {
    HcclTask,
    Mc2Comm,
    ContextId,
    Unknown,
}

pub fn classify_additional(type_id: u32, type_name: &str, data_len: u32) -> AdditionalKind {
    if type_name == "mc2_comm_info" {
        return AdditionalKind::Mc2Comm;
    }
    if type_name == "context_id_info" {
        return AdditionalKind::ContextId;
    }
    if type_id == PROF_TASK_HCCL_INFO || data_len as usize >= MSPROF_HCCL_INFO_MIN {
        return AdditionalKind::HcclTask;
    }
    if data_len as usize >= MSPROF_MC2_HEADER + 8 && data_len <= 4096 {
        return AdditionalKind::Mc2Comm;
    }
    if data_len as usize >= 8 && data_len <= 64 {
        return AdditionalKind::ContextId;
    }
    AdditionalKind::Unknown
}

pub fn is_hccl_op_compact(type_name: &str, data_len: u32) -> bool {
    type_name.contains("hccl_op") || data_len as usize == MSPROF_HCCL_OP_INFO_MIN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hccl_info_size_sane() {
        const {
            assert!(MSPROF_HCCL_INFO_MIN >= 96 && MSPROF_HCCL_INFO_MIN <= 256);
        }
    }

    #[test]
    fn decode_plane_bits() {
        // plane=3, rank_size=8, rank=5 -> (3<<28)|(8<<16)|5
        let plane_id = (3u64 << 28) | (8u64 << 16) | 5;
        assert_eq!(decode_plane(plane_id as u32), (3, 5, 8));
    }

    #[test]
    fn classify_task_by_len() {
        assert_eq!(
            classify_additional(99, "", MSPROF_HCCL_INFO_MIN as u32),
            AdditionalKind::HcclTask
        );
    }
}
