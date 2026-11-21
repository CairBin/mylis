// Mylis std 文件后端（Rust -> C ABI）。
//
// 这个库只负责文件句柄和原始字节读写，不分配 Mylis GC 对象，也不调用 Mylis 代码。
// `u8[]` 参数在 Mylis ABI 里是 { u64 len; u8 items[]; } 的指针；这里按首 8 字节取长度，
// 数据区从 +8 开始。

use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::raw::c_char;
use std::ptr;

fn errno(e: std::io::Error) -> i64 {
    #[cfg(windows)]
    {
        let raw = e.raw_os_error().unwrap_or(5);
        let posix = match raw {
            2 | 3 => 2,
            5 | 32 | 33 => 13,
            80 | 183 => 17,
            87 | 123 | 206 => 22,
            109 | 232 => 32,
            995 | 258 => 110,
            _ => 5,
        };
        -(posix as i64)
    }
    #[cfg(not(windows))]
    {
        -(e.raw_os_error().unwrap_or(5) as i64)
    }
}

fn path_str<'a>(path: *const c_char) -> Result<&'a str, i64> {
    if path.is_null() { return Err(-22); }
    unsafe { CStr::from_ptr(path) }.to_str().map_err(|_| -22)
}

fn cstring(text: String) -> *mut c_char {
    CString::new(text).map(|s| s.into_raw()).unwrap_or(ptr::null_mut())
}

