// Mylis std TCP 后端（Rust -> C ABI）。
//
// 只做阻塞/非阻塞 socket 与原始字节读写，不分配 Mylis GC 对象，不执行 Mylis 回调。
// u8[] 参数布局同 file.rs：{ u64 len; u8 items[]; }，数据区从 +8 开始。

use std::ffi::CStr;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::os::raw::c_char;
use std::time::Duration;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

fn errno(e: std::io::Error) -> i64 {
    -(e.raw_os_error().unwrap_or(5) as i64)
}

fn array_len(data: *const u8) -> Option<usize> {
    if data.is_null() {
        return None;
    }
    Some(unsafe { *(data as *const u64) } as usize)
}

fn array_len_i64(data: *const i64) -> Option<usize> {
    if data.is_null() { return None; }
    Some(unsafe { *(data as *const u64) } as usize)
}

fn cstr(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

#[no_mangle]
pub extern "C" fn mylis_tcp_bind(address: *const c_char) -> i64 {
    let a = match cstr(address) { Some(v) => v, None => return -22 };
    match TcpListener::bind(a) {
        Ok(l) => Box::into_raw(Box::new(l)) as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_listener_port(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    let l = unsafe { &*(handle as *const TcpListener) };
    match l.local_addr() {
        Ok(a) => a.port() as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_accept(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    let l = unsafe { &*(handle as *const TcpListener) };
    match l.accept() {
        Ok((s, _)) => Box::into_raw(Box::new(s)) as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_connect(address: *const c_char) -> i64 {
    let a = match cstr(address) { Some(v) => v, None => return -22 };
    match TcpStream::connect(a) {
        Ok(s) => Box::into_raw(Box::new(s)) as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_listener_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut TcpListener)); }
    0
}

#[no_mangle]
pub extern "C" fn mylis_udp_bind(address: *const c_char) -> i64 {
    let a = match cstr(address) { Some(v) => v, None => return -22 };
    match UdpSocket::bind(a) {
        Ok(s) => Box::into_raw(Box::new(s)) as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_udp_port(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    match s.local_addr() {
        Ok(a) => a.port() as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_udp_send(
    handle: i64,
    address: *const c_char,
    data: *const u8,
    offset: i64,
    count: i64,
) -> i64 {
    if handle == 0 || address.is_null() || data.is_null() || offset < 0 || count < 0 { return -22; }
    let a = match cstr(address) { Some(v) => v, None => return -22 };
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start { return -22; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    let buf = unsafe { std::slice::from_raw_parts(data.add(8 + start), n) };
    match s.send_to(buf, a) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_udp_recv(
    handle: i64,
    data: *mut u8,
    offset: i64,
    count: i64,
) -> i64 {
    if handle == 0 || data.is_null() || offset < 0 || count < 0 { return -22; }
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start { return -22; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    let buf = unsafe { std::slice::from_raw_parts_mut(data.add(8 + start), n) };
    match s.recv(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_udp_recv_from(
    handle: i64,
    data: *mut u8,
    offset: i64,
    count: i64,
    from: *mut u8,
    from_meta: *mut i64,
) -> i64 {
    if handle == 0 || data.is_null() || from.is_null() || from_meta.is_null() || offset < 0 || count < 0 { return -22; }
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let flen = match array_len(from) { Some(n) => n, None => return -22 };
    let mlen = match array_len_i64(from_meta) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start || mlen < 1 { return -22; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    let buf = unsafe { std::slice::from_raw_parts_mut(data.add(8 + start), n) };
    match s.recv_from(buf) {
        Ok((got, addr)) => {
            let text = addr.to_string();
            let bytes = text.as_bytes();
            let copy = if bytes.len() < flen { bytes.len() } else { flen };
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), from.add(8), copy);
                *(from_meta.add(1)) = copy as i64;
            }
            got as i64
        }
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_udp_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    match s.set_nonblocking(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_udp_fd(handle: i64) -> i32 {
    if handle == 0 { return -1; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    s.as_raw_fd()
}

#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_udp_fd(_handle: i64) -> i32 { -1 }

#[no_mangle]
pub extern "C" fn mylis_udp_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut UdpSocket)); }
    0
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_tcp_stream_fd(handle: i64) -> i32 {
    if handle == 0 { return -1; }
    let s = unsafe { &*(handle as *const TcpStream) };
    s.as_raw_fd()
}

#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_tcp_stream_fd(_handle: i64) -> i32 { -1 }

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_tcp_listener_fd(handle: i64) -> i32 {
    if handle == 0 { return -1; }
    let l = unsafe { &*(handle as *const TcpListener) };
    l.as_raw_fd()
}

#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_tcp_listener_fd(_handle: i64) -> i32 { -1 }

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut TcpStream)); }
    0
}

#[no_mangle]
pub extern "C" fn mylis_tcp_listener_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let l = unsafe { &*(handle as *const TcpListener) };
    match l.set_nonblocking(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let s = unsafe { &*(handle as *const TcpStream) };
    match s.set_nonblocking(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_set_timeout(handle: i64, milliseconds: i64) -> i64 {
    if handle == 0 || milliseconds < 0 { return -22; }
    let s = unsafe { &*(handle as *const TcpStream) };
    let d = if milliseconds == 0 { None } else { Some(Duration::from_millis(milliseconds as u64)) };
    if let Err(e) = s.set_read_timeout(d) { return errno(e); }
    if let Err(e) = s.set_write_timeout(d) { return errno(e); }
    0
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_read(
    handle: i64,
    data: *mut u8,
    offset: i64,
    count: i64,
) -> i64 {
    if handle == 0 || data.is_null() || offset < 0 || count < 0 { return -22; }
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start { return -22; }
    let s = unsafe { &mut *(handle as *mut TcpStream) };
    let buf = unsafe { std::slice::from_raw_parts_mut(data.add(8 + start), n) };
    match s.read(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_write(
    handle: i64,
    data: *const u8,
    offset: i64,
    count: i64,
) -> i64 {
    if handle == 0 || data.is_null() || offset < 0 || count < 0 { return -22; }
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start { return -22; }
    let s = unsafe { &mut *(handle as *mut TcpStream) };
    let buf = unsafe { std::slice::from_raw_parts(data.add(8 + start), n) };
    match s.write(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}
