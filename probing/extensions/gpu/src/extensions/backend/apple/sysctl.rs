use std::ffi::CStr;
use std::io;

fn sysctl_raw(name: &str, buf: &mut [u8]) -> io::Result<usize> {
    let c_name = CStringLike::new(name)?;
    let mut size = buf.len();
    let rc = unsafe {
        libc::sysctlbyname(
            c_name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(size)
}

struct CStringLike {
    inner: Vec<u8>,
}

impl CStringLike {
    fn new(s: &str) -> io::Result<Self> {
        let mut inner = s.as_bytes().to_vec();
        inner.push(0);
        Ok(Self { inner })
    }

    fn as_ptr(&self) -> *const libc::c_char {
        self.inner.as_ptr().cast()
    }
}

pub fn cpu_brand_string() -> Option<String> {
    let mut buf = [0u8; 256];
    let len = sysctl_raw("machdep.cpu.brand_string", &mut buf).ok()?;
    parse_cstr(&buf[..len])
}

pub fn hw_memsize() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = CStringLike::new("hw.memsize").ok()?;
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    Some(value)
}

fn parse_cstr(bytes: &[u8]) -> Option<String> {
    let cstr = CStr::from_bytes_until_nul(bytes).ok()?;
    let s = cstr.to_str().ok()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}
