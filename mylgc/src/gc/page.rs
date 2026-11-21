use super::*;
use super::api::*;
use super::heap::*;
use super::mark::*;
use super::roots::*;
use super::stats::*;
use super::sys::*;

pub(crate) fn page_hash(page: usize) -> usize {
    ((page ^ (page >> 12)).wrapping_mul(0x9E37_79B9_7F4A_7C15)) & unsafe { PAGE_MASK }
}

pub(crate) unsafe fn find_block(addr: usize) -> Option<*mut Block> {
    if PAGE_BUCKETS.is_null() || PAGE_MASK == 0 { return None; }
    let hdr = core::mem::size_of::<Block>();
    let mut e = ptr::read(PAGE_BUCKETS.add(page_hash(addr >> PAGE_SHIFT)));
    while !e.is_null() {
        let cur = (*e).block;
        let payload = (cur as *mut u8).add(hdr) as usize;
        if addr >= payload && addr < payload + (*cur).size { return Some(cur); }
        e = (*e).next;
    }
    None
}

/// 标记工作用的动态数组（直接走 libc malloc，避免与 GC 堆互相影响）
pub(crate) unsafe fn insert_block_entries(cur: *mut Block) -> bool {
    let hdr = core::mem::size_of::<Block>();
    let start = (cur as *mut u8).add(hdr) as usize >> PAGE_SHIFT;
    let end = (cur as *mut u8).add(hdr) as usize + (*cur).size;
    let last = end.saturating_sub(1) >> PAGE_SHIFT;
    let mut pg = start;
    while pg <= last {
        if PAGE_ENTRY_POS >= ENTRY_CAP { return false; }
        let slot = page_hash(pg);
        let e = ENTRY_POOL.add(PAGE_ENTRY_POS);
        ptr::write(e, PageEntry { block: cur, next: ptr::read(PAGE_BUCKETS.add(slot)) });
        ptr::write(PAGE_BUCKETS.add(slot), e);
        PAGE_ENTRY_POS += 1;
        pg = pg.saturating_add(1);
    }
    true
}

pub(crate) unsafe fn rebuild_page_table_sync() {
    let hdr = core::mem::size_of::<Block>();
    let mut entries = 0usize;
    let mut cur = HEAP;
    while !cur.is_null() {
        let start = (cur as *mut u8).add(hdr) as usize >> PAGE_SHIFT;
        let last = ((cur as *mut u8).add(hdr) as usize + (*cur).size).saturating_sub(1) >> PAGE_SHIFT;
        entries += last.saturating_sub(start) + 1;
        cur = (*cur).next;
    }
    if entries > ENTRY_CAP {
        let buf = sys_alloc(entries * core::mem::size_of::<PageEntry>()) as *mut PageEntry;
        if !buf.is_null() {
            if !ENTRY_POOL.is_null() { sys_free(ENTRY_POOL as *mut u8); }
            ENTRY_POOL = buf; ENTRY_CAP = entries;
        }
    }
    for i in 0..(PAGE_MASK + 1) { ptr::write(PAGE_BUCKETS.add(i), ptr::null_mut()); }
    PAGE_ENTRY_POS = 0;
    let mut b = HEAP;
    while !b.is_null() { insert_block_entries(b); b = (*b).next; }
    PAGE_BUILD_ACTIVE = false;
    start_mark_phase();
}

pub(crate) unsafe fn begin_collection(minor: bool) {
    finish_sweep();
    MINOR_ACTIVE = minor;
    ALLOC_SINCE = 0;
    COLLECTIONS += 1;

    let n = BLOCK_COUNT;
    let mut want_b = 1024usize;
    while want_b < n.saturating_mul(2) { want_b = want_b.saturating_mul(2); }
    let cur_b = if PAGE_BUCKETS.is_null() { 0 } else { PAGE_MASK + 1 };
    if cur_b < want_b || cur_b > want_b.saturating_mul(4) {
        let buf = sys_alloc(want_b * core::mem::size_of::<*mut PageEntry>()) as *mut *mut PageEntry;
        if !buf.is_null() {
            if !PAGE_BUCKETS.is_null() { sys_free(PAGE_BUCKETS as *mut u8); }
            PAGE_BUCKETS = buf;
            PAGE_MASK = want_b - 1;
        }
    }
    let want_e = n.saturating_mul(3).saturating_add(1024);
    if ENTRY_CAP < want_e || ENTRY_CAP > want_e.saturating_mul(4) {
        let buf = sys_alloc(want_e * core::mem::size_of::<PageEntry>()) as *mut PageEntry;
        if !buf.is_null() {
            if !ENTRY_POOL.is_null() { sys_free(ENTRY_POOL as *mut u8); }
            ENTRY_POOL = buf;
            ENTRY_CAP = want_e;
        }
    }
    PAGE_CLEAR_POS = 0;
    PAGE_ENTRY_POS = 0;
    PAGE_BUILD_CURSOR = HEAP;
    PAGE_SNAPSHOT = HEAP;
    PAGE_BUILD_ACTIVE = !HEAP.is_null();
    if !PAGE_BUILD_ACTIVE { start_mark_phase(); }
}

pub(crate) unsafe fn page_build_step(budget: usize) {
    if !PAGE_BUILD_ACTIVE { return; }
    let t0 = now_cycles();
    let bucket_n = PAGE_MASK + 1;
    let mut left = budget;
    while left > 0 && PAGE_CLEAR_POS < bucket_n {
        ptr::write(PAGE_BUCKETS.add(PAGE_CLEAR_POS), ptr::null_mut());
        PAGE_CLEAR_POS += 1;
        left -= 1;
    }
    if PAGE_CLEAR_POS >= bucket_n {
        while left > 0 && !PAGE_BUILD_CURSOR.is_null() {
            if !insert_block_entries(PAGE_BUILD_CURSOR) {
                rebuild_page_table_sync();
                return;
            }
            PAGE_BUILD_CURSOR = (*PAGE_BUILD_CURSOR).next;
            left -= 1;
        }
        if PAGE_BUILD_CURSOR.is_null() {
            // 建表期间新分配的块在堆头，补进页表
            let mut cur = HEAP;
            while !cur.is_null() && cur != PAGE_SNAPSHOT {
                if !insert_block_entries(cur) { rebuild_page_table_sync(); return; }
                cur = (*cur).next;
            }
            PAGE_BUILD_ACTIVE = false;
            start_mark_phase();
        }
    }
    let cycles = now_cycles().saturating_sub(t0);
    if cycles > MAX_MARK_CYCLES { MAX_MARK_CYCLES = cycles; }
}

pub(crate) unsafe fn free_index() {
    // 页表跨回收复用，无需释放
}
