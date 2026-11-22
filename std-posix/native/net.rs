// Mylis std TCP 后端（Rust -> C ABI）。
//
// 只做阻塞/非阻塞 socket 与原始字节读写，不分配 Mylis GC 对象，不执行 Mylis 回调。
// u8[] 参数布局同 file.rs：{ u64 len; u8 items[]; }，数据区从 +8 开始。

use std::ffi::CStr;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::net::ToSocketAddrs;
use std::os::raw::{c_char, c_int, c_void};
use std::time::Duration;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, FromRawSocket};

fn errno(e: std::io::Error) -> i64 {
    #[cfg(windows)]
    {
        let raw = e.raw_os_error().unwrap_or(5);
        let posix = match raw {
            2 | 3 => 2,
            5 | 32 | 33 => 13,
            80 | 183 => 17,
            87 | 123 | 206 => 22,
            10035 => 11,
            10060 | 1460 => 110,
            10054 | 109 | 232 => 9,
            10061 => 111,
            10065 => 113,
            _ => 5,
        };
        -(posix as i64)
    }
    #[cfg(not(windows))]
    {
        -(e.raw_os_error().unwrap_or(5) as i64)
    }
}

fn socket_fd<T: SocketFd>(socket: &T) -> i64 {
    socket.raw_fd()
}

trait SocketFd {
    fn raw_fd(&self) -> i64;
}

