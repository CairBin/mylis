# MIR v0：中间表示与 `--emit-ir`

MIR 是 `mylc` 前端与后端之间的第一层显式 IR。目标是让 sema 之后的程序不再直接交给
C codegen，而是先降到一层与具体 C 方言无关的函数级 CFG；C 后端和将来的 LLVM 后端
消费同一份 MIR。

当前状态：**MIR 已定义并可从 AST lowering、可 dump、可结构校验，C 后端仍走原有 AST 路径**。
也就是说 `--emit-ir` 已经闭环，但“C 后端改为消费 MIR”是下一步，不是本轮。

## 1. 管线

```text
lexer -> parser -> program(loader) -> sema
                                      |
                                      v
                                   mir::lower
                                      |
                     +----------------+----------------+
                     |                                 |
                 mir::dump (文本)                (下一步) C / LLVM
```

`--emit-ir` 不检测 C 工具链、不依赖 GC，sema 通过即可产出：

```bash
mylc --emit-ir out.myl.ir app.myl
mylc --emit-ir out.myl.ir app.myl --verbose
```

`--verbose` 会打印函数数、基本块数和 MIR verifier 的错误数。

## 2. 结构

```rust
struct MirProgram {
    funcs: Vec<MirFunction>,
    classes: Vec<MirClass>,
}

struct MirFunction {
    name: String,          // module::Class::method / module::fn
    params: Vec<MirLocal>,
    locals: Vec<MirLocal>, // 参数 + 局部变量
    ret: Type,
    blocks: Vec<MirBlock>,
}

struct MirBlock {
    label: String,
    insts: Vec<MirInst>,
    term: Option<MirTerm>,
}
```

- 值：`%n`，SSA 形式，每条产生值的指令分配一个编号；
- 局部：`$n`，显式内存槽；`store $n, %v` 写入，读取时直接引用 `$n`；
- 参数放在 `locals[0..]`，方法额外插入 `$0 this`；
- 函数名保留模块与所属类，避免跨模块同名歧义。

终结指令：

```text
ret %v | ret void
goto bbN
br %cond, bbT, bbF
unreachable
```

## 3. 指令集（v0）

包调用在 MIR 中保留完整限定名：`A::ADD(...)` 降低为 `call @A::ADD(...)`，`mathlib::vector::triple(...)` 降低为 `call @mathlib::vector::triple(...)`。C 后端按 `funcs[].name` 和叶子名 `call_map` 查找实际符号；LLVM 后端直接用限定名 mangling。

| 类别 | 指令 |
| --- | --- |
| 常量/占位 | `const`, `undef` |
| 局部 | `store $n, %v` |
| 算术/逻辑 | `add sub mul div rem eq ne lt le gt ge and or bitand bitor bitxor shl shr` |
| 一元 | `neg not bitnot load addr` |
| 调用 | `call @fn(args...)`, `call %recv.name(args...)`, `call @A::name(...)` / `call @A::sub::name(...)` |
| 对象 | `new @Class(args...)`, `field`, `store.field` |
| 数组 | `index`, `store.index`, `array.new`, `array.lit` |
| 转换/控制 | `cast`, `try` |
| 字符串 | `interp "lit", %v, ...` |
| 名称 | `global @name`, `static @Class::name` |
| 模式 | `match.test %subject, Pattern`, `bind %subject, Pattern, [$n...]` |
| GC | `safepoint roots=[$n, %v, ...]`（调用/分配点前） |
| 未展开 | `opaque "..."`（异常/尚未覆盖的赋值目标） |

`match` 已展开为 CFG：每条臂生成 `match.test`，命中后 `bind` 把载荷写进局部槽，
再进入臂体；未命中落到下一条臂，所有臂都结束后是显式 `unreachable`。
绑定槽的类型暂为 `Infer`，等 sema 把模式载荷类型带进 MIR 后再补精确类型。

## 4. 示例

输入：

