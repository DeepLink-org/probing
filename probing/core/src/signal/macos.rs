//! Deliver signals to live pthread/Mach threads on macOS.

use std::io;

#[allow(non_camel_case_types)]
mod mach {
    use std::io;
    use std::ptr;

    pub type kern_return_t = i32;
    pub type mach_port_t = u32;
    pub type integer_t = i32;
    pub type thread_act_t = mach_port_t;
    pub type task_t = mach_port_t;

    pub const KERN_SUCCESS: kern_return_t = 0;
    pub const THREAD_IDENTIFIER_INFO: i32 = 4;

    #[repr(C)]
    pub struct thread_identifier_info {
        pub thread_id: u64,
        pub thread_handle: u64,
        pub dispatch_qaddr: u64,
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
    }

    pub fn thread_id_for_port(port: thread_act_t) -> Option<u64> {
        let mut ident: thread_identifier_info = unsafe { std::mem::zeroed() };
        let mut ident_count =
            (std::mem::size_of::<thread_identifier_info>() / std::mem::size_of::<i32>()) as u32;
        let kr = unsafe {
            thread_info(
                port,
                THREAD_IDENTIFIER_INFO,
                &mut ident as *mut _ as *mut i32,
                &mut ident_count,
            )
        };
        if kr == KERN_SUCCESS {
            Some(ident.thread_id)
        } else {
            None
        }
    }

    pub fn signal_sigusr2_on_port(port: thread_act_t) -> io::Result<()> {
        extern "C" {
            fn pthread_kill(thread: libc::pthread_t, sig: i32) -> i32;
        }
        unsafe {
            let pthread = pthread_from_mach_thread_np(port);
            if pthread == 0 {
                return Err(io::Error::other(
                    "pthread_from_mach_thread_np returned null",
                ));
            }
            if pthread_kill(pthread, libc::SIGUSR2) != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }

    pub fn list_thread_ports() -> io::Result<Vec<thread_act_t>> {
        let mut list: *mut thread_act_t = ptr::null_mut();
        let mut count: u32 = 0;
        let kr = unsafe { task_threads(mach_task_self(), &mut list, &mut count) };
        if kr != KERN_SUCCESS {
            return Err(io::Error::other(format!("task_threads failed: {kr}")));
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

use mach::{mach_port_deallocate, mach_task_self};

/// Deliver `SIGUSR2` to a live thread by Mach/pthread thread id.
pub fn send_sigusr2_to_thread_id(thread_id: i32) -> io::Result<()> {
    if thread_id <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid thread id {thread_id}"),
        ));
    }
    let task = unsafe { mach_task_self() };
    let ports = mach::list_thread_ports()?;
    let target = thread_id as u64;

    for port in ports {
        let matched = mach::thread_id_for_port(port)
            .map(|id| id == target)
            .unwrap_or(false);
        let result = if matched {
            mach::signal_sigusr2_on_port(port)
        } else {
            Ok(())
        };
        unsafe {
            let _ = mach_port_deallocate(task, port);
        }
        if matched {
            return result;
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no live thread with id {thread_id}"),
    ))
}
