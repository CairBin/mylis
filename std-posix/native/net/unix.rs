// std native Unix domain socket：POSIX 实现与 Windows AF_UNIX 适配。
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::raw::c_char;

use super::common::*;

// ---- Unix domain socket（POSIX 与 Windows AF_UNIX） ----

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_bind(path: *const c_char) -> i64 {
    let p = match cstr(path) { Some(v) => v, None => return -22 };
    match UnixListener::bind(p) {
        Ok(l) => Box::into_raw(Box::new(l)) as i64,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_accept(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    let l = unsafe { &*(handle as *const UnixListener) };
    match l.accept() {
        Ok((s, _)) => Box::into_raw(Box::new(s)) as i64,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_connect(path: *const c_char) -> i64 {
    let p = match cstr(path) { Some(v) => v, None => return -22 };
    match UnixStream::connect(p) {
        Ok(s) => Box::into_raw(Box::new(s)) as i64,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    socket_fd(unsafe { &*(handle as *const UnixListener) })
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    socket_fd(unsafe { &*(handle as *const UnixStream) })
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut UnixListener)); }
    0
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut UnixStream)); }
    0
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let l = unsafe { &*(handle as *const UnixListener) };
    match l.set_nonblocking(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let s = unsafe { &*(handle as *const UnixStream) };
    match s.set_nonblocking(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_timeout(handle: i64, milliseconds: i64) -> i64 {
    if handle == 0 || milliseconds < 0 { return -22; }
    let s = unsafe { &*(handle as *const UnixStream) };
    let d = if milliseconds == 0 { None } else { Some(Duration::from_millis(milliseconds as u64)) };
    if let Err(e) = s.set_read_timeout(d) { return errno(e); }
    if let Err(e) = s.set_write_timeout(d) { return errno(e); }
    0
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_read(
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
    let s = unsafe { &mut *(handle as *mut UnixStream) };
    let buf = unsafe { std::slice::from_raw_parts_mut(data.add(8 + start), n) };
    match s.read(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_write(
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
    let s = unsafe { &mut *(handle as *mut UnixStream) };
    let buf = unsafe { std::slice::from_raw_parts(data.add(8 + start), n) };
    match s.write(buf) {
        Ok(v) => v as i64,
        Err(e) => errno(e),
    }
}

#[cfg(unix)]
#[no_mangle]
pub extern "C" fn mylis_unix_remove(path: *const c_char) -> i64 {
    let p = match cstr(path) { Some(v) => v, None => return -22 };
    match std::fs::remove_file(p) {
        Ok(()) => 0,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => errno(e),
    }
}

#[cfg(windows)]
#[repr(C)]
struct WinSockAddrUn {
    family: u16,
    path: [u8; 108],
}

#[cfg(windows)]
struct WinUnixSocket {
    fd: usize,
}

#[cfg(windows)]
impl Drop for WinUnixSocket {
    fn drop(&mut self) {
        unsafe { closesocket(self.fd); }
    }
}

#[cfg(windows)]
fn wsa_errno() -> i64 {
    errno(std::io::Error::from_raw_os_error(unsafe { WSAGetLastError() }))
}

#[cfg(windows)]
fn win_unix_socket() -> Result<usize, i64> {
    let mut wsa = [0u8; 512];
    let started = unsafe { WSAStartup(0x0202, wsa.as_mut_ptr()) };
    if started != 0 {
        return Err(errno(std::io::Error::from_raw_os_error(started)));
    }
    let fd = unsafe { socket(1, 1, 0) };
    if fd == usize::MAX {
        return Err(wsa_errno());
    }
    Ok(fd)
}

#[cfg(windows)]
fn win_unix_addr(path: &str) -> Result<WinSockAddrUn, i64> {
    if path.is_empty() {
        return Err(-22);
    }
    let resolved = if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        match std::env::current_dir() {
            Ok(base) => base.join(path).to_string_lossy().into_owned(),
            Err(e) => return Err(errno(e)),
        }
    };
    let bytes = resolved.as_bytes();
    if bytes.len() >= 108 {
        return Err(-36);
    }
    let mut addr = WinSockAddrUn { family: 1, path: [0; 108] };
    addr.path[..bytes.len()].copy_from_slice(bytes);
    Ok(addr)
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_bind(path: *const c_char) -> i64 {
    let p = match cstr(path) { Some(v) => v, None => return -22 };
    let addr = match win_unix_addr(p) { Ok(v) => v, Err(e) => return e };
    let fd = match win_unix_socket() { Ok(v) => v, Err(e) => return e };
    let rc = unsafe {
        bind(fd, &addr as *const WinSockAddrUn as *const u8, std::mem::size_of::<WinSockAddrUn>() as i32)
    };
    if rc != 0 {
        let code = wsa_errno();
        unsafe { closesocket(fd); }
        return code;
    }
    let rc = unsafe { listen(fd, 128) };
    if rc != 0 {
        let code = wsa_errno();
        unsafe { closesocket(fd); }
        return code;
    }
    Box::into_raw(Box::new(WinUnixSocket { fd })) as i64
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_accept(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    let listener = unsafe { &*(handle as *const WinUnixSocket) };
    let fd = unsafe { accept(listener.fd, std::ptr::null_mut(), std::ptr::null_mut()) };
    if fd == usize::MAX { return wsa_errno(); }
    Box::into_raw(Box::new(WinUnixSocket { fd })) as i64
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_connect(path: *const c_char) -> i64 {
    let p = match cstr(path) { Some(v) => v, None => return -22 };
    let addr = match win_unix_addr(p) { Ok(v) => v, Err(e) => return e };
    let fd = match win_unix_socket() { Ok(v) => v, Err(e) => return e };
    let rc = unsafe {
        connect(fd, &addr as *const WinSockAddrUn as *const u8, std::mem::size_of::<WinSockAddrUn>() as i32)
    };
    if rc != 0 {
        let code = wsa_errno();
        unsafe { closesocket(fd); }
        return code;
    }
    Box::into_raw(Box::new(WinUnixSocket { fd })) as i64
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    unsafe { (*(handle as *const WinUnixSocket)).fd as i64 }
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    unsafe { (*(handle as *const WinUnixSocket)).fd as i64 }
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut WinUnixSocket)); }
    0
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut WinUnixSocket)); }
    0
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let listener = unsafe { &*(handle as *const WinUnixSocket) };
    let mut mode: u32 = if enabled != 0 { 1 } else { 0 };
    let rc = unsafe { ioctlsocket(listener.fd, 0x8004667e, &mut mode) };
    if rc != 0 { return wsa_errno(); }
    0
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_nonblocking(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let stream = unsafe { &*(handle as *const WinUnixSocket) };
    let mut mode: u32 = if enabled != 0 { 1 } else { 0 };
    let rc = unsafe { ioctlsocket(stream.fd, 0x8004667e, &mut mode) };
    if rc != 0 { return wsa_errno(); }
    0
}

#[cfg(windows)]
fn win_unix_timeout(handle: i64, option: i32, milliseconds: i64) -> i64 {
    if handle == 0 || milliseconds < 0 { return -22; }
    let socket = unsafe { &*(handle as *const WinUnixSocket) };
    let value: u32 = if milliseconds == 0 { 0 } else { milliseconds as u32 };
    let rc = unsafe {
        setsockopt(
            socket.fd,
            sol_socket(),
            option,
            &value as *const u32 as *const u8,
            std::mem::size_of::<u32>() as i32,
        )
    };
    if rc != 0 { return wsa_errno(); }
    0
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_timeout(handle: i64, milliseconds: i64) -> i64 {
    let read = win_unix_timeout(handle, 0x1006, milliseconds);
    if read != 0 { return read; }
    win_unix_timeout(handle, 0x1005, milliseconds)
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_read(
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
    let stream = unsafe { &*(handle as *const WinUnixSocket) };
    let rc = unsafe { recv(stream.fd, data.add(8 + start), n as i32, 0) };
    if rc < 0 { return wsa_errno(); }
    rc as i64
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_write(
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
    let stream = unsafe { &*(handle as *const WinUnixSocket) };
    let rc = unsafe { send(stream.fd, data.add(8 + start), n as i32, 0) };
    if rc < 0 { return wsa_errno(); }
    rc as i64
}

#[cfg(windows)]
#[no_mangle]
pub extern "C" fn mylis_unix_remove(path: *const c_char) -> i64 {
    let p = match cstr(path) { Some(v) => v, None => return -22 };
    match std::fs::remove_file(p) {
        Ok(()) => 0,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => errno(e),
    }
}

#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_bind(_path: *const c_char) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_accept(_handle: i64) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_connect(_path: *const c_char) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_fd(_handle: i64) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_fd(_handle: i64) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_close(_handle: i64) -> i64 { 0 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_close(_handle: i64) -> i64 { 0 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_set_nonblocking(_handle: i64, _enabled: i32) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_nonblocking(_handle: i64, _enabled: i32) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_timeout(_handle: i64, _milliseconds: i64) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_read(_handle: i64, _data: *mut u8, _offset: i64, _count: i64) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_write(_handle: i64, _data: *const u8, _offset: i64, _count: i64) -> i64 { -95 }
#[cfg(all(not(unix), not(windows)))]
#[no_mangle]
pub extern "C" fn mylis_unix_remove(_path: *const c_char) -> i64 { -95 }