```myl
fn add(i32 a, i32 b) -> i32 {
    return a + b;
}

pub fn main() -> i32 {
    i32 x = 3;
    if (x > 2) { x = add(x, 1); }
    return x;
}
```

输出（节选）：

```text
fn @app::add(
  param $0 a: i32,
  param $1 b: i32
) -> i32 {
  locals {
    $0 a: i32 (const)
    $1 b: i32 (const)
  }
  bb0:
    %1 = add $0, $1; line 2
    ret %1
}

fn @app::main() -> i32 {
  locals {
    $0 x: i32
  }
  bb0:
    %1 = const 3; line 6
    store $0, %1; line 6
    %2 = const 2
    %3 = gt $0, %2
    br %3, if.then1, if.else2
  if.then1:
    %3 = const 1
    %4 = call @add($0, %3)
    store $0, %4
    goto if.end3
  if.else2:
    goto if.end3
  if.end3:
    ret $0
}
```

实际文本以 `mylc --emit-ir` 输出为准；立即数也会先落成 SSA 值。

## 5. 不变量与 verifier

`mir::verify` 检查：

- 块标签唯一；
- 每条 `goto/br` 的目标存在；
- 每个块有终结指令；
- 值 `%n` 在使用前已定义（`%0` 作为 void/undef 占位保留）；
- 局部 `$n` 在 `locals` 范围内。

`--verbose` 会输出 `verify_errors=N`。当前所有正向用例为 0。

## 6. 已知边界（v0）

- 模式绑定的类型仍是 `Infer`（载荷类型未从 sema 带到 MIR）；
- `opaque` 仅用于尚未 lowering 的赋值目标等异常路径；
- 没有显式 `TypeId` 表，类型仍用 `ast::Type`；
- 没有别名分析/借用检查/优化 pass；
- 默认后端已切换为 LLVM；`--backend mir` 显式 MIR-C，`--backend ast` / `--backend c` 回退 AST-C；
- GC 安全点已在 MIR 标注（`safepoint roots=[...]`）；roots 由反向活跃性
  数据流分析得到，只保留在安全点之后仍活跃的引用槽/值。LLVM 与 MIR-C 后端
  为每个 safepoint 生成当前 roots 的帧根表快照（`frame_set_roots`），运行时可用
  `MYLIS_GC_PRECISE=1` 只扫描显式根与帧表。

## 7. MIR-C 后端

默认选择 `backend::LlvmBackend`；`--backend mir` 选择 `backend::MirCBackend`，`--backend ast` / `--backend c` 回退 AST。MIR-C 是“C 后端消费 MIR”的可执行实现，
不是完整替代。当前支持：

- 基本类型（i8..u64/f32/f64/bool）的参数、局部、返回值；
- 常量、局部 `store`、算术/比较/位运算、一元运算；
- `if/while/for` 生成的 `goto`/条件分支；
- 直接函数调用（按 MIR 全名或源名解析）；
- 数值/指针 `cast`；类/枚举→接口 `cast` 生成 `{ .obj, .vt }`；接口 `cast` 分三类：接口→类（比对 vtable 的 `type_id`）、
  接口→接口（走适配函数 `A_from_to`）、接口→基本类型（比对隐藏装箱 id 后拆箱）；
  `as` 失败调用 `myrt_panic`，`as?` 失败返回 `Option<T>` 的 `None`；
- 基本类：结构体布局、字段读写（`Res::Field` 的 class id + 下标）、
  构造器调用（`Res::Ctor` + `is_ctor`）、方法调用（`Res::Method` → 声明 id）；
- 字符串：字面量 `mylrt_str_lit`、`==/!=` 走 `mylrt_str_eq`、插值走
  `mylrt_str_concat` + 运行时数字转字符串；
- 数组：按 `{myrt_u64 len; T items[]}` 生成元素类型结构，`mylrt_arr_alloc`、
  `mylrt_arr_index`，引用元素写屏障；`.len()` 读首字段；
- 内置 `StrLen` / `ArrLen`；
- 枚举：`struct E_x { i32 tag; void* data; }` + 每变体 payload 结构 +
  静态构造器；`MatchTest` 比较 tag，`BindPattern` 从 payload 绑定槽；
