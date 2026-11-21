//! Mylis 自研 GC：非移动两代 mark-sweep（保守扫描），导出 C ABI 供 `myrt` 调用。
//!
//! 设计取舍：
//! * 单线程运行时（语言层无线程原语）；GC 只有一个堆与一份根集合。
//! * 不增加第三方依赖（`no_std`，只用 libc 的 malloc/free）。
//! * 根集合 = 显式注册区间（`myrt` 的 `g_roots`）+ 静态根表 + 每帧根表 + 当前线程栈。
//! * 保守扫描：栈/根区间里任何指向堆块内部（含内部指针）的字都视为存活。
//! * 不做压缩；分代采用非移动的两代策略：年轻代回收时老年代仅作为根扫描，
//!   存活年轻对象晋升老年代，定期做全量回收。
//!
//! 局限（有意保留的边界）：
//! * 寄存器里暂存的指针可能漏扫（调用 collect 前通常已溢出到栈）；
//! * 不扫描模块数据段，静态字段由 codegen 在 `myl_init_statics` 里写入静态根表。

#![no_std]

use core::ptr;

#[repr(C)]
struct Block {
    size: usize,   // 本次请求的负载大小（扫描范围）
    cap: usize,    // 块的实际容量（含头）
    mark: usize,
    gen: usize,    // 0 = 年轻代，1 = 老年代
    next: *mut Block,
}

#[repr(C)]
struct Root { start: *mut u8, bytes: usize }
const ROOT_MIN: usize = 64;
static mut ROOTS: *mut Root = ptr::null_mut();
static mut ROOT_CAP: usize = 0;

static mut HEAP: *mut Block = ptr::null_mut();
const EXACT_LIMIT: usize = 4096;
const EXACT_BINS: usize = EXACT_LIMIT / 8;      // 8..4096，按 8 字节取整
const BIN_COUNT: usize = EXACT_BINS + 16;
/// 按尺寸分级的空闲块链：头 32B 起，每级约翻倍
static mut BINS: [*mut Block; BIN_COUNT] = [ptr::null_mut(); BIN_COUNT];
/// 增量清扫状态：标记完成后，把旧堆从分配链上摘下来，按分配节奏分批清扫
static mut SWEEP_ACTIVE: bool = false;
static mut SWEEP_HEAD: *mut Block = ptr::null_mut();
static mut SWEEP_CURSOR: *mut Block = ptr::null_mut();
static mut SWEEP_PREV: *mut Block = ptr::null_mut();
static mut NEW_TAIL: *mut Block = ptr::null_mut();
static mut STACK_BASE: usize = 0;
static mut NROOTS: usize = 0;
/// 自动回收阈值（字节）：累计分配超过它就在下一次分配前回收；0 = 关闭
static mut THRESHOLD: usize = 512 * 1024;
/// 只扫描显式注册的根区间，不扫描线程栈（LLVM 帧根注册模式）
static mut PRECISE: bool = false;
/// 下一次回收的触发点：至少 THRESHOLD，且不低于存活量的 2 倍（避免堆变大后还频繁回收）
static mut NEXT_LIMIT: usize = 512 * 1024;
static mut ALLOC_SINCE: usize = 0;
static mut INITED: bool = false;
/// 分代策略：每 MAJOR_EVERY 次回收做一次全量回收，其余只回收年轻代
static mut MINOR_ACTIVE: bool = false;
static mut MINOR_ROUNDS: usize = 0;
const MAJOR_EVERY: usize = 8;
/// 回收期间的块索引（按块地址排序），把 find_block 从 O(heap) 降到 O(log n)
/// 页哈希：页号 -> 覆盖该页的块链；建表 O(n)，查询 O(1)
#[repr(C)]
struct PageEntry { block: *mut Block, next: *mut PageEntry }
static mut PAGE_BUCKETS: *mut *mut PageEntry = ptr::null_mut();
static mut ENTRY_POOL: *mut PageEntry = ptr::null_mut();
static mut ENTRY_CAP: usize = 0;
static mut PAGE_MASK: usize = 0;
const PAGE_SHIFT: usize = 12;
static mut BLOCK_COUNT: usize = 0;
static mut PAGE_BUILD_ACTIVE: bool = false;
static mut PAGE_BUILD_CURSOR: *mut Block = ptr::null_mut();
static mut PAGE_SNAPSHOT: *mut Block = ptr::null_mut();
static mut PAGE_CLEAR_POS: usize = 0;
static mut PAGE_ENTRY_POS: usize = 0;
/// 标记工作队列缓冲：跨回收复用，避免每轮分配/释放元数据
static mut WORK_BUF: *mut usize = ptr::null_mut();
static mut WORK_CAP: usize = 0;
/// 统计
static mut COLLECTIONS: usize = 0;
static mut MAX_MARK_CYCLES: usize = 0;
static mut MAX_SWEEP_CYCLES: usize = 0;
static mut LIVE_BYTES: usize = 0;
static mut TOTAL_BYTES: usize = 0;

extern "C" {
    fn malloc(n: usize) -> *mut u8;
    fn free(p: *mut u8);
}

type SysAlloc = unsafe extern "C" fn(usize) -> *mut u8;
type SysFree = unsafe extern "C" fn(*mut u8);
/// 可替换的底层分配器：裸机可换成静态 arena，宿主默认 libc。
static mut ALLOC_FN: Option<SysAlloc> = None;
static mut FREE_FN: Option<SysFree> = None;
/// 可显式指定的栈范围（裸机从链接符号传入）；为 0 时走平台默认。
static mut STACK_TOP_OVERRIDE: usize = 0;
struct Work {
    buf: *mut usize,
    len: usize,
    cap: usize,
}

static mut WORK: Work = Work { buf: ptr::null_mut(), len: 0, cap: 0 };
static mut WORK_POS: usize = 0;
static mut MARK_ACTIVE: bool = false;
static mut MARK_START: usize = 0;

mod api;
mod heap;
mod mark;
mod page;
mod roots;
mod stats;
mod sys;

use api::*;
use heap::*;
use mark::*;
use page::*;
use roots::*;
use stats::*;
use sys::*;
