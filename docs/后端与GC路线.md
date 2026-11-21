# 后端与 GC 架构（现状）

前端（lexer → parser → program → sema）只产 `ast::Program`；后端通过 `Backend` trait 接入，
GC 通过 C ABI 接入。当前默认链路是 `MIR → LLVM IR → clang`，C 侧保留 MIR-C 与 AST-C 两条路径。

## 1. 目录结构（当前）

```text
mylis/
├── mylc/src/
│   ├── lexer.rs parser.rs program.rs sema.rs ast.rs   # 前端（与后端无关）
│   ├── mir.rs                                        # MIR 定义与 verify
│   ├── mir/pass.rs                                   # MIR pass：可达性/DCE/常量折叠/小函数内联
│   ├── backend/
│   │   ├── mod.rs                                    # Backend trait / Artifact / ExportMode
│   │   ├── ir.rs                                     # 兼容 IR 后端入口（MIR-C）
│   │   ├── c.rs                                      # AST → C11
│   │   ├── mir_c.rs                                  # MIR → C11
│   │   └── llvm.rs                                   # MIR → 文本 LLVM IR
│   ├── codegen/                                      # C 后端实现细节（names/support/emit/stmt/expr）
│   ├── driver.rs                                     # 编排：产物模式、工具链、链接
│   └── toolchain.rs                                  # 外部 cc 的“家族/规格”（gnu/clang/msvc）
├── mylrt-rs/                                         # Rust 运行时（Runtime / Gc trait + 实现）
├── mylgc/                                            # 自研 GC（Rust staticlib）
├── myrt/
│   ├── mylrt.h                                       # 运行时 C ABI
│   ├── mylrt_gc.h                                    # GC 后端抽象
│   └── gc/
│       ├── gc_boehm.c                                # 可选：Boehm
│       └── gc_simple.c                               # 可选：malloc 不回收（调试）
└── toolchains/                                       # 编译器规格（*.toml，插件式）
```

`backend/mod.rs` 的接口：

```rust
pub enum ArtifactKind { CSource, Ir, LlvmIr }
pub struct Artifact { kind, text, error }
pub enum ExportMode { Default, DllExport }
pub trait Backend {
    fn name(&self) -> &'static str;
    fn emit_exe(&self, prog: &Program) -> Artifact;
    fn emit_lib(&self, prog: &Program, mode: ExportMode) -> Artifact;
    fn header(&self, prog: &Program) -> String;
    fn emit_exe_mir(&self, prog: &Program, ir: &MirProgram) -> Artifact;
}
```

`driver` 只认 `Backend`；默认 `llvm` 走 `LlvmBackend`，`mir` 走 `MirCBackend`，`ast` 走 `CBackend`。新增后端只需实现该 trait 并注册，不改前端与工具链层。

LLVM 后端已接入：MIR → 文本 LLVM IR，`--emit-llvm` 可单独导出 `.ll`，
`--backend llvm` 用 clang（15+）编译 `.ll` 并链接 myrt。当前覆盖标量、字符串、
CFG、直接调用、类/对象、静态字段、数组、enum、接口 vtable 与动态派发，以及
Option/Result 的构造、`?`、内建方法、基础模式匹配、接口装箱/拆箱与
`Option<Iface>`/`Result<Iface>`；静态库/动态库已接入；泛型在 sema 阶段单态化，后端只消费具体声明，没有运行时泛型 lowering 需求，
未支持结构会明确报错，不回退。

## 2. Windows MSVC `--shared`（已完成）

- `ExportMode::DllExport` → 生成的 C 给 `@export` 函数与 `<stem>_init` 加 `__declspec(dllexport)`；
- `driver` 的 msvc 分支用 `cl /LD /Fe:<dll>`，链接 Rust 运行时 `mylrt.lib`，GC 链 `mylgc.lib`；
- GNU 家族不变（继续用链接器 `--export-all-symbols`，不污染 C 源码）。

## 3. 自研 GC（当前默认）

`mylgc/`（`no_std` staticlib，C ABI）已实现非移动两代 mark-sweep：

