# 后端与 GC 路线（结构预留说明）

三条线并行推进，但**结构先定型**：前端（lexer→parser→program→sema）只产 `ast::Program`，
后端通过 trait 接入，GC 通过 C ABI 接入。这样后续加 IR、换 GC 都不动前端。

## 1. 目录结构（当前）

```
mylis/
├── mylc/src/
│   ├── lexer.rs parser.rs program.rs sema.rs ast.rs   # 前端（与后端无关）
│   ├── backend/
│   │   ├── mod.rs        # Backend trait / Artifact / ExportMode（后端注册点）
│   │   └── c.rs          # C 后端：AST → C11 源码
│   ├── codegen/          # C 后端实现细节（names/support/emit/stmt/expr）
│   ├── driver.rs         # 编排：产物模式、工具链、链接
│   └── toolchain.rs      # 外部 cc 的“家族/规格”（gnu/msvc，数据驱动）
├── mylrt-rs/             # Rust 运行时（Runtime / Gc trait + 实现）
├── mylgc/                # 自研 GC（Rust staticlib）
├── myrt/
│   ├── mylrt.h           # 运行时 C ABI（MyStr、数组、打印、panic）
│   ├── mylrt_gc.h        # GC 后端抽象（init/alloc/collect/add_root_range）
│   └── gc/
│       ├── gc_boehm.c    # 后端 A：Boehm（可选，archive 链接）
│       └── gc_simple.c   # 后端 B：malloc 不回收（仅显式调试）
└── toolchains/           # 编译器规格（*.toml，插件式）
```

`backend/mod.rs` 的接口：

```rust
pub enum ArtifactKind { CSource, Ir, LlvmIr } // 未来: Object
pub struct Artifact { kind, text, error }
pub enum ExportMode { Default, DllExport }
pub trait Backend {
    fn emit_exe(&self, prog: &Program) -> Artifact;
    fn emit_lib(&self, prog: &Program, mode: ExportMode) -> Artifact;
    fn header(&self, prog: &Program) -> String;
}
```

`driver` 只认 `Backend`，不再直接调 `codegen::generate*`。LLVM 后端 = 新增
`backend/llvm.rs` 实现该 trait + 在 `driver` 里按 `--backend` 选择，不改前端与工具链层。

LLVM 后端已接入：MIR → 文本 LLVM IR，`--emit-llvm` 可单独导出 `.ll`，
`--backend llvm` 用 clang（15+）编译 `.ll` 并链接 myrt。当前覆盖标量、字符串、
CFG、直接调用、类/对象、静态字段、数组、enum、接口 vtable 与动态派发，以及
Option/Result 的构造、`?`、内建方法、基础模式匹配、接口装箱/拆箱与
`Option<Iface>`/`Result<Iface>`；静态库/动态库已接入；泛型在 sema 阶段单态化，后端只消费具体声明，没有运行时泛型 lowering 需求，
未支持结构会明确报错，不回退。

## 2. 阶段一：MSVC `--shared`（已完成）

- `ExportMode::DllExport` → 生成的 C 给 `@export` 函数与 `<stem>_init` 加 `__declspec(dllexport)`；
- `driver` 的 msvc 分支用 `cl /LD /Fe:<dll>`，链接 Rust 运行时 `mylrt.lib`，GC 链 `mylgc.lib`；
- GNU 家族不变（继续用链接器 `--export-all-symbols`，不污染 C 源码）。

## 3. 阶段二：自研 GC（Rust）——已落地最小闭环

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
| 暂停统计 | `GcStats.max_mark_cycles` / `max_sweep_cycles`（x86_64 TSC 周期）；默认 512KB 阈值下 mark ≈ 0.02ms、sweep ≈ 0.01ms；8MB 阈值下 mark ≈ 0.3ms |
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

已完成：精确根（LLVM/MIR-C 逐 safepoint 帧根表 + `MYLIS_GC_PRECISE`）、静态根表、非移动两代 GC。后续：MSVC 版 `mylgc.lib`、remembered set 去重/分桶优化。

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

计划：

1. 新 crate `mylgc/`（Rust，`staticlib`），导出上面 4 个符号；
2. 变体 `myrt/gc/gc_own.c`：桥接声明 + 弱引用，或直接用同名符号链接进运行时；
3. `MYLIS_GC=own` 是**默认**，`Makefile` 的 `GC_BACKEND=own`，包内带 `libmylgc.a`/`mylgc.lib`；
4. 语义：精确/保守先二选一；根集合来自栈扫描（保守）或编译器插桩（精确，需要 codegen 配合）。
5. 与 IR 的耦合点：GC 根表由后端生成（C 后端用 `GC_add_roots` 式区域；IR 后端可精确到 SSA 值）。

## 4. 阶段三：IR（保留 C 后端）

参考 Go：前端 → 自研 IR →（可选）优化 → C 后端 / 原生后端。

已落地（MIR v0，详见 [`docs/MIR.md`](MIR.md)）：

- `mylc/src/mir.rs`：函数级 CFG、SSA 值 `%n`、局部槽 `$n`、显式终结指令；
- `mir::lower` 从 AST 生成 MIR；`mir::dump` 输出 `; Mylis MIR v0` 文本；
- `mir::verify` 校验标签/终结指令/值定义；
- `ArtifactKind::Ir`、`backend::IrBackend`、`--emit-ir <path>`。

尚未完成：

- C 后端仍消费 AST，尚未改为消费 MIR；
- `match` 只保留为结构化节点，模式分支/绑定待 lowering；
- 没有 TypeId 表、别名分析、优化 pass；GC safepoint 标注、路径敏感活跃性分析和 LLVM/MIR-C 逐 safepoint 帧根表已完成，精确根模式可选开启。

下一步按 `docs/MIR.md` 的顺序：C 后端消费 MIR → match lowering → pass 框架 → LLVM 优化。

## 5. 变更影响面

| 变更 | 前端 | driver | toolchain | 运行时 |
| --- | --- | --- | --- | --- |
| MSVC shared | 无 | msvc 分支 | 无 | 无 |
| 自研 GC | 无 | 选后端 | 增加归档 | 新增 `gc_own` |
| IR | 需补引用/cast | 选 backend | 不变 | 根表接口 |

结论：前端稳定，改动集中在 `backend/`、`driver` 的编排、`myrt/gc/` 三处。
