// std native 网络公共部分：错误码、句柄抽象、数组布局、socket option 与原始 socket 声明。
use std::ffi::CStr;
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::os::raw::c_char;
#[cfg(unix)]
use std::os::raw::{c_int, c_void};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use std::os::windows::io::AsRawSocket;

pub(crate) fn errno(e: std::io::Error) -> i64 {
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

pub(crate) fn socket_fd<T: SocketFd>(socket: &T) -> i64 {
    socket.raw_fd()
}

pub(crate) trait SocketFd {
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

pub(crate) fn array_len(data: *const u8) -> Option<usize> {
    if data.is_null() {
        return None;
    }
    Some(unsafe { *(data as *const u64) } as usize)
}

pub(crate) fn array_len_i64(data: *const i64) -> Option<usize> {
    if data.is_null() { return None; }
    Some(unsafe { *(data as *const u64) } as usize)
}

pub(crate) fn cstr(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

// ---- socket option（跨平台）：优先用 std 稳定方法，缓冲区/keepalive 走原生 setsockopt。 ----

#[cfg(unix)]
extern "C" {
    pub(crate) fn setsockopt(
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
    pub(crate) fn setsockopt(s: usize, level: i32, optname: i32, optval: *const u8, optlen: i32) -> i32;
    pub(crate) fn WSAGetLastError() -> i32;
}

#[cfg(target_os = "linux")]
pub(crate) fn sol_socket() -> i32 { 1 }
#[cfg(target_os = "linux")]
pub(crate) fn so_keepalive() -> i32 { 9 }
#[cfg(target_os = "linux")]
pub(crate) fn so_recvbuf() -> i32 { 8 }
#[cfg(target_os = "linux")]
pub(crate) fn so_sndbuf() -> i32 { 7 }
#[cfg(target_os = "linux")]
pub(crate) fn so_reuseaddr() -> i32 { 2 }
#[cfg(target_os = "linux")]
pub(crate) fn so_reuseport() -> i32 { 15 }

#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn sol_socket() -> i32 { 0xffff }
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn so_keepalive() -> i32 { 0x0008 }
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn so_recvbuf() -> i32 { 0x1002 }
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn so_sndbuf() -> i32 { 0x1001 }
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn so_reuseaddr() -> i32 { 0x0004 }
#[cfg(all(unix, not(target_os = "linux")))]
pub(crate) fn so_reuseport() -> i32 { 0x0200 }

#[cfg(windows)]
pub(crate) fn sol_socket() -> i32 { 0xffff }
#[cfg(windows)]
pub(crate) fn so_keepalive() -> i32 { 0x0008 }
#[cfg(windows)]
pub(crate) fn so_recvbuf() -> i32 { 0x1002 }
#[cfg(windows)]
pub(crate) fn so_sndbuf() -> i32 { 0x1001 }
#[cfg(windows)]
pub(crate) fn so_reuseaddr() -> i32 { 0x0004 }

pub(crate) fn set_socket_option(fd: i64, level: i32, name: i32, value: i32) -> i64 {
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
    pub(crate) fn socket(domain: c_int, socket_type: c_int, protocol: c_int) -> c_int;
    pub(crate) fn bind(fd: c_int, address: *const u8, length: u32) -> c_int;
    pub(crate) fn listen(fd: c_int, backlog: c_int) -> c_int;
    pub(crate) fn close(fd: c_int) -> c_int;
}

#[cfg(windows)]
#[link(name = "ws2_32")]
extern "system" {
    pub(crate) fn socket(domain: i32, socket_type: i32, protocol: i32) -> usize;
    pub(crate) fn bind(s: usize, name: *const u8, namelen: i32) -> i32;
    pub(crate) fn listen(s: usize, backlog: i32) -> i32;
    pub(crate) fn accept(s: usize, name: *mut u8, namelen: *mut i32) -> usize;
    pub(crate) fn connect(s: usize, name: *const u8, namelen: i32) -> i32;
    pub(crate) fn recv(s: usize, buf: *mut u8, len: i32, flags: i32) -> i32;
    pub(crate) fn send(s: usize, buf: *const u8, len: i32, flags: i32) -> i32;
    pub(crate) fn ioctlsocket(s: usize, cmd: u32, argp: *mut u32) -> i32;
    pub(crate) fn closesocket(s: usize) -> i32;
    pub(crate) fn WSAStartup(version: u16, data: *mut u8) -> i32;
}
