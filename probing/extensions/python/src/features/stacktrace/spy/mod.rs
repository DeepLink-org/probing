pub(crate) mod python_bindings;

pub(crate) mod python_interpreters;

pub(crate) mod call;
pub(crate) mod ffi;

pub use python_bindings::version::Version;

use crate::features::stacktrace::snapshot::MAX_PY;
use crate::features::stacktrace::spy::call::RawCallLocation;
use std::sync::atomic::{fence, AtomicUsize, Ordering};

pub(crate) struct PublishedPyStack {
    version: AtomicUsize,
    depth: AtomicUsize,
    frames: [AtomicUsize; MAX_PY],
}

impl PublishedPyStack {
    pub const fn new() -> Self {
        Self {
            version: AtomicUsize::new(0),
            depth: AtomicUsize::new(0),
            frames: [const { AtomicUsize::new(0) }; MAX_PY],
        }
    }

    // A registry slot has exactly one writer: its owning Python thread. Signal
    // handlers and sampler threads only call `copy_into`. The Acquire RMW keeps
    // following payload stores after the odd version on weakly ordered targets;
    // the final Release store publishes them.
    #[inline]
    pub fn enter(&self, depth: usize, callee: usize) {
        let next = self.version.fetch_add(1, Ordering::Acquire).wrapping_add(2);
        if let Some(index) = depth.checked_sub(1).filter(|index| *index < MAX_PY) {
            self.frames[index].store(callee, Ordering::Relaxed);
        }
        self.depth.store(depth, Ordering::Release);
        self.version.store(next, Ordering::Release);
    }

    #[inline]
    pub fn leave(&self, depth: usize) {
        let next = self.version.fetch_add(1, Ordering::Acquire).wrapping_add(2);
        self.depth.store(depth, Ordering::Release);
        self.version.store(next, Ordering::Release);
    }

    pub fn clear(&self) {
        let next = self.version.fetch_add(1, Ordering::Acquire).wrapping_add(2);
        self.depth.store(0, Ordering::Relaxed);
        for frame in &self.frames {
            frame.store(0, Ordering::Relaxed);
        }
        self.version.store(next, Ordering::Release);
    }

    /// Returns `(copied, truncated, stable)`.
    pub fn copy_into(&self, out: &mut [usize]) -> (usize, bool, bool) {
        let version_before = self.version.load(Ordering::Acquire);
        if version_before & 1 != 0 {
            return (0, false, false);
        }
        let depth_before = self.depth.load(Ordering::Acquire);
        let copied = depth_before.min(out.len()).min(MAX_PY);
        for (dst, src) in out.iter_mut().zip(self.frames.iter()).take(copied) {
            *dst = src.load(Ordering::Relaxed);
        }
        fence(Ordering::Acquire);
        let version_after = self.version.load(Ordering::Acquire);
        (
            copied,
            depth_before > out.len().min(MAX_PY),
            version_before == version_after,
        )
    }
}

pub(crate) struct ThreadSpyState {
    pub stacks: Vec<RawCallLocation>,
    pub published: *const PublishedPyStack,
    pub published_pid: u32,
    pub frame_eval: ffi::_PyFrameEvalFunction,
}

thread_local! {
    static SPY_STATE: std::cell::UnsafeCell<ThreadSpyState> =
        std::cell::UnsafeCell::new(ThreadSpyState {
            stacks: Vec::new(),
            published: std::ptr::null(),
            published_pid: 0,
            frame_eval: ffi::_PyEval_EvalFrameDefault,
        });
}

/// Access thread-local spy state. Hot path: one TLS lookup per eval-frame call.
#[inline(always)]
pub(crate) fn with_spy_state<R>(f: impl FnOnce(*mut ThreadSpyState) -> R) -> R {
    SPY_STATE.with(|cell| f(cell.get()))
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

#[cfg(test)]
mod published_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn published_stack_tracks_enter_leave_and_truncation() {
        let stack = PublishedPyStack::new();
        stack.enter(1, 11);
        stack.enter(2, 22);

        let mut out = [0usize; MAX_PY];
        let (copied, truncated, stable) = stack.copy_into(&mut out);
        assert_eq!(&out[..copied], &[11, 22]);
        assert!(!truncated);
        assert!(stable);

        stack.leave(1);
        let (copied, truncated, stable) = stack.copy_into(&mut out);
        assert_eq!(&out[..copied], &[11]);
        assert!(!truncated);
        assert!(stable);

        stack.enter(MAX_PY + 1, 33);
        let (copied, truncated, stable) = stack.copy_into(&mut out);
        assert_eq!(copied, MAX_PY);
        assert!(truncated);
        assert!(stable);
    }

    #[test]
    fn published_stack_never_exposes_mixed_frames() {
        let stack = Arc::new(PublishedPyStack::new());
        let done = Arc::new(AtomicBool::new(false));
        let writer_stack = Arc::clone(&stack);
        let writer_done = Arc::clone(&done);
        let writer = std::thread::spawn(move || {
            for value in 1..=10_000 {
                writer_stack.enter(1, value);
                writer_stack.enter(2, value);
            }
            writer_done.store(true, Ordering::Release);
        });

        while !done.load(Ordering::Acquire) {
            let mut out = [0usize; MAX_PY];
            let (copied, _, stable) = stack.copy_into(&mut out);
            if stable && copied == 2 {
                assert_eq!(out[0], out[1]);
            }
        }
        writer.join().expect("writer");
    }
}
