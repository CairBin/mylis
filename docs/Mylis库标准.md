# Mylis 库标准

> 状态：`v0.1` 实现基线。**P0 源码包、P1 MIR 库、P2 本地 registry 均已落地**；P2 尚未覆盖远程 HTTP 索引、发布命令与远程缓存。本文区分 **Mylis 原生库**、**Mylis 包/MIR 库** 和 **C FFI 导出库** 三种边界；C FFI 部分由 `mylc --static/--shared` 提供。

## 1. 目标

Mylis 自己的库生态不能建立在 C ABI 上。C ABI 是给其他语言用的边界，不是 Mylis 到 Mylis 的调用约定。

本标准定义：

1. Mylis 源码库如何被发现、导入和链接；
2. `pub` 如何形成包级公开 API；
3. 预编译 Mylis 库使用什么格式；
4. C FFI 导出与 Mylis 原生 API 如何分离；
5. 运行时与 GC 在库场景中的唯一性约束。

## 2. 三层模型

| 层 | 用途 | 传输物 | 是否经过 C ABI |
| --- | --- | --- | --- |
| Mylis 原生 API | Mylis 调 Mylis | `.myl` 源码 | 否 |
| Mylis 包 / MIR 库 | 分发预编译 Mylis 库 | `.mylib`（MIR + 接口元数据） | 否 |
| C FFI 导出 | 给 C/C++/Rust/Go 等外部语言 | `.a` / `.so` / `.dll` + `.h` | 是 |

约束：

- 一个最终 Mylis 程序只有一个 runtime、一个 GC 实例。
- Mylis 原生库不生成 `<stem>_init()`，也不携带独立 runtime。
- C FFI 库的 `<stem>_init()` 只用于外部调用者初始化该库所在的 runtime。
- `@export` 只属于 C FFI 层；`pub` 属于 Mylis 原生 API 层。

## 3. Mylis 原生 API

### 3.1 可见性

- 顶层声明默认包内可见。
- `pub fn`、`pub class`、`pub enum`、`pub interface`、`pub static` 构成包公开 API。
- `pub import { A, B } from "..."` 表示把指定名字再导出给上层包/模块；目标模块本身仍私有，不随名字再导出暴露。
- `pub import "..."` 表示把目标模块以原名再导出给上层：上层可写 `pkg::module::f()`，但不产生裸名。
- 非 `pub` 声明不得被其他包引用。

### 3.2 模块与导入

当前语法：

```mylis
import "mathlib";                  // 只加载包，调用写 mathlib::add(...)
import { add } from "mathlib";     // 选择性导入，可裸写 add(...)
import "mathlib/vector";           // 多段模块，调用写 mathlib::vector::triple(...)
import { triple } from "mathlib/vector";
pub import { add } from "mathlib"; // 再导出给上层
```

解析顺序：

1. 当前包内相对路径；
2. 依赖包名；
3. 工具链标准库；
4. 未找到时报 `MYL` 诊断，不静默回退。

包名与文件路径的映射由 `mylis.toml` 决定，源码中不硬编码依赖目录。
`import "A"` 不把 `A` 的公开符号放进当前裸名作用域，必须写 `A::name`；
只有 `import { name } from "A"` 才允许裸名调用。

### 3.3 调用形式

包内公开项默认使用模块限定名：

```mylis
import "mathlib";
import { Io } from "std/Io";

fn main() -> i32 {
    i32 x = mathlib::add(1, 2);
    Io::PrintlnI32(x);
    return x;
}
```

也可以选择性导入后裸写：

```mylis
import { add } from "mathlib";
import { println_i32 } from "std/Io";

fn main() -> i32 {
    i32 x = add(1, 2);
    println_i32(x);
    return x;
}
```

多段模块路径：

```mylis
import "mathlib/vector";
fn main() -> i32 { return mathlib::vector::triple(4); }
```

同名冲突规则：

- 两个 `import { ADD } from "..."` 导入同名符号 → `MYL011`；
- 同时使用 A、B 的 `ADD` 时，只选择性导入其中一个，另一个必须写 `A::ADD(...)` / `B::ADD(...)`；
- 选择性导入名不得与当前模块本地声明同名（`MYL011`）。

### 3.4 类型身份

同一最终编译单元内：

- 每个 class/enum/interface 有唯一类型身份；
- 跨包传递的对象保持同一 GC 头与类型描述符；
- 不做跨包类型擦除；
- 不使用 C 的不完整类型指针作为 Mylis 内部 API。

这是 Mylis 原生库不采用 C ABI 的核心原因。

## 4. Mylis 包格式（源码优先）

### 4.1 目录

```text
mathlib/
├── mylis.toml
├── mylis.lock
├── src/
│   ├── lib.myl
│   └── vector.myl
└── tests/
```

### 4.2 `mylis.toml`

