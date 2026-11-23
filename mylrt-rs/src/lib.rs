//! Mylis 运行时（Rust 实现）。
//!
//! C ABI 与 `myrt/mylrt.h` 完全一致；内部用 `Runtime` / `Gc` trait 分层。
//! 通过 `MYLRT_IMPL_RS` 选择实现文件：默认 `impls/own.rs`，也可指向第三方
//! freestanding 实现目录里的 `lib.rs`。

#![allow(non_snake_case)]
#![cfg_attr(not(feature = "std"), no_std)]

pub mod gc;
pub mod runtime;
mod abi;
mod plat;
mod selected;

pub use abi::MyStr;
pub use gc::Gc;
pub use runtime::Runtime;

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "small-roots")]
const ROOT_CAP: usize = 512;
#[cfg(not(feature = "small-roots"))]
const ROOT_CAP: usize = 65536;

struct Roots { data: [usize; ROOT_CAP], len: usize }
impl Roots { const fn new() -> Roots { Roots { data: [0; ROOT_CAP], len: 0 } } }

struct SpinLock<T> { locked: AtomicBool, data: UnsafeCell<T> }
unsafe impl<T: Send> Sync for SpinLock<T> {}
impl<T> SpinLock<T> {
    const fn new(data: T) -> SpinLock<T> { SpinLock { locked: AtomicBool::new(false), data: UnsafeCell::new(data) } }
    fn lock(&self) -> SpinGuard<'_, T> {
        while self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        SpinGuard { lock: self }
    }
}
struct SpinGuard<'a, T> { lock: &'a SpinLock<T> }
impl<T> Deref for SpinGuard<'_, T> { type Target = T; fn deref(&self) -> &T { unsafe { &*self.lock.data.get() } } }
impl<T> DerefMut for SpinGuard<'_, T> { fn deref_mut(&mut self) -> &mut T { unsafe { &mut *self.lock.data.get() } } }
impl<T> Drop for SpinGuard<'_, T> { fn drop(&mut self) { self.lock.locked.store(false, Ordering::Release); } }

static ROOTS: SpinLock<Roots> = SpinLock::new(Roots::new());

fn rt() -> &'static dyn Runtime { selected::runtime() }

#[cfg(not(feature = "std"))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(all(not(feature = "std"), not(feature = "no-panic-handler")))]
#[panic_handler]
fn mylrt_panic_handler(_info: &core::panic::PanicInfo) -> ! {
    plat::platform().write_stderr(b"mylrt: internal panic\n");
    plat::platform().abort()
}

#[no_mangle]
pub extern "C" fn myrt_init() {
    rt().gc().init();
    if let Some(n) = plat::platform().gc_threshold() { rt().gc().set_threshold(n); }
    #[cfg(feature = "std")]
    {
        let precise = std::env::var_os("MYLIS_GC_PRECISE")
            .map(|v| v != "0" && v != "false")
            .unwrap_or(false);
        rt().gc().set_precise(precise);
    }
    let mut roots = ROOTS.lock();
    rt().gc().add_root_range(roots.data.as_mut_ptr() as *mut u8, ROOT_CAP * core::mem::size_of::<usize>());
}

#[no_mangle]
pub extern "C" fn myrt_gc_collect() { rt().gc().collect(); }

#[no_mangle]
pub extern "C" fn myrt_gc_safepoint() { rt().gc().safepoint(); }

#[no_mangle]
pub extern "C" fn myrt_gc_add_root_range(start: *mut u8, bytes: u64) {
    rt().gc().add_root_range(start, bytes as usize);
}

#[no_mangle]
pub extern "C" fn myrt_gc_remove_root_range(start: *mut u8) {
    rt().gc().remove_root_range(start);
}

#[repr(C)]
pub struct MyrtGcRootDesc { pub start: *mut u8, pub bytes: u64 }

#[no_mangle]
pub extern "C" fn myrt_gc_frame_enter() -> u64 { rt().gc().frame_enter() }

#[no_mangle]
pub extern "C" fn myrt_gc_frame_set_roots(id: u64, entries: *const MyrtGcRootDesc, count: u64) {
    rt().gc().frame_set_roots(id, entries as *const u8, count as usize);
}

#[no_mangle]
pub extern "C" fn myrt_gc_frame_exit(id: u64) { rt().gc().frame_exit(id); }

#[no_mangle]
pub extern "C" fn myrt_gc_static_set_roots(entries: *const u8, count: u64) {
    rt().gc().static_set_roots(entries, count as usize);
}

