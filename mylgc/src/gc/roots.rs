use super::*;
use super::api::*;
use super::heap::*;
use super::mark::*;
use super::page::*;
use super::stats::*;
use super::sys::*;

pub(crate) unsafe fn ensure_root_cap(need: usize) -> bool {
    if ROOT_CAP >= need { return true; }
    let new_cap = core::cmp::max(ROOT_MIN, core::cmp::max(need, ROOT_CAP.saturating_mul(2)));
    let raw = sys_alloc(new_cap * core::mem::size_of::<Root>()) as *mut Root;
    if raw.is_null() { return false; }
    for i in 0..NROOTS { ptr::write(raw.add(i), ptr::read(ROOTS.add(i))); }
    if !ROOTS.is_null() { sys_free(ROOTS as *mut u8); }
    ROOTS = raw;
    ROOT_CAP = new_cap;
    true
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_add_root_range(start: *mut u8, bytes: usize) {
    unsafe {
        if ensure_root_cap(NROOTS + 1) {
            ptr::write(ROOTS.add(NROOTS), Root { start, bytes });
            NROOTS += 1;
        }
    }
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_remove_root_range(start: *mut u8) {
    unsafe {
        for i in 0..NROOTS {
            if ptr::read(ROOTS.add(i)).start == start {
                NROOTS -= 1;
                ptr::write(ROOTS.add(i), ptr::read(ROOTS.add(NROOTS)));
                return;
            }
        }
    }
}

/// 切换精确根模式：为 true 时只扫描显式根区间，不扫描线程栈。
#[no_mangle]
pub extern "C" fn myrt_gc_backend_set_precise(on: i32) {
    unsafe { PRECISE = on != 0; }
}

/// 批量注册根区间（codegen 可把所有静态字段槽一次传进来，降低启动开销）
#[repr(C)]
pub struct MylRootRange { pub start: *mut u8, pub bytes: usize }

#[no_mangle]
pub extern "C" fn myrt_gc_backend_add_root_ranges(ranges: *const MylRootRange, n: usize) -> usize {
    let mut added = 0;
    unsafe {
        if !ensure_root_cap(NROOTS + n) { return 0; }
        for i in 0..n {
            let r = &*ranges.add(i);
            ptr::write(ROOTS.add(NROOTS), Root { start: r.start, bytes: r.bytes });
            NROOTS += 1;
            added += 1;
        }
    }
    added
}
/* ---- 每帧根表：安全点只扫描当前帧的活跃根快照 ---- */

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FrameRootDesc {
    pub start: *mut u8,
    pub bytes: u64,
}

#[derive(Clone, Copy)]
pub struct FrameRoots {
    pub id: u64,
    pub entries: *const FrameRootDesc,
    pub count: usize,
    pub active: bool,
}

pub(crate) const FRAME_CAP: usize = 256;
pub(crate) const EMPTY_FRAME: FrameRoots = FrameRoots {
    id: 0,
    entries: core::ptr::null(),
    count: 0,
    active: false,
};
pub(crate) static mut FRAMES: [FrameRoots; FRAME_CAP] = [EMPTY_FRAME; FRAME_CAP];
static mut NEXT_FRAME_ID: u64 = 1;
/// 静态字段根表（全局、长期有效），与帧表分开避免和调用栈帧 ID 混用。
pub(crate) static mut STATIC_ENTRIES: *const FrameRootDesc = core::ptr::null();
pub(crate) static mut STATIC_COUNT: usize = 0;

#[no_mangle]
pub extern "C" fn myrt_gc_backend_frame_enter() -> u64 {
    unsafe {
        for i in 0..FRAME_CAP {
            if !FRAMES[i].active {
                let id = NEXT_FRAME_ID;
                NEXT_FRAME_ID = NEXT_FRAME_ID.wrapping_add(1);
                FRAMES[i] = FrameRoots { id, entries: core::ptr::null(), count: 0, active: true };
                return id;
            }
        }
        0
    }
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_frame_set_roots(id: u64, entries: *const u8, count: u64) {
    if id == 0 { return; }
    unsafe {
        for i in 0..FRAME_CAP {
            if FRAMES[i].active && FRAMES[i].id == id {
                FRAMES[i].entries = entries as *const FrameRootDesc;
                FRAMES[i].count = count as usize;
                return;
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_frame_exit(id: u64) {
    if id == 0 { return; }
    unsafe {
        for i in 0..FRAME_CAP {
            if FRAMES[i].active && FRAMES[i].id == id {
                FRAMES[i] = EMPTY_FRAME;
                return;
            }
        }
    }
}


#[no_mangle]
pub extern "C" fn myrt_gc_backend_static_set_roots(entries: *const u8, count: u64) {
    unsafe {
        STATIC_ENTRIES = entries as *const FrameRootDesc;
        STATIC_COUNT = count as usize;
    }
}


/* ---- 老年代 → 年轻代 remembered set ---- */
static mut REMEMBERED: *mut usize = core::ptr::null_mut();
static mut REM_CAP: usize = 0;
static mut REM_LEN: usize = 0;

pub(crate) unsafe fn remember_owner(owner: usize) {
    if owner == 0 { return; }
    for i in 0..REM_LEN {
        if ptr::read(REMEMBERED.add(i)) == owner { return; }
    }
    if REM_LEN >= REM_CAP {
        let ncap = if REM_CAP == 0 { 64 } else { REM_CAP * 2 };
        let buf = sys_alloc(ncap * core::mem::size_of::<usize>()) as *mut usize;
        if buf.is_null() { return; }
        if !REMEMBERED.is_null() {
            ptr::copy_nonoverlapping(REMEMBERED, buf, REM_LEN);
            sys_free(REMEMBERED as *mut u8);
        }
        REMEMBERED = buf;
        REM_CAP = ncap;
    }
    ptr::write(REMEMBERED.add(REM_LEN), owner);
    REM_LEN += 1;
}

pub(crate) unsafe fn clear_remembered() {
    REM_LEN = 0;
}

/// 扫描 remembered set 中老年代对象的载荷，标记它们引用的年轻代对象。
pub(crate) unsafe fn scan_remembered() {
    let hdr = core::mem::size_of::<Block>();
    for i in 0..REM_LEN {
        let owner = ptr::read(REMEMBERED.add(i));
        if let Some(b) = find_block(owner) {
            if (*b).gen != 0 {
                let payload = (b as *mut u8).add(hdr) as usize;
                scan_range(payload, (*b).size, &mut WORK);
            }
        }
    }
    REM_LEN = 0;
}
