// 最小裸机运行时：固定 arena 代替 malloc，不实现回收。

use crate::gc::Gc;
use crate::runtime::StandardRuntime;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

const ARENA_SIZE: usize = 1 << 20;

struct ArenaGc;
struct Arena(UnsafeCell<[u8; ARENA_SIZE]>);
unsafe impl Sync for Arena {}
static ARENA: Arena = Arena(UnsafeCell::new([0; ARENA_SIZE]));
static OFF: AtomicUsize = AtomicUsize::new(0);

impl Gc for ArenaGc {
    fn init(&self) {}
    fn alloc(&self, size: usize) -> *mut u8 {
        let need = (size + 15) & !15;
        let mut cur = OFF.load(Ordering::Relaxed);
        loop {
            if cur.saturating_add(need) > ARENA_SIZE { return core::ptr::null_mut(); }
            match OFF.compare_exchange_weak(cur, cur + need, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return unsafe { (ARENA.0.get() as *mut u8).add(cur) },
                Err(v) => cur = v,
            }
        }
    }
    fn collect(&self) {}
    fn add_root_range(&self, _start: *mut u8, _len: usize) {}
    fn write_barrier(&self, _value: *mut u8) {}
}

static RT: StandardRuntime<ArenaGc> = StandardRuntime { gc: ArenaGc };
pub fn runtime() -> &'static dyn crate::runtime::Runtime { &RT }