#[no_mangle]
pub extern "C" fn myrt_gc_set_precise(on: i32) {
    rt().gc().set_precise(on != 0);
}

#[no_mangle]
pub extern "C" fn myrt_gc_write_barrier(value: *mut u8) { rt().gc().write_barrier(value); }

#[no_mangle]
pub extern "C" fn myrt_gc_write_barrier_owner(owner: *mut u8, value: *mut u8) {
    rt().gc().write_barrier_owner(owner, value);
}

#[no_mangle]
pub extern "C" fn myrt_gc_stats_dump() { rt().stats_dump(); }

#[no_mangle]
pub extern "C" fn myrt_alloc(size: u64) -> *mut u8 { rt().alloc(size as usize) }

#[no_mangle]
pub extern "C" fn myrt_register_root(obj: *mut u8) {
    if obj.is_null() { return; }
    let mut roots = ROOTS.lock();
    if roots.len < ROOT_CAP { let i = roots.len; roots.data[i] = obj as usize; roots.len = i + 1; }
}

#[no_mangle]
pub extern "C" fn myrt_unregister_root(obj: *mut u8) {
    if obj.is_null() { return; }
    let mut roots = ROOTS.lock();
    if let Some(i) = roots.data[..roots.len].iter().position(|p| *p == obj as usize) {
        roots.len -= 1;
        let last = roots.data[roots.len];
        roots.data[i] = last;
    }
}

#[no_mangle]
pub extern "C" fn myrt_str_cstr(p: *mut u8) -> *mut MyStr {
    if p.is_null() { return mylrt_str_lit(b"\0".as_ptr(), 0); }
    let mut n = 0usize;
    unsafe { while *p.add(n) != 0 { n += 1; } }
    mylrt_str_from_buf(p, n as u64)
}

#[no_mangle]
pub extern "C" fn myrt_str_ptr(s: *mut MyStr) -> *mut u8 {
    if s.is_null() { return b"\0".as_ptr() as *mut u8; }
    unsafe { (*s).data }
}

#[no_mangle]
pub extern "C" fn myrt_str_len(s: *mut MyStr) -> u64 {
    if s.is_null() { 0 } else { unsafe { (*s).len } }
}

#[no_mangle]
pub extern "C" fn myrt_print(s: *mut MyStr) { rt().print(s); }

#[no_mangle]
pub extern "C" fn myrt_println(s: *mut MyStr) { rt().println(s); }

#[no_mangle]
pub extern "C" fn myrt_panic(msg: *mut MyStr) -> ! { rt().panic(msg) }

#[no_mangle]
pub extern "C" fn mylrt_panic_msg(what: *const u8) -> ! {
    rt().panic_text(if what.is_null() { "" } else { unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(what, c_strlen(what))) } })
}

#[no_mangle]
pub extern "C" fn mylrt_panic_unwrap() -> ! {
    rt().panic_text("unwrap() called on a failed value")
}

#[no_mangle]
pub extern "C" fn mylrt_assert(cond: i32, what: *const u8) {
    if cond == 0 {
        let s = if what.is_null() { "" } else { unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(what, c_strlen(what))) } };
        rt().panic_text(s);
    }
}

#[no_mangle]
pub extern "C" fn mylrt_str_from_buf(s: *const u8, len: u64) -> *mut MyStr { rt().str_from_buf(s, len as usize) }

#[no_mangle]
pub extern "C" fn mylrt_str_lit(s: *const u8, len: u64) -> *mut MyStr { rt().str_lit(s, len as usize) }

#[no_mangle]
pub extern "C" fn mylrt_str_concat(a: *mut MyStr, b: *mut MyStr) -> *mut MyStr { rt().str_concat(a, b) }

#[no_mangle]
pub extern "C" fn mylrt_str_eq(a: *mut MyStr, b: *mut MyStr) -> i32 { rt().str_eq(a, b) as i32 }

#[no_mangle]
pub extern "C" fn mylrt_str_cmp(a: *mut MyStr, b: *mut MyStr) -> i32 { rt().str_cmp(a, b) }

#[no_mangle]
pub extern "C" fn mylrt_hash_u64(x: u64) -> u64 {
    let mut v = x;
    v ^= v >> 30;
    v = v.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94d0_49bb_1331_11eb);
    v ^= v >> 31;
    v
}

#[no_mangle]
pub extern "C" fn mylrt_hash_f64(x: f64) -> u64 {
    mylrt_hash_u64(x.to_bits())
}

