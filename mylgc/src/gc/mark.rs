use super::*;
use super::api::*;
use super::heap::*;
use super::page::*;
use super::roots::*;
use super::stats::*;
use super::sys::*;

#[no_mangle]
pub extern "C" fn myrt_gc_backend_write_barrier(value: *mut u8) {
    if value.is_null() { return; }
    unsafe {
        if !MARK_ACTIVE { return; }
        if let Some(b) = find_block(value as usize) {
            if (*b).mark == 0 {
                (*b).mark = 1;
                if MINOR_ACTIVE && (*b).gen != 0 { return; }
                let hdr = core::mem::size_of::<Block>();
                WORK.push((b as *mut u8).add(hdr) as usize, (*b).size);
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_write_barrier_owner(owner: *mut u8, value: *mut u8) {
    if value.is_null() { return; }
    unsafe {
        // 先按原有屏障语义处理“标记进行中”的写入，避免当前回收漏掉新值。
        if MARK_ACTIVE {
            if let Some(b) = find_block(value as usize) {
                if (*b).mark == 0 {
                    (*b).mark = 1;
                    if !(MINOR_ACTIVE && (*b).gen != 0) {
                        let hdr = core::mem::size_of::<Block>();
                        WORK.push((b as *mut u8).add(hdr) as usize, (*b).size);
                    }
                }
            }
            return;
        }
        // 记录写入位置；minor 时再按 owner 所在块的老年代属性决定是否扫描。
        remember_owner(owner as usize);
    }
}

pub(crate) unsafe fn scan_range(start: usize, bytes: usize, work: &mut Work) {
    let end = start.saturating_add(bytes);
    let mut p = start;
    while p + core::mem::size_of::<usize>() <= end {
        let word = ptr::read_unaligned(p as *const usize);
        if word != 0 {
            if let Some(b) = find_block(word) {
                if (*b).mark == 0 {
                    (*b).mark = 1;
                    if !(MINOR_ACTIVE && (*b).gen != 0) {
                        let payload = (b as *mut u8).add(core::mem::size_of::<Block>()) as usize;
                        work.push(payload, (*b).size);
                    }
                }
            }
        }
        p += core::mem::size_of::<usize>();
    }
}

impl Work {
    unsafe fn new() -> Work {
        Work { buf: WORK_BUF, len: 0, cap: WORK_CAP }
    }

    unsafe fn push(&mut self, start: usize, bytes: usize) {
        if self.len + 2 > self.cap {
            let ncap = if self.cap == 0 { 256 } else { self.cap * 2 };
            let nbuf = sys_alloc(ncap * core::mem::size_of::<usize>()) as *mut usize;
            if nbuf.is_null() {
                return;
            }
            if !self.buf.is_null() {
                ptr::copy_nonoverlapping(self.buf, nbuf, self.len);
                sys_free(self.buf as *mut u8);
            }
            self.buf = nbuf;
            self.cap = ncap;
        }
        ptr::write(self.buf.add(self.len), start);
        ptr::write(self.buf.add(self.len + 1), bytes);
        self.len += 2;
    }

    unsafe fn drop_buf(&mut self) {
        WORK_BUF = self.buf;
        WORK_CAP = self.cap;
        self.len = 0;
    }
}

pub(crate) unsafe fn scan_roots() {
    for i in 0..NROOTS {
        let r = ptr::read(ROOTS.add(i));
        if !r.start.is_null() && r.bytes > 0 {
            scan_range(r.start as usize, r.bytes, &mut WORK);
        }
    }
    // 静态字段根表：启动时由 myl_init_statics 填充，长期有效。
    for j in 0..STATIC_COUNT {
        let d = ptr::read(STATIC_ENTRIES.add(j));
        if !d.start.is_null() && d.bytes > 0 {
            scan_range(d.start as usize, d.bytes as usize, &mut WORK);
        }
    }
    // 每帧根表快照：只扫描当前 safepoint 填进去的活跃根。
    for fi in 0..FRAME_CAP {
        let f = ptr::read(&raw const FRAMES[fi]);
        if !f.active || f.entries.is_null() || f.count == 0 { continue; }
        for j in 0..f.count {
            let d = ptr::read(f.entries.add(j));
            if !d.start.is_null() && d.bytes > 0 {
                scan_range(d.start as usize, d.bytes as usize, &mut WORK);
            }
        }
    }
    if PRECISE { return; }
    let mut regs = [0usize; 8];
    #[cfg(target_arch = "x86_64")]
    core::arch::asm!(
        "mov [{p}], rbx",
        "mov [{p} + 8], rbp",
        "mov [{p} + 16], r12",
        "mov [{p} + 24], r13",
        "mov [{p} + 32], r14",
        "mov [{p} + 40], r15",
        "mov [{p} + 48], rdi",
        "mov [{p} + 56], rsi",
        p = in(reg) regs.as_mut_ptr(),
        options(nostack, preserves_flags)
    );
    let sp = read_sp();
    let lo = core::cmp::min(sp, STACK_BASE);
    let hi = core::cmp::max(sp, STACK_BASE);
    scan_range(lo, hi.saturating_sub(lo), &mut WORK);
    scan_range(regs.as_ptr() as usize, core::mem::size_of_val(&regs), &mut WORK);
}

/// 从工作队列消费至多 `budget` 项（budget=usize::MAX 表示清空）
pub(crate) unsafe fn drain_work(mut budget: usize) -> bool {
    while WORK_POS < WORK.len && budget > 0 {
        let start = ptr::read(WORK.buf.add(WORK_POS));
        let bytes = ptr::read(WORK.buf.add(WORK_POS + 1));
        scan_range(start, bytes, &mut WORK);
        WORK_POS += 2;
        budget -= 1;
    }
    if WORK_POS >= WORK.len {
        WORK.len = 0;
        WORK_POS = 0;
        true
    } else {
        false
    }
}

/// 标记收尾：重扫根与栈（捕获标记期间写到栈上的指针），然后清空工作队列。
pub(crate) unsafe fn finish_mark() {
    scan_roots();
    while !drain_work(usize::MAX) {}
    MARK_ACTIVE = false;
    free_index();
    begin_sweep();
}

/// 推进增量标记；工作队列清空时进入收尾（重扫根 + 清扫）。
pub(crate) unsafe fn mark_step(budget: usize) {
    if !MARK_ACTIVE { return; }
    let t0 = now_cycles();
    if drain_work(budget) {
        finish_mark();
    }
    let cycles = now_cycles().saturating_sub(t0);
    if cycles > MAX_MARK_CYCLES { MAX_MARK_CYCLES = cycles; }
}

pub(crate) unsafe fn start_mark_phase() {
    let t0 = now_cycles();
    WORK.len = 0;
    WORK_POS = 0;
    MARK_ACTIVE = true;
    if MINOR_ACTIVE {
        // 年轻代回收：只扫描 remembered set 里的老年代对象。
        scan_remembered();
    } else {
        clear_remembered();
    }
    scan_roots();
    mark_step(1024);
    let cycles = now_cycles().saturating_sub(t0);
    if cycles > MAX_MARK_CYCLES { MAX_MARK_CYCLES = cycles; }
}

pub(crate) unsafe fn collect_now(full: bool) {
    if full {
        MINOR_ROUNDS = 0;
        begin_collection(false);
    } else {
        let minor = MINOR_ROUNDS < MAJOR_EVERY;
        MINOR_ROUNDS = if minor { MINOR_ROUNDS + 1 } else { 0 };
        begin_collection(minor);
    }
    if full {
        while PAGE_BUILD_ACTIVE { page_build_step(usize::MAX); }
        if MARK_ACTIVE { mark_step(usize::MAX); }
        finish_sweep();
    }
}
