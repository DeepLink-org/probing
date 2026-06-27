use std::io;

use super::sample::{ProcessSample, ThreadSample};
use super::sampler::CpuHostSampler;

#[allow(non_camel_case_types)]
mod mach {
    use std::ptr;

    pub type kern_return_t = i32;
    pub type mach_port_t = u32;
    pub type integer_t = i32;
    pub type thread_act_t = mach_port_t;
    pub type task_t = mach_port_t;

    pub const KERN_SUCCESS: kern_return_t = 0;
    pub const THREAD_BASIC_INFO: i32 = 3;
    pub const THREAD_IDENTIFIER_INFO: i32 = 4;
    pub const THREAD_EXTENDED_INFO: i32 = 5;

    #[repr(C)]
    pub struct time_value_t {
        pub seconds: integer_t,
        pub microseconds: integer_t,
    }

    #[repr(C)]
    pub struct thread_basic_info {
        pub user_time: time_value_t,
        pub system_time: time_value_t,
        pub cpu_usage: integer_t,
        pub policy: integer_t,
        pub run_state: integer_t,
        pub suspend_count: integer_t,
        pub sleep_time: integer_t,
        pub flags: integer_t,
    }

    #[repr(C)]
    pub struct thread_identifier_info {
        pub thread_id: u64,
        pub thread_handle: u64,
        pub dispatch_qaddr: u64,
    }

    #[repr(C)]
    pub struct thread_extended_info {
        pub pth_flags: integer_t,
        pub pth_sw_flags: integer_t,
        pub pth_faults: integer_t,
        pub pth_pageins: integer_t,
        pub pth_suspended: integer_t,
        pub pth_runstate: integer_t,
        pub pth_cpu_percent: integer_t,
        pub pth_priority: integer_t,
        pub pth_sleep_time: integer_t,
        pub pth_name: [i8; 64],
    }

    extern "C" {
        pub fn mach_task_self() -> task_t;
        pub fn task_threads(
            target_task: task_t,
            act_list: *mut *mut thread_act_t,
            act_count: *mut u32,
        ) -> kern_return_t;
        pub fn thread_info(
            target_act: thread_act_t,
            flavor: i32,
            thread_info_out: *mut integer_t,
            thread_info_out_cnt: *mut u32,
        ) -> kern_return_t;
        pub fn mach_port_deallocate(task: task_t, name: mach_port_t) -> kern_return_t;
        pub fn vm_deallocate(target_task: task_t, address: u64, size: u64) -> kern_return_t;
        pub fn pthread_from_mach_thread_np(thread_port: thread_act_t) -> libc::pthread_t;
        pub fn pthread_getname_np(thread: libc::pthread_t, name: *mut i8, len: libc::size_t)
            -> i32;
    }

    pub fn pthread_name(port: thread_act_t) -> Option<String> {
        unsafe {
            let pthread = pthread_from_mach_thread_np(port);
            if pthread == 0 {
                return None;
            }
            let mut buf = [0i8; 256];
            if pthread_getname_np(pthread, buf.as_mut_ptr(), buf.len()) != 0 {
                return None;
            }
            let name = std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .trim()
                .to_string();
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        }
    }

    pub fn list_thread_ports() -> std::io::Result<Vec<thread_act_t>> {
        let mut list: *mut thread_act_t = ptr::null_mut();
        let mut count: u32 = 0;
        let kr = unsafe { task_threads(mach_task_self(), &mut list, &mut count) };
        if kr != KERN_SUCCESS {
            return Err(std::io::Error::other(format!("task_threads failed: {kr}")));
        }
        if list.is_null() || count == 0 {
            return Ok(Vec::new());
        }

        let ports = unsafe { std::slice::from_raw_parts(list, count as usize).to_vec() };
        let size = (count as u64).saturating_mul(std::mem::size_of::<thread_act_t>() as u64);
        unsafe {
            let _ = vm_deallocate(mach_task_self(), list as u64, size);
        }
        Ok(ports)
    }
}

use mach::{
    mach_port_deallocate, mach_task_self, thread_act_t, thread_basic_info, thread_extended_info,
    thread_identifier_info, thread_info, time_value_t, KERN_SUCCESS, THREAD_BASIC_INFO,
    THREAD_EXTENDED_INFO, THREAD_IDENTIFIER_INFO,
};

pub struct MacSampler;