```toml
[package]
name = "mathlib"
version = "0.1.0"
edition = "2026"
description = "Math helpers for Mylis"
license = "MIT"

[lib]
entry = "src/lib.myl"

[dependencies]
core = { path = "../core" }
```

字段约束：

- `name`：小写、数字、`-`/`_`，全局唯一；
- `version`：语义版本；
- `edition`：语言版本；
- `entry`：包入口，必须存在；
- `dependencies`：路径、git 或 registry 源。

### 4.3 `mylis.lock`

锁定：

- 每个依赖的精确版本；
- 源类型与地址；
- 内容校验和；
- 解析后的依赖图。

锁文件必须提交到版本库；构建不得静默改写锁文件。

### 4.3.1 菱形依赖与版本唯一性

Mylis 原生包在编译期合并为一个 `Program`，因此同一包名**只能解析出一个版本**。

包身份按以下信息判定：

```text
(name, canonical source, version)
```

菱形依赖 `A -> B -> D`、`A -> C -> D` 的处理：

- 两边指向同一个 canonical 包根、版本相同：D 只加载一次；
- 同名但不同路径：报依赖冲突，不加载第二份；
- 同名但版本不同：报依赖冲突；
- 依赖图成环：报循环依赖。

`mylis.lock` 中一个包名只出现一条记录。冲突诊断会打印本次依赖链和已解析路径。

### 4.4 构建行为

编译一个包时：

1. 读取 `mylis.toml`；
2. 解析依赖图并检测循环；
3. 按拓扑顺序加载 `.myl`；
4. 只把 `pub` 项加入跨包符号表；
5. 合并为一个 MIR 程序；
6. 由当前后端生成最终产物。

源码包不产生中间 `.a`，不产生独立 GC。

### 4.5 P0 当前实现状态

已实现：

- `[package]`（兼容旧 `[project]`）读取 `name` / `version`；
- `[lib] entry` 作为包入口；
- `[dependencies]` 支持字符串路径与 `{ path = "..." }`；
- `import "pkg"` 解析依赖包的 `[lib] entry`；
- `import "pkg/sub"` 依次查找 `<dep>/src/sub.myl`、`<dep>/sub.myl`、`<dep>/sub`；
- 依赖从当前模块最近的 `mylis.toml` 向上发现，支持传递依赖；
- 跨文件非 `pub` 声明报 `MYL005`，包边界复用同一规则；
- 首次构建生成 `mylis.lock`（依赖名、版本、路径源、包内容 SHA-256）；已有锁文件不静默改写。

P0 基线边界：

- 包内公开函数既可扁平调用，也支持 `mathlib::add` / `pkg::module::fn` 的限定名；跨包 `pub import` 递归导出已实现。
- 锁文件首次构建生成；`mylc build --update-lock` 显式刷新，源/checksum 不一致会拒绝。
- P0 只要求本地 `path` 依赖；Git、HTTP `.mylib` 与本地 registry 属于 P2，见后文。
- `mylc build` 对只有 `[lib]` 的包会合成一个静态库目标。
- 路径依赖的校验和覆盖该包全部 `.myl` 与 `mylis.toml`；修改依赖源码后用 `--update-lock` 刷新。

## 5. MIR 库格式（预编译分发）

### 5.1 目标

在需要“不提供源码”或“减少重复解析”时，分发 MIR 库，而不是平台原生库。

MIR 是后端无关的，LLVM 与 MIR-C 都可消费；这避免了：

- 每个库带一份 runtime/GC；
- 跨平台对象格式问题；
- 类布局和 vtable 的原生 ABI 承诺；
- 跨后端调用约定差异。

### 5.2 文件布局

`.mylib` 是一个归档包：

```text
mathlib-0.1.0.mylib
├── manifest.toml
├── api.mir          # 公开接口与类型签名
├── impl.mir         # 函数实现
└── checksums.toml
```

> 实现说明：P1 当前实现采用**单文件文本归档** `MYLIB 1`，用 `--- manifest/--- api/--- mir` 分段；`api.mir`/`impl.mir` 拆成独立文件、压缩归档和反序列化仍属后续迭代。单文件归档不携带 runtime/GC，符合本节边界。

泛型声明在 API 段导出模板签名（`pub class Box<T>`、`pub fn identity<T>`），
在 MIR 段导出库内已实例化的具体声明（`Box__i64`、`identity__i64`）。
具体实例名由类型实参稳定生成，消费端按同一规则实例化后链接到库内实现。

### 5.3 `manifest.toml`

```toml
[mylis]
abi = "mylis-1"
compiler_min = "0.1.0"

[package]
name = "mathlib"
version = "0.1.0"
edition = "2026"

[lib]
api = "api.mir"
impl = "impl.mir"
```

### 5.4 版本与校验

