// Mylis EventLoop 的事件后端。
// Linux: epoll；其他 POSIX: poll 后备；Windows: ENOSYS 占位。
// 只等待 fd 就绪并返回 fd + revents；不执行 Mylis 回调。

fn len_i32(data: *const i32) -> Option<usize> {
    if data.is_null() { return None; }
    Some(unsafe { *(data as *const u64) } as usize)
}

fn fill_output(data: *mut i32, index: usize, value: i32) {
    unsafe { *(data.add(2 + index)) = value; } // i32[] 数据区从 +8 开始
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{fill_output, len_i32};
    use std::io::Error;

    const EPOLL_CTL_ADD: i32 = 1;
    const EPOLL_CTL_DEL: i32 = 2;
    const EPOLLIN: u32 = 0x001;
    const EPOLLERR: u32 = 0x008;
    const EPOLLOUT: u32 = 0x004;
    const EPOLLHUP: u32 = 0x010;

    #[repr(C, packed)]
    struct EpollEvent {
        events: u32,
        data: u64,
    }

    extern "C" {
        fn epoll_create1(flags: i32) -> i32;
        fn epoll_ctl(epfd: i32, op: i32, fd: i32, event: *mut EpollEvent) -> i32;
        fn epoll_wait(epfd: i32, events: *mut EpollEvent, maxevents: i32, timeout: i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    fn errno() -> i64 {
        -(Error::last_os_error().raw_os_error().unwrap_or(5) as i64)
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_new() -> i64 {
        let fd = unsafe { epoll_create1(0) };
        if fd < 0 { return errno(); }
        fd as i64
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_add(handle: i64, fd: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let mut ev = EpollEvent { events: EPOLLIN, data: fd as u64 };
        let rc = unsafe { epoll_ctl(handle as i32, EPOLL_CTL_ADD, fd, &mut ev) };
        if rc < 0 { return errno(); }
        0
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_modify(handle: i64, fd: i32, write_enabled: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let mut ev = EpollEvent {
            events: if write_enabled != 0 { EPOLLIN | 0x004 } else { EPOLLIN },
            data: fd as u64,
        };
        let rc = unsafe { epoll_ctl(handle as i32, 3, fd, &mut ev) }; // EPOLL_CTL_MOD
        if rc < 0 { return errno(); }
        0
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_remove(handle: i64, fd: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let rc = unsafe { epoll_ctl(handle as i32, EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
        if rc < 0 { return errno(); }
        0
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_wait(
        handle: i64,
        timeout_ms: i32,
        fds: *mut i32,
        revents: *mut i32,
        max: i32,
    ) -> i32 {
        if handle <= 0 || max < 0 || fds.is_null() || revents.is_null() { return -22; }
        let n = max as usize;
        let flen = match len_i32(fds) { Some(v) => v, None => return -22 };
        let rlen = match len_i32(revents) { Some(v) => v, None => return -22 };
        if n > flen || n > rlen { return -22; }
        if n == 0 { return 0; }
        let mut events: Vec<EpollEvent> = Vec::with_capacity(n);
        for _ in 0..n { events.push(EpollEvent { events: 0, data: 0 }); }
        let rc = unsafe { epoll_wait(handle as i32, events.as_mut_ptr(), n as i32, timeout_ms) };
        if rc < 0 { return -(Error::last_os_error().raw_os_error().unwrap_or(5) as i32); }
        for i in 0..(rc as usize) {
            let fd = events[i].data as i32;
            let ev = events[i].events;
            let mut mask = 0i32;
            if (ev & EPOLLIN) != 0 { mask |= 1; }
            if (ev & EPOLLOUT) != 0 { mask |= 4; }
            if (ev & EPOLLERR) != 0 { mask |= 8; }
            if (ev & EPOLLHUP) != 0 { mask |= 16; }
            fill_output(fds, i, fd);
            fill_output(revents, i, mask);
        }
        rc
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_close(handle: i64) -> i64 {
        if handle <= 0 { return 0; }
        unsafe { close(handle as i32); }
        0
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
mod posix_poll {
    use super::{fill_output, len_i32};

    #[repr(C)]
    struct PollFd { fd: i32, events: i16, revents: i16 }

    extern "C" { fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32; }

    struct PollState { fds: Vec<i32>, events: Vec<i32> }

    #[no_mangle]
    pub extern "C" fn mylis_event_new() -> i64 {
        Box::into_raw(Box::new(PollState { fds: Vec::new(), events: Vec::new() })) as i64
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_add(handle: i64, fd: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let s = unsafe { &mut *(handle as *mut PollState) };
        if !s.fds.contains(&fd) {
            s.fds.push(fd);
            s.events.push(1);
        }
        0
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_modify(handle: i64, fd: i32, write_enabled: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let mut ev = EpollEvent {
            events: if write_enabled != 0 { EPOLLIN | 0x004 } else { EPOLLIN },
            data: fd as u64,
        };
        let rc = unsafe { epoll_ctl(handle as i32, 3, fd, &mut ev) }; // EPOLL_CTL_MOD
        if rc < 0 { return errno(); }
        0
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_modify(handle: i64, fd: i32, write_enabled: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let s = unsafe { &mut *(handle as *mut PollState) };
        let mut i = 0;
        while i < s.fds.len() {
            if s.fds[i] == fd {
                s.events[i] = if write_enabled != 0 { 1 | 4 } else { 1 };
                return 0;
            }
            i += 1;
        }
        -2
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_remove(handle: i64, fd: i32) -> i64 {
        if handle <= 0 || fd < 0 { return -22; }
        let s = unsafe { &mut *(handle as *mut PollState) };
        let mut i = 0;
        while i < s.fds.len() {
            if s.fds[i] == fd {
                s.fds.remove(i);
                s.events.remove(i);
                return 0;
            }
            i += 1;
        }
        0
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_wait(
        handle: i64,
        timeout_ms: i32,
        fds: *mut i32,
        revents: *mut i32,
        max: i32,
    ) -> i32 {
        if handle <= 0 || max < 0 || fds.is_null() || revents.is_null() { return -22; }
        let n = max as usize;
        let flen = match len_i32(fds) { Some(v) => v, None => return -22 };
        let rlen = match len_i32(revents) { Some(v) => v, None => return -22 };
        if n > flen || n > rlen { return -22; }
        let s = unsafe { &mut *(handle as *mut PollState) };
        let count = if s.fds.len() < n { s.fds.len() } else { n };
        let mut pfds: Vec<PollFd> = Vec::with_capacity(count);
        for i in 0..count {
            pfds.push(PollFd { fd: s.fds[i], events: s.events[i] as i16, revents: 0 });
        }
        let rc = unsafe { poll(pfds.as_mut_ptr(), count as u64, timeout_ms) };
        if rc < 0 { return -(std::io::Error::last_os_error().raw_os_error().unwrap_or(5) as i32); }
        for i in 0..(rc as usize) {
            fill_output(fds, i, pfds[i].fd);
            fill_output(revents, i, pfds[i].revents as i32);
        }
        rc
    }

    #[no_mangle]
    pub extern "C" fn mylis_event_close(handle: i64) -> i64 {
        if handle <= 0 { return 0; }
        unsafe { drop(Box::from_raw(handle as *mut PollState)); }
        0
    }
}

#[cfg(not(unix))]
mod unsupported {
    #[no_mangle]
    pub extern "C" fn mylis_event_new() -> i64 { -38 }
    #[no_mangle]
    pub extern "C" fn mylis_event_add(_handle: i64, _fd: i32) -> i64 { -38 }
    #[no_mangle]
    pub extern "C" fn mylis_event_modify(_handle: i64, _fd: i32, _write_enabled: i32) -> i64 { -38 }
    #[no_mangle]
    pub extern "C" fn mylis_event_remove(_handle: i64, _fd: i32) -> i64 { -38 }
    #[no_mangle]
    pub extern "C" fn mylis_event_wait(_handle: i64, _timeout_ms: i32, _fds: *mut i32, _revents: *mut i32, _max: i32) -> i32 { -38 }
    #[no_mangle]
    pub extern "C" fn mylis_event_close(_handle: i64) -> i64 { 0 }
}
