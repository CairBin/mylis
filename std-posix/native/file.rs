// Mylis std 文件后端（Rust -> C ABI）。
//
// 这个库只负责文件句柄和原始字节读写，不分配 Mylis GC 对象，也不调用 Mylis 代码。
// `u8[]` 参数在 Mylis ABI 里是 { u64 len; u8 items[]; } 的指针；这里按首 8 字节取长度，
// 数据区从 +8 开始。

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::raw::c_char;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;

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

// ---- 异步文件：固定 Rust 工作线程 + 完成队列 ----
//
// 工作线程只做 std::fs 阻塞 I/O，完成结果放在 Rust 堆里；Mylis 主线程通过
// poll/take 把字节拷回自己的 u8[]。工作线程不接触 Mylis GC 对象，也不调用
// Mylis 代码。

const FILE_ASYNC_READ: i64 = 1;
const FILE_ASYNC_WRITE: i64 = 2;
const FILE_ASYNC_CANCELED: i64 = 125;

struct AsyncJob {
    id: i64,
    kind: i64,
    path: String,
    data: Vec<u8>,
}

struct AsyncDone {
    id: i64,
    kind: i64,
    status: i64,
    len: i64,
    data: Vec<u8>,
}

struct AsyncPool {
    jobs: Mutex<VecDeque<AsyncJob>>,
    job_cv: Condvar,
    done: Mutex<VecDeque<AsyncDone>>,
    pending: Mutex<HashMap<i64, AsyncDone>>,
    canceled: Mutex<HashSet<i64>>,
    next_id: AtomicU64,
}

static ASYNC_POOL: OnceLock<Arc<AsyncPool>> = OnceLock::new();

fn async_thread_count() -> usize {
    let requested = std::env::var("MYLIS_ASYNC_FILE_THREADS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2);
    if requested == 0 {
        1
    } else if requested > 8 {
        8
    } else {
        requested
    }
}

fn async_worker(pool: Arc<AsyncPool>) {
    loop {
        let job = {
            let mut jobs = match pool.jobs.lock() {
                Ok(guard) => guard,
                Err(_) => return,
            };
            loop {
                if let Some(job) = jobs.pop_front() {
                    break job;
                }
                jobs = match pool.job_cv.wait(jobs) {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
            }
        };

        let (status, len, data) = match job.kind {
            FILE_ASYNC_READ => match std::fs::read(&job.path) {
                Ok(bytes) => {
                    let length = bytes.len() as i64;
                    (0, length, bytes)
                }
                Err(e) => (errno(e), 0, Vec::new()),
            },
            FILE_ASYNC_WRITE => {
                let length = job.data.len() as i64;
                match std::fs::write(&job.path, &job.data) {
                    Ok(()) => (0, length, Vec::new()),
                    Err(e) => (errno(e), 0, Vec::new()),
                }
            }
            _ => (-22, 0, Vec::new()),
        };

        let canceled = match pool.canceled.lock() {
            Ok(mut flags) => flags.remove(&job.id),
            Err(_) => false,
        };
        let (status, len, data) = if canceled {
            (-FILE_ASYNC_CANCELED, 0, Vec::new())
        } else {
            (status, len, data)
        };

        if let Ok(mut done) = pool.done.lock() {
            done.push_back(AsyncDone {
                id: job.id,
                kind: job.kind,
                status,
                len,
                data,
            });
        }
    }
}

fn async_pool() -> &'static Arc<AsyncPool> {
    ASYNC_POOL.get_or_init(|| {
        let pool = Arc::new(AsyncPool {
            jobs: Mutex::new(VecDeque::new()),
            job_cv: Condvar::new(),
            done: Mutex::new(VecDeque::new()),
            pending: Mutex::new(HashMap::new()),
            canceled: Mutex::new(HashSet::new()),
            next_id: AtomicU64::new(1),
        });
        let count = async_thread_count();
        for _ in 0..count {
            let worker = Arc::clone(&pool);
            // staticlib 中的后台线程随进程退出；这里不保留 JoinHandle。
            let _ = thread::Builder::new()
                .name("mylis-file-async".to_string())
                .spawn(move || async_worker(worker));
        }
        pool
    })
}

fn async_submit(kind: i64, path: String, data: Vec<u8>) -> i64 {
    let pool = async_pool();
    let id = pool.next_id.fetch_add(1, Ordering::Relaxed) as i64;
    if id <= 0 {
        return -5;
    }
    match pool.jobs.lock() {
        Ok(mut jobs) => {
            jobs.push_back(AsyncJob { id, kind, path, data });
            pool.job_cv.notify_one();
            id
        }
        Err(_) => -5,
    }
}

fn array_len_i64(data: *const i64) -> Option<usize> {
    if data.is_null() {
        return None;
    }
    Some(unsafe { *(data as *const u64) } as usize)
}

#[no_mangle]
pub extern "C" fn mylis_file_read_all_bytes_async(path: *const c_char) -> i64 {
    let p = match path_str(path) {
        Ok(value) => value.to_string(),
        Err(code) => return code,
    };
    async_submit(FILE_ASYNC_READ, p, Vec::new())
}

#[no_mangle]
pub extern "C" fn mylis_file_write_all_bytes_async(path: *const c_char, data: *const u8) -> i64 {
    let p = match path_str(path) {
        Ok(value) => value.to_string(),
        Err(code) => return code,
    };
    let len = match array_len(data) {
        Some(value) => value,
        None => return -22,
    };
    let bytes = if len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data.add(8), len).to_vec() }
    };
    async_submit(FILE_ASYNC_WRITE, p, bytes)
}

#[no_mangle]
pub extern "C" fn mylis_file_async_cancel(id: i64) -> i64 {
    if id <= 0 {
        return -22;
    }
    match async_pool().canceled.lock() {
        Ok(mut flags) => {
            flags.insert(id);
            0
        }
        Err(_) => -5,
    }
}

#[no_mangle]
pub extern "C" fn mylis_file_async_poll(meta: *mut i64) -> i64 {
    let meta_len = match array_len_i64(meta) {
        Some(value) => value,
        None => return -22,
    };
    if meta_len < 4 {
        return -22;
    }

    let done = match async_pool().done.lock() {
        Ok(mut queue) => queue.pop_front(),
        Err(_) => return -5,
    };
    let done = match done {
        Some(value) => value,
        None => return 0,
    };

    let id = done.id;
    let kind = done.kind;
    let status = done.status;
    let len = done.len;
    if let Ok(mut flags) = async_pool().canceled.lock() {
        flags.remove(&id);
    }
    if let Ok(mut pending) = async_pool().pending.lock() {
        pending.insert(id, done);
    } else {
        return -5;
    }
    unsafe {
        *(meta.add(1)) = id;
        *(meta.add(2)) = kind;
        *(meta.add(3)) = status;
        *(meta.add(4)) = len;
    }
    1
}

#[no_mangle]
pub extern "C" fn mylis_file_async_take(id: i64, data: *mut u8) -> i64 {
    if id <= 0 {
        return -22;
    }
    let out_len = if data.is_null() {
        0
    } else {
        match array_len(data) {
            Some(value) => value,
            None => return -22,
        }
    };
    let done = match async_pool().pending.lock() {
        Ok(mut pending) => pending.remove(&id),
        Err(_) => return -5,
    };
    let done = match done {
        Some(value) => value,
        None => return -22,
    };
    if done.data.len() > out_len {
        return -22;
    }
    if !done.data.is_empty() {
        if data.is_null() {
            return -22;
        }
        unsafe {
            ptr::copy_nonoverlapping(done.data.as_ptr(), data.add(8), done.data.len());
        }
    }
    0
}
