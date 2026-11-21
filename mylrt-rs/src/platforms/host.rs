// 宿主平台：std 默认实现，以及 no_std 下的 POSIX / Win32 兜底。

use crate::plat::Platform;

pub struct HostPlatform;
static HOST: HostPlatform = HostPlatform;

impl Platform for HostPlatform {
    fn write_stdout(&self, bytes: &[u8]) { write_stdout(bytes); }
    fn write_stderr(&self, bytes: &[u8]) { write_stderr(bytes); }
    fn abort(&self) -> ! { abort() }
    fn clock(&self) -> f64 { clock() }
    fn gc_threshold(&self) -> Option<usize> { gc_threshold() }
    fn format_f64(&self, v: f64) -> ([u8; 64], usize) { format_f64(v) }
    fn format_f64_prec(&self, v: f64, precision: i32) -> ([u8; 64], usize) { format_f64_prec(v, precision) }
}

pub fn platform() -> &'static dyn Platform { &HOST }

#[cfg(feature = "std")]
pub fn write_stdout(bytes: &[u8]) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(bytes);
}

#[cfg(feature = "std")]
pub fn write_stderr(bytes: &[u8]) {
    use std::io::Write;
    let _ = std::io::stderr().write_all(bytes);
}

#[cfg(feature = "std")]
pub fn abort() -> ! { std::process::abort() }

#[cfg(feature = "std")]
pub fn clock() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[cfg(feature = "std")]
pub fn gc_threshold() -> Option<usize> {
    std::env::var("MYLIS_GC_THRESHOLD").ok()?.parse().ok()
}

#[cfg(feature = "std")]
pub fn format_f64(v: f64) -> ([u8; 64], usize) {
    let s = format!("{}", v);
    let mut out = [0u8; 64];
    let n = s.len().min(out.len());
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    (out, n)
}

#[cfg(feature = "std")]
pub fn format_f64_prec(v: f64, precision: i32) -> ([u8; 64], usize) {
    let p = precision.max(0) as usize;
    let s = format!("{:.*}", p, v);
    let mut out = [0u8; 64];
    let n = s.len().min(out.len());
    out[..n].copy_from_slice(&s.as_bytes()[..n]);
    (out, n)
}

#[cfg(not(feature = "std"))]
pub use imp::{abort, clock, format_f64, format_f64_prec, gc_threshold, write_stderr, write_stdout};

// ---------------------------------------------------------------------------
// no_std：POSIX
// ---------------------------------------------------------------------------
#[cfg(all(not(feature = "std"), unix))]
mod imp {
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicU64, Ordering};

    #[repr(C)]
    struct TimeVal { sec: i64, usec: i64 }

    extern "C" {
        fn write(fd: i32, buf: *const u8, count: usize) -> isize;
        #[link_name = "abort"]
        fn c_abort() -> !;
        fn getenv(name: *const u8) -> *const u8;
        fn gettimeofday(tv: *mut TimeVal, tz: *mut c_void) -> i32;
        fn snprintf(buf: *mut u8, size: usize, fmt: *const u8, ...) -> i32;
    }

    pub fn write_stdout(bytes: &[u8]) {
        unsafe { let _ = write(1, bytes.as_ptr(), bytes.len()); }
    }

    pub fn write_stderr(bytes: &[u8]) {
        unsafe { let _ = write(2, bytes.as_ptr(), bytes.len()); }
    }

    pub fn abort() -> ! { unsafe { c_abort() } }

    fn micros() -> u64 {
        let mut tv = TimeVal { sec: 0, usec: 0 };
        unsafe { let _ = gettimeofday(&mut tv, core::ptr::null_mut()); }
        (tv.sec as u64).saturating_mul(1_000_000).saturating_add(tv.usec as u64)
    }

    pub fn clock() -> f64 {
        static START: AtomicU64 = AtomicU64::new(0);
        let now = micros();
        let mut start = START.load(Ordering::Relaxed);
        if start == 0 {
            let _ = START.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
            start = START.load(Ordering::Relaxed);
        }
        now.saturating_sub(start) as f64 / 1_000_000.0
    }

    pub fn gc_threshold() -> Option<usize> {
        let p = unsafe { getenv(b"MYLIS_GC_THRESHOLD\0".as_ptr()) };
        if p.is_null() { return None; }
        let mut n = 0usize;
        unsafe {
            let mut q = p;
            while *q != 0 {
                let c = *q;
                if !c.is_ascii_digit() { return None; }
                n = n.checked_mul(10)?.checked_add((c - b'0') as usize)?;
                q = q.add(1);
            }
        }
        Some(n)
    }

    pub fn format_f64(v: f64) -> ([u8; 64], usize) {
        let mut out = [0u8; 64];
        let n = unsafe { snprintf(out.as_mut_ptr(), out.len(), b"%.15g\0".as_ptr(), v) };
        if n <= 0 { return (out, 0); }
        (out, (n as usize).min(out.len()))
    }

    pub fn format_f64_prec(v: f64, precision: i32) -> ([u8; 64], usize) {
        let mut out = [0u8; 64];
        let p = precision.clamp(0, 17);
        let n = unsafe { snprintf(out.as_mut_ptr(), out.len(), b"%.*f\0".as_ptr(), p, v) };
        if n <= 0 { return (out, 0); }
        (out, (n as usize).min(out.len()))
    }
}