- 接口：`struct I_x { void* obj; const Vt* vt; }`；每个 vtable 首字段是
  `myrt_i32 type_id`，按实现组合生成 `VT_<type>_<iface>`；接口方法走
  vtable，类/枚举到接口的 `coerce` 转成 `{ .obj, .vt }`；
- 空接口 `Any` 的装箱：`i8..u64/f32/f64/bool/string` 可隐式或显式 `as Any`
  装箱，生成 `MylBox_<key>{ v }`、`myl_box_<key>`/`myl_unbox_<key>` 和
  `VT_BOX_<key>_<iid>`；装箱类型 id 用负数（i8=-1 … str=-12），
  因此 `Any as i32` 可直接按动态类型拆箱；非空接口不做隐式装箱；
- 数组插值生成 `AS_<elem>` 辅助函数；接口/类/枚的 `to_string` 可用于插值；
- 静态字段生成 `S_<decl>_<idx>` 全局，启动时写入静态根表 `myl_static_roots` 并写入字面量初值；
- `Result`/`Option` 生成 `MylR_*` / `MylO_*` 结构；类型键递归编码
  （`Option<Option<i32>>` → `MylO_oi32`），并按依赖顺序先声明内层结构；
  `Option<Iface>` 使用专用
  `{ is_some; void* obj; const void* vt; }` 布局，绕开 `union` 对完整接口
  结构体的依赖，`Some/None`、`unwrap`、`unwrap_or`、`match` 绑定均已支持；
  `Result` 的 Ok/Err 任一为接口时也生成专用布局，`?` 会按目标布局重打包错误值；
- `Ok/Err/Some/None`、
  `unwrap`/`unwrap_or`/`is_ok`/`is_err`/`is_some`/`is_none`、`?` 和
  `Ok/Err/Some/None` 模式绑定均已支持；
- 接口组合：按具体实现生成目标接口的 vtable，并用适配函数
  `A_<from>_<to>` 选择，不做 vt 指向前缀强转；
- 原生指针：`T*` → `void*`，`load`/`store` 生成解引用；外部 C 声明
  直接使用源码符号名（如 `strlen`）。

当前 `tests/positive` 下的程序已全部能在 MIR-C 下生成、编译并与 AST 后端
输出一致；59 项正向/反向用例在 boehm/own/simple 三种 GC 下均通过。
AST-C 也已补齐 `Any` 装箱、接口 cast、`Option<Iface>`、`Result<Iface>` 和
`Option`/`Result` 的 `?` 传播；对拍不再跳过这些用例。接口→接口适配器按
“所有具体类型 × 所有接口”生成，不依赖源码里是否显式发生过转换。
库模式（`--static` / `--shared`）也由 MIR-C 生成：按 `@export` 生成导出
包装函数、`<stem>_init()`、头文件与 MSVC 的 `__declspec(dllexport)`。
遇到这些结构会生成 `#error`，不会静默产出错误代码。

```bash
mylc --backend mir --emit-c out.c app.myl
```

后端对拍脚本 `tests/test_backend_parity.sh` 对每个正向程序分别用 MIR-C 与
AST-C 编译，比较 stdout 与退出码。

验收脚本 `tests/test_mir_c.sh` 覆盖多个程序：带循环/函数调用的退出码 10、
带字段/构造器/方法的基本类退出码 6、字符串插值 + 字符串/整数数组输出、
枚举/接口/变体载荷/match/数组插值、静态字段/方法、Result/Option/`?`、
接口组合、原生指针、外部 C 符号。

## 8. 下一步

