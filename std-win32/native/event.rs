// Mylis Windows 事件后端（Winsock WSAPoll）。
// 只等待 socket 就绪并返回 fd + revents；不执行 Mylis 回调。
// fd 是 Windows SOCKET 的整数值，和 POSIX 事件后端一样以 i64 传递。

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Once;

const POLLRDNORM: i16 = 0x0100;
const POLLWRNORM: i16 = 0x0010;
const POLLERR: i16 = 0x0001;
const POLLHUP: i16 = 0x0002;
const POLLNVAL: i16 = 0x0004;

#[repr(C, align(8))]
struct WsaData {
    bytes: [u8; 512],
}

#[repr(C)]
struct WsaPollFd {
    fd: usize,
    events: i16,
    revents: i16,
}

#[link(name = "ws2_32")]
extern "system" {
    fn WSAStartup(version: u16, data: *mut WsaData) -> i32;
    fn WSAPoll(fds: *mut WsaPollFd, count: u32, timeout_ms: i32) -> i32;
    fn WSAGetLastError() -> i32;
}

static START: Once = Once::new();
static START_ERROR: AtomicI32 = AtomicI32::new(0);

fn wsa_startup() -> Result<(), i64> {
    START.call_once(|| {
        let mut data = WsaData { bytes: [0; 512] };
        let rc = unsafe { WSAStartup(0x0202, &mut data) };
        if rc != 0 {
            START_ERROR.store(rc, Ordering::SeqCst);
        }
    });
    let rc = START_ERROR.load(Ordering::SeqCst);
    if rc == 0 { Ok(()) } else { Err(-(rc as i64)) }
}

struct EventState {
    fds: Vec<u64>,
    events: Vec<i16>,
}

fn len_i64(data: *const i64) -> Option<usize> {
    if data.is_null() { return None; }
    Some(unsafe { *(data as *const u64) } as usize)
}

fn fill_output(data: *mut i64, index: usize, value: i64) {
    unsafe { *(data.add(1 + index)) = value; } // i64[] 数据区从 +8 开始
}

#[no_mangle]
pub extern "C" fn mylis_event_new() -> i64 {
    match wsa_startup() {
        Ok(()) => Box::into_raw(Box::new(EventState { fds: Vec::new(), events: Vec::new() })) as i64,
        Err(code) => code,
    }
}

#[no_mangle]
pub extern "C" fn mylis_event_add(handle: i64, fd: i64) -> i64 {
    if handle <= 0 || fd <= 0 { return -22; }
    let state = unsafe { &mut *(handle as *mut EventState) };
    let socket = fd as u64;
    if let Some(index) = state.fds.iter().position(|value| *value == socket) {
        state.events[index] = POLLRDNORM;
    } else {
        state.fds.push(socket);
        state.events.push(POLLRDNORM);
    }
    0
}

#[no_mangle]
pub extern "C" fn mylis_event_modify(handle: i64, fd: i64, write_enabled: i32) -> i64 {
    if handle <= 0 || fd <= 0 { return -22; }
    let state = unsafe { &mut *(handle as *mut EventState) };
    let socket = fd as u64;
    if let Some(index) = state.fds.iter().position(|value| *value == socket) {
        state.events[index] = if write_enabled != 0 { POLLRDNORM | POLLWRNORM } else { POLLRDNORM };
        return 0;
    }
    -2
}

#[no_mangle]
pub extern "C" fn mylis_event_remove(handle: i64, fd: i64) -> i64 {
    if handle <= 0 || fd <= 0 { return -22; }
    let state = unsafe { &mut *(handle as *mut EventState) };
    let socket = fd as u64;
    if let Some(index) = state.fds.iter().position(|value| *value == socket) {
        state.fds.remove(index);
        state.events.remove(index);
    }
    0
}

#[no_mangle]
pub extern "C" fn mylis_event_wait(
    handle: i64,
    timeout_ms: i32,
    fds: *mut i64,
    revents: *mut i64,
    max: i32,
) -> i32 {
    if handle <= 0 || max < 0 || fds.is_null() || revents.is_null() { return -22; }
    let n = max as usize;
    let flen = match len_i64(fds) { Some(value) => value, None => return -22 };
    let rlen = match len_i64(revents) { Some(value) => value, None => return -22 };
    if n > flen || n > rlen { return -22; }
    if n == 0 { return 0; }

    let state = unsafe { &mut *(handle as *mut EventState) };
    if state.fds.is_empty() {
        if timeout_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(timeout_ms as u64));
        }
        return 0;
    }

    let mut poll_fds: Vec<WsaPollFd> = Vec::with_capacity(state.fds.len());
    for index in 0..state.fds.len() {
        poll_fds.push(WsaPollFd {
            fd: state.fds[index] as usize,
            events: state.events[index],
            revents: 0,
        });
    }
    let rc = unsafe { WSAPoll(poll_fds.as_mut_ptr(), poll_fds.len() as u32, timeout_ms) };
    if rc < 0 {
        return -(unsafe { WSAGetLastError() });
    }
    let ready = std::cmp::min(rc as usize, n);
    let mut written = 0usize;
    for index in 0..poll_fds.len() {
        let re = poll_fds[index].revents;
        if re == 0 {
            continue;
        }
        if written >= ready {
            break;
        }
        let mut mask = 0i32;
        if (re & POLLRDNORM) != 0 { mask |= 1; }
        if (re & POLLWRNORM) != 0 { mask |= 4; }
        if (re & POLLERR) != 0 { mask |= 8; }
        if (re & POLLHUP) != 0 { mask |= 16; }
        if (re & POLLNVAL) != 0 { mask |= 32; }
        fill_output(fds, written, poll_fds[index].fd as i64);
        fill_output(revents, written, mask as i64);
        written += 1;
    }
    written as i32
}

#[no_mangle]
pub extern "C" fn mylis_event_close(handle: i64) -> i64 {
    if handle <= 0 { return 0; }
    unsafe { drop(Box::from_raw(handle as *mut EventState)); }
    0
}
