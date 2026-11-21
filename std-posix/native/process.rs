//  Synchronous process backend for POSIX and Windows hosts.
//
//  `run` mirrors Lua's `os.execute`: one shell command runs to completion,
//  its stdout/stderr are captured, and the exit status is reported. `spawn`
//  returns an opaque handle for pid/wait/kill/close; `popen` exposes a one-way
//  byte pipe. No Mylis object is shared with the child; all data crossing the
//  boundary is copied.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;
#[cfg(windows)]
use std::time::Duration;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

struct LastRun {
    exit_code: i64,
    signal: i64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl LastRun {
    const fn empty() -> Self {
        Self {
            exit_code: -1,
            signal: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }
}

static LAST: Mutex<LastRun> = Mutex::new(LastRun::empty());
static CHILDREN: Mutex<Vec<Option<Child>>> = Mutex::new(Vec::new());
static LAST_WAIT_CODE: Mutex<i64> = Mutex::new(-1);
static LAST_WAIT_SIGNAL: Mutex<i64> = Mutex::new(0);

#[cfg(windows)]
fn win_error_code(raw: u32) -> i64 {
    match raw {
        2 | 3 => 2,
        5 | 32 | 33 => 13,
        80 | 183 => 17,
        87 | 123 | 206 => 22,
        109 | 232 => 32,
        995 | 258 => 110,
        _ => 5,
    }
}

fn error_code(error: std::io::Error) -> i64 {
    #[cfg(windows)]
    {
        -win_error_code(error.raw_os_error().unwrap_or(5) as u32)
    }
    #[cfg(not(windows))]
    {
        -(error.raw_os_error().unwrap_or(22) as i64)
    }
}

struct PipeEntry {
    child: Child,
    input: Option<ChildStdin>,
    output: Option<ChildStdout>,
    line_buffer: Vec<u8>,
    last_read: Vec<u8>,
    mode: u8, // 1 = read stdout, 2 = write stdin
}

#[cfg(unix)]
mod poller {
    use std::os::raw::{c_int, c_short};

    #[repr(C)]
    pub struct PollFd {
        pub fd: c_int,
        pub events: c_short,
        pub revents: c_short,
    }

    extern "C" {
        pub fn poll(fds: *mut PollFd, nfds: usize, timeout: c_int) -> c_int;
    }
}

#[cfg(unix)]
fn wait_event(fd: i32, events: i16, timeout_ms: i64) -> Result<bool, i64> {
    let mut pfd = poller::PollFd {
        fd,
        events: events as std::os::raw::c_short,
        revents: 0,
    };
    let timeout = if timeout_ms < 0 {
        -1
    } else if timeout_ms > i32::MAX as i64 {
        i32::MAX
    } else {
        timeout_ms as i32
    };
    loop {
        let rc = unsafe { poller::poll(&mut pfd, 1, timeout) };
        if rc >= 0 {
            return Ok(rc > 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        return Err(error_code(error));
    }
}

#[cfg(unix)]
fn wait_readable(fd: i64, timeout_ms: i64) -> Result<bool, i64> {
    wait_event(fd as i32, 1, timeout_ms) // POLLIN
}

enum ReadWait {
    Ready,
    Exited,
    Timeout,
}

#[cfg(windows)]
fn child_exited(child: &mut Child) -> Result<bool, i64> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    #[link(name = "kernel32")]
    extern "system" {
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn GetLastError() -> u32;
    }
    let handle = child.as_raw_handle() as *mut c_void;
    match unsafe { WaitForSingleObject(handle, 0) } {
        0 => Ok(true),      // WAIT_OBJECT_0
        258 => Ok(false),   // WAIT_TIMEOUT
        _ => Err(-win_error_code(unsafe { GetLastError() })),
    }
}

#[cfg(windows)]
fn wait_readable_or_exit(
    handle: i64,
    timeout_ms: i64,
    child: &mut Child,
) -> Result<ReadWait, i64> {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        fn PeekNamedPipe(
            named_pipe: *mut c_void,
            buffer: *mut c_void,
            buffer_size: u32,
            bytes_read: *mut u32,
            total_bytes_avail: *mut u32,
            bytes_left: *mut u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }
    if handle <= 0 {
        return Err(-22);
    }
    let start = Instant::now();
    loop {
        let mut available = 0u32;
        let ok = unsafe {
            PeekNamedPipe(
                handle as *mut c_void,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let raw = unsafe { GetLastError() };
            // ERROR_BROKEN_PIPE / ERROR_PIPE_NOT_CONNECTED: a subsequent read sees EOF.
            if raw == 109 || raw == 232 {
                return Ok(ReadWait::Ready);
            }
            return Err(-win_error_code(raw));
        }
        if available > 0 {
            return Ok(ReadWait::Ready);
        }
        // On Windows the pipe can stay open after the child exits. Do not turn
        // a real EOF into ETIMEDOUT just because handle inheritance kept the
        // write end alive.
        if child_exited(child)? {
            return Ok(ReadWait::Exited);
        }
        if timeout_ms >= 0 && start.elapsed().as_millis() as i64 >= timeout_ms {
            return Ok(ReadWait::Timeout);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(unix)]
fn wait_readable_or_exit(
    fd: i64,
    timeout_ms: i64,
    child: &mut Child,
) -> Result<ReadWait, i64> {
    if wait_readable(fd, timeout_ms)? {
        return Ok(ReadWait::Ready);
    }
    // POSIX pipes close reliably; the extra check only covers unusual fd leaks.
    match child.try_wait() {
        Ok(Some(_)) => Ok(ReadWait::Exited),
        Ok(None) => Ok(ReadWait::Timeout),
        Err(error) => Err(error_code(error)),
    }
}

#[cfg(not(any(unix, windows)))]
fn wait_readable_or_exit(
    _fd: i64,
    _timeout_ms: i64,
    _child: &mut Child,
) -> Result<ReadWait, i64> {
    Err(-95)
}

#[cfg(not(any(unix, windows)))]
fn wait_readable(_fd: i64, timeout_ms: i64) -> Result<bool, i64> {
    if timeout_ms >= 0 {
        return Err(-95);
    }
    Ok(true)
}

#[cfg(unix)]
fn wait_writable(fd: i64, timeout_ms: i64) -> Result<bool, i64> {
    wait_event(fd as i32, 4, timeout_ms) // POLLOUT
}

#[cfg(not(any(unix, windows)))]
fn wait_writable(_fd: i64, _timeout_ms: i64) -> Result<bool, i64> {
    Err(-95)
}

#[cfg(unix)]
fn stdout_handle(output: &ChildStdout) -> i64 {
    use std::os::unix::io::AsRawFd;
    output.as_raw_fd() as i64
}

#[cfg(windows)]
fn stdout_handle(output: &ChildStdout) -> i64 {
    output.as_raw_handle() as usize as i64
}

#[cfg(not(any(unix, windows)))]
fn stdout_handle(_output: &ChildStdout) -> i64 { 0 }

#[cfg(unix)]
fn write_with_timeout(input: &mut ChildStdin, bytes: &[u8], timeout_ms: i64) -> Result<usize, i64> {
    use std::os::unix::io::AsRawFd;
    let start = Instant::now();
    let mut written = 0usize;
    while written < bytes.len() {
        let remaining = if timeout_ms < 0 {
            -1
        } else {
            let elapsed = start.elapsed().as_millis() as i64;
            let left = timeout_ms - elapsed;
            if left < 0 {
                return Err(-110); // ETIMEDOUT
            }
            left
        };
        if !wait_writable(input.as_raw_fd() as i64, remaining)? {
            return Err(-110);
        }
        // 512 字节不超过 POSIX 保证的 PIPE_BUF，poll 报可写后不会阻塞在 write。
        let chunk = std::cmp::min(bytes.len() - written, 512);
        match input.write(&bytes[written..written + chunk]) {
            Ok(0) => return Err(-32), // EPIPE
            Ok(n) => written += n,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error_code(error)),
        }
    }
    Ok(written)
}

#[cfg(windows)]
fn write_with_timeout(input: &mut ChildStdin, bytes: &[u8], timeout_ms: i64) -> Result<usize, i64> {
    use std::ffi::c_void;
    use std::sync::mpsc;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn DuplicateHandle(
            source_process: *mut c_void,
            source: *mut c_void,
            target_process: *mut c_void,
            target: *mut *mut c_void,
            desired_access: u32,
            inherit: i32,
            options: u32,
        ) -> i32;
        fn WriteFile(
            file: *mut c_void,
            buffer: *const c_void,
            bytes_to_write: u32,
            bytes_written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
        fn CancelSynchronousIo(thread: *mut c_void) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn GetLastError() -> u32;
    }
    if timeout_ms < 0 {
        return match input.write_all(bytes) {
            Ok(()) => Ok(bytes.len()),
            Err(error) => Err(error_code(error)),
        };
    }
    if bytes.len() > u32::MAX as usize {
        return Err(-22);
    }
    let source = input.as_raw_handle() as *mut c_void;
    let mut duplicate: *mut c_void = std::ptr::null_mut();
    let ok = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            source,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            0,
            2, // DUPLICATE_SAME_ACCESS
        )
    };
    if ok == 0 {
        return Err(-win_error_code(unsafe { GetLastError() }));
    }
    let data = bytes.to_vec();
    let duplicate_value = duplicate as usize;
    let (sender, receiver) = mpsc::channel::<Result<usize, i64>>();
    let writer = std::thread::spawn(move || {
        let handle = duplicate_value as *mut c_void;
        let mut written = 0u32;
        let result = unsafe {
            WriteFile(
                handle,
                data.as_ptr() as *const c_void,
                data.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        };
        let outcome = if result != 0 {
            Ok(written as usize)
        } else {
            Err(-win_error_code(unsafe { GetLastError() }))
        };
        let _ = sender.send(outcome);
        unsafe { CloseHandle(handle) };
    });
    match receiver.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
        Ok(outcome) => {
            // The writer closes its duplicated handle before exiting; join so a
            // completed write never leaves a detached cleanup thread behind.
            let _ = writer.join();
            outcome
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Cancel the blocking WriteFile on the writer thread. Without this,
            // a timed-out pipe write can keep a duplicated handle and a thread
            // alive until the child exits. Retry briefly: the thread may not
            // have entered WriteFile yet when the first cancel is issued.
            let raw_thread = writer.as_raw_handle() as *mut c_void;
            let deadline = Instant::now() + Duration::from_millis(100);
            loop {
                unsafe {
                    CancelSynchronousIo(raw_thread);
                }
                if unsafe { WaitForSingleObject(raw_thread, 10) } == 0 {
                    let _ = writer.join();
                    break;
                }
                if Instant::now() >= deadline {
                    break;
                }
            }
            Err(-110)
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = writer.join();
            Err(-32)
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn write_with_timeout(input: &mut ChildStdin, bytes: &[u8], _timeout_ms: i64) -> Result<usize, i64> {
    match input.write_all(bytes) {
        Ok(()) => Ok(bytes.len()),
        Err(error) => Err(error_code(error)),
    }
}

enum LineOutcome {
    Line(Vec<u8>),
    Eof,
    Timeout,
}

fn take_line(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let pos = buffer.iter().position(|&byte| byte == b'\n')?;
    let mut line: Vec<u8> = buffer.drain(..=pos).collect();
    if line.ends_with(&[b'\n']) {
        line.pop();
        if line.ends_with(&[b'\r']) {
            line.pop();
        }
    }
    Some(line)
}

fn read_line_from(
    output: &mut ChildStdout,
    buffer: &mut Vec<u8>,
    timeout_ms: i64,
    child: &mut Child,
) -> Result<LineOutcome, i64> {
    if let Some(line) = take_line(buffer) {
        return Ok(LineOutcome::Line(line));
    }
    let start = Instant::now();
    loop {
        if timeout_ms >= 0 {
            let elapsed = start.elapsed().as_millis() as i64;
            let remaining = timeout_ms - elapsed;
            if remaining <= 0 {
                return Ok(LineOutcome::Timeout);
            }
            match wait_readable_or_exit(stdout_handle(output), remaining, child)? {
                ReadWait::Ready => {}
                ReadWait::Exited => {
                    if buffer.is_empty() {
                        return Ok(LineOutcome::Eof);
                    }
                    return Ok(LineOutcome::Line(std::mem::take(buffer)));
                }
                ReadWait::Timeout => return Ok(LineOutcome::Timeout),
            }
        } else {
            match wait_readable_or_exit(stdout_handle(output), -1, child)? {
                ReadWait::Ready => {}
                ReadWait::Exited => {
                    if buffer.is_empty() {
                        return Ok(LineOutcome::Eof);
                    }
                    return Ok(LineOutcome::Line(std::mem::take(buffer)));
                }
                ReadWait::Timeout => return Ok(LineOutcome::Timeout),
            }
        }
        let mut chunk = [0u8; 4096];
        match output.read(&mut chunk) {
            Ok(0) => {
                if buffer.is_empty() {
                    return Ok(LineOutcome::Eof);
                }
                return Ok(LineOutcome::Line(std::mem::take(buffer)));
            }
            Ok(n) => {
                buffer.extend_from_slice(&chunk[..n]);
                if let Some(line) = take_line(buffer) {
                    return Ok(LineOutcome::Line(line));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
                if buffer.is_empty() {
                    return Ok(LineOutcome::Eof);
                }
                return Ok(LineOutcome::Line(std::mem::take(buffer)));
            }
            Err(error) => return Err(error_code(error)),
        }
    }
}

fn read_line_entry(entry: &mut PipeEntry, timeout_ms: i64) -> Result<LineOutcome, i64> {
    let PipeEntry {
        child,
        output,
        line_buffer,
        ..
    } = entry;
    let output = match output.as_mut() {
        Some(output) => output,
        None => return Err(-22),
    };
    read_line_from(output, line_buffer, timeout_ms, child)
}

static PIPES: Mutex<Vec<Option<PipeEntry>>> = Mutex::new(Vec::new());
static LAST_PIPE_STATUS: Mutex<i64> = Mutex::new(0);

fn lock_last_pipe_status() -> std::sync::MutexGuard<'static, i64> {
    LAST_PIPE_STATUS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_pipes() -> std::sync::MutexGuard<'static, Vec<Option<PipeEntry>>> {
    PIPES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn array_len(data: *const u8) -> Option<usize> {
    if data.is_null() {
        return None;
    }
    Some(unsafe { *(data as *const u64) } as usize)
}

fn lock_children() -> std::sync::MutexGuard<'static, Vec<Option<Child>>> {
    CHILDREN.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_last_wait_code() -> std::sync::MutexGuard<'static, i64> {
    LAST_WAIT_CODE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_last_wait_signal() -> std::sync::MutexGuard<'static, i64> {
    LAST_WAIT_SIGNAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn child_index(handle: i64) -> Option<usize> {
    if handle <= 0 {
        return None;
    }
    Some((handle - 1) as usize)
}

fn lock_last() -> std::sync::MutexGuard<'static, LastRun> {
    LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cstring_bytes(bytes: &[u8]) -> *mut c_char {
    // The Mylis string ABI is NUL-terminated; strip embedded NUL bytes.
    let mut clean = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte != 0 {
            clean.push(byte);
        }
    }
    CString::new(clean)
        .unwrap_or_else(|_| CString::new("").expect("empty CString"))
        .into_raw()
}

fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

#[cfg(windows)]
fn direct_command(command: &str) -> Option<Command> {
    use std::path::Path;
    let text = command.trim();
    let executable = if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        &text[1..text.len() - 1]
    } else {
        text
    };
    if executable.is_empty()
        || executable.contains(['&', '|', '<', '>', '^', '%', '!', '*', '?'])
    {
        return None;
    }
    let path = Path::new(executable);
    if !path.is_file() {
        return None;
    }
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    if extension != "exe" {
        return None;
    }
    Some(Command::new(executable))
}

fn process_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let direct = direct_command(command);
        direct.unwrap_or_else(|| shell_command(command))
    }
    #[cfg(not(windows))]
    {
        shell_command(command)
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_run(command: *const c_char) -> i64 {
    if command.is_null() {
        return -22; // EINVAL
    }
    let text = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(text) => text,
        Err(_) => return -22,
    };
    let mut cmd = process_command(text);
    // Do not let a child consume the parent's interactive stdin.
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match cmd.output() {
        Ok(output) => {
            #[cfg(unix)]
            let signal = {
                use std::os::unix::process::ExitStatusExt;
                output.status.signal().unwrap_or(0) as i64
            };
            #[cfg(not(unix))]
            let signal = 0;
            let exit_code = output.status.code().unwrap_or(-1) as i64;
            let mut last = lock_last();
            last.exit_code = exit_code;
            last.signal = signal;
            last.stdout = output.stdout;
            last.stderr = output.stderr;
            0
        }
        Err(error) => {
            let mut last = lock_last();
            last.exit_code = -1;
            last.signal = 0;
            last.stdout.clear();
            last.stderr.clear();
            error_code(error)
        }
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_popen(command: *const c_char, mode: *const c_char) -> i64 {
    if command.is_null() || mode.is_null() {
        return -22;
    }
    let text = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(text) => text,
        Err(_) => return -22,
    };
    let mode_text = match unsafe { CStr::from_ptr(mode) }.to_str() {
        Ok(text) => text,
        Err(_) => return -22,
    };
    let (mode, mut cmd) = match mode_text {
        "r" => {
            let mut cmd = process_command(text);
            cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::inherit());
            (1u8, cmd)
        }
        "w" => {
            let mut cmd = process_command(text);
            cmd.stdin(Stdio::piped()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
            (2u8, cmd)
        }
        _ => return -22,
    };
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return error_code(error),
    };
    let input = child.stdin.take();
    let output = child.stdout.take();
    let entry = PipeEntry {
        child,
        input,
        output,
        line_buffer: Vec::new(),
        last_read: Vec::new(),
        mode,
    };
    let mut table = lock_pipes();
    if let Some(index) = table.iter().position(|slot| slot.is_none()) {
        table[index] = Some(entry);
        (index + 1) as i64
    } else {
        table.push(Some(entry));
        table.len() as i64
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_pid(handle: i64) -> i64 {
    let table = lock_pipes();
    match child_index(handle).and_then(|index| table.get(index)).and_then(|slot| slot.as_ref()) {
        Some(entry) => entry.child.id() as i64,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_read_line(handle: i64) -> *mut c_char {
    pipe_read_line_common(handle, -1)
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_read_line_timeout(handle: i64, timeout_ms: i64) -> *mut c_char {
    pipe_read_line_common(handle, timeout_ms)
}

fn pipe_read_line_common(handle: i64, timeout_ms: i64) -> *mut c_char {
    let mut table = lock_pipes();
    let index = match child_index(handle) {
        Some(index) => index,
        None => { *lock_last_pipe_status() = -9; return cstring_bytes(b""); }
    };
    let entry = match table.get_mut(index).and_then(|slot| slot.as_mut()) {
        Some(entry) => entry,
        None => { *lock_last_pipe_status() = -9; return cstring_bytes(b""); }
    };
    if entry.mode != 1 && entry.mode != 3 {
        *lock_last_pipe_status() = -22;
        return cstring_bytes(b"");
    }
    match read_line_entry(entry, timeout_ms) {
        Ok(LineOutcome::Line(line)) => {
            *lock_last_pipe_status() = 1;
            cstring_bytes(&line)
        }
        Ok(LineOutcome::Eof) => {
            *lock_last_pipe_status() = 0;
            cstring_bytes(b"")
        }
        Ok(LineOutcome::Timeout) => {
            *lock_last_pipe_status() = -110; // ETIMEDOUT
            cstring_bytes(b"")
        }
        Err(code) => {
            *lock_last_pipe_status() = code;
            cstring_bytes(b"")
        }
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_last_status() -> i64 {
    *lock_last_pipe_status()
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_read_all(handle: i64) -> i64 {
    let mut table = lock_pipes();
    let index = match child_index(handle) {
        Some(index) => index,
        None => return -9,
    };
    let entry = match table.get_mut(index).and_then(|slot| slot.as_mut()) {
        Some(entry) => entry,
        None => return -9,
    };
    if entry.mode != 1 && entry.mode != 3 {
        return -22;
    }
    let output = match entry.output.as_mut() {
        Some(output) => output,
        None => return -22,
    };
    let mut buffer = std::mem::take(&mut entry.line_buffer);
    match output.read_to_end(&mut buffer) {
        Ok(_) => {
            entry.last_read = buffer;
            entry.last_read.len() as i64
        }
        Err(error) => error_code(error),
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_read_copy(
    handle: i64,
    data: *mut u8,
    offset: i64,
    count: i64,
) -> i64 {
    if data.is_null() || offset < 0 || count < 0 {
        return -22;
    }
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start {
        return -22;
    }
    let table = lock_pipes();
    let index = match child_index(handle) { Some(index) => index, None => return -9 };
    let entry = match table.get(index).and_then(|slot| slot.as_ref()) { Some(entry) => entry, None => return -9 };
    if start > entry.last_read.len() || n > entry.last_read.len() - start {
        return -22;
    }
    let bytes = &entry.last_read[start..start + n];
    let out = unsafe { std::slice::from_raw_parts_mut(data.add(8 + start), n) };
    out.copy_from_slice(bytes);
    n as i64
}

fn pipe_write_common(
    handle: i64,
    data: *const u8,
    offset: i64,
    count: i64,
    timeout_ms: i64,
) -> i64 {
    if data.is_null() || offset < 0 || count < 0 {
        return -22;
    }
    let len = match array_len(data) { Some(n) => n, None => return -22 };
    let start = offset as usize;
    let n = count as usize;
    if start > len || n > len - start {
        return -22;
    }
    let mut table = lock_pipes();
    let index = match child_index(handle) { Some(index) => index, None => return -9 };
    let entry = match table.get_mut(index).and_then(|slot| slot.as_mut()) { Some(entry) => entry, None => return -9 };
    if entry.mode != 2 && entry.mode != 3 {
        return -22;
    }
    let input = match entry.input.as_mut() {
        Some(input) => input,
        None => return -22,
    };
    let bytes = unsafe { std::slice::from_raw_parts(data.add(8 + start), n) };
    match write_with_timeout(input, bytes, timeout_ms) {
        Ok(written) if written == n => written as i64,
        Ok(_) => {
            -110
        }
        Err(-110) => {
            -110
        }
        Err(code) => code,
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_write(
    handle: i64,
    data: *const u8,
    offset: i64,
    count: i64,
) -> i64 {
    pipe_write_common(handle, data, offset, count, -1)
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_write_timeout(
    handle: i64,
    data: *const u8,
    offset: i64,
    count: i64,
    timeout_ms: i64,
) -> i64 {
    pipe_write_common(handle, data, offset, count, timeout_ms)
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_close_input(handle: i64) -> i64 {
    let mut table = lock_pipes();
    let index = match child_index(handle) {
        Some(index) => index,
        None => return -9,
    };
    let entry = match table.get_mut(index).and_then(|slot| slot.as_mut()) {
        Some(entry) => entry,
        None => return -9,
    };
    if entry.mode != 2 && entry.mode != 3 {
        return -22;
    }
    entry.input.take();
    0
}

fn pipe_finish(handle: i64, force: bool) -> i64 {
    let mut table = lock_pipes();
    let index = match child_index(handle) {
        Some(index) => index,
        None => return -9,
    };
    if index >= table.len() {
        return -9;
    }
    let mut entry = match table[index].take() {
        Some(entry) => entry,
        None => return -9,
    };
    // Closing the pipe before wait prevents pclose-like deadlocks.
    entry.input.take();
    entry.output.take();
    if force {
        // Ignore kill errors such as ESRCH: the child may already have exited.
        let _ = entry.child.kill();
    }
    let status = match entry.child.wait() {
        Ok(status) => status,
        Err(error) => {
            *lock_last_wait_code() = -1;
            *lock_last_wait_signal() = 0;
            return error_code(error);
        }
    };
    let code = status.code().unwrap_or(-1) as i64;
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal().unwrap_or(0) as i64
    };
    #[cfg(not(unix))]
    let signal = 0;
    *lock_last_wait_code() = code;
    *lock_last_wait_signal() = signal;
    0
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_close(handle: i64) -> i64 {
    pipe_finish(handle, false)
}

#[no_mangle]
pub extern "C" fn mylis_process_pipe_terminate(handle: i64) -> i64 {
    pipe_finish(handle, true)
}

#[no_mangle]
pub extern "C" fn mylis_process_popen2(command: *const c_char) -> i64 {
    if command.is_null() {
        return -22;
    }
    let text = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(text) => text,
        Err(_) => return -22,
    };
    let mut cmd = process_command(text);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return error_code(error),
    };
    let input = child.stdin.take();
    let output = child.stdout.take();
    let entry = PipeEntry {
        child,
        input,
        output,
        line_buffer: Vec::new(),
        last_read: Vec::new(),
        mode: 3,
    };
    let mut table = lock_pipes();
    if let Some(index) = table.iter().position(|slot| slot.is_none()) {
        table[index] = Some(entry);
        (index + 1) as i64
    } else {
        table.push(Some(entry));
        table.len() as i64
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_spawn(command: *const c_char) -> i64 {
    if command.is_null() {
        return -22;
    }
    let text = match unsafe { CStr::from_ptr(command) }.to_str() {
        Ok(text) => text,
        Err(_) => return -22,
    };
    let mut cmd = process_command(text);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => return error_code(error),
    };
    let mut table = lock_children();
    if let Some(index) = table.iter().position(|slot| slot.is_none()) {
        table[index] = Some(child);
        (index + 1) as i64
    } else {
        table.push(Some(child));
        table.len() as i64
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_pid(handle: i64) -> i64 {
    let table = lock_children();
    match child_index(handle).and_then(|index| table.get(index)).and_then(|slot| slot.as_ref()) {
        Some(child) => child.id() as i64,
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_wait(handle: i64) -> i64 {
    let mut table = lock_children();
    let index = match child_index(handle) {
        Some(index) => index,
        None => return -9,
    };
    let mut child = match table.get_mut(index).and_then(|slot| slot.take()) {
        Some(child) => child,
        None => return -9,
    };
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            *lock_last_wait_code() = -1;
            *lock_last_wait_signal() = 0;
            return error_code(error);
        }
    };
    let code = status.code().unwrap_or(-1) as i64;
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal().unwrap_or(0) as i64
    };
    #[cfg(not(unix))]
    let signal = 0;
    *lock_last_wait_code() = code;
    *lock_last_wait_signal() = signal;
    0
}

#[no_mangle]
pub extern "C" fn mylis_process_last_wait_exit_code() -> i64 {
    *lock_last_wait_code()
}

#[no_mangle]
pub extern "C" fn mylis_process_last_wait_signal() -> i64 {
    *lock_last_wait_signal()
}

#[no_mangle]
pub extern "C" fn mylis_process_kill(handle: i64) -> i64 {
    let mut table = lock_children();
    let index = match child_index(handle) {
        Some(index) => index,
        None => return -9,
    };
    match table.get_mut(index).and_then(|slot| slot.as_mut()) {
        Some(child) => match child.kill() {
            Ok(()) => 0,
            Err(error) => error_code(error),
        },
        None => -9,
    }
}

#[no_mangle]
pub extern "C" fn mylis_process_close(handle: i64) -> i64 {
    let mut table = lock_children();
    let index = match child_index(handle) {
        Some(index) => index,
        None => return -9,
    };
    if index >= table.len() {
        return -9;
    }
    if table[index].is_some() {
        return -16; // EBUSY: wait or kill+wait first
    }
    table[index] = None;
    0
}

#[no_mangle]
pub extern "C" fn mylis_process_last_exit_code() -> i64 {
    lock_last().exit_code
}

#[no_mangle]
pub extern "C" fn mylis_process_last_signal() -> i64 {
    lock_last().signal
}

#[no_mangle]
pub extern "C" fn mylis_process_last_stdout() -> *mut c_char {
    cstring_bytes(&lock_last().stdout)
}

#[no_mangle]
pub extern "C" fn mylis_process_last_stderr() -> *mut c_char {
    cstring_bytes(&lock_last().stderr)
}

#[no_mangle]
pub extern "C" fn mylis_process_free_string(text: *mut c_char) {
    if !text.is_null() {
        unsafe { drop(CString::from_raw(text)); }
    }
}