#[no_mangle]
pub extern "C" fn mylrt_str_hash(a: *mut MyStr) -> u64 {
    if a.is_null() { return 0; }
    unsafe {
        let bytes = core::slice::from_raw_parts((*a).data, (*a).len as usize);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
}

#[no_mangle]
pub extern "C" fn mylrt_arr_alloc(elem_size: u64, len: u64) -> *mut u8 { rt().arr_alloc(elem_size as usize, len as usize) }

#[no_mangle]
pub extern "C" fn mylrt_arr_index(len: u64, idx: i64) -> u64 { rt().arr_index(len as usize, idx) as u64 }

#[no_mangle]
pub extern "C" fn myrt_i64_to_string(v: i64) -> *mut MyStr { rt().i64_to_string(v) }
#[no_mangle]
pub extern "C" fn myrt_u64_to_string(v: u64) -> *mut MyStr { rt().u64_to_string(v) }
#[no_mangle]
pub extern "C" fn myrt_f64_to_string(v: f64) -> *mut MyStr { rt().f64_to_string(v) }
#[no_mangle]
pub extern "C" fn myrt_f64_to_string_prec(v: f64, precision: i32) -> *mut MyStr {
    rt().f64_to_string_prec(v, precision)
}
#[no_mangle]
pub extern "C" fn myrt_f64_to_string_fmt(v: f64, width: i32, precision: i32, flags: i32) -> *mut MyStr {
    rt().f64_to_string_fmt(v, width, precision, flags)
}
#[no_mangle]
pub extern "C" fn mylrt_str_starts_with(s: *mut MyStr, prefix: *mut MyStr) -> i32 {
    if rt().str_starts_with(s, prefix) { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn mylrt_str_ends_with(s: *mut MyStr, suffix: *mut MyStr) -> i32 {
    if rt().str_ends_with(s, suffix) { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn mylrt_str_index_of(s: *mut MyStr, needle: *mut MyStr) -> i64 {
    rt().str_index_of(s, needle)
}
#[no_mangle]
pub extern "C" fn mylrt_str_contains(s: *mut MyStr, needle: *mut MyStr) -> i32 {
    if rt().str_index_of(s, needle) >= 0 { 1 } else { 0 }
}
#[no_mangle]
pub extern "C" fn mylrt_str_substring(s: *mut MyStr, start: u64, length: u64) -> *mut MyStr {
    rt().str_substring(s, start, length)
}
#[no_mangle]
pub extern "C" fn mylrt_str_trim(s: *mut MyStr) -> *mut MyStr { rt().str_trim(s) }
#[no_mangle]
pub extern "C" fn mylrt_str_repeat(s: *mut MyStr, count: u64) -> *mut MyStr {
    rt().str_repeat(s, count)
}
#[no_mangle]
pub extern "C" fn mylrt_str_replace(s: *mut MyStr, from: *mut MyStr, to: *mut MyStr) -> *mut MyStr {
    rt().str_replace(s, from, to)
}
#[no_mangle]
pub extern "C" fn mylrt_str_char_len(s: *mut MyStr) -> u64 { rt().str_char_len(s) }
#[no_mangle]
pub extern "C" fn mylrt_str_char_index_of(s: *mut MyStr, needle: *mut MyStr) -> i64 {
    rt().str_char_index_of(s, needle)
}
#[no_mangle]
pub extern "C" fn mylrt_str_char_substring(s: *mut MyStr, start: u64, count: u64) -> *mut MyStr {
    rt().str_char_substring(s, start, count)
}
#[no_mangle]
pub extern "C" fn mylrt_str_char_at(s: *mut MyStr, index: u64) -> *mut MyStr {
    rt().str_char_at(s, index)
}
#[no_mangle]
pub extern "C" fn mylrt_str_char_code_at(s: *mut MyStr, index: u64) -> u32 {
    rt().str_char_code_at(s, index)
}
#[no_mangle]
pub extern "C" fn mylrt_str_to_bytes(s: *mut MyStr) -> *mut u8 { rt().str_to_bytes(s) }
#[no_mangle]
pub extern "C" fn mylrt_str_from_bytes(arr: *mut u8) -> *mut MyStr { rt().str_from_bytes(arr) }
#[no_mangle]
pub extern "C" fn myrt_clock() -> f64 { rt().clock() }

fn c_strlen(p: *const u8) -> usize {
    unsafe { let mut n = 0; while *p.add(n) != 0 { n += 1; } n }
}