// ---------------------------------------------------------------------------
// no_std：Win32
// ---------------------------------------------------------------------------
#[cfg(all(not(feature = "std"), windows))]
mod imp {
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicU64, Ordering};

    type Handle = *mut c_void;

    extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn WriteFile(handle: Handle, buf: *const u8, len: u32, written: *mut u32, ov: *mut c_void) -> i32;
        fn ExitProcess(code: u32) -> !;
        fn GetEnvironmentVariableA(name: *const u8, buf: *mut u8, size: u32) -> u32;
        fn QueryPerformanceCounter(out: *mut i64) -> i32;
        fn QueryPerformanceFrequency(out: *mut i64) -> i32;
        fn _snprintf(buf: *mut u8, size: usize, fmt: *const u8, ...) -> i32;
    }

    fn write_handle(which: u32, bytes: &[u8]) {
        let h = unsafe { GetStdHandle(which) };
        let mut written = 0u32;
        unsafe { let _ = WriteFile(h, bytes.as_ptr(), bytes.len() as u32, &mut written, core::ptr::null_mut()); }
    }

    pub fn write_stdout(bytes: &[u8]) { write_handle((-11i32) as u32, bytes); }
    pub fn write_stderr(bytes: &[u8]) { write_handle((-12i32) as u32, bytes); }
    pub fn abort() -> ! { unsafe { ExitProcess(1) } }

    fn now_units() -> u64 {
        let mut c = 0i64;
        unsafe { let _ = QueryPerformanceCounter(&mut c); }
        c as u64
    }

    fn freq() -> u64 {
        static FREQ: AtomicU64 = AtomicU64::new(0);
        let mut f = FREQ.load(Ordering::Relaxed);
        if f == 0 {
            let mut v = 0i64;
            unsafe { let _ = QueryPerformanceFrequency(&mut v); }
            f = v.max(1) as u64;
            FREQ.store(f, Ordering::Relaxed);
        }
        f
    }

    pub fn clock() -> f64 {
        static START: AtomicU64 = AtomicU64::new(0);
        let now = now_units();
        let mut start = START.load(Ordering::Relaxed);
        if start == 0 {
            let _ = START.compare_exchange(0, now, Ordering::Relaxed, Ordering::Relaxed);
            start = START.load(Ordering::Relaxed);
        }
        now.saturating_sub(start) as f64 / freq() as f64
    }

    pub fn gc_threshold() -> Option<usize> {
        let mut buf = [0u8; 64];
        let n = unsafe { GetEnvironmentVariableA(b"MYLIS_GC_THRESHOLD\0".as_ptr(), buf.as_mut_ptr(), buf.len() as u32) };
        if n == 0 || n as usize >= buf.len() { return None; }
        let mut v = 0usize;
        for &c in &buf[..n as usize] {
            if !c.is_ascii_digit() { return None; }
            v = v.checked_mul(10)?.checked_add((c - b'0') as usize)?;
        }
        Some(v)
    }

    pub fn format_f64(v: f64) -> ([u8; 64], usize) {
        let mut out = [0u8; 64];
        let n = unsafe { _snprintf(out.as_mut_ptr(), out.len(), b"%.15g\0".as_ptr(), v) };
        if n <= 0 { return (out, 0); }
        (out, (n as usize).min(out.len()))
    }

    pub fn format_f64_prec(v: f64, precision: i32) -> ([u8; 64], usize) {
        let mut out = [0u8; 64];
        let p = precision.clamp(0, 17);
        let n = unsafe { _snprintf(out.as_mut_ptr(), out.len(), b"%.*f\0".as_ptr(), p, v) };
        if n <= 0 { return (out, 0); }
        (out, (n as usize).min(out.len()))
    }
}

#[cfg(not(any(feature = "std", unix, windows)))]
compile_error!("no_std runtime platform is not implemented for this target");
