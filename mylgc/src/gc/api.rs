use super::*;
use super::heap::*;
use super::mark::*;
use super::page::*;
use super::roots::*;
use super::stats::*;
use super::sys::*;

#[no_mangle]
pub extern "C" fn myrt_gc_backend_init() {
    unsafe {
        if INITED {
            return;   /* 幂等：重复调用不得清空堆或根集合 */
        }
        INITED = true;
        HEAP = ptr::null_mut();
        NROOTS = 0;
        STACK_BASE = stack_top();
        if STACK_BASE == 0 {
            let local = 0u8;
            STACK_BASE = &local as *const u8 as usize;
        }
    }
}

#[inline]
#[no_mangle]
pub extern "C" fn myrt_gc_backend_set_threshold(bytes: usize) {
    unsafe {
        THRESHOLD = bytes;
        NEXT_LIMIT = bytes;
        ALLOC_SINCE = 0;
    }
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_collect() {
    unsafe { collect_now(true); }
}

/// 安全点轮询：只在达到阈值时做一次增量标记；完整回收留给显式 collect。
#[no_mangle]
pub extern "C" fn myrt_gc_backend_safepoint() {
    unsafe {
        if THRESHOLD != 0 && NEXT_LIMIT != 0 && ALLOC_SINCE >= NEXT_LIMIT && !HEAP.is_null() {
            collect_now(false);
        }
    }
}