| 项 | 实现 |
| --- | --- |
| 分配 | 每块带 40 字节头（size/cap/mark/gen/next），payload 返回给运行时 |
| 根 | ① `myrt` 注册的区间（`g_roots`、静态字段槽）② 当前线程栈 `[sp, init 时栈基]` |
| 标记 | 保守：任何指向块内部的字都算存活；调用方寄存器先溢出到栈再扫（否则 Windows 会把栈上的字符串误回收）；工作队列走 libc `malloc` |
| 清扫 | 未标记块按代回收；存活年轻代块晋升老年代，老年代不移动、不压缩 |
| 自动回收 | 累计分配达到 `max(MYLIS_GC_THRESHOLD, 2×存活)`（默认阈值 512KB，`0` 关闭）时，在分配前先回收一次 |
| 查找 | 回收期间按块地址排序建索引，指针归属查找 O(log n)（早期 O(堆) 是主要瓶颈） |
| 统计 | `myrt_gc_backend_stats` / `myrt_gc_stats_dump`：回收次数、存活字节、累计分配、阈值 |
| 分配器抽象 | `myrt_gc_backend_set_allocator(a, f)`：宿主默认 libc，裸机可传静态 arena；arena 实测 30 万次分配只占 378KB |
| 栈范围 | `myrt_gc_backend_set_stack_range(lo, hi)`：裸机从链接脚本符号传入，替代 `__libc_stack_end` |
| 根批量注册 | `myrt_gc_backend_add_root_ranges(ranges, n)`：一次登记多个静态槽 |
| 根表 | 根区间表按需翻倍扩容，不再有 256 条上限（已用 500 条静态根验证） |
| 大块复用 | 复用块比请求大时切出尾部回收到对应尺寸桶，减少大对象浪费 |
| 压力验证 | 12 万次随机尺寸（8B~13KB）分配 + 1MB 阈值回收 + 500 个根，数据完整 |
| 增量标记 | 标记工作队列在分配路径上分批推进；指针写入走 `myrt_gc_write_barrier_owner`（Dijkstra barrier + owner 反查），新分配对象在标记期间直接置黑 |
| 页哈希 | 4KB 页 → 块表，查询 O(1)；建表本身也分批（清桶/插块各走小步） |
| 动态页表 | 桶数按 `2×块数`、入口池按 `3×块数` 估算；堆缩小时超过 4× 需求就缩容 |
| 模块布局 | `mylgc/src/gc/{api,heap,mark,page,roots,stats,sys}.rs` + `mod.rs` 共享状态 |
| 增量清扫 | 标记后旧堆脱离分配链，后续分配每次顺带扫 256 块 |
| 暂停统计 | `GcStats.max_mark_cycles` / `max_sweep_cycles`（x86_64 TSC 周期）；默认 512KB 阈值下 mark ≈ 0.02ms、sweep ≈ 0.01ms；8MB 压力基准下 mark ≈ 0.3ms |
| 选择 | `MYLIS_GC=own`（默认）/ `MYLIS_GC=boehm` / `MYLIS_GC=simple` |

产物拆分（`make all` 后 `bin/build/`）：

```
libmylrt.a       运行时本体（不含 GC 实现）
libgc_boehm.a    myrt_gc_backend_* → GC_* 包装层（与 libgc.a 一起链）
libgc_simple.a   malloc 不回收
libmylgc.a        自研 GC（Rust）
```

生成的 C 只 include `mylrt.h`：基本类型由 `myrt_i8..myrt_u64`/`myrt_bool` 定义（C11
静态断言保证宽度），不出现 stdint/stdbool/stddef/stdlib/string；libc 只在运行时与 GC 的
`.c` 里。运行时单线程（无 thread/spawn），GC 堆与根集合均为单例。

C 客户端链接顺序：`client.o libmylrt.a <后端库> [libgc.a] ...`。

线程模型：**运行时单线程**（语言层无 `thread/spawn`），GC 只有一个堆与一份根集合；
这是当前明确的语言语义，不是临时缺陷。多线程需要每线程堆/根 + 安全点，留给将来。

已知边界：不扫描模块数据段（静态字段由 codegen 注册）；
运行时已迁到 `mylrt-rs/`，C ABI 保持 `myrt/mylrt.h`；`Runtime` / `Gc` trait 允许 `make RT_IMPL=<own|boehm|simple|path.rs>` 选择实现，`RT_NO_STD=1` 走 no_std 构建（POSIX/Win32 平台薄层）；`PLATFORM_IMPL` 可替换成裸机平台，`mylrt-rs/examples/qemu-arm/` 是 Cortex-M3 + QEMU 样例。