#[cfg(unix)]
impl SocketFd for TcpListener {
    fn raw_fd(&self) -> i64 { self.as_raw_fd() as i64 }
}
#[cfg(unix)]
impl SocketFd for TcpStream {
    fn raw_fd(&self) -> i64 { self.as_raw_fd() as i64 }
}
#[cfg(unix)]
impl SocketFd for UdpSocket {
    fn raw_fd(&self) -> i64 { self.as_raw_fd() as i64 }
}
#[cfg(unix)]
impl SocketFd for UnixListener {
    fn raw_fd(&self) -> i64 { self.as_raw_fd() as i64 }
}
#[cfg(unix)]
impl SocketFd for UnixStream {
    fn raw_fd(&self) -> i64 { self.as_raw_fd() as i64 }
}
#[cfg(windows)]
impl SocketFd for TcpListener {
    fn raw_fd(&self) -> i64 { self.as_raw_socket() as i64 }
}
#[cfg(windows)]
impl SocketFd for TcpStream {
    fn raw_fd(&self) -> i64 { self.as_raw_socket() as i64 }
}
#[cfg(windows)]
impl SocketFd for UdpSocket {
    fn raw_fd(&self) -> i64 { self.as_raw_socket() as i64 }
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

// ---- socket option（跨平台）：优先用 std 稳定方法，缓冲区/keepalive 走原生 setsockopt。 ----

#[cfg(unix)]
extern "C" {
    fn setsockopt(
        fd: c_int,
        level: c_int,
        optname: c_int,
        optval: *const c_void,
        optlen: c_int,
    ) -> c_int;
}

#[cfg(windows)]
#[link(name = "ws2_32")]
extern "system" {
    fn setsockopt(s: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32;
    fn WSAGetLastError() -> i32;
}

#[cfg(target_os = "linux")]
fn sol_socket() -> i32 { 1 }
#[cfg(target_os = "linux")]
fn so_keepalive() -> i32 { 9 }
#[cfg(target_os = "linux")]
fn so_recvbuf() -> i32 { 8 }
#[cfg(target_os = "linux")]
fn so_sndbuf() -> i32 { 7 }
#[cfg(target_os = "linux")]
fn so_reuseaddr() -> i32 { 2 }
#[cfg(target_os = "linux")]
fn so_reuseport() -> i32 { 15 }

#[cfg(all(unix, not(target_os = "linux")))]
fn sol_socket() -> i32 { 0xffff }
#[cfg(all(unix, not(target_os = "linux")))]
fn so_keepalive() -> i32 { 0x0008 }
#[cfg(all(unix, not(target_os = "linux")))]
fn so_recvbuf() -> i32 { 0x1002 }
#[cfg(all(unix, not(target_os = "linux")))]
fn so_sndbuf() -> i32 { 0x1001 }
#[cfg(all(unix, not(target_os = "linux")))]
fn so_reuseaddr() -> i32 { 0x0004 }
#[cfg(all(unix, not(target_os = "linux")))]
fn so_reuseport() -> i32 { 0x0200 }

#[cfg(windows)]
fn sol_socket() -> i32 { 0xffff }
#[cfg(windows)]
fn so_keepalive() -> i32 { 0x0008 }
#[cfg(windows)]
fn so_recvbuf() -> i32 { 0x1002 }
#[cfg(windows)]
fn so_sndbuf() -> i32 { 0x1001 }
#[cfg(windows)]
fn so_reuseaddr() -> i32 { 0x0004 }

fn set_socket_option(fd: i64, level: i32, name: i32, value: i32) -> i64 {
    if fd <= 0 {
        return -9;
    }
    #[cfg(unix)]
    {
        let rc = unsafe {
            setsockopt(
                fd as c_int,
                level,
                name,
                &value as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as c_int,
            )
        };
        if rc == 0 { return 0; }
        return errno(std::io::Error::last_os_error());
    }
    #[cfg(windows)]
    {
        let rc = unsafe {
            setsockopt(
                fd as usize,
                level,
                name,
                &value as *const i32 as *const u8,
                std::mem::size_of::<i32>() as i32,
            )
        };
        if rc == 0 { return 0; }
        let code = unsafe { WSAGetLastError() };
        return errno(std::io::Error::from_raw_os_error(code));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (fd, level, name, value);
        -95
    }
}

#[cfg(unix)]
extern "C" {
    fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int;
    fn bind(fd: c_int, address: *const u8, length: u32) -> c_int;
    fn listen(fd: c_int, backlog: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
}

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
#[link(name = "ws2_32")]
extern "system" {
    fn socket(domain: i32, socket_type: i32, protocol: i32) -> usize;
    fn bind(s: usize, name: *const u8, namelen: i32) -> i32;
    fn listen(s: usize, backlog: i32) -> i32;
    fn closesocket(s: usize) -> i32;
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
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

#[no_mangle]
pub extern "C" fn mylis_udp_fd(handle: i64) -> i64 {
    if handle == 0 { return -1; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    socket_fd(s)
}

#[no_mangle]
pub extern "C" fn mylis_udp_close(handle: i64) -> i64 {
    if handle == 0 { return -9; }
    unsafe { drop(Box::from_raw(handle as *mut UdpSocket)); }
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

#[no_mangle]
pub extern "C" fn mylis_udp_set_reuse_address(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let fd = socket_fd(unsafe { &*(handle as *const UdpSocket) });
    set_socket_option(fd, sol_socket(), so_reuseaddr(), if enabled != 0 { 1 } else { 0 })
}

#[no_mangle]
pub extern "C" fn mylis_udp_set_broadcast(handle: i64, enabled: i32) -> i64 {
    if handle == 0 { return -9; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    match s.set_broadcast(enabled != 0) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

#[no_mangle]
pub extern "C" fn mylis_udp_set_ttl(handle: i64, ttl: i32) -> i64 {
    if handle == 0 || ttl <= 0 { return -22; }
    let s = unsafe { &*(handle as *const UdpSocket) };
    match s.set_ttl(ttl as u32) {
        Ok(()) => 0,
        Err(e) => errno(e),
    }
}

// ---- Unix domain socket（POSIX；Windows 返回 EOPNOTSUPP） ----

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

#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_bind(_path: *const c_char) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_accept(_handle: i64) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_connect(_path: *const c_char) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_fd(_handle: i64) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_fd(_handle: i64) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_close(_handle: i64) -> i64 { 0 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_close(_handle: i64) -> i64 { 0 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_listener_set_nonblocking(_handle: i64, _enabled: i32) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_nonblocking(_handle: i64, _enabled: i32) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_set_timeout(_handle: i64, _milliseconds: i64) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_read(_handle: i64, _data: *mut u8, _offset: i64, _count: i64) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_stream_write(_handle: i64, _data: *const u8, _offset: i64, _count: i64) -> i64 { -95 }
#[cfg(not(unix))]
#[no_mangle]
pub extern "C" fn mylis_unix_remove(_path: *const c_char) -> i64 { -95 }