- `abi` 是 MIR 库 ABI，不是 C ABI；
- `compiler_min` 是能消费该 MIR 的最低编译器版本；
- 每个文件带 SHA-256；
- 编译器在加载时校验 ABI、版本、校验和；
- 不兼容时直接报错，不做静默降级。

### 5.5 命令

目标命令：

```bash
mylc --emit-mylib mathlib -o mathlib-0.1.0.mylib
mylc --link-mylib mathlib-0.1.0.mylib app.myl -o app
```

约束：

- `--link-mylib` 只接受与当前 `mylis-abi` 匹配的包；
- MIR 库不得携带 runtime/GC 二进制；
- 最终程序仍只初始化一次 runtime/GC；
- 后端在链接 MIR 后再选择，不在库内固定。

## 6. C FFI 导出层

### 6.1 定位

C FFI 层用于其他语言调用 Mylis，不用于 Mylis 调 Mylis。

### 6.2 命令命名

推荐：

```bash
mylc --ffi-static mathlib.myl -o libmathlib
mylc --ffi-shared mathlib.myl -o libmathlib
```

兼容旧命令：

```bash
mylc --static ...
mylc --shared ...
```

旧命令保留为别名，但文档必须标注为 FFI 导出。

### 6.3 导出规则

- 只有顶层 `pub fn` + `@export("c_name")` 进入头文件；
- 生成 `<stem>_init()`；
- 类/方法/构造器不直接导出，通过工厂与访问函数包装；
- `Option`/`Result` 只支持有限返回场景；
- enum 导出、`Option/Result` 参数仍受限制；
- 头文件依赖 `mylrt.h`；
- 第三方负责把长期持有的 Mylis 指针登记为 GC 根；
- 单线程 runtime，第三方多线程调用必须串行化。

### 6.4 与原生 API 的边界

同一份源码可以同时拥有：

```mylis
pub fn add(i32 a, i32 b) -> i32 { ... }          // Mylis 原生 API

@export("math_add")
pub fn c_add(i32 a, i32 b) -> i32 { return add(a, b); }  // C FFI
```

两者不是同一个契约：

- `pub fn` 可被 Mylis 直接调用，保留完整类型信息；
- `@export` 函数只承诺 C 可表达的类型与调用约定。

## 7. 运行时与 GC 规则

1. 一个最终程序只有一个 `myrt_init`。
2. 一个最终程序只有一个 GC 堆。
3. Mylis 源码包和 MIR 包不得嵌入第二份 runtime/GC。
4. C FFI 动态库若嵌入 runtime，必须明确其边界；多个这样的库共存时不得假设共享 GC。
5. 跨 FFI 边界的 Mylis 对象由 GC 管理，外部语言不得 `free`。
6. 精确模式下，C 侧长期引用必须通过 `myrt_gc_add_root_range` 登记。
7. 当前 runtime 单线程；并发调用由宿主串行化。

## 8. 版本策略

| 对象 | 版本字段 | 说明 |
| --- | --- | --- |
| 语言 | `edition` | 语法/语义版本 |
| 包 | `version` | 语义版本 |
| MIR 库 | `abi = "mylis-1"` | MIR 接口/序列化 ABI |
| 编译器 | `compiler_min` | 最低消费版本 |
| C FFI | 头文件 + 平台 ABI | 只承诺 C 可表达类型 |

`mylis-1` 的第一条规则：

- MIR 序列化字段只增不改；
- 删除字段必须升 `mylis-2`；
- 类型布局变化必须升 ABI；
- 包版本升级不自动意味着 ABI 升级。

## 9. 实施顺序

### P0：源码包标准

- `mylis.toml` 解析；
- 包名 import；
- `pub` 跨包可见性；
- 依赖图与循环检测；
- 锁文件；
- 仍不生成 C ABI。

### P1：MIR 库

- MIR 序列化/反序列化；
- `api.mir` / `impl.mir`；
- `mylis-1` 校验；
- `--emit-mylib` / `--link-mylib`。

P1 当前实现已提供 `--emit-mylib` 和 `--link-mylib`：

