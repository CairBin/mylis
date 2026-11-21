# mylrt-rs

Mylis 的 Rust 运行时。导出的符号与 `myrt/mylrt.h` 一一对应，生成代码不需要知道运行时是 Rust 还是 C。

## 结构

- `src/runtime.rs` 定义 `Runtime`，并提供字符串、数组、打印、panic、格式化等默认实现。
- `src/gc.rs` 定义 `Gc`，默认 `BackendGc` 委托给 `myrt_gc_backend_*`（mylgc / Boehm / simple）。
- `src/plat.rs` 定义 `Platform`：I/O、abort、时钟、环境变量、f64 格式化。
- `src/platforms/host.rs` 是默认平台；`build.rs` 读取 `MYLRT_PLATFORM_RS` 替换它。
- `src/impls/own.rs` 默认实现；`boehm.rs` / `simple.rs` 目前复用同一套 ABI 壳，GC 由链接的后端决定。
- `build.rs` 同时读取 `MYLRT_IMPL_RS` / `MYLRT_PLATFORM_RS`。

## 选择实现

```bash
make all RT_IMPL=own
make all RT_IMPL=boehm
make all RT_IMPL=simple
make all RT_IMPL=/path/to/my_runtime.rs
make all RT_IMPL=/path/to/rt_dir          # 目录下的 runtime.rs
make all RT_IMPL=myimpl RT_IMPL_DIR=/path/to/impls
make all RT_NO_STD=1                       # no_std 运行时
make all RT_NO_STD=1 PLATFORM_IMPL=/path/to/platform.rs
make all RT_NO_STD=1 RT_NO_BACKEND_GC=1 PLATFORM_IMPL=/path/to/platform.rs
```

`/path/to/my_runtime.rs` 需要提供：

```rust
use crate::{Gc, Runtime};

pub struct MyRuntime;
static GC: MyGc = MyGc;       // 你的 GC 实现
static RT: MyRuntime = MyRuntime;

impl Runtime for MyRuntime {
    fn gc(&self) -> &dyn Gc { &GC }
}

pub fn runtime() -> &'static dyn Runtime { &RT }
```

只实现 `Gc` 就能复用默认的字符串、数组、打印和 panic；freestanding 也可以覆盖 `Runtime::print`、`Runtime::panic`、`Runtime::alloc` 等。

默认构建使用 `std`；`RT_NO_STD=1` 会走 `#![no_std]`，根表改为固定数组 + 自旋锁，输出/时钟/环境变量走 `platforms/host.rs` 的 POSIX 或 Win32 薄层。

裸机/第三方平台只需要写一个 `platform.rs`：

```rust
use crate::plat::Platform;
pub struct MyPlatform;
static P: MyPlatform = MyPlatform;
impl Platform for MyPlatform { /* write_stdout / abort / clock / ... */ }
pub fn platform() -> &'static dyn Platform { &P }
```

裸机运行时如果自带 `Gc`，用 `RT_NO_BACKEND_GC=1` 去掉默认 `BackendGc`，避免静态库里残留 `myrt_gc_backend_*` 引用。

完整样例：

- `examples/freestanding/`：Linux x86_64，无 libc，直接 syscall + 固定 arena GC。
- `examples/qemu-arm/`：Cortex-M3，semihosting 输出，固定 arena GC；`cargo build --target thumbv7m-none-eabi` 后用 QEMU `lm3s6965evb` 运行。