fn timeval_to_ns(tv: libc::timeval) -> u64 {
    (tv.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add((tv.tv_usec as u64).saturating_mul(1_000))
}

fn time_value_to_ns(tv: time_value_t) -> u64 {
    (tv.seconds as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add((tv.microseconds as u64).saturating_mul(1_000))
}

fn run_state_label(state: mach::integer_t) -> &'static str {
    match state {
        1 => "R", // TH_STATE_RUNNING
        2 => "S", // TH_STATE_STOPPED
        3 => "D", // TH_STATE_WAITING
        4 => "U", // TH_STATE_UNINTERRUPTIBLE
        5 => "H", // TH_STATE_HALTED
        _ => "?",
    }
}

fn sample_one_thread(port: thread_act_t) -> Option<ThreadSample> {
    let mut basic: thread_basic_info = unsafe { std::mem::zeroed() };
    let mut basic_count =
        (std::mem::size_of::<thread_basic_info>() / std::mem::size_of::<i32>()) as u32;
    let kr = unsafe {
        thread_info(
            port,
            THREAD_BASIC_INFO,
            &mut basic as *mut _ as *mut i32,
            &mut basic_count,
        )
    };
    if kr != KERN_SUCCESS {
        return None;
    }

    let mut ident: thread_identifier_info = unsafe { std::mem::zeroed() };
    let mut ident_count =
        (std::mem::size_of::<thread_identifier_info>() / std::mem::size_of::<i32>()) as u32;
    let tid = if unsafe {
        thread_info(
            port,
            THREAD_IDENTIFIER_INFO,
            &mut ident as *mut _ as *mut i32,
            &mut ident_count,
        )
    } == KERN_SUCCESS
    {
        ident.thread_id as i32
    } else {
        port as i32
    };

    let mut ext: thread_extended_info = unsafe { std::mem::zeroed() };
    let mut ext_count =
        (std::mem::size_of::<thread_extended_info>() / std::mem::size_of::<i32>()) as u32;
    let mut comm = if unsafe {
        thread_info(
            port,
            THREAD_EXTENDED_INFO,
            &mut ext as *mut _ as *mut i32,
            &mut ext_count,
        )
    } == KERN_SUCCESS
    {
        let raw = unsafe { std::ffi::CStr::from_ptr(ext.pth_name.as_ptr()) };
        raw.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    if comm.trim().is_empty() {
        comm = mach::pthread_name(port).unwrap_or_default();
    }

    Some(ThreadSample {
        tid,
        comm,
        state: Some(run_state_label(basic.run_state).to_string()),
        wchan: None,
        cputime_user_ns: time_value_to_ns(basic.user_time),
        cputime_sys_ns: time_value_to_ns(basic.system_time),
    })
}

impl CpuHostSampler for MacSampler {
    fn platform(&self) -> &'static str {
        "macos"
    }

    fn sample_process(&self) -> io::Result<ProcessSample> {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let thread_count = mach::list_thread_ports()?.len() as u32;

        Ok(ProcessSample {
            cputime_user_ns: timeval_to_ns(ru.ru_utime),
            cputime_sys_ns: timeval_to_ns(ru.ru_stime),
            // macOS reports ru_maxrss in bytes.
            rss_bytes: ru.ru_maxrss as u64,
            thread_count,
            vol_ctxt: ru.ru_nvcsw as u64,
            invol_ctxt: ru.ru_nivcsw as u64,
        })
    }

    fn sample_threads(&self, top_n: usize) -> io::Result<Vec<ThreadSample>> {
        if top_n == 0 {
            return Ok(Vec::new());
        }

        let ports = mach::list_thread_ports()?;
        let task = unsafe { mach_task_self() };
        let mut threads = Vec::with_capacity(ports.len());

        for port in ports {
            if let Some(sample) = sample_one_thread(port) {
                threads.push(sample);
            }
            unsafe {
                let _ = mach_port_deallocate(task, port);
            }
        }

        if top_n > 0 && threads.len() > top_n {
            threads.sort_by_key(|t| std::cmp::Reverse(t.total_cputime_ns()));
            threads.truncate(top_n);
        }

        Ok(threads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn lists_at_least_one_thread() {
        let _worker = thread::spawn(|| thread::sleep(Duration::from_millis(50)));
        let sampler = MacSampler;
        let threads = sampler.sample_threads(8).expect("sample_threads");
        assert!(!threads.is_empty(), "expected Mach thread samples on macOS");
        let process = sampler.sample_process().expect("sample_process");
        assert!(process.thread_count >= 1);
    }
}
