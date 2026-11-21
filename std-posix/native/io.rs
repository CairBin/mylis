//! stdio / process environment native backend for Linux and other POSIX hosts.
//! Every returned char* is owned by the caller and must be released with
//! mylis_io_free_string.

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

#[no_mangle]
pub extern "C" fn mylis_io_read_line() -> *mut c_char {
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
    std::process::exit(code);
}