- 归档包含 `mylis-1` ABI、包名/版本、`entry_stem`、`decl_count`、API 声明、MIR 文本、MIR JSON、跨包声明引用表和 SHA-256；`--- refs` 用 `f <id> <限定名>` / `t <id> <限定名>` 记录 MIR 引用到的外部函数与类型，并把本包调用到的外部具体实例（如 std `Map<string, i64>`）按闭包带入归档，消费端按限定名复用或追加，避免 `base + id` 串到别的声明；可选 `--- native-static <文件名>` / `--- native-shared <文件名>` 段保存原生库的 Base64 载荷，checksum 覆盖这些载荷；同一输入重复生成的归档字节一致（`refs` 与 safepoint roots 均按 id 排序）。
- API 声明支持 `pub fn`、`pub class`、`pub enum`、`pub interface`、`pub type`：类字段、构造器、公开方法、枚举变体、接口方法都会写入归档；API 按模块分为 `--- api lib`、`--- api mathlib::vector`、`--- api mathlib::vector::A` 等段；根包子模块段也带包名前缀，依赖包段用 `dep::...`；
- `--link-mylib` 在 sema 前注入无体声明（函数和类型）；无原生载荷时在 lower 后用 MIR JSON 合并函数与类型表，并把库 decl id 对齐到 API 声明 id；有原生载荷时跳过 MIR 合并，直接链接归档中的原生库；
- MIR 符号按包名 mangling：包入口模块使用 `<包名>::<名字>`，包子模块使用 `<包名>::<模块>::<名字>`，std 保持模块名；同名 `lib` 包不再冲突；
- 归档会带上根包及传递依赖包的 MIR（不含 std），因此库内部调用依赖包函数也能在最终程序里解析；
- LLVM/MIR-C/IR 后端支持可执行产物；静态/动态库导出需要带原生载荷的 `.mylib`，并用 C 后端消费；
- ABI 或校验和不匹配时拒绝加载。

当前限制：

- API 按包内模块分段；根 API 是 `--- api lib`，根包子模块是 `--- api 包名::vector`、`--- api 包名::vector::A`；依赖包 API 用 `dep::lib`、`dep::vector::A` 这种包名限定段；
- 归档仍是单文件文本，`api.mir` / `impl.mir` 独立文件和压缩归档留后。
- 原生载荷按 **API 声明名** 提供 C ABI 符号。第三方 C 库必须导出归档 API 中的顶层函数名；用 Mylis 自己生成时，在源函数上加 `@export("answer")` 让 `mylc --static` / `--shared` 导出稳定符号，再嵌入归档。例如 `@export("answer") pub fn answer() -> i32 { ... }`。
- 带原生载荷的归档可由 `--backend mir`（C 后端）或 `--backend llvm` 消费；LLVM 后端为 API 顶层函数生成外部 `declare`，不生成空定义，并直接链接原生库。动态库载荷在 Linux/macOS 通过 `-Wl,-rpath` 指向提取目录；Windows 支持与 API 名一致的 `.lib` 导入库（MSVC）或 `.a` / `.dll.a`（MinGW）。

接口 API 已端到端验证：库返回 class，应用声明为 interface，链接后能正确装箱并动态派遣到库内方法。

### P2：包注册与分发

- registry 索引；
- 校验和；
- 版本解析；
- 私有 registry。

P2 当前实现已提供本地 registry：

- 依赖写法支持 `name = { version = "^0.1.0", registry = "default" }`；
- 版本约束支持 `*`、`^`、`~`、`=`，按 SemVer 选择满足约束的最高版本；
- registry 默认根 `~/.mylis/registry`，布局 `<registry>/<包名>/<版本>/mylis.toml`；支持 `MYLIS_REGISTRY` 和 `MYLIS_REGISTRY_<NAME>`；
- 支持 Git 依赖：`name = { git = "URL", tag/rev/branch = ... }`，克隆到 `~/.mylis/git`，锁文件记录 `git+URL#commit`；不发布、不依赖公共 registry；
- 支持 `.mylib` 依赖：`name = { mylib = "URL 或本地路径", checksum = "..." }`；归档下载到 `~/.mylis/mylib`，锁文件记录 `mylib+URL` 与 SHA-256；
- `mylis.lock` 记录 registry/path 源的版本与 checksum；锁存在时强制按锁解析，版本/源/checksum 不一致都会报错；
- `--update-lock` 显式重新解析并重写锁文件。

当前限制：registry 还是文件系统布局，没有 HTTP 索引、发布命令和远程缓存；P2 完整版再补。

### P3：原生 Mylis ABI（可选）

只有在明确需要闭源原生库、且 MIR 分发不能满足性能/保护需求时才做：

- 稳定符号 mangling；
- 类型 ID 与 GC 描述符；
- 原生调用约定；
- 跨 runtime 版本校验。

在此之前，不承诺原生 Mylis ABI。

## 10. 验收标准

P0 完成时：

- 两个本地包可以互相 `import`；
- 非 `pub` 项跨包不可见；
- 循环依赖报错；
- 锁文件可复现构建；
- 最终程序只有一个 runtime/GC。

P1 当前验收：

- `.mylib` 可被 LLVM 与 MIR-C 两种后端消费；
- ABI/版本/校验和不匹配时拒绝加载；
- 加载 MIR 库后不出现第二份 runtime/GC；
- `@export` 与 `pub` 的边界测试通过。

P2 当前验收：

- 可从本地 registry 解析依赖；
- 可复现锁定与校验；
- 离线构建可用缓存。
- 尚未覆盖远程 HTTP 索引、发布命令与远程缓存。
