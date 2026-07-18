pub(crate) mod python_bindings;

pub(crate) mod python_interpreters;

pub(crate) mod call;
pub(crate) mod ffi;

pub use python_bindings::version::Version;

use crate::features::stacktrace::spy::call::RawCallLocation;

pub(crate) struct ThreadSpyState {
    pub stacks: Vec<RawCallLocation>,
    pub writing: bool,
    pub frame_eval: ffi::_PyFrameEvalFunction,
}

thread_local! {
    static SPY_STATE: std::cell::UnsafeCell<ThreadSpyState> =
        std::cell::UnsafeCell::new(ThreadSpyState {
            stacks: Vec::new(),
            writing: false,
            frame_eval: ffi::_PyEval_EvalFrameDefault,
        });
}

/// Access thread-local spy state. Hot path: one TLS lookup per eval-frame call.
#[inline(always)]
pub(crate) fn with_spy_state<R>(f: impl FnOnce(*mut ThreadSpyState) -> R) -> R {
    SPY_STATE.with(|cell| f(cell.get()))
}

/// Raw addresses for SIGPROF registry (normal context only).
pub(crate) fn spy_tls_addrs() -> (*mut Vec<RawCallLocation>, *mut bool) {
    with_spy_state(|state| unsafe {
        (
            core::ptr::addr_of_mut!((*state).stacks),
            core::ptr::addr_of_mut!((*state).writing),
        )
    })
}

pub(crate) static mut PYVERSION: Version = Version {
    major: 0,
    minor: 0,
    patch: 0,
    release_flags: String::new(),
    build_metadata: None,
};

/// 获取当前线程执行的Python frame指针
/// 这个函数适用于在信号处理函数中调用
#[inline(always)]
pub fn get_current_frame(ver: &Version) -> Option<usize> {
    unsafe {
        // 获取当前线程状态
        let threadstate: usize = get_current_threadstate()?;

        match (ver.major, ver.minor) {
            (3, 4) | (3, 5) | (3, 6) | (3, 7) | (3, 8) | (3, 9) | (3, 10) => {
                // Python 3.4 to 3.10
                let ts = threadstate as *const python_bindings::v3_10_0::PyThreadState;
                let frame = (*ts).frame;
                if !frame.is_null() {
                    Some(frame as usize)
                } else {
                    None
                }
            }
            (3, 11) => {
                // Python 3.11
                let ts = threadstate as *const python_bindings::v3_11_0::PyThreadState;
                let cframe = (*ts).cframe;
                if !cframe.is_null() {
                    let current_frame = (*cframe).current_frame;
                    if !current_frame.is_null() {
                        Some(current_frame as usize)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            (3, 12) => {
                // Python 3.12
                let ts = threadstate as *const python_bindings::v3_12_0::PyThreadState;
                let cframe = (*ts).cframe;
                if !cframe.is_null() {
                    let current_frame = (*cframe).current_frame;
                    if !current_frame.is_null() {
                        Some(current_frame as usize)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            (3, 13) => {
                // Python 3.13
                let ts = threadstate as *const python_bindings::v3_13_0::PyThreadState;
                let current_frame = (*ts).current_frame;
                if !current_frame.is_null() {
                    Some(current_frame as usize)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[inline(always)]
pub fn get_prev_frame(ver: &Version, frame_addr: usize) -> Option<usize> {
    match (ver.major, ver.minor) {
        (3, 4) | (3, 5) | (3, 6) | (3, 7) | (3, 8) | (3, 9) | (3, 10) => {
            let frame = frame_addr as *const python_bindings::v3_10_0::_frame;
            let prev_frame = unsafe { (*frame).f_back };
            if !prev_frame.is_null() && prev_frame.is_aligned() && prev_frame as usize > 0xffffff {
                Some(prev_frame as usize)
            } else {
                None
            }
        }
        (3, 11) => {
            let iframe = frame_addr as *const python_bindings::v3_11_0::_PyInterpreterFrame;
            let prev_frame = unsafe { (*iframe).previous };
            if !prev_frame.is_null() && prev_frame.is_aligned() && prev_frame as usize > 0xffffff {
                Some(prev_frame as usize)
            } else {
                None
            }
        }
        (3, 12) => {
            let iframe = frame_addr as *const python_bindings::v3_12_0::_PyInterpreterFrame;
            let prev_frame = unsafe { (*iframe).previous };
            if !prev_frame.is_null() && prev_frame.is_aligned() && prev_frame as usize > 0xffffff {
                Some(prev_frame as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// 获取当前线程的PyThreadState指针
/// 这个函数使用Python C API来获取当前线程状态
#[inline(always)]
pub fn get_current_threadstate() -> Option<usize> {
    extern "C" {
        fn PyThreadState_Get() -> *mut std::ffi::c_void;
    }

    let threadstate = unsafe { PyThreadState_Get() };
    if !threadstate.is_null() {
        Some(threadstate as usize)
    } else {
        None
    }
}
