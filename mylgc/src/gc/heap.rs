use super::*;
use super::api::*;
use super::mark::*;
use super::page::*;
use super::roots::*;
use super::stats::*;
use super::sys::*;

pub(crate) fn round_up(n: usize) -> usize { (n + 7) & !7 }

#[inline]
pub(crate) fn bin_of(cap: usize) -> Option<usize> {
    if cap <= EXACT_LIMIT {
        return Some((cap.saturating_sub(8)) / 8);
    }
    let bits = (usize::BITS - cap.leading_zeros()) as usize;
    let c = EXACT_BINS + bits.saturating_sub(13);
    if c < BIN_COUNT { Some(c) } else { None }
}

pub(crate) unsafe fn take_from_bins(need: usize) -> *mut Block {
    let start = match bin_of(need) { Some(c) => c, None => return ptr::null_mut() };
    for c in start..BIN_COUNT {
        if BINS[c].is_null() { continue; }
        if c < EXACT_BINS {
            // 精确尺寸桶：所有块容量都等于该桶尺寸，头块必然够用
            let b = BINS[c];
            BINS[c] = (*b).next;
            return b;
        }
        // 大块桶：桶内尺寸不一，沿链找第一个够大的
        let mut prev: *mut Block = ptr::null_mut();
        let mut b = BINS[c];
        while !b.is_null() {
            if (*b).cap >= need {
                if prev.is_null() { BINS[c] = (*b).next; } else { (*prev).next = (*b).next; }
                return b;
            }
            prev = b;
            b = (*b).next;
        }
    }
    ptr::null_mut()
}

/// 写屏障（Dijkstra insertion barrier）：标记期间把新存入的堆指针染黑。
/// 只对值做判断；目标对象是否是堆对象由 codegen 在堆写入点决定。
#[no_mangle]
pub extern "C" fn myrt_gc_backend_alloc(n: usize) -> *mut u8 {
    let size = if n == 0 { 1 } else { n };
    unsafe {
        // 标记/建页表/清扫都在分配路径上分批推进
        if MARK_ACTIVE {
            mark_step(256);
        } else if PAGE_BUILD_ACTIVE {
            page_build_step(256);
        } else if SWEEP_ACTIVE {
            sweep_some(256);
        }
        if !MARK_ACTIVE && !PAGE_BUILD_ACTIVE && !SWEEP_ACTIVE
            && NEXT_LIMIT != 0 && ALLOC_SINCE >= NEXT_LIMIT && !HEAP.is_null() {
            collect_now(false);
        }
        let live_mark = if MARK_ACTIVE { 1usize } else { 0 };
        let hdr = core::mem::size_of::<Block>();
        let need = round_up(hdr + size);
        let b = take_from_bins(need);
        let mut total = if b.is_null() { need } else { (*b).cap };
        // 大块复用：够大就切出尾部，避免整块浪费
        if !b.is_null() && total >= need + hdr + 8 {
            let rest = (b as *mut u8).add(need) as *mut Block;
            let rest_cap = total - need;
            let rest_size = rest_cap - hdr;
            ptr::write(rest, Block { size: rest_size, cap: rest_cap, mark: 0, gen: 0, next: ptr::null_mut() });
            if let Some(c) = bin_of(rest_cap) {
                (*rest).next = BINS[c];
                BINS[c] = rest;
                BLOCK_COUNT += 1;
            } else {
                sys_free(rest as *mut u8);
            }
            total = need;
        }
        if b.is_null() {
            let raw = sys_alloc(need);
            if raw.is_null() {
                return ptr::null_mut();
            }
            ptr::write(raw as *mut Block, Block { size, cap: need, mark: live_mark, gen: 0, next: HEAP });
            let nb = raw as *mut Block;
            (*nb).next = HEAP;
            HEAP = nb;
            BLOCK_COUNT += 1;
            if SWEEP_ACTIVE && NEW_TAIL.is_null() { NEW_TAIL = nb; }
            ALLOC_SINCE += need;
            LIVE_BYTES += need;
            TOTAL_BYTES += need;
            return (nb as *mut u8).add(hdr);
        }
        ptr::write(b, Block { size, cap: total, mark: live_mark, gen: 0, next: HEAP });
        HEAP = b;
        BLOCK_COUNT += 1;
        if SWEEP_ACTIVE && NEW_TAIL.is_null() { NEW_TAIL = b; }
        ALLOC_SINCE += total;
        LIVE_BYTES += total;
        TOTAL_BYTES += total;
        (b as *mut u8).add(hdr)
    }
}

pub(crate) unsafe fn recalc_next_limit() {
    let live_limit = LIVE_BYTES.saturating_mul(2);
    NEXT_LIMIT = if THRESHOLD == 0 { 0 } else { core::cmp::max(THRESHOLD, live_limit) };
}

pub(crate) unsafe fn begin_sweep() {
    SWEEP_HEAD = HEAP;
    SWEEP_CURSOR = HEAP;
    SWEEP_PREV = ptr::null_mut();
    HEAP = ptr::null_mut();
    NEW_TAIL = ptr::null_mut();
    SWEEP_ACTIVE = !SWEEP_HEAD.is_null();
    if !SWEEP_ACTIVE { recalc_next_limit(); }
}

pub(crate) unsafe fn sweep_some(mut budget: usize) {
    if !SWEEP_ACTIVE { return; }
    let t0 = now_cycles();
    while budget > 0 && !SWEEP_CURSOR.is_null() {
        let cur = SWEEP_CURSOR;
        let next = (*cur).next;
        if (*cur).mark == 0 && !(MINOR_ACTIVE && (*cur).gen != 0) {
            if SWEEP_PREV.is_null() { SWEEP_HEAD = next; } else { (*SWEEP_PREV).next = next; }
            let total = (*cur).cap;
            LIVE_BYTES = LIVE_BYTES.saturating_sub(total);
            BLOCK_COUNT = BLOCK_COUNT.saturating_sub(1);
            match bin_of(total) {
                Some(c) => { (*cur).next = BINS[c]; BINS[c] = cur; }
                None => sys_free(cur as *mut u8),
            }
        } else {
            (*cur).mark = 0;
            if (*cur).gen == 0 { (*cur).gen = 1; }
            SWEEP_PREV = cur;
        }
        SWEEP_CURSOR = next;
        budget -= 1;
    }
    let elapsed = now_cycles().saturating_sub(t0);
    if elapsed > MAX_SWEEP_CYCLES { MAX_SWEEP_CYCLES = elapsed; }
    if SWEEP_CURSOR.is_null() {
        if !SWEEP_HEAD.is_null() {
            if HEAP.is_null() {
                HEAP = SWEEP_HEAD;
            } else {
                if NEW_TAIL.is_null() { NEW_TAIL = HEAP; }
                (*NEW_TAIL).next = SWEEP_HEAD;
            }
        }
        SWEEP_HEAD = ptr::null_mut();
        SWEEP_PREV = ptr::null_mut();
        NEW_TAIL = ptr::null_mut();
        SWEEP_ACTIVE = false;
        recalc_next_limit();
    }
}

pub(crate) unsafe fn finish_sweep() { sweep_some(usize::MAX); }
