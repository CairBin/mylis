use super::*;
use super::api::*;
use super::heap::*;
use super::mark::*;
use super::page::*;
use super::roots::*;
use super::sys::*;

/// GC 统计（供诊断/测试）：collections、live_bytes、total_allocated、threshold
#[repr(C)]
pub struct GcStats {
    pub collections: usize,
    pub live_bytes: usize,
    pub total_allocated: usize,
    pub threshold: usize,
    pub max_mark_cycles: usize,
    pub max_sweep_cycles: usize,
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_stats(out: *mut GcStats) {
    if out.is_null() { return; }
    unsafe {
        (*out).collections = COLLECTIONS;
        (*out).live_bytes = LIVE_BYTES;
        (*out).total_allocated = TOTAL_BYTES;
        (*out).threshold = THRESHOLD;
        (*out).max_mark_cycles = MAX_MARK_CYCLES;
        (*out).max_sweep_cycles = MAX_SWEEP_CYCLES;
    }
}