MSVC 家族：`MYLIS_GC=boehm/own` 都落到自研 GC（`mylgc.lib`，Rust `x86_64-pc-windows-msvc` 目标），只
有显式 `simple` 才不回收；MSVC 版自研库随包提供，源码构建需 `rustup target add x86_64-pc-windows-msvc`。
GNU 侧 `libmylgc.a` 以 PIC 编译（`RUSTFLAGS="-C relocation-model=pic"`，运行时也加 `-fPIC`），
因此 `--shared --embed-runtime` + `MYLIS_GC=own` 可用。

已完成：精确根（LLVM/MIR-C 逐 safepoint 帧根表 + `MYLIS_GC_PRECISE`）、静态根表、非移动两代 GC、Windows MSVC `mylgc.lib`。后续：remembered set 去重/分桶优化。

裸机/RTOS 接入：GC 不实现具体分配器，只调用 `set_allocator(alloc, free)`。
`free` 必须把块还回可复用池；完整契约、最小实现和已验证数据见
[`docs/裸机分配器契约.md`](裸机分配器契约.md)。

接口约定（`myrt/mylrt_gc.h`，C ABI，Rust 侧用 `#[no_mangle] extern "C"` 实现）：

```c
void  myrt_gc_backend_init(void);
void* myrt_gc_backend_alloc(size_t n);
void  myrt_gc_backend_collect(void);
void  myrt_gc_backend_add_root_range(void* lo, void* hi);
```

与原计划对应的落地结果：

1. `mylgc/` 作为 Rust `staticlib` 导出上述符号，产物为 `libmylgc.a` / `mylgc.lib`。
2. 运行时经 `Gc` trait + `BackendGc` 调用后端；`MYLIS_GC=own` 是默认，`simple` 与 `boehm` 仍可显式选择。
3. 保守栈扫描已落地；LLVM/MIR-C 另支持逐 safepoint 帧根表的精确模式（`MYLIS_GC_PRECISE`）。
4. GC 根表由后端生成：C 后端注册静态槽区间，LLVM/MIR-C 使用帧根表；MIR 已带 safepoint 信息。

注意：`make GC_BACKEND=...` 只是构建提示，不会改变编译时的 GC 选择；实际选择看 `MYLIS_GC`。

## 4. MIR 与后端（当前主线）

参考 Go：前端 → MIR →（可选）优化 → LLVM / C 后端。MIR 当前是默认 LLVM 与 MIR-C 的共同输入，详见 [`docs/MIR.md`](MIR.md)。

已落地：

- `mylc/src/mir.rs`：函数级 CFG、SSA 值 `%n`、局部槽 `$n`、显式终结指令；
- `mir::lower` 从 AST 生成 MIR；`mir::dump` 输出 `; Mylis MIR v0` 文本；
- `mir::verify` 校验标签/终结指令/值定义；
- `ArtifactKind::Ir`、`backend::IrBackend`、`--emit-ir <path>`；
- MIR-C（`--backend mir`）消费 MIR，覆盖当前全部正向程序；
- LLVM（`--backend llvm`，默认）消费 MIR，输出文本 LLVM IR 后交给 clang；
- `match` 的模式分支/绑定已 lower 到 MIR。

尚未完成 / 保留：

- AST-C（`--backend ast`）仍消费 AST，作为显式回退路径保留；
- 没有 TypeId 表、别名分析与更强的优化 pass；LLVM/MIR-C 的 GC safepoint 与帧根表已落地；
- 纯 MIR `.mylib` 仍不能直接作为 `--static/--shared` 的链接输入，需要原生载荷。

下一步：补 TypeId/别名分析、优化 pass；保持 `--backend ast` 仅作回退，不再扩功能。

## 5. 变更影响面

| 变更 | 前端 | driver | toolchain | 运行时 |
| --- | --- | --- | --- | --- |
| MSVC shared | 无 | msvc 分支 | 无 | 无 |
| 自研 GC | 无 | 选后端 | 增加归档 | 新增 `gc_own` |
| MIR / LLVM | lower 需补引用/cast | 选 backend | 不变 | 根表接口 |

结论：前端稳定，改动集中在 `mir.rs`、`backend/`、`driver` 的编排与 `mylrt-rs` / `mylgc` 两处运行时实现。