1. **补 MIR 语义信息**（已完成）：`Expr.res` / `coerce`、类字段下标、方法/构造器 id 已进 MIR；
2. **扩展 MIR-C**：对象/字符串/数组/方法/接口，覆盖全部正向程序；
3. **切换默认后端**：默认走 LLVM；`--backend mir` 走 MIR-C，`--backend ast` 保留回退；
4. **库模式迁到 MIR-C**：已完成（静态库、动态库、头文件、DLL 导出）；
5. **长期**：MIR-C/AST-C 作为显式可选路径保留，默认稳定在 LLVM；
2. **match lowering**：tag 测试、绑定槽、臂作用域、穷尽性交给 sema；
3. **MIR pass 框架**：`mir/pass.rs` 已做入口可达性分析、删除不可达块、
   整数/布尔/`f64` 常量折叠、纯指令 DCE，以及小函数内联（单函数 ≤4 块/24 指令、
   标量 ABI、非递归）；内联生成 `copy` 指令承接返回值；
4. **GC safepoint 标注、精确根与活跃性分析**（已完成）：调用点与分配点生成
   `safepoint roots=[...]`；反向活跃性分析裁剪 roots；LLVM 与 MIR-C 生成逐
   safepoint 帧根表，支持 `MYLIS_GC_PRECISE=1`；
5. **LLVM 后端**：`backend/llvm.rs` 直接消费 MIR，不再碰 AST。
   已支持标量类型、局部槽、算术/比较/分支、直接函数调用、数字转换、字符串字面量/
   拼接/相等/插值、类与对象（字段、实例/静态方法、静态字段 `@S_*`）、数组
   （`ArrayNew`/`ArrayLit`/索引读写/`len`/格式化）、Option/Result 的构造、`?`、
   `unwrap`/`unwrap_or`/`is_ok`/`is_err`/`is_some`/`is_none`，enum 的 tag+payload
   聚合、变体构造/方法、模式测试与绑定，接口值 `{obj, vtable}`、class/enum 装箱、
   接口方法动态派发、接口格式化、接口到接口适配，以及标量/字符串装箱与 `as`/`as?`
   拆箱，`Option<Iface>`/`Result<Iface>`，以及 Result/Option 模式测试与绑定；`--emit-llvm` 输出 `.ll`，`--backend llvm` 交给 clang
   （LLVM 15+，opaque pointers）链接 myrt 产出可执行。
   泛型在 sema 阶段完成单态化，后端只消费具体声明，没有运行时泛型 lowering 需求；库模式已生成 `_init` 与导出包装函数，
   遇到会明确报错，不回退 C。
   验收脚本 `tests/test_llvm.sh` 覆盖 Arith、Loops、Basics、Print、GcAuto、Objects、
   Arrays、Const、Ctor2、Account、Registry、Results、NestedOption、Match、Enums、Enum 与 `.ll` 解析。

## 9. MIR 库 `.mylib`（P1 第一版）

`mylc --emit-mylib <path>` 生成单文件文本归档：

```text
MYLIB 1
abi = "mylis-1"
name = "mathlib"
version = "0.1.0"
checksum = "sha256-..."
--- manifest
...
--- api
fn add 0(a: i32, b: i32) -> i32
--- mir
; Mylis MIR v0
fn @lib::add(...)
```

当前实现边界：

- 收录根包及传递依赖包的模块，不把 std 重复打进 `.mylib`；
- API 段生成可解析的无体 `pub fn`、`pub class`、`pub enum`、`pub interface` 声明，链接时作为虚拟模块注入 sema；
- MIR 段同时保留文本 dump 和 `mir-json`，后者用于还原 `MirProgram`；
- 符号按包名 mangling：入口 `<包名>::<名字>`，包子模块 `<包名>::<模块>::<名字>`；
- `--link-mylib` 已接入：函数 decl id 与类型 decl id 都对齐到 API 声明，类字段/构造器/方法、枚举变体、接口方法同步重映射；合并后由 LLVM/MIR-C/IR 后端直接发射；
- `checksum` 覆盖 API、MIR dump 与 MIR JSON，ABI 固定为 `mylis-1`；
- 接口 API 已接入：接口方法 decl id 按 `<接口名>::<方法名>` 对齐，class->interface 装箱和动态派遣跨 `.mylib` 可运行；
- 静态/动态库导出暂不支持链接。
