//  stdio / process environment native backend for Linux and other POSIX hosts.
//  Every returned char* is owned by the caller and must be released with
//  mylis_io_free_string.

use std::ffi::{CStr, CString};
use std::io::Read;
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicI64, Ordering};

static LAST_IO_STATUS: AtomicI64 = AtomicI64::new(0);

fn set_status(status: i64) {
    LAST_IO_STATUS.store(status, Ordering::SeqCst);
}

fn cstring(text: String) -> *mut c_char {
    // Never hand a NULL pointer to Mylis: embedded NUL bytes are stripped.
    let clean = text.replace('\0', "");
    CString::new(clean)
        .unwrap_or_else(|_| CString::new("").expect("empty CString"))
        .into_raw()
}

fn arg_string(index: i64) -> Option<String> {
    if index < 0 { return None; }
    std::env::args_os()
        .nth(index as usize)
        .map(|s| s.to_string_lossy().into_owned())
}

#[cfg(windows)]
fn read_line_windows() -> (i64, Vec<u8>) {
    use std::ffi::c_void;
    use std::sync::Mutex;
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetStdHandle(which: u32) -> Handle;
        fn GetFileType(handle: Handle) -> u32;
        fn ReadFile(
            handle: Handle,
            buffer: *mut c_void,
            bytes_to_read: u32,
            bytes_read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn GetLastError() -> u32;
    }
    const STD_INPUT_HANDLE: u32 = (-10i32) as u32;
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_TYPE_PIPE: u32 = 3;
    // Bytes already read from the pipe but not yet returned as a line. Rust's
    // stdin buffering can wait for more than one line when several Rust
    // staticlibs are linked into the same Mylis executable, so read the raw
    // pipe directly and carry the remainder across calls.
    static PENDING: Mutex<Vec<u8>> = Mutex::new(Vec::new());
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle.is_null() || handle as isize == -1 {
        return (-1, Vec::new());
    }
    let kind = unsafe { GetFileType(handle) };
    if kind != FILE_TYPE_PIPE && kind != FILE_TYPE_DISK {
        return (-1, Vec::new()); // caller falls back to std::io::stdin
    }
    let mut pending = {
        let mut guard = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        std::mem::take(&mut *guard)
    };
    loop {
        if let Some(pos) = pending.iter().position(|&byte| byte == b'\n') {
            let mut line: Vec<u8> = pending.drain(..=pos).collect();
            line.pop(); // drop the newline
            if line.ends_with(b"\r") {
                line.pop();
            }
            let status = if line.is_empty() { 0 } else { 1 };
            let mut guard = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            *guard = pending;
            return (status, line);
        }

        // A one-byte ReadFile on a pipe created by Rust's Command is
        // occasionally not satisfied even when PeekNamedPipe reports bytes
        // available. Read in chunks instead; the pending buffer above keeps
        // any bytes that belong to the next line.
        let mut buffer = [0u8; 4096];
        let mut read = 0u32;
        let ok = unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr() as *mut c_void,
                buffer.len() as u32,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let raw = unsafe { GetLastError() };
            if raw == 109 || raw == 232 {
                break;
            }
            return (-1, Vec::new());
        }
        if read == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..read as usize]);
    }
    if pending.ends_with(b"\r") {
        pending.pop();
    }
    let status = if pending.is_empty() { 0 } else { 1 };
    let mut guard = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Vec::new();
    (status, pending)
}

#[no_mangle]
pub extern "C" fn mylis_io_read_line() -> *mut c_char {
    #[cfg(windows)]
    {
        let (status, bytes) = read_line_windows();
        if status >= 0 {
            set_status(status);
            return cstring(String::from_utf8_lossy(&bytes).into_owned());
        }
        // FILE_TYPE_CHAR (console) keeps Rust's line-editing behavior.
    }
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) => { set_status(0); cstring(String::new()) }
        Ok(_) => {
            set_status(1);
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') { line.pop(); }
            }
            cstring(line)
        }
        Err(_) => { set_status(-1); cstring(String::new()) }
    }
}

#[no_mangle]
pub extern "C" fn mylis_io_read_all() -> *mut c_char {
    let mut text = String::new();
    match std::io::stdin().read_to_string(&mut text) {
        Ok(_) => { set_status(1); cstring(text) }
        Err(_) => { set_status(-1); cstring(String::new()) }
    }
}

#[no_mangle]
pub extern "C" fn mylis_io_last_status() -> i64 {
    LAST_IO_STATUS.load(Ordering::SeqCst)
}

#[no_mangle]
pub extern "C" fn mylis_io_free_string(text: *mut c_char) {
    if !text.is_null() {
        unsafe { drop(CString::from_raw(text)); }
    }
}

#[no_mangle]
pub extern "C" fn mylis_io_arg_count() -> i64 {
    std::env::args_os().count() as i64
}

#[no_mangle]
pub extern "C" fn mylis_io_arg(index: i64) -> *mut c_char {
    match arg_string(index) {
        Some(text) => cstring(text),
        None => ptr::null_mut(),
    }
}

fn env_key(key: *const c_char) -> Option<&'static str> {
    if key.is_null() { return None; }
    unsafe { CStr::from_ptr(key) }.to_str().ok()
}

#[no_mangle]
pub extern "C" fn mylis_io_env_exists(key: *const c_char) -> i64 {
    match env_key(key) {
        Some(k) => if std::env::var_os(k).is_some() { 1 } else { 0 },
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn mylis_io_env_get(key: *const c_char) -> *mut c_char {
    let key = match env_key(key) { Some(k) => k, None => return cstring(String::new()) };
    match std::env::var_os(key) {
        Some(value) => cstring(value.to_string_lossy().into_owned()),
        None => cstring(String::new()),
    }
}

#[no_mangle]
pub extern "C" fn mylis_io_exit(code: i32) {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn TerminateProcess(handle: *mut std::ffi::c_void, code: u32) -> i32;
            fn ExitProcess(code: u32) -> !;
        }
        // std::process::exit and ExitProcess both run loader/CRT teardown.
        // With several Rust staticlibs linked into one Mylis executable that
        // teardown can occasionally deadlock, so a worker would stay alive
        // after Stdio::Exit. Stdio::Exit is specified as an immediate process
        // exit; terminate without running DLL detach or atexit handlers.
        unsafe {
            TerminateProcess(GetCurrentProcess(), code as u32);
            ExitProcess(code as u32)
        }
    }
    #[cfg(not(windows))]
    {
        std::process::exit(code);
    }
}
