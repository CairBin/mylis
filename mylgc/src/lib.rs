#![no_std]

//! Mylis 自研 GC：非移动两代 mark-sweep（保守扫描），导出 C ABI 供 `myrt` 调用。
//! 代码按职责拆分：`gc::heap`（分配/空闲链/清扫）、`gc::mark`（标记/屏障）、
//! `gc::page`（页哈希表）、`gc::roots`（根区间）、`gc::sys`（平台/分配器）、`gc::api`。

mod gc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}