fn dir_names(path: *const c_char) -> Result<Vec<String>, i64> {
    let p = path_str(path)?;
    let rd = std::fs::read_dir(p).map_err(errno)?;
    let mut names: Vec<String> = Vec::new();
    for entry in rd {
        let entry = entry.map_err(errno)?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    Ok(names)
}

fn array_len(data: *const u8) -> Option<usize> {
    if data.is_null() {
        return None;
    }
    Some(unsafe { *(data as *const u64) } as usize)
}

#[no_mangle]
pub extern "C" fn mylis_file_open(path: *const c_char, mode: i32) -> i64 {
    if path.is_null() {
        return -22;
    }
    let p = match unsafe { CStr::from_ptr(path) }.to_str() {
        Ok(s) => s,
        Err(_) => return -22,
    };
    let mut opts = OpenOptions::new();
    if mode == 1 {
        opts.write(true).create(true).truncate(true);
    } else if mode == 2 {
        opts.append(true).create(true);
    } else {
        opts.read(true);
    }
    match opts.open(p) {
        Ok(f) => Box::into_raw(Box::new(f)) as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_exists(path: *const c_char) -> i64 {
    if path.is_null() { return -22; }
    let p = match unsafe { CStr::from_ptr(path) }.to_str() { Ok(s) => s, Err(_) => return -22 };
    match std::fs::metadata(p) {
        Ok(_) => 1,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound { 0 } else { errno(e) }
        }
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_remove(path: *const c_char) -> i64 {
    if path.is_null() { return -22; }
    let p = match unsafe { CStr::from_ptr(path) }.to_str() { Ok(s) => s, Err(_) => return -22 };
    match std::fs::remove_file(p) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_rename(from: *const c_char, to: *const c_char) -> i64 {
    if from.is_null() || to.is_null() { return -22; }
    let a = match unsafe { CStr::from_ptr(from) }.to_str() { Ok(s) => s, Err(_) => return -22 };
    let b = match unsafe { CStr::from_ptr(to) }.to_str() { Ok(s) => s, Err(_) => return -22 };
    match std::fs::rename(a, b) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_is_dir(path: *const c_char) -> i64 {
    let p = match path_str(path) { Ok(p) => p, Err(e) => return e };
    match std::fs::metadata(p) {
        Ok(m) => if m.is_dir() { 1 } else { 0 },
        Err(e) => if e.kind() == std::io::ErrorKind::NotFound { 0 } else { errno(e) },
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_mkdir(path: *const c_char) -> i64 {
    let p = match path_str(path) { Ok(p) => p, Err(e) => return e };
    match std::fs::create_dir(p) { Ok(()) => 0, Err(e) => errno(e) }
}

#[no_mangle]
pub extern "C" fn mylis_file_mkdir_all(path: *const c_char) -> i64 {
    let p = match path_str(path) { Ok(p) => p, Err(e) => return e };
    match std::fs::create_dir_all(p) { Ok(()) => 0, Err(e) => errno(e) }
}

#[no_mangle]
pub extern "C" fn mylis_file_remove_dir(path: *const c_char) -> i64 {
    let p = match path_str(path) { Ok(p) => p, Err(e) => return e };
    match std::fs::remove_dir(p) { Ok(()) => 0, Err(e) => errno(e) }
}

#[no_mangle]
pub extern "C" fn mylis_file_remove_dir_all(path: *const c_char) -> i64 {
    let p = match path_str(path) { Ok(p) => p, Err(e) => return e };
    match std::fs::remove_dir_all(p) { Ok(()) => 0, Err(e) => errno(e) }
}

#[no_mangle]
pub extern "C" fn mylis_file_metadata_size(path: *const c_char) -> i64 {
    let p = match path_str(path) { Ok(p) => p, Err(e) => return e };
    match std::fs::metadata(p) { Ok(m) => m.len() as i64, Err(e) => errno(e) }
}

#[no_mangle]
pub extern "C" fn mylis_file_read_dir_count(path: *const c_char) -> i64 {
    match dir_names(path) { Ok(names) => names.len() as i64, Err(e) => e }
}

#[no_mangle]
pub extern "C" fn mylis_file_read_dir_entry(path: *const c_char, index: i64) -> *mut c_char {
    if index < 0 { return cstring(String::new()); }
    let names = match dir_names(path) { Ok(n) => n, Err(_) => return cstring(String::new()) };
    match names.get(index as usize) {
        Some(name) => cstring(name.clone()),
        None => cstring(String::new()),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_free_string(text: *mut c_char) {
    if !text.is_null() {
        unsafe { drop(CString::from_raw(text)); }
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_tell(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    let f = unsafe { &mut *(handle as *mut File) };
    match f.stream_position() { Ok(n) => n as i64, Err(e) => errno(e) }
}

#[no_mangle]
pub extern "C" fn mylis_file_close(handle: i64) -> i64 {
    if handle == 0 {
        return -9;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut File));
    }
    0
}

#[no_mangle]
pub extern "C" fn mylis_file_size(handle: i64) -> i64 {
    if handle == 0 {
        return -9;
    }
    let f = unsafe { &*(handle as *const File) };
    match f.metadata() {
        Ok(m) => m.len() as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_read(
    handle: i64,
    data: *mut u8,
    offset: i64,
    count: i64,
) -> i64 {
    if handle == 0 || data.is_null() || offset < 0 || count < 0 {
        return -22;
    }
    let len = match array_len(data) {
        Some(n) => n,
        None => return -22,
    };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start {
        return -22;
    }
    let f = unsafe { &mut *(handle as *mut File) };
    let buf = unsafe { std::slice::from_raw_parts_mut(data.add(8 + start), n) };
    match f.read(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_write(
    handle: i64,
    data: *const u8,
    offset: i64,
    count: i64,
) -> i64 {
    if handle == 0 || data.is_null() || offset < 0 || count < 0 {
        return -22;
    }
    let len = match array_len(data) {
        Some(n) => n,
        None => return -22,
    };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start {
        return -22;
    }
    let f = unsafe { &mut *(handle as *mut File) };
    let buf = unsafe { std::slice::from_raw_parts(data.add(8 + start), n) };
    match f.write(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_seek(handle: i64, offset: i64) -> i64 {
    if handle == 0 || offset < 0 {
        return -22;
    }
    let f = unsafe { &mut *(handle as *mut File) };
    match f.seek(SeekFrom::Start(offset as u64)) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_flush(handle: i64) -> i64 {
    if handle == 0 {
        return -9;
    }
    let f = unsafe { &mut *(handle as *mut File) };
    match f.flush() {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}
