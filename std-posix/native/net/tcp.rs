// std native TCP：监听/连接/读写与 TCP socket option。
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::net::ToSocketAddrs;
use std::os::raw::c_char;
#[cfg(unix)]
use std::os::raw::{c_int, c_void};
use std::time::Duration;
#[cfg(unix)]
use std::os::unix::io::FromRawFd;
#[cfg(windows)]
use std::os::windows::io::FromRawSocket;

use super::common::*;

#[cfg(unix)]
#[repr(C)]
struct SockAddrIn {
    family: u16,
    port: u16,
    address: u32,
    zero: [u8; 8],
}

#[cfg(unix)]
fn bind_reuse_unix(address: &str) -> i64 {
    let parsed = match address.to_socket_addrs() {
        Ok(mut values) => match values.next() {
            Some(std::net::SocketAddr::V4(v4)) => v4,
            _ => return -22,
        },
        Err(e) => return errno(e),
    };
    let fd = unsafe { socket(2, 1, 0) };
    if fd < 0 { return errno(std::io::Error::last_os_error()); }
    let value: i32 = 1;
    let opt = unsafe {
        setsockopt(
            fd,
            sol_socket(),
            so_reuseaddr(),
            &value as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as c_int,
        )
    };
    if opt != 0 {
        let code = errno(std::io::Error::last_os_error());
        unsafe { close(fd); }
        return code;
    }
    // Linux 用 SO_REUSEPORT 允许多个监听器同时绑定同一地址；不支持的平台忽略。
    let _ = unsafe {
        setsockopt(
            fd,
            sol_socket(),
            so_reuseport(),
            &value as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as c_int,
        )
    };
    let addr = SockAddrIn {
        family: 2,
        port: parsed.port().to_be(),
        address: u32::from(*parsed.ip()).to_be(),
        zero: [0; 8],
    };
    let rc = unsafe { bind(fd, &addr as *const SockAddrIn as *const u8, std::mem::size_of::<SockAddrIn>() as u32) };
    if rc != 0 {
        let code = errno(std::io::Error::last_os_error());
        unsafe { close(fd); }
        return code;
    }
    let rc = unsafe { listen(fd, 128) };
    if rc != 0 {
        let code = errno(std::io::Error::last_os_error());
        unsafe { close(fd); }
        return code;
    }
    let listener = unsafe { TcpListener::from_raw_fd(fd) };
    Box::into_raw(Box::new(listener)) as i64
}


#[cfg(windows)]
#[repr(C)]
struct WinSockAddrIn {
    family: u16,
    port: u16,
    address: u32,
    zero: [u8; 8],
}

#[cfg(windows)]
fn bind_reuse_windows(address: &str) -> i64 {
    let parsed = match address.to_socket_addrs() {
        Ok(mut values) => match values.next() {
            Some(std::net::SocketAddr::V4(v4)) => v4,
            _ => return -22,
        },
        Err(e) => return errno(e),
    };
    let mut wsa = [0u8; 512];
    let started = unsafe { WSAStartup(0x0202, wsa.as_mut_ptr()) };
    if started != 0 { return errno(std::io::Error::from_raw_os_error(started)); }
    let fd = unsafe { socket(2, 1, 0) };
    if fd == 0 { return errno(std::io::Error::last_os_error()); }
    let value: i32 = 1;
    let opt = unsafe {
        setsockopt(
            fd,
            sol_socket(),
            so_reuseaddr(),
            &value as *const i32 as *const u8,
            std::mem::size_of::<i32>() as i32,
        )
    };
    if opt != 0 {
        let code = errno(std::io::Error::last_os_error());
        unsafe { closesocket(fd); }
        return code;
    }
    let addr = WinSockAddrIn {
        family: 2,
        port: parsed.port().to_be(),
        address: u32::from(*parsed.ip()).to_be(),
        zero: [0; 8],
    };
    let rc = unsafe { bind(fd, &addr as *const WinSockAddrIn as *const u8, std::mem::size_of::<WinSockAddrIn>() as i32) };
    if rc != 0 {
        let code = errno(std::io::Error::last_os_error());
        unsafe { closesocket(fd); }
        return code;
    }
    let rc = unsafe { listen(fd, 128) };
    if rc != 0 {
        let code = errno(std::io::Error::last_os_error());
        unsafe { closesocket(fd); }
        return code;
    }
    let listener = unsafe { TcpListener::from_raw_socket(fd as u64) };
    Box::into_raw(Box::new(listener)) as i64
}


#[no_mangle]
pub extern "C" fn mylis_tcp_bind_reuse(address: *const c_char) -> i64 {
    let a = match cstr(address) { Some(v) => v, None => return -22 };
    #[cfg(unix)]
    {
        return bind_reuse_unix(a);
    }
    #[cfg(windows)]
    {
        return bind_reuse_windows(a);
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = a;
        return mylis_tcp_bind(address);
    }
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
pub extern "C" fn mylis_tcp_stream_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    let s = unsafe { &*(handle as *const TcpStream) };
    socket_fd(s)
}

#[no_mangle]
pub extern "C" fn mylis_tcp_listener_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    let l = unsafe { &*(handle as *const TcpListener) };
    socket_fd(l)
}

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

// ---- socket options ----

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_set_nodelay(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let s = unsafe { &*(handle as *const TcpStream) };
    match s.set_nodelay(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_set_keepalive(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let fd = socket_fd(unsafe { &*(handle as *const TcpStream) });
    set_socket_option(fd, sol_socket(), so_keepalive(), if enabled != 0 { 1 } else { 0 })
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_set_recv_buffer(handle: i64, bytes: i32) -> i64 {
    if handle == 0 || bytes <= 0 { return -22; }
    let fd = socket_fd(unsafe { &*(handle as *const TcpStream) });
    set_socket_option(fd, sol_socket(), so_recvbuf(), bytes)
}

#[no_mangle]
pub extern "C" fn mylis_tcp_stream_set_send_buffer(handle: i64, bytes: i32) -> i64 {
    if handle == 0 || bytes <= 0 { return -22; }
    let fd = socket_fd(unsafe { &*(handle as *const TcpStream) });
    set_socket_option(fd, sol_socket(), so_sndbuf(), bytes)
}

#[no_mangle]
pub extern "C" fn mylis_tcp_listener_set_reuse_address(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let fd = socket_fd(unsafe { &*(handle as *const TcpListener) });
    set_socket_option(fd, sol_socket(), so_reuseaddr(), if enabled != 0 { 1 } else { 0 })
}
