pub type AllocFn = unsafe extern "C" fn(usize) -> *mut u8;
pub type FreeFn = unsafe extern "C" fn(*mut u8);

pub trait Gc: Send + Sync {
    fn init(&self);
    fn alloc(&self, size: usize) -> *mut u8;
    fn collect(&self);
    fn add_root_range(&self, start: *mut u8, len: usize);
    fn remove_root_range(&self, _start: *mut u8) {}
    fn set_precise(&self, _on: bool) {}
    fn frame_enter(&self) -> u64 { 0 }
    fn frame_set_roots(&self, _id: u64, _entries: *const u8, _count: usize) {}
    fn frame_exit(&self, _id: u64) {}
    fn static_set_roots(&self, _entries: *const u8, _count: usize) {}
    fn write_barrier(&self, value: *mut u8);
    /// owner 是写入位置所在的对象/槽地址，用于记录老年代→年轻代引用。
    fn write_barrier_owner(&self, _owner: *mut u8, _value: *mut u8) {}
    /// GC 安全点轮询：调用/分配前由编译器插入；默认 no-op，自有 GC 可在此检查阈值。
    fn safepoint(&self) {}
    fn set_threshold(&self, _bytes: usize) {}
    fn set_allocator(&self, _alloc: AllocFn, _free: FreeFn) {}
}

/// 默认 GC：委托给现有 `myrt_gc_backend_*`（mylgc / Boehm / simple）。
#[cfg(not(feature = "no-backend-gc"))]
pub struct BackendGc;

#[cfg(not(feature = "no-backend-gc"))]
extern "C" {
    fn myrt_gc_backend_init();
    fn myrt_gc_backend_alloc(n: usize) -> *mut u8;
    fn myrt_gc_backend_collect();
    fn myrt_gc_backend_safepoint();
    fn myrt_gc_backend_remove_root_range(start: *mut u8);
    fn myrt_gc_backend_set_precise(on: i32);
    fn myrt_gc_backend_frame_enter() -> u64;
    fn myrt_gc_backend_frame_set_roots(id: u64, entries: *const u8, count: u64);
    fn myrt_gc_backend_frame_exit(id: u64);
    fn myrt_gc_backend_static_set_roots(entries: *const u8, count: u64);
    fn myrt_gc_backend_add_root_range(start: *mut u8, bytes: usize);
    fn myrt_gc_backend_write_barrier(value: *mut u8);
    fn myrt_gc_backend_write_barrier_owner(owner: *mut u8, value: *mut u8);
    fn myrt_gc_backend_set_threshold(bytes: usize);
    fn myrt_gc_backend_set_allocator(a: AllocFn, f: FreeFn);
}

#[cfg(not(feature = "no-backend-gc"))]
impl Gc for BackendGc {
    fn init(&self) { unsafe { myrt_gc_backend_init() } }
    fn alloc(&self, n: usize) -> *mut u8 {
        let p = unsafe { myrt_gc_backend_alloc(n) };
        if p.is_null() {
            crate::plat::platform().write_stderr(b"mylrt: out of memory\n");
            crate::plat::platform().abort();
        }
        p
    }
    fn collect(&self) { unsafe { myrt_gc_backend_collect() } }
    fn add_root_range(&self, start: *mut u8, len: usize) { unsafe { myrt_gc_backend_add_root_range(start, len) } }
    fn remove_root_range(&self, start: *mut u8) { unsafe { myrt_gc_backend_remove_root_range(start) } }
    fn write_barrier(&self, value: *mut u8) { unsafe { myrt_gc_backend_write_barrier(value) } }
    fn write_barrier_owner(&self, owner: *mut u8, value: *mut u8) {
        unsafe { myrt_gc_backend_write_barrier_owner(owner, value) }
    }
    fn safepoint(&self) { unsafe { myrt_gc_backend_safepoint() } }
    fn set_precise(&self, on: bool) { unsafe { myrt_gc_backend_set_precise(if on { 1 } else { 0 }) } }
    fn frame_enter(&self) -> u64 { unsafe { myrt_gc_backend_frame_enter() } }
    fn frame_set_roots(&self, id: u64, entries: *const u8, count: usize) {
        unsafe { myrt_gc_backend_frame_set_roots(id, entries, count as u64) }
    }
    fn frame_exit(&self, id: u64) { unsafe { myrt_gc_backend_frame_exit(id) } }
    fn static_set_roots(&self, entries: *const u8, count: usize) {
        unsafe { myrt_gc_backend_static_set_roots(entries, count as u64) }
    }
    fn set_threshold(&self, bytes: usize) { unsafe { myrt_gc_backend_set_threshold(bytes) } }
    fn set_allocator(&self, a: AllocFn, f: FreeFn) { unsafe { myrt_gc_backend_set_allocator(a, f) } }
}
