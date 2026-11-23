// std native UDP：绑定/收发/广播/TTL 与 UDP socket option。
use std::net::UdpSocket;
use std::os::raw::c_char;

use super::common::*;

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
