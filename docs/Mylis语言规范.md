# Mylis 编程语言 —— 语言规范（100% 完整版）

> **Μυλίς, ἁ καθαρή καὶ τάξις** —— 清澄极简，秩序永存
>
> 本文档是《Mylis 编程语言｜从零开发 100% 定稿交接文档》的**补全定稿版**：
> 原文档第 4、5、7、8、9、10、11、12、14 章的空白处已全部写实，且每一条规则都与仓库中的
> `mylc`（Rust 编译器）与 `mylrt`（Rust 运行时，C ABI 保持不变）实现 **1:1 对应**。凡本文档规定的行为，均可由
> `make test` 的用例验证。

---

## 0. 项目终极定案

| 项 | 定案 |
| --- | --- |
| 语言名称 | **Mylis** |
| 寓意 | 古希腊清灵宁芙，极简、纯净、秩序、克制、优雅 |
| 源文件后缀 | `.myl` |
| 编译器 | `mylc`（Rust 实现；默认 LLVM 后端，MIR-C / AST-C 可回退） |
| 运行时 | `mylrt-rs`（Rust 实现，默认自研 GC；C ABI 见 `myrt/mylrt.h`） |
| 可执行产物 | 原生 ELF / PE（默认 LLVM IR → clang；也可 MIR-C/AST-C → C11 → CC），链接运行时与所选 GC 后端 |

**核心定位**：C 级性能 + Rust 安全错误体系 + Java 工程结构 + Go 组合模型 + 纯净可控底层 + 用户可扩展注解。

**无魔法、无隐式行为、无冗余语法、无历史包袱、完全自洽。**

---

## 1. 永久固定红线（绝对禁止实现）

以下功能**永远不做**。编译器不提供、语法不预留、后端不隐藏：

| 编号 | 禁止项 | 后果 |
| --- | --- | --- |
| R1 | 无 `let` 变量 | 全程 C 风格类型前置声明 |
| R2 | 无类型推导 | 所有变量/参数/返回值必须手写类型 |
| R3 | 无运行时泛型、无类型擦除 | 泛型在 sema 单态化，实例以具体类型进入 MIR；不做 `Any` 装箱，不保留运行期类型参数 |
| R4 | 无函数重载 | 同文件同名函数直接报错 |
| R5 | 无默认参数 | 实参数量必须与形参完全一致 |
| R6 | 无类继承、无多态继承 | 只有组合 + 接口鸭子类型 |
| R7 | 无 `extern` 关键字 | C 符号通过 `*_bind.myl` 中的无体 `unsafe fn` 声明 |
| R8 | 无 try/catch 异常 | 只有 `Result` / `Option` / `panic` |
| R9 | 无 `protected` 权限 | 只有 `pub` 与默认私有 |
| R10 | 无隐式拷贝 | class 实例永远引用语义 |
| R11 | 无隐式类型转换 | 转换必须显式 `as`，字面量除外（见 4.4） |
| R12 | 无语法糖污染 | 唯一语法糖是 `?`（第 6 章） |
| R13 | 无闭包捕获 | 只有顶层函数、类方法、C 绑定函数三种函数 |
| R14 | 无内置预处理器 | 无 `#define`、无宏、无条件编译 |

---

## 2. 顶层语言设计规范

### 2.1 代码组织规范（强制工程化）

1. **单文件单主类规则**
   - 一个 `.myl` 文件**最多一个** `pub class`；
   - `pub class` 名称**必须完全匹配文件名**（不含后缀）：`File.myl` → `pub class File`；
   - 违反分别报 `MYL001` / `MYL002`。
2. **非 pub 类、接口、函数**可自由存在于本文件（数量不限）。
3. **唯一豁免文件：`*_bind.myl`**
   - 不要求主类；
   - 不限顶层项数量；
   - 专门存放所有 C FFI 绑定；
   - 业务文件禁止出现任何 C 绑定声明（`MYL008`）。

> 说明：文件可以只有 `pub fn`（例如 `Shape.myl` 中只有 `pub interface Shape`），
> “单主类”约束只在**声明了 `pub class`** 时生效。

### 2.2 可见性系统（仅两级）

| 修饰 | 语义 |
| --- | --- |
| `pub` | 跨文件可见（跨模块解析时可见） |
| 默认（无修饰） | 仅本文件（本模块）可见 |

可修饰：`class`、`interface`、字段、方法、顶层函数。**没有第三种权限**。
跨文件访问非 `pub` 实体报 `MYL005`。

### 2.3 复用模型（Go 式纯净组合）

- 只有**组合 + 接口鸭子类型**；
- 接口**无需实现声明**，没有 `impl` 关键字；
- **方法签名匹配 = 自动实现接口**；
- 完全无继承，无 `extends` / `super` / 虚基类。

### 2.4 错误模型（Rust 式强制安全）

- 内置 `Result<T, E>` / `Option<T>`；
- 强制显式错误处理：`Result` 不做任何隐式解包；
- 支持 `?` 错误传播（第 6 章）；
- 禁止裸错误（没有“抛出任意值”的通道）、禁止异常流；
- 无法恢复的错误使用 `panic`（运行时 `myrt_panic`，进程 abort 并打印 `panic: …`）。

### 2.5 内存模型

- **GC：默认自研分代 mark-sweep**（`mylgc`，Rust staticlib；`MYLIS_GC=own`）；可选 `boehm` 或 `simple`（不回收），统一走 `myrt_gc_backend_*` C ABI；
- `new` → GC 堆对象，**引用语义、无拷贝**；
- 值类型（`i8..u64`、`f32/f64`、`bool`、`Result`、`Option`）→ 栈分配；
- `string` → GC 托管、不可变、驻留（相同字面量共享同一对象）；
- C 长期持有 Mylis 对象必须手动 `myrt_register_root`（第 11 章）；
- GC 默认保守扫描当前线程栈与已注册静态根区间；LLVM/MIR-C 在 `MYLIS_GC_PRECISE=1` 时可只扫描显式根与 safepoint 帧根表。

### 2.6 安全模型

- 默认全程安全：安全代码无法触碰裸指针、无法直接调用 C、无法操作 GC 根；
- 裸指针、C 调用、原始内存操作**必须**在 `unsafe` 域内（第 10 章）；
- 安全代码若能间接获得裸指针，则无法解引用（`MYL007`）。

---

## 3. 完整类型系统（固定不可改）

### 3.1 栈值类型

| Mylis | 位宽 | C 表示 | 取值 |
| --- | --- | --- | --- |
| `i8` | 8 | `int8_t` | −128 … 127 |
| `i16` | 16 | `int16_t` | −32768 … 32767 |
| `i32` | 32 | `int32_t` | −2^31 … 2^31−1 |
| `i64` | 64 | `int64_t` | −2^63 … 2^63−1 |
| `u8` | 8 | `uint8_t` | 0 … 255 |
| `u16` | 16 | `uint16_t` | 0 … 65535 |
| `u32` | 32 | `uint32_t` | 0 … 2^32−1 |
| `u64` | 64 | `uint64_t` | 0 … 2^64−1 |
| `f32` | 32 | `float` | IEEE-754 binary32 |
| `f64` | 64 | `double` | IEEE-754 binary64 |
| `bool` | 1 | `bool`（`<stdbool.h>`） | `true` / `false` |

### 3.2 GC 托管堆类型

| Mylis | C 表示 | 说明 |
| --- | --- | --- |
| `string` | `MyStr*` | UTF-8 字节串，`len` 为字节长度，不可变，引用语义 |

### 3.3 语义内置类型

| Mylis | C 表示 | 语义 |
| --- | --- | --- |
| `Result<T, E>` | 生成 `MylR_<mangle>` 结构体 | `is_ok` 标志 + `union { T ok; E err; }`，值语义 |
| `Option<T>` | 生成 `MylO_<mangle>` 结构体 | `is_some` 标志 + `union { T some; }`，值语义 |

`T` / `E` 必须是值类型：不允许 `Result<void, E>`、`Result<T*, E>` 作为载荷被声明
（裸指针必须放在 `unsafe` 语义之外，避免安全代码持有裸指针）。`string` 与 class 类型可以作载荷。

### 3.4 指针类型

| Mylis | C 表示 | 约束 |
| --- | --- | --- |
| `T*` | `T*`（`i8*` → `char*`，`u8*` → `unsigned char*`） | **仅 unsafe 可用** |

指针可以多层：`i8**`。`void*` 允许出现（C 绑定时使用）。

### 3.5 用户自定义类型

| Mylis | C 表示 |
| --- | --- |
| `class Name` | `C_<前缀>_<Name>*`（永远是指针/引用） |
| `interface Name` | `I_<前缀>_<Name>`（值结构 `{ void* obj; const Vt* vt; }`） |
| `enum Name { ... }` | `E_<前缀>_<Name>*`（堆对象 `{ int32_t tag; void* data; }`，`data` 指向变体载荷 `P_<前缀>_<Name>_<Variant>`） |

### 3.6 数组类型（v0.2）

| Mylis | C 表示 | 说明 |
| --- | --- | --- |
| `T[]` | `MylA_<mangle>*` | GC 堆数组，头 8 字节存长度 `uint64_t len`，其后柔性数组存元素；引用语义 |

`[]` 是类型后缀，可叠加：`i32[][]` 是「元素为 `i32[]` 的数组」。元素类型可以是任意值类型、
`string`、class / enum（存对象指针）、`Result` / `Option`。完整规则见 **§17.1**。

### 3.7 函数返回类型

`void` 仅用于返回位置；`void` 不能作为变量类型、字段类型、`Result` 载荷。

---

## 4. 变量声明语法（强制 C 风格）

### 4.1 语法

```ebnf
var_decl  ::= ["const"] type IDENT "=" expr ";"
```

- **禁止** `let`（R1）；
- **禁止**未初始化变量（R2、`MYL003`）：语法上允许写出 `type name;`，但语义阶段必定报错，
  因此等价于必须初始化；
- 声明即定义，没有“先声明后赋值”的路径。

### 4.1.1 `const` 限定符

`const` 表示**不可变绑定**（不是编译期常量表达式，也不做常量折叠）：

```mylis
const i32 base = 7;
const string tag = "v1";

class Point {
    const i32 id;          // 必须在构造器内赋值一次
    const i32 scale = 2;   // 或在声明处初始化（二者只能选一个）
    pub fn init(i32 id) { this.id = id; }
}

fn boost(const i32 v, i32 bonus) -> i32 {   // 参数只读
    return v * 10 + bonus;
}
```

规则：

| 规则 | 违反 |
| --- | --- |
| 局部 `const` 必须初始化（与普通局部变量一致） | `MYL003` |
| 给 `const` 局部变量或参数重新赋值 | `MYL015` |
| `const` 字段只能在**所属类的构造器**中赋值，且只能赋一次 | `MYL015` |
| `const` 字段既未在声明处初始化、构造器里也没赋值 | `MYL016` |
| `const` 字段已在声明处初始化，构造器里又赋值 | `MYL015` |

codegen 对局部变量与参数额外发出 C 的 `const`，让 C 编译器再挡一层；字段不加 C `const`
（构造器内赋值在 C 里会成为 UB），由语义阶段保证。

### 4.1.2 顶层编译期常量 `pub const`

顶层常量是**编译期字面量替换**，与局部 `const`（不可变绑定）不同：

```mylis
pub const f64 PI = 3.14159;
pub const i32 NEG = -2;
pub const string TAG = "core";

fn main() -> i32 {
    return NEG;
}
```

规则：

| 规则 | 说明 |
| --- | --- |
| 只能出现在顶层 | 类内静态常量使用 `pub static const`（见 §17.4） |
| 初值必须是字面量 | `INT` / `FLOAT` / `STRING` / `true` / `false` / `null`，可带一元负号 |
| 不支持常量表达式 | 不能引用其他常量或做算术；需要计算时写成普通函数 |
| 导入 | 支持 `import { PI } from "m.myl"` 与限定名 `m::PI` |
| 生命周期 | 无运行时存储；使用处直接替换为字面量 |

### 4.1.3 类型重命名 `type A = B`

```mylis
type Id = i32;
pub type Name = string;
type Ids = Id[];          // 别名可嵌套，也可以被数组/指针修饰

fn show(Id v) -> Name { return "id={v}"; }
```

规则：

| 规则 | 说明 |
| --- | --- |
| 纯别名 | 不产生新类型，`Id` 与 `i32` 完全等价（可互相赋值，无强制转换） |
| 作用域 | 顶层声明；`pub` 决定能否跨文件使用（跨文件非 pub 报 `MYL005`） |
| 位置 | 必须在类型被使用之前可解析（同文件任意顺序、跨文件按 import 图） |
| 循环 | `type A = A;` 或互指环报 `MYL015` |

### 4.2 合法示例

```mylis
i32 count = 0;
u64 total = 1000;
f64 ratio = 3.14159;
bool ok = true;
string name = "Mylis";
string empty = "";
Result<i32, string> r = Ok(7);
Option<f64> maybe = Some(2.5);
Option<i32> nothing = None;
Counter c = new Counter(10);
Shape s = new Circle(1.0);      // class → interface 自动转换（鸭子类型）
i32* p = null;                  // 仅当处于 unsafe 域内
```

### 4.3 非法示例

```mylis
let x = 1;                      // MYL101：没有 let（'let' 会被当作类型名，随即报语法错）
i32 y;                          // MYL003：局部变量必须初始化
x = 2;                          // MYL102：未声明的标识符
i32 z = 1.0;                    // MYL012：严格类型匹配，字面量 1.0 是 f64
u8 b = 200 + 100;               // MYL012：常量表达式不折叠，200+100 仍是 u8（运行时回绕需显式 as）
i32* raw = null;                // MYL007：裸指针只能出现在 unsafe 域
```

### 4.4 字面量与字面量定向

| 字面量 | 默认类型 | 定向规则 |
| --- | --- | --- |
| 整数字面量 `123` / `0xFF` | `i32` | 若期望类型为整数或浮点，则取期望类型；并做**取值范围检查**（越界 `MYL012`）。正字面量支持完整 `u64` 范围（最大 `18446744073709551615` / `0xFFFFFFFFFFFFFFFF`）；无期望类型且超过 `i64::MAX` 时默认为 `u64`；超过 `u64` 范围报 `MYL100` |
| 浮点字面量 `1.5` / `1e3` | `f64` | 若期望类型为 `f32`/`f64`，取期望类型 |
| 字符串 `"..."` | `string` | 固定 |
| `true` / `false` | `bool` | 固定 |
| `null` | `T*` | 只能用于指针/class 类型位置 |

**这不是类型推导**：变量、参数、返回值、字段类型永远由人写出，字面量只是适配那个已写出的类型。
二元运算中若一侧是数值字面量，则按另一侧的类型定向（`1 + x`，`x : i64` → `1` 为 `i64`）。

### 4.4.1 整数语义（固定，不可依赖 C 的实现行为）

| 运算 | 语义 | 生成方式 |
| --- | --- | --- |
| `+ - *` | **回绕（wrapping）**，与 C 无符号整数一致；`2147483647 + 1 == -2147483648` | 有符号类型先转同宽无符号运算再转回，绕开 C 的有符号溢出 UB |
| 一元 `-` | 回绕（`-i32::MIN == i32::MIN`） | 同上，`(int32_t)(0u - (uint32_t)x)` |
| `/`、`%` 的除数为 0 | **运行时 panic**（打印诊断后 `abort`），不是 UB | 生成除零检查 |
| `i32::MIN / -1`、`i32::MIN % -1` | 由 C 侧行为决定，**规范暂不承诺**（后续可加特判） |  |
| 移位 | C 语义（移位量不做检查） |  |

这意味着 Mylis 的整数行为在 `-O2` 下也是确定的；代价是 `+ - *` 多一层类型转换（编译器可优化掉）。

### 4.4.2 接口转换与 `Any` 装箱

空接口 `interface Any {}` 可以承载任意装箱基本类型。装箱只覆盖
`i8/i16/i32/i64/u8/u16/u32/u64/f32/f64/bool/string`，非空接口不做隐式装箱。

```mylis
interface Any { }
interface I32 { fn get() -> i32; }

Any a = 42;          // 隐式装箱
Any b = 7 as Any;    // 显式装箱
i32 x = a as i32;    // 拆箱

Option<i32> ok = a as? i32;   // 成功：Some(42)
Option<i32> no = b as? string; // 失败：None
```

规则：

| 写法 | 运行时行为 |
| --- | --- |
| `v as T` | 成功返回 `T`；动态类型不匹配调用 `myrt_panic("invalid cast")` |
| `v as? T` | 成功 `Some(v)`；失败 `None`，不 panic |

允许的接口转换：

| 源 → 目标 | 检查方式 |
| --- | --- |
| 接口 → 类 | vtable 首字段 `type_id` 必须精确等于目标类 id；不是“实现同一接口”就放行 |
| 接口 → 接口 | 走具体实现适配函数 `A_from_to`，没有共同具体实现时失败 |
| 接口 → 基本类型 | 必须由编译器隐藏的装箱对象产生；按装箱 id 拆箱，用户类不能伪装成基本类型 |

`Option<Iface>` 使用专用布局 `{ is_some; obj; vt; }`，因此 `Any as? I32`
可以安全返回 `Option<I32>`；`Some(接口值)`、`unwrap`、`unwrap_or`、
`match Some(v)` 均已支持。

`Result<Iface, E>` / `Result<T, Iface>` / `Result<Iface, Iface>` 也使用专用
布局，`Ok/Err`、`unwrap/unwrap_err/unwrap_or`、`?` 提前返回和 `match`
绑定均已支持；同时支持 class/enum → interface 的载荷转换。
接口 → 类的 `type_id` 精确匹配，未来引入继承时需要重新设计基类/子类兼容规则。

### 4.5 数组字面量与字符串插值（v0.2，详见 §17）

| 形式 | 示例 | 说明 |
| --- | --- | --- |
| 数组字面量 | `i32[] a = [3, 1, 4];` | 元素类型由期望类型定向；`[]` 必须出现在有类型的位置 |
| 数组分配 | `i32[] z = new i32[3];` | 长度 `n` 为整数表达式，元素零初始化 |
| 字符串插值 | `"id = {x}, name = {this.name}"` | `{expr}` 插入；`{{` / `}}` 为字面花括号 |

### 4.6 作用域

- 块作用域：`{}`、`if` 分支、循环体、`for` 头部各自建立作用域；
- 同一作用域内同名局部变量重复声明报 `MYL102`；
- 无变量遮蔽规则（不同作用域可同名，内层优先）；
- 局部变量必须先声明后使用，且声明处必初始化。

---

## 5. 函数系统完整规范

### 5.1 普通安全函数

```ebnf
fn_decl ::= "pub"? "fn" IDENT "(" params? ")" ( "->" type )? ( block | ";" )
params  ::= param ("," param)*
param   ::= type IDENT
```

```mylis
pub fn add(i32 a, i32 b) -> i32 {
    return a + b;
}

fn log_nothing() -> void {
    return;
}
```

规则：

- 参数必须逐个手写类型，无默认值（R5），无重载（R4）；
- 返回类型缺省为 `void`；
- 非 `void` 函数在**所有路径**上都必须 `return`（保守判定：函数体最后一条语句必须是
  `return`，或 `if/else` 两分支都返回，或嵌套块的末句返回），否则报 `MYL102`；
- `return;` 只在 `void` 函数中合法；`void` 函数 `return expr;` 报 `MYL012`；
- 顶层函数之间没有前向声明问题：编译器两遍处理，任意顺序调用均可。

### 5.2 不安全函数

```mylis
unsafe fn peek(i8* p) -> i8 {
    return *p;                       // 裸指针解引用只在 unsafe 中合法
}

unsafe fn patch(i8* p, i8 v) -> void {
    *p = v;                          // 指针写入同理
}
```

- `unsafe fn` 的**整个函数体**处于 unsafe 域（等价于整体包在 `unsafe { }` 内）；
- 参数类型含裸指针时，函数**必须**是 `unsafe fn`，否则 `MYL007`；
- 调用任何 `unsafe fn`（无论有无函数体）都必须位于 unsafe 域内，否则 `MYL007`。

### 5.3 C FFI 绑定函数（核心隔离架构）

**严格二分**

| 形态 | 含义 | 允许位置 |
| --- | --- | --- |
| `unsafe fn xxx(...) -> T;`（无体） | C 外部符号 | **仅** `*_bind.myl` |
| `unsafe fn xxx(...) -> T { ... }`（有体） | Mylis 本地 unsafe 函数 | 任意文件 |

**强制隔离规则**

- 所有无体 `unsafe fn` 只能写在 `*_bind.myl`（`MYL008`）；
- 业务文件禁止写任何 C 绑定；
- 无体声明必须是 `unsafe`（否则 `MYL102`）；
- 跨文件使用的绑定必须 `pub`（`MYL005`）。

**C ABI 类型映射**

| Mylis | C |
| --- | --- |
| `i8` `i16` `i32` `i64` | `int8_t` `int16_t` `int32_t` `int64_t` |
| `u8` `u16` `u32` `u64` | `uint8_t` `uint16_t` `uint32_t` `uint64_t` |
| `f32` `f64` | `float` `double` |
| `bool` | `bool` |
| `string` | `MyStr*` |
| `i8*` | `char*` |
| `u8*` | `unsigned char*` |
| 其他 `T*` | `T*` |
| `void` | `void` |

**`@header` 与符号生成**

```mylis
@header("stdio.h")
pub unsafe fn puts(i8* s) -> i32;
```

- 编译器生成 `#include <stdio.h>`；
- 为避免与头文件原型冲突（`char*` vs `int8_t*` 等），编译器**不重复声明**该符号，而是生成
  一层只做指针类型适配的本地包装：

  ```c
  static int32_t FFI_<模块前缀>_puts(char* p_s) { return puts(p_s); }
  ```

  Mylis 侧调用被解析到这个包装函数；
- 不带 `@header` 的无体声明则直接 `extern` 声明，调用点使用原始符号名，此时类型必须与 C 完全一致。

### 5.4 注解系统（内置 + 用户自定义）

**注解总规则**

- 格式：`@name` 或 `@name(arg, arg, ...)`；
- 参数只允许**常量字面量**：`int` / `float` / `string` / `bool`（可多个，逗号分隔）；
- 不可嵌套、不可表达式、不可变量。

**可挂载位置（白名单）**

- `class`
- `interface`
- 字段
- 函数 / 方法

**禁止挂载**

- 语句、局部变量（`MYL010`）
- 表达式、类型、参数

**内置特殊注解（独占校验）**

| 注解 | 参数 | 允许位置 | 语义 / 诊断 |
| --- | --- | --- | --- |
| `@header("x.h")` | 1 个字符串 | **仅** `*_bind.myl` 中的无体 `unsafe fn` | 该符号来自 `x.h`；违反报 `MYL009` |
| `@export("c_name")` | 1 个字符串 | 顶层 `pub fn`（`--lib` 导出） | 导出符号名改为 `c_name`；违反报 `MYL102` |
| `@deprecated("建议")` | 0 或 1 个字符串 | 函数 / 方法 / class | 调用点告警 `MYLW02`（不阻断编译） |
| `@inline` | 0 | 函数 / 方法 | 生成 `static inline`（仅提示，GCC 仍自行判断） |

- `@header` 存入 AST 的独立字段（`Decl.c_header`）供工具链与代码生成识别；
- 参数缺失或类型错误报 `MYL009`（`@header`）/ `MYL102`（`@export`）。

**拼写保护（MYLW01）**

注解名与内置注解**高度相似**（编辑距离 ≤ 2）时告警并给出建议，例如：

```
A.myl:2:1: warning[MYLW01]: 未识别的注解 @exprot：是否想写 @export？
```

这条告警只针对「疑似拼错内置注解」的情况；用户自定义注解（`@route` / `@col` 等）**保持静默**，
不影响“注解当结构化注释用”的既有用法。

**用户自定义注解（完全自由）**

```mylis
@route("/users")
@auth("admin")
pub class Users {
    @json("id")
    pub i64 id;

    @deprecated
    pub fn legacy() -> void { }
}
```

- 任意未内置的注解全部由用户自定义；
- **编译器默认忽略，仅存元数据**（不参与语义检查，不影响代码生成）；
- 例外是内置 `@derive(ToString)`：编译器在编译期读取 class 字段并生成 `toString` 方法（见 §17.7），不生成运行时类型表，也没有通用宏或运行时反射。

---

## 6. 问号传播语法 `?`（固定语义）

### 6.1 规则

1. 仅能在**返回 `Result<_, E>` 或 `Option<T>` 的函数**内使用（方法同理）；
2. 仅能作用于 `Result<T, E>` 或 `Option<T>` 类型，两种协议不能混用；
3. 语义糖：

   | 情形 | 语义 |
   | --- | --- |
   | `Ok(v)` | 解包为 `v`，继续求值 |
   | `Err(e)` | 立即 `return Err(e)`（保留原错误值） |
   | `Some(v)` | 解包为 `v`，继续求值 |
   | `None` | 立即 `return None` |

4. `?` 的错误类型 / Option 载荷必须与**当前函数返回类型**严格一致，否则 `MYL006`；
5. `?` 是一个**后缀表达式**，优先级高于所有二元/一元运算，低于成员访问：

```mylis
i32 q = div(a, b)?;              // 变量初始化
return Ok(div(a, b)? + 1);       // 表达式中使用
i32 n = c.try_div(2)?.to_i32();  // 与方法调用链混用
```

### 6.2 生成代码

```c
MylR_ri32_str t_try_1 = <表达式>;
if (!t_try_1.is_ok) {
    return ((MylR_ri32_str){ .is_ok = false, .u = { .err = t_try_1.u.err } });
}
/* 表达式值 = t_try_1.u.ok */

MylO_oi32 t_try_2 = <Option 表达式>;
if (!t_try_2.is_some) {
    return ((MylO_oi32){ .is_some = 0 });
}
/* 表达式值 = t_try_2.u.some；接口载荷则还原 obj/vt */
```

### 6.3 非法示例

```mylis
fn f() -> i32 {
    Result<i32, string> r = Ok(1);
    i32 v = r?;            // MYL006：函数返回 i32，不是 Result
    return v;
}
```

---

## 7. 模块系统

### 7.1 导入语法

```ebnf
import_decl ::= "pub"? "import" STRING ";"
              | "pub"? "import" "{" ident ("," ident)* "}" "from" STRING ";"
```

两种导入形式的作用域不同：

- `import "A";` —— **只加载模块，不绑定裸名**。`A` 进入当前文件的可见模块集合，
  调用函数、引用类型必须写模块限定名：`A::f(...)`、`A::Foo`；
  多段模块路径写 `A::sub::f(...)`、`A::sub::Foo`。
- `import { f, Foo } from "A";` —— **选择性导入**。只有列出的名字进入当前文件的裸名作用域，
  可以直接写 `f(...)`、`Foo`；未列出的名字仍然只能通过 `A::name` 访问。
- `pub import { f, Foo } from "A";` —— 在选择性导入的同时把 `f`、`Foo` 再导出给上层模块；
  上层可以继续 `import { f } from "当前模块"`，查找会沿 `pub import` 链穿透。
- `pub import "A";` —— 加载模块，并把目标模块以原名对上层可见；上层可以写 `A::f(...)`，
  但不产生裸名。需要再导出名字时写 `pub import { f } from "A";`。
- `pub import { f } from "A";` —— 只把列出的名字再导出；**A 模块本身不向上层可见**，
  上层不能写 `A::g(...)`。如果 A 模块也要公开，必须另外写 `pub import "A";`。

```mylis
import { Io } from "std/Io";          // Io::Println(...) 可用
import { println } from "std/Io";      // 也可以裸写 println(...)
import "./local.myl";             // 相对当前文件
import "../shared/x.myl";         // 向上相对
import "mathlib";                 // 多段限定：mathlib::vector::triple(...)
```

名字冲突规则：

- 同一文件内两个 `import { ... }` 导入了同一个名字，直接报 `MYL011`；
- 选择性导入名与当前文件本地声明同名，报 `MYL011`；
- 需要同时使用 `A`、`B` 两个包的同名 `ADD` 时，只对其中一个写 `import { ADD }`，
  另一个写 `A::ADD(...)` / `B::ADD(...)`：裸 `ADD` 不唯一，编译器不会做隐式覆盖。

- `import` 只能出现在文件顶层（声明与语句之外），位置任意；
- 路径以 `./` 或 `../` 开头视为相对路径；否则依次在「当前文件目录、core 根、`-I` 目录、
  `MYLIS_PATH` 各段、包依赖根」中查找第一个命中；
- 不存在的模块报 `MYL103`。

### 7.2 编译器行为

1. **递归收集所有依赖**（按需加载，同一物理文件只解析一次，路径以 canonicalize 去重）；
2. **自动检测循环导入并报错**（DFS 状态机，命中访问中节点即 `MYL011`）；
3. **Kahn 拓扑排序**确定编译顺序（依赖在前，被依赖模块先发射）；
4. **跨文件仅 `pub` 可见**（`MYL005`）：类型、函数、方法、字段分别检查；
5. 可见性范围 = **本模块的直接导入** + **沿 `pub import { ... }` 链传递**的名字（Rust 式：私有导入不外泄）；`import "A"` 只提供 `A::name` 限定访问；
5.1 裸名查找顺序：当前模块本地声明 → 当前模块选择性导入的名字 → 当前模块 `pub import` 再导出的名字（递归穿透）；
6. 同一模块内同名类型/函数报 `MYL102`（禁止重载）。

### 7.3 模块与前缀

每个模块拥有唯一「C 前缀」，用于生成全局符号：

- **模块完整名** = `包名::相对包根路径`，包根就是包名；例如 `src/vector/A.myl` 是
  `mathlib::vector::A`，不会再按文件名主干压成 `mathlib::A`；
- C 签名/MIR mangling 用的「前缀」仍然取文件名主干（非 ASCII 字母数字转 `_`），同名冲突时追加 `_1`、`_2`…；
- 生成 C 符号：`F_<前缀>_<函数>`、`M_<前缀>_<类>_<方法>`、`C_<前缀>_<类>`、`I_<前缀>_<接口>`、
  `VT_<前缀>_<类>_<接口>`、`NEW_<前缀>_<类>`；`.mylib` API 符号使用完整模块名。

---

## 8. 类与对象系统

### 8.1 语法

```ebnf
enum_decl   ::= "pub"? "enum" IDENT "{" ( variant | method )* "}"
variant     ::= IDENT ("(" type ("," type)* ")")? ","?
class_decl  ::= "pub"? "class" IDENT "{" (field | method)* "}"
field       ::= "pub"? type IDENT ("=" expr)? ";"
method      ::= "pub"? ("unsafe" "fn" | "fn") IDENT "(" params? ")" ("->" type)? block
                (* 名字等于类名 = 构造器，可省略 "-> void" *)
```

```mylis
// Counter.myl
pub class Counter {
    pub i32 value;          // 无初值 → 必须在 init 中赋值（MYL004）
    i32 step = 1;           // 声明处赋初值

    pub fn init(i32 start) -> void {
        this.value = start;
    }

    pub fn bump() -> i32 {
        this.value = this.value + this.step;
        return this.value;
    }

    pub unsafe fn poke(Counter* other) -> void {
        other.value = 0;    // 裸指针参数 → 方法本身必须 unsafe
    }
}
```

### 8.2 规则

- **字段必须全部初始化**（编译强检，`MYL004`）：字段要么在声明处有初值，要么在 `init` 中被赋值
  （在 `init` 体内任意嵌套位置检查 `this.f = …`）；
- 有未初始化字段的类必须声明 `init`；无 `init` 且存在无初值字段 → `MYL004`；
- **构造器写法**：方法与**类同名**即构造器，**返回类型可省略**（省略即 `void`）：

  ```mylis
  pub class Point {
      pub i32 x;
      pub i32 y;

      pub fn Point(i32 a, i32 b) {      // 推荐：同名构造器，不写 -> void
          this.x = a;
          this.y = b;
      }
  }

  Point p = new Point(2, 5);
  ```

  兼容写法 `pub fn init(...) -> void` 仍然有效；
- **一个类最多一个构造器**（同名方法 + `init` 一起出现 → `MYL102`），参数即 `new` 的实参；
- 构造器**不能有返回类型**（写了 `-> T` 即 `MYL012`），也不能 `static`（`MYL102`）；
- 调用构造器只有一条路：`new`（`p.Point(...)` / `p.init(...)` 都视为「没有方法」`MYL102`）；
- 字段初值表达式**不能使用 `this`**（`MYL102`）：字段初值在 `init` 之前按声明顺序执行，此时其它字段还是零值，
  需要依赖其它字段的逻辑必须写进 `init`；
- class **永远引用语义，无拷贝**：赋值、传参、返回都是指针；
- `new ClassName(args)` 分配 GC 堆对象，先执行字段初值，再调用 `init`；
- **需要多个「构造器」时用静态工厂方法**：`pub static fn empty(...) -> Class`，返回 `new` 出来的实例（见 §8.4）；
- `this` 只在类方法内合法（`MYL102`），类型为该 class；
- 字段访问 `obj.field`，方法调用 `obj.method(args)`；
- 字段名、方法名在同一 class 内不可重复（`MYL102`）；
- 无继承；静态字段与静态方法**已支持**（v0.2，见 §17.4），访问一律 `Class::name`。

### 8.3 多构造器：静态工厂方法

语言不允许重载，因此「多构造器」的标准写法是**命名的静态工厂方法**：它们各自返回本类实例，
内部调用唯一的 `init` 或直接组装对象。

```mylis
pub class Account {
    pub string owner;
    pub i32 balance;

    pub fn init(string who, i32 initial) -> void {   // 唯一的 init
        this.owner = who;
        this.balance = initial;
    }

    pub static fn empty(string who) -> Account {     // 命名构造器 1
        return new Account(who, 0);
    }

    pub static fn bonus(string who, i32 base) -> Account {   // 命名构造器 2
        Account a = new Account(who, base);
        a.balance = a.balance + 5;
        return a;
    }

    pub static fn vip(string who) -> Account {      // 组合其它工厂
        Account a = Account::bonus(who, 100);
        a.balance = a.balance * 2;
        return a;
    }
}

Account a = new Account("ann", 10);      // 直接构造
Account b = Account::empty("bob");       // 命名构造
```

约定（不是编译器强制）：工厂以 `new_class` / `empty` / `from_*` 之类命名，返回类型写成类名，
调用点一律 `Class::factory(...)`。`tests/positive/Account.myl` 是这条规则的用例。

### 8.4 `new` 的生成代码

```c
static C_Counter_Counter* NEW_Counter_Counter(int32_t p_start) {
    C_Counter_Counter* self = (C_Counter_Counter*)myrt_alloc(sizeof(C_Counter_Counter));
    self->step = ((int32_t)1);
    M_Counter_Counter_init(self, p_start);
    return self;
}
```

`myrt_alloc` 返回 GC 后端分配并已清零的内存，因此未显式赋值的字节恒为 0。

---

### 8.5 枚举（Rust 式带载荷）

**声明**：

```mylis
pub enum Shape {
    Circle(f64),          // tuple 变体：携带 1 个 f64
    Rect(f64, f64),       // 多个载荷
    Empty,                // unit 变体
    pub fn area() -> f64 { ... }        // 实例方法
    pub fn to_string() -> string { ... }
    pub static fn unit() -> Shape { return Shape::Empty; }   // 静态方法
}
```

- 变体：`Name` 或 `Name(T1, T2, ...)`，载荷类型必须是值类型 / `string` / class / enum（与数组元素规则一致）；
- **没有命名载荷变体**（Rust 的 `Move { x, y }` 暂不支持），需要多个字段时用 tuple 变体并在 `match` 里按位置绑定；
- 变体之间用 `,` 分隔；方法与变体可混排，方法以 `pub` / `fn` / `static` / `unsafe` / `@` 开头；
- `pub enum` 的名字必须与文件名一致（同 `pub class` 规则，`MYL002`），一个文件仍然只有一个 `pub` 类型。

**表示与构造**：

```mylis
Shape c = Shape::Circle(2.0);   // 变体构造：E::Variant(args)
Shape e = Shape::Empty;         // unit 变体：不带括号，也是表达式
```

- enum 是 **GC 堆对象**（指针语义，和 class 一样可以放进数组、`Option`、`Result`）；
- **不能用 `new E(...)`**（`MYL102`，提示改用 `E::Variant(...)`）；
- **不能 `==` / `!=`** 比较（`MYL102`：比较的是指针身份），要区分情况请用 `match`；
- enum 没有字段：`e.f0` 一律 `MYL102`，载荷只能在 `match` 中解构。

**方法**：写在 enum 体内的方法与类方法同构，`this` 的类型就是该 enum。典型的 `to_string` 写法是对 `this` 做穷尽 `match`。

**接口**：enum 与 class 一样参与鸭子类型 —— enum 只要具备接口要求的方法（签名完全一致），就能自动转换为接口值并参与动态派发：

```mylis
interface Show { fn label() -> string; fn to_string() -> string; }

Show s = Shape::Rect(1.0, 2.0);
println("{s.label()} / {s}");   // 走 vtable
```

**字符串插值**：enum 需要定义 `fn to_string() -> string` 才能插入字符串（否则 `MYL102`）。

**生成代码**（`--emit-c` 可查看）：

```c
struct E_Shape { int32_t tag; void* data; };
struct P_Shape_Circle { double f0; };
static E_Shape* NEW_Shape_Circle(double v0) {
    E_Shape* e = (E_Shape*)myrt_alloc(sizeof(E_Shape));
    e->tag = 0;
    P_Shape_Circle* p = (P_Shape_Circle*)myrt_alloc(sizeof(P_Shape_Circle));
    p->f0 = v0;
    e->data = (void*)p;
    return e;
}
```

`match` 在枚举上降级为 `tag` 比较 + `data` 指针转换，无虚函数、无额外分配点（每次构造恰好 N+1 次 `myrt_alloc`，由 GC 后端回收）。

## 9. 接口系统（鸭子类型）

### 9.1 语法

```ebnf
iface_decl ::= "pub"? "interface" IDENT "{" iface_member* "}"
iface_member ::= signature | IDENT ";"        (* 内嵌已有接口名：接口组合 *)
signature  ::= "fn" IDENT "(" params? ")" ("->" type)? ";"
```

```mylis
pub interface Shape {
    fn area() -> f64;
    fn name() -> string;
}
```

### 9.2 规则

- 接口只包含方法签名，**不允许函数体**、字段、常量；
- **自动实现、无 `impl` 关键字**：若 class 的方法集合满足接口全部签名（名称 + 参数类型逐一 +
  返回类型 + `unsafe` 性一致），则该 class 实现该接口；
- 实现关系只在**使用点**（`class → interface` 赋值/传参/返回/显式 `as`）被检查并记录；
- 不满足时报 `MYL102`（缺方法 / 参数不一致 / 返回不一致）；
- 接口变量可被重新赋值为另一个实现类，调用走运行时派发；
- 接口值不能访问字段、不能用 `new` 实例化。

### 9.3 接口组合（内嵌）

接口体里写 `名字;` 表示**内嵌另一个接口**，方法集取并集（等价于“接口继承”，但不需要关键字）：

```mylis
interface Reader { fn read(i32 n) -> i32; }
interface Writer { fn write(i32 n) -> void; }
interface Closer { fn close() -> i32; }

pub interface ReadWriteCloser {
    Reader;
    Writer;
    Closer;
    fn flush() -> i32;      // 也可以在内嵌之外补自己的方法
}
```

规则（先展平，再检查，展平对 codegen 不可见——vtable 只有一份并集）：

| 情形 | 处理 |
| --- | --- |
| 内嵌名不是 `interface`（class / enum / 变量） | `MYL013` |
| 内嵌名找不到 | `MYL013` |
| 内嵌接口跨文件可见但未 `pub` | `MYL013` |
| 循环内嵌（`A { B; }` + `B { A; }`，含自嵌） | `MYL013` |
| 同名同签名（含“自身方法与父接口方法同签名”） | 静默合并 |
| 同名不同签名 | `MYL014`，信息里给出两边的完整签名 |
| 外层显式重写以“消歧” | 不允许，同样按同名不同签名报 `MYL014` |

- 展平后，实现检查、vtable 生成、运行时派发都不再关心层级关系：`class → 组合接口` 就是
  “方法集是否覆盖并集”的鸭子类型检查；
- 内嵌可以多层（`A; B;` 里 B 自身又内嵌 C），展平按后序（父先于子）完成；
- 组合不引入任何运行时开销，也不引入“父接口指针”之类的表示。

### 9.3 动态派发实现

```c
struct Vt_Shape_Shape {              // 每个接口一张 vtable 类型
    double (*fn_area)(void*);
    MyStr* (*fn_name)(void*);
};
struct I_Shape_Shape {               // 接口值：值语义的两字结构
    void* obj;
    const Vt_Shape_Shape* vt;
};

static double W_Circle_Circle_Shape_0(void* s) {   // 适配包装
    return M_Circle_Circle_area((C_Circle_Circle*)s);
}
static const Vt_Shape_Shape VT_Circle_Circle_Shape = { .fn_area = W_…, .fn_name = W_… };
```

- 转换点：`(I_Shape_Shape){ .obj = (void*)circle, .vt = &VT_Circle_Circle_Shape }`；
- 调用点：`t.vt->fn_area(t.obj)`；
- 类型安全由 Sema 保证，`void*` 只在编译生成的代码中出现。

---

## 10. Unsafe 完整规则

### 10.1 安全代码禁止

| 操作 | 诊断 |
| --- | --- |
| 裸指针类型定义（局部变量 / 参数 / 字段初始化） | `MYL007` |
| 裸指针解引用 `*p` | `MYL007` |
| 取地址 `&x` | `MYL007` |
| 直接调用 C 绑定函数或任意 `unsafe fn` | `MYL007` |
| 手动 GC 根操作（`myrt_register_root` 等） | `MYL007` |
| 指针间 `as` 转换 | `MYL007` |

### 10.2 unsafe 两个形态

```mylis
unsafe {                       // 1) 块 unsafe：仅块内不安全
    i8* p = myrt_alloc(8);
    *p = 1;
}

unsafe fn raw_copy(...) -> void {   // 2) 函数 unsafe：整函数不安全
    ...
}
```

### 10.3 边界说明

- `unsafe` 块不改变类型系统与所有其他检查（仍然是强类型、仍然禁止隐式转换）；
- 安全函数可以返回裸指针（类型层面允许），但调用者拿到后无法在安全域内解引用；
- `unsafe` 之外没有任何“后门”：没有 `@suppress`、没有编译器开关。

---

## 11. 运行时 `myrt` 固定 API

所有 `myrt_*` 符号**仅 unsafe 可调用**（它们都是 `core/Mylrt_bind.myl` 中的无体 `unsafe fn`）。
安全层封装见 `std/src/Io.myl`、`core/Str.myl`、`std/src/Math.myl`。

| 原型 | 语义 |
| --- | --- |
| `i8* myrt_alloc(u64 size)` | GC 分配 `size` 字节（已清零），返回裸指针 |
| `void myrt_register_root(i8* obj)` | 把对象登记为 GC 根（C 侧长期持有时使用） |
| `void myrt_unregister_root(i8* obj)` | 注销根 |
| `string myrt_str_cstr(i8* p)` | 以 `\0` 结尾的 C 串 → 新 Mylis string（复制） |
| `i8* myrt_str_ptr(string s)` | Mylis string → 以 `\0` 结尾的只读缓冲区（不复制） |
| `u64 myrt_str_len(string s)` | 字节长度 |
| `void myrt_gc_collect()` | 触发一次完整回收 |
| `void myrt_print(string s)` | 输出到 stdout（不换行） |
| `void myrt_println(string s)` | 输出到 stdout（换行） |
| `void myrt_panic(string msg)` | 打印 `panic: msg` 并 `abort()` |
| `string myrt_i64_to_string(i64 v)` | 十进制格式化 |
| `string myrt_u64_to_string(u64 v)` | 十进制格式化 |
| `string myrt_f64_to_string(f64 v)` | `%.10g` 格式化（NaN/inf 特判） |
| `f64 myrt_clock()` | 进程 CPU 秒数 |

**编译器内部辅助符号**（不属于语言 API，用户代码不可依赖）：
`mylrt_str_lit`、`mylrt_str_concat`、`mylrt_str_eq`、`mylrt_str_cmp`、`mylrt_panic_msg`、
`mylrt_panic_unwrap`、`mylrt_assert`。

**约定**

- string 恒以 `'\0'` 结尾，`len` 不含结尾字节；
- 字面量驻留：相同字节串字面量共享同一对象（进程内常量时间比较捷径存在但不依赖）；
- `myrt_str_ptr` 的指针在对象存活期内有效，禁止写入。

---

## 12. 完整 AST 最终定稿（与 `mylc/src/ast.rs` 1:1）

### 12.1 类型

```rust
pub enum Type {
    Void,
    I8, I16, I32, I64,
    U8, U16, U32, U64,
    F32, F64,
    Bool,
    Str,                        // string
    Result(Box<Type>, Box<Type>),  // Result<T, E>
    Option(Box<Type>),          // Option<T>
    Ptr(Box<Type>),             // T*
    Array(Box<Type>),           // T[]（v0.2）
    Named(String),              // 解析前：源码中的类型名
    Class(usize),               // 解析后：class 声明 id
    Iface(usize),               // 解析后：interface 声明 id
    Infer,                      // 内部占位：Ok/Err/Some/None 待定
}
```

### 12.2 注解

```rust
pub enum AnnotArg { Int(i64), Float(f64), Str(String), Bool(bool) }

pub struct Annot {
    pub name: String,
    pub args: Vec<AnnotArg>,
    pub line: u32,
    pub col: u32,
}
```

### 12.3 表达式

```rust
pub enum BinOp {
    Add, Sub, Mul, Div, Rem,
    Eq, Ne, Lt, Le, Gt, Ge,
    AndAnd, OrOr,
    BitAnd, BitOr, BitXor, Shl, Shr,
}

pub enum UnOp { Neg, Not, BitNot, Deref, AddrOf }

pub enum Builtin {
    ResIsOk, ResIsErr, ResUnwrap, ResUnwrapErr, ResUnwrapOr,
    OptIsSome, OptIsNone, OptUnwrap, OptUnwrapOr,
    StrLen,
    ArrLen,                      // a.len()（v0.2，返回 i32）
}

pub enum Res {
    None,
    Fn(usize),                   // 顶层函数 / C 绑定 声明 id
    Method(usize),               // 方法 / 接口签名 声明 id
    Field(usize, usize),         // (class id, 字段下标)
    Var,                         // 局部变量 / 参数（codegen 按作用域解析）
    Ctor(usize),                 // new 的 class id
    IfaceCast(usize, usize),     // (class id, iface id) 自动实现转换
    StaticMethod(usize, usize),  // (class id, 成员下标) —— Class::f(...)
    StaticField(usize, usize),   // (class id, 字段下标) —— Class::f
    Builtin(Builtin),            // 内置方法
}

pub enum ExprKind {
    Int(i64), Float(f64), Str(String), Bool(bool),
    Null, This,
    Ident(String),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Un(UnOp, Box<Expr>),
    Call(String, Vec<Expr>),
    Method(Box<Expr>, String, Vec<Expr>),
    Field(Box<Expr>, String),
    New(String, Vec<Expr>),
    Ok(Box<Expr>), Err(Box<Expr>), Some_(Box<Expr>), None_,
    Cast(Box<Expr>, Type),
    Try(Box<Expr>),
    Index(Box<Expr>, Box<Expr>),           // a[i]（v0.2）
    ArrayLit(Vec<Expr>),                   // [a, b, c]
    ArrayNew(Type, Box<Expr>),             // new T[n]
    Interp(Vec<InterpPart>),               // "{x} / {y}" 拆出的片段序列
    StaticPath { class: String, name: String, args: Vec<Expr>, call: bool },
                                           // Class::name / Class::name(args)
}

/// 字符串插值的片段：字面部分或子表达式
pub enum InterpPart { Lit(String), Expr(Expr) }

pub struct Expr {
    pub kind: ExprKind,
    pub line: u32, pub col: u32,
    pub ty: Type,     // sema 填充的最终类型
    pub res: Res,     // sema 填充的解析结果
}
```

### 12.4 语句

```rust
pub struct Block { pub stmts: Vec<Stmt>, pub line: u32, pub col: u32 }

pub enum StmtKind {
    Var { ty: Type, name: String, init: Option<Expr> },
    Assign { target: Expr, value: Expr },
    ExprStmt(Expr),
    If { cond: Expr, then_b: Block, else_b: Option<Box<Stmt>> },
    While { cond: Expr, body: Block },
    For { init: Box<Stmt>, cond: Option<Expr>, step: Option<Box<Stmt>>, body: Block },
    Return(Option<Expr>),
    Break,
    Continue,
    Unsafe(Block),
    Block(Block),
    Match { subject: Expr, arms: Vec<MatchArm> },   // v0.2
}

pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Block,
    pub line: u32, pub col: u32,
}

pub enum Pattern {
    Wildcard,                 // _
    Int(i64), Float(f64), Str(String), Bool(bool), Null,
    Ok(Option<String>), Err(Option<String>),   // 绑定名；None = _
    Some(Option<String>),
    None,
}

pub struct Stmt { pub kind: StmtKind, pub line: u32, pub col: u32 }
```

### 12.5 声明

```rust
pub enum DeclKind { Class, Iface, Fn }

pub struct Param { pub name: String, pub ty: Type, pub line: u32, pub col: u32 }

pub struct Field {
    pub name: String,
    pub ty: Type,
    pub init: Option<Expr>,
    pub is_static: bool,          // static 字段（v0.2）
    pub is_pub: bool,
    pub annots: Vec<Annot>,
    pub line: u32, pub col: u32,
}

pub struct Decl {
    pub kind: DeclKind,
    pub name: String,
    pub is_pub: bool,
    pub annots: Vec<Annot>,
    pub mod_id: usize,
    pub line: u32, pub col: u32,

    pub fields: Vec<Field>,        // class 字段
    pub members: Vec<usize>,       // 方法 / 接口签名（decls 下标）
    pub member_asts: Vec<Decl>,    // 解析期暂存，loader 展开后清空

    pub params: Vec<Param>,        // fn / 签名
    pub ret: Type,
    pub body: Option<Block>,       // None = C 绑定 / 接口签名
    pub is_unsafe: bool,
    pub is_static: bool,          // static fn（v0.2）
    pub is_ctor: bool,             // 名为 init 的类方法
    pub c_header: Option<String>,  // @header("…")
    pub owner: Option<usize>,      // 所属 class / interface
}
```

### 12.5.1 枚举（v0.3）

```rust
DeclKind::Enum
Decl.variants: Vec<Variant>          // 变体表
Variant { name: String, fields: Vec<Type>, line, col }
Type::Enum(id)
Res::Variant(enum_id, variant_idx)   // E::Variant 的解析结果
Pattern::Variant { ename, name, binds: Vec<Option<String>>, enum_id, vidx }
Expr.coerce: Option<(usize, usize)>  // enum -> interface 自动转换（与 res 分离）
```

### 12.6 模块与程序

```rust
pub struct ModuleInfo {
    pub path: String,      // 绝对路径（判定去重键）
    pub stem: String,      // 文件名主干（校验 pub class 名）
    pub module_path: Vec<String>, // 相对包根路径；包根为空
    pub pkg_name: Option<String>, // 所属包；std/独立文件为 None
    pub dir: String,       // 所在目录（相对 import 基准）
    pub is_bind: bool,     // *_bind.myl
    pub prefix: String,    // C 符号前缀
    pub import_raw: Vec<(String, u32, u32, bool, bool)>, // path/line/col/is_pub/selective
    pub imports: Vec<usize>,
    pub decls: Vec<usize>,
    pub state: u8,         // 0 未访问 / 1 访问中 / 2 已完成
    pub topo: usize,
}

pub struct Program {
    pub modules: Vec<ModuleInfo>,
    pub decls: Vec<Decl>,
    pub type_syms: Vec<Vec<(String, usize)>>,
    pub fn_syms: Vec<Vec<(String, usize)>>,
    pub insts: Vec<Type>,          // Result/Option 实例化表
    pub impls: Vec<(usize, usize)>,// (class, iface) 转换点
    pub arrays: Vec<Type>,         // 用到的数组类型（生成 MylA_* 结构体）
    pub tostring_iface: Option<usize>, // 预置 ToString 接口的声明 id
    pub entry: usize,
    pub order: Vec<usize>,         // 拓扑编译顺序
}
```

---

## 13. Sema 必须实现的 12 条强制检查

| 编号 | 检查 | 触发条件 | 诊断 |
| --- | --- | --- | --- |
| 1 | 单文件最多一个 pub class | 同文件第 2 个 `pub class` | `MYL001` |
| 2 | pub class 必须与文件名一致 | `pub class Foo` 出现在 `Bar.myl` | `MYL002` |
| 3 | 局部变量必须初始化 | `type name;` | `MYL003` |
| 4 | 类字段全覆盖初始化 | 字段无初值且 `init` 未赋值（或无 `init`） | `MYL004` |
| 5 | 跨文件私有访问 | 非 `pub` 的类型/函数/方法/字段被其他模块使用 | `MYL005` |
| 6 | `?` 语法合法性 | 非 Result/Option 函数、协议混用、`E`/载荷不一致 | `MYL006` |
| 7 | 裸指针仅 unsafe 可用 | 裸指针声明/解引用/取址、unsafe 调用、指针 `as` | `MYL007` |
| 8 | 无体 unsafe fn 仅限 `_bind.myl` | 业务文件出现无体 `unsafe fn` | `MYL008` |
| 9 | `@header` 仅可用于 C 绑定 | 用在有体函数、非 unsafe、非 bind 文件、参数错误 | `MYL009` |
| 10 | 注解只能挂载合法节点 | 注解出现在语句 / 局部变量 / 表达式 | `MYL010` |
| 11 | 循环导入检测 | 模块互相 import | `MYL011` |
| 12 | 严格类型匹配、参数数量匹配 | 类型不等、实参数量不符、字面量越界 | `MYL012` |
| 13 | 接口组合非法 | 内嵌非接口/找不到/未 `pub`/循环内嵌 | `MYL013` |
| 14 | 接口组合签名冲突 | 同名不同签名（含外层重写） | `MYL014` |

其余强制项（属通用语义错误 `MYL102`）：重复声明、重载、`this` 误用、`break/continue` 越界、
缺返回值、未知类型/标识符/函数/字段/方法、接口签名不满足（鸭子类型失败）、void 误用。

### 13.1 v0.2 增补的强制检查

| 编号 | 检查 | 触发条件 | 诊断 |
| --- | --- | --- | --- |
| 13 | 插值只接受可格式化类型 | 插入 `void`、未实现 `to_string` 的 class、未声明 `to_string` 的 interface | `MYL102` |
| 14 | `match` 必须穷尽 | 无 `_`，且模式集合未覆盖 `Result` 的 Ok+Err / `Option` 的 Some+None / `bool` 的 true+false / enum 的全部变体 | `MYL102` |
| 15 | enum 构造必须走变体 | `new Enum(...)` / `Enum` 缺参数 / `E::X` 不存在 | `MYL102` / `MYL012` |
| 16 | enum 不可比较、不可取字段 | `==` / `!=` / `.f0` | `MYL102` |
| 15 | 静态成员访问方式 | 用实例访问静态方法或静态字段（应改用 `::`） | `MYL102` |
| 16 | 数组不可比较 | `==` / `!=` 作用于两个数组 | `MYL102` |
| 17 | 下标只能用于数组 | `x[i]` 中 `x` 不是数组 | `MYL102` |
| 18 | 静态字段初值必须是常量字面量 | `static T f = <表达式>;` | `MYL102` |
| 19 | 数组字面量元素类型一致 | 元素类型与期望类型不符 | `MYL012` |
| 20 | `init` 返回类型必须是 `void` | `pub fn init(...) -> T`（`T ≠ void`） | `MYL012` |
| 21 | 字段初值不得使用 `this` | `pub i32 x = this.y;` | `MYL102` |
| 22 | `@export` 只能用于顶层 `pub fn` | 标在方法 / 非 pub 函数 / 类上 | `MYL102` |

诊断格式：

```
<file>:<line>:<col>: error[MYL0NN]: <中文说明>
```

---

## 14. 固定项目目录结构与实现流水线

### 14.1 目录结构

```
mylis/
├── Makefile
├── mylc/                 # 编译器（Rust crate）
│   ├── Cargo.toml
│   └── src/{main,lexer,ast,parser,program,sema,codegen,diag}.rs
├── myrt/                 # 运行时 C ABI 头
│   └── {mylrt.h,mylrt_gc.h}
├── mylrt-rs/             # 运行时实现（Rust；Runtime / Gc trait）
├── core/                 # 语言 core 层（隐式提供）
├── std/                  # 独立 std 包（显式依赖）
├── tests/{positive,negative}/ + run_tests.sh
├── tools/bootstrap_gc.sh
├── build/                # 产物：mylc、libmylrt.a、*.myl.c
└── docs/                 # 规范文档
```

### 14.2 编译流水线

```
加载入口
  └─ 递归 import 解析（canonicalize 去重 / 状态机查环 / 后序生成拓扑序）
词法分析    → token 序列（关键字、字面量、运算符；`>>` 在泛型闭合处自动拆分）
语法分析    → AST（递归下降 + 优先级爬升；声明/表达式歧义用回溯试探）
语义分析    → 类型解析、12 条检查、Result/Option 实例化表、接口实现表
MIR 生成    → AST → 函数级 CFG（SSA 值 + 局部槽 + 显式终结指令），safepoint 根表
后端        → 默认 LLVM：MIR → 文本 LLVM IR → clang → 运行时 + GC
            → MIR-C：MIR → C11 → CC/AR；AST-C 仅作显式回退
```

### 14.3 test 目标

```bash
make test
# 正向/反向合计 129 项；MIR-C/AST-C 对拍 81 项
# 另含 std native、库模式、工具链、MIR、LSP 与裸机验收
```

---

## 15. 官方语言标语

**Mylis —— 清澄极简，秩序永存**

古希腊文：**Μυλίς, ἁ καθαρή καὶ τάξις**

---

## 16. 最终总结（你现在拥有的）

- 从零完整的语言规范（本文档）
- 全部语法定义（附录 C 的 EBNF 全量语法）
- 全部 AST 结构（第 12 章，与 `ast.rs` 1:1）
- 全部语义检查（第 13 章 12 条 + 通用项）
- 全部安全规则（第 10 章）
- 全部 FFI 隔离架构（5.3）
- 全部注解系统（5.4）
- 全部模块系统（第 7 章）
- 全部内存模型（2.5 + 第 11 章）
- 全部红线禁用项（第 1 章）

> **这份文档 + 同仓库的 `mylc` / `mylrt` 源码 = Mylis 语言 100% 完整定义与可运行实现。**

---

## 17. v0.2 增补特性：数组 / 字符串插值 / `match` / 静态成员

本章的四条能力在 v0.2 落地，与 `mylc/src/{lexer,parser,sema,codegen}.rs`、`myrt/mylrt.{h,c}`、
`std/ToString.myl` **1:1 对应**。它们不触碰任何红线（第 1 章）：没有隐式类型转换、没有隐式所有权、
没有运行时反射。

### 17.1 数组 `T[]`

**类型**：`[]` 是类型后缀，可叠加 —— `i32[][]` 是「元素为 `i32[]` 的数组」。元素类型可为任意
值类型、`string`、class（存对象指针）、`Result` / `Option`。

**创建**：

| 形式 | 含义 |
| --- | --- |
| `[e1, e2, e3]` | 数组字面量；元素类型由期望类型定向，逐元素严格匹配（否则 `MYL012`） |
| `[]` | 空数组字面量，必须出现在**有类型**的位置：`Item[] xs = [];` |
| `new T[n]` | 分配长度为 `n` 的数组，**元素全部零初始化**；`n` 为任意整数表达式 |

**访问**：

| 形式 | 结果 |
| --- | --- |
| `a[i]` | 元素；`i` 可为任意整数类型；越界或负值 → 运行时 panic：`panic: array index N out of bounds (len M)` |
| `a.len()` | 长度，类型 **`i32`**（刻意不用 `u64`：`for (i32 i = 0; i < a.len(); …)` 无需转换） |

**规则**：

- 数组是 **GC 堆对象、引用语义**：赋值 / 传参 / 返回只搬指针，`b = a` 后二者指向同一块内存；
- **禁止 `==` / `!=`**（`MYL102`）：数组按引用比较极易误判，请比较 `len()` 后逐元素比较；
- 长度在分配时固定，没有 `push`；扩容需 `new` 更大数组并逐元素搬移（见 `tests/positive/Arrays.myl` 的扩容写法）；
- 数组可以作字段、参数、返回值，也可以作 `Result` / `Option` 的载荷。

**生成代码**（`MylA_<mangle>`：长度前置 + 柔性数组成员）：

```c
typedef struct { uint64_t len; int32_t items[]; } MylA_ai32;

MylA_ai32* a = (MylA_ai32*)mylrt_arr_alloc(sizeof(int32_t), 5);
a->items[0] = 3;
/* a[i] 的实际展开 */
a->items[mylrt_arr_index(a->len, (int64_t)i)]
/* a.len() 的实际展开 */
((int32_t)(a->len))
```

**运行时 API（§11 的增补）**：

| 符号 | 说明 |
| --- | --- |
| `void* mylrt_arr_alloc(uint64_t elem_size, uint64_t len)` | 分配 `8 + elem_size × len` 字节并清零，头部写入 `len` |
| `uint64_t mylrt_arr_index(uint64_t len, int64_t idx)` | 下标检查；负值 / 越界 → stderr 打印 `panic: array index N out of bounds (len M)` 后 `abort()` |

### 17.2 字符串插值

字符串字面量内的 `{expr}` 即插值：

```mylis
i32 n = 3;
println("n = {n}, double = {n * 2}");
println("braces: {{literal}}");
```

**词法规则**：

- `{{` → 字面 `{`，`}}` → 字面 `}`；
- 单独的 `}`（无 `}}` 配对）→ `MYL101`；
- `{` 直到字面串结束仍未闭合 → `MYL101`；
- 插值在**解析期**拆成 `InterpPart::Lit` / `InterpPart::Expr` 序列；每个子表达式用子解析器重新解析，
  行号按前置换行对齐，因此插值内的错误定位与源码一致。

**可插入的类型**：

| 类型 | 输出 |
| --- | --- |
| 所有整数 / 浮点 | 十进制；浮点用 `%g` 风格 |
| `bool` | `true` / `false` |
| `string` | 原样（不加引号） |
| 数组 | `[a, b, c]`，元素递归格式化 |
| 实现 ToString 的 class / interface | `to_string()` 的返回值 |

**复杂类型必须实现 ToString 接口**：class 需要

```mylis
pub class Point {
    pub i32 x;
    pub i32 y;

    pub fn init(i32 a, i32 b) -> void { this.x = a; this.y = b; }

    pub fn to_string() -> string { return "({this.x}, {this.y})"; }
}
```

否则 `MYL102`「需要实现 ToString 接口（定义 `pub fn to_string() -> string`）」。interface 必须在
其成员中声明 `fn to_string() -> string`，否则同样 `MYL102`。鸭子类型判定沿用第 9 章：只要 class 有
`pub fn to_string() -> string`，就自动满足 ToString。

**预置接口** `std/ToString.myl`：

```mylis
pub interface ToString {
    fn to_string() -> string;
}
```

loader 在加载程序时把它 **隐式挂到每个模块的 imports**（`attach_prelude()`），因此工程内任何文件都能
引用 `ToString`，无需显式 `import`；TOML 里也不需要在每个文件重复声明。

**生成代码**：从左到右累加拼接。

```c
MyStr* t = mylrt_str_lit("", 0);
t = mylrt_str_concat(t, mylrt_str_lit("price = ", 8));
t = mylrt_str_concat(t, myrt_f64_to_string((double)(x)));
```

**统一打印所有基本类型**：`println` 只有一个签名 `println(string)`（无重载，红线），
所以「用同一个 println 打印 u32 / i64 / f32 / bool」的标准写法是**直接插值**：

```mylis
u32 a = 7;
u8  b = 200;
f32 r = 1.5;
println("a = {a}, b = {b}, r = {r}");     // 任意数值/bool/string/数组/ToString
```

插值对全部 12 种值类型（`i8/i16/i32/i64/u8/u16/u32/u64/f32/f64/bool/string`）都生效，
格式化规则见 §17.2 的表格。若确实要「函数名相同、参数类型不同」，那就是重载，被第 1 章红线禁止；
需要显式函数时用带类型后缀的 helper：`println_u32 / println_i64 / println_f32 / println_bool …`
（`std/src/Io.myl` 提供 `print_*` 与 `println_*` 两套，覆盖全部数值类型）。

**已知词法边界**：插值内不能再写字面串（`"{ "x" }"` 会被外层引号提前截断）。需要时先赋变量：

```mylis
string word = "str";
println("word = {word}");
```

### 17.2.1 显式统一格式化 `FmtSpec` / `FmtArg`

插值用于快速输出；需要宽度、对齐、零填充、固定小数位时使用显式格式化入口，避免为每个数值类型堆叠
`F64Width`、`I64Pad` 之类的组合方法。

```mylis
import { Fmt, FmtSpec, FmtArg } from "Fmt.myl";

FmtSpec spec = new FmtSpec(8, 2, FmtSpec::ZERO);
println(Fmt::Format(FmtArg::F64(3.14159), spec)); // 00003.14
println(Fmt::FormatI64(-42, new FmtSpec(6, -1, FmtSpec::ZERO))); // -00042
```

- `FmtSpec` 字段：`width`（最小字符宽度，`0` 表示不补位）、`precision`（浮点小数位，`-1` 表示默认格式）、
  `flags`（位标志）。
- `flags`：`FmtSpec::LEFT`（左对齐）、`FmtSpec::PLUS`（正数加 `+`）、`FmtSpec::ZERO`（零填充）、
  `FmtSpec::SPACE`（正数前空格）。`LEFT` 优先于 `ZERO`；零填充插在负号之后，不会生成 `0-42`。
- `FmtArg` 变体：`I64` / `U64` / `F64` / `Bool` / `Str`。`Fmt::Format` 是统一入口。
- 类型化快捷方法：`Fmt::FormatI8/I16/I32/I64/U8/U16/U32/U64/F32/F64/Bool/Str`，
  内部按 `I64` / `U64` / `F64` 提升，不引入隐式类型转换。
- `FmtSpec::Default()`、`FmtSpec::Width(w)`、`FmtSpec::Left(w)`、`FmtSpec::Zero(w)`、`FmtSpec::Plus(w)`
  是常用快捷构造。
- `width` 与 `Str::Len` 一样按 UTF-8 字符计数，不按字节。
- 整数与 `bool` 不支持 `precision`；`precision` 仅对浮点有意义。
- `Bool` / `Str` 只应用 `width` 与 `LEFT`；`PLUS` / `ZERO` / `SPACE` 对它们无意义并被忽略。
- 插值语法本身仍不支持 `{x:.2f}`；显式入口用于需要控制格式的场合。

### 17.3 `match`

**语法**：

```ebnf
match_stmt ::= "match" "(" expr ")" "{" match_arm* "}"
match_arm  ::= pattern "=>" block
pattern    ::= "_" | 字面量 | "Ok" "(" bind ")" | "Err" "(" bind ")" | "Some" "(" bind ")"
             | "None" | "null" | IDENT "::" IDENT ("(" bind ("," bind)* ")")?
pattern    ::= "_"
             | "Ok" "(" ("_" | IDENT) ")"
             | "Err" "(" ("_" | IDENT) ")"
             | "Some" "(" ("_" | IDENT) ")"
             | "None"
             | INT | FLOAT | STRING | "true" | "false" | "null"
```

- `match` 是**语句**（不是表达式），分支体是 `{}` 块；
- 分支箭头是 `=>`（`->` 只用于函数返回类型，二者是不同 token）；
- 绑定：`Ok(v) => { … }` 在分支块内把载荷绑定到 `v`（类型即载荷类型）；写 `_` 表示忽略；
- 语义：按书写顺序取**首个匹配分支**；字面量模式用同型 `==` 比较；
- 整型模式支持完整 `u64` 正范围：超过 `i64::MAX` 的字面量模式（如 `18446744073709551615`）
  只能用于 `u64` 被匹配值；用于有符号/浮点类型会报类型错误。负数模式仍不支持，请用 `_` 或先转变量判断；
- **穷尽性**（编译期强制，`MYL102`）：出现 `_`；或 `Result` 的 `Ok` + `Err` 齐；或 `Option` 的
  `Some` + `None` 齐；或 `bool` 的 `true` + `false` 齐；
- **生成代码**：`if / else if` 链 + 末尾 `else { mylrt_panic_msg("non-exhaustive match"); }`，
  即使静态分析漏判也不会静默走错分支；
- 所有分支都 `return` 时，该 `match` 计为「一定返回」，可满足非 `void` 函数的返回检查
  （`block_returns` 判定）。

```mylis
fn describe(Result<i32, string> r) -> string {
    match (r) {
        Ok(v) => { return "ok {v}"; }
        Err(e) => { return "err {e}"; }
    }
}

fn grade(i32 score) -> string {
    match (score) {
        100 => { return "perfect"; }
        _ => {
            if (score >= 60) { return "pass"; }
            return "fail";
        }
    }
}
```

### 17.4 静态方法与静态成员

类体里可写两类静态成员：

```ebnf
static_method ::= "static" "fn" IDENT "(" params? ")" ("->" type)? block
static_field  ::= "static" ["const"] type IDENT "=" const_lit ";"
const_lit     ::= INT | FLOAT | STRING | "true" | "false" | "null"
```

```mylis
pub class Registry {
    pub static i32 count = 0;
    pub static string title = "registry";

    pub static fn next() -> i32 {
        Registry::count = Registry::count + 1;
        return Registry::count;
    }

    pub static fn describe() -> string {
        return "{Registry::title}: {Registry::count}";
    }
}
```

**规则**：

- 静态方法没有 `self` / `this`；方法体内使用 `this` → `MYL102`；
- 静态字段**必须**在声明处给常量字面量初值（数值 / `bool` / `string` / `null`）；表达式或 `new` → `MYL102`；
- `static const` 用于类内编译期常量，例如 `Math::PI`、`Math::E`；语义同顶层 `pub const`，访问必须带类名；
- 访问一律 `Class::name`：静态方法 `Class::f(args)`，静态字段 `Class::f = v`；
- 用实例访问静态成员 → `MYL102`（提示改用 `::`）；
- 可见性沿用第 2 章：非 `pub` 静态成员跨模块访问 → `MYL005`；
- 静态成员本质是「挂在类命名空间下的全局量」，但**不是**顶层全局变量：类型与名字都必须经类名限定，
  同名不冲突（`A::count` 与 `B::count` 是两个变量）。

**生成代码**：

```c
/* 静态字段 → 文件级全局变量 */
static int32_t S_registry_Registry_count;
static MyStr* S_registry_Registry_title;

/* 静态方法 → 无 self 参数的普通函数 */
static int32_t SM_registry_Registry_next(void) { ... }

/* 统一初始化，main 在 myrt_init() 之后调用 */
static void myl_init_statics(void) {
    S_registry_Registry_count = 0;
    S_registry_Registry_title = mylrt_str_lit("registry", 8);
}
```

### 17.5 新增词法记号

| 记号 | 文本 | 用途 |
| --- | --- | --- |
| `KwStatic` | `static` | 静态成员 |
| `KwMatch` | `match` | 分支语句 |
| `ColonColon` | `::` | 静态路径 |
| `FatArrow` | `=>` | `match` 分支箭头 |

`=>` 与 `->`、`::` 与 `:` 之间没有任何关系：前者已随 `->` 存在，后者在语法中原本不存在。

### 17.6 示例与测试覆盖

- `tests/positive/Arrays.myl`：一个文件串起四个特性 —— 数组字面量 / `new` / 下标 / `len` / 扩容、
  插值、class `to_string`、`match`、静态字段与静态方法；
- `tests/positive/{Arrays,Interp,Match,Registry}.myl`：四特性正向用例；
- `tests/negative/{M014_interp_tostring,M015_match_exhaustive,M016_static_instance,M017_array_eq}.myl`：
  反向用例，命中 `MYL102`；
- `bash tests/run_tests.sh` 共 **129 组**（正向/反向），全绿；默认 LLVM 后端另有 `tests/test_llvm.sh` 34 项（含泛型/泛型接收者）。

### 17.7 编译期派生 `@derive(ToString)` / `@derive(Json)`

`@derive(ToString)` 与 `@derive(Json)` 是本版仅有的编译期派生能力，都只接受 class：

```mylis
@derive(ToString)
class Point {
    pub i64 x;
    pub i64 y;

    pub fn init(i64 x, i64 y) {
        this.x = x;
        this.y = y;
    }
}
```

编译器读取 class 的**实例字段**（static 字段跳过），生成等价的：

```mylis
pub fn toString() -> string {
    return "Point { x: {this.x}, y: {this.y} }";
}
```

规则：

- 生成发生在编译期，运行时不保留字段名、偏移或类型表，也不存在按名字查字段的 API；
- 除检查已有 `toString` / `to_string` 冲突外，只对**字段列表**做拼接，不读取方法体、其他注解或继承信息；
- 字段类型必须能格式化：数值 / bool / string / class / enum 走字符串插值规则；`Option<T>` 生成 `Some(<T>)` / `None`；数组 `T[]` 与 `Vec<T>` 生成逐元素拼接的 `[...]`；`Map<K, T>` 生成 `{key: value}`；
- `Option` / 数组 / `Vec` / `Map` 会递归展开，因此 `Option<Option<T>>`、`Option<Vec<T>>`、`Vec<Option<T>>`、`Map<string, Vec<T>>`、`Option<Map<string, T>>` 等任意层嵌套都支持；Map 键按 ToString 规则输出（string 键不加引号）；
- class / enum 字段保留原始类型直接调用其 `toString` / `to_string`，不会在格式化时改写字段访问的语义；
- 类已有 `toString` 或 `to_string` 时报 `MYL102`，不会覆盖手写方法；
- 当前版本不支持泛型 class（`class Box<T>`）：字段类型 `T` 的字符串化链路尚未打通，`@derive(ToString)` 会直接报 `MYL102`；
- `Result` 字段暂不支持；Map 字段的声明仍需 `import { Map } from "Map.myl";`；
- 未知派生（如 `@derive(Yaml)`）、把 `@derive(ToString)` 用在 enum / interface / 函数上报 `MYL102`；
- 同一 class 可写多个 `@derive(...)`，当前支持 `ToString` 与 `Json`，其余派生名全部报未实现。

#### `@derive(Json)`

`@derive(Json)` 读取同一份实例字段列表，生成：

```mylis
pub fn toJson() -> Json
```

生成的方法体引用 `std/Json`，因此使用该派生的模块要显式导入：

```mylis
import { Json } from "Json.myl";
```

字段含 `Map<string, T>` 时还要导入 `Map`：

```mylis
import { Map } from "Map.myl";
```

字段值按声明顺序写入 Json 对象（static 字段跳过），空 class 生成 `{}`。序列化文本由 `Json::Stringify` 负责：

```mylis
@derive(Json)
class Point {
    pub i64 x;
    pub i64 y;

    pub fn init(i64 x, i64 y) {
        this.x = x;
        this.y = y;
    }
}

// Json::Stringify(new Point(1, 2).toJson()) == "{\"x\":1,\"y\":2}"
```

字段映射规则：

- 所有整型（`i8`…`i64` / `u8`…`u64`）转为 `Json::Int`，统一按 `i64` 表示；
- `f32` / `f64` 转为 `Json::Float`，`bool` 转为 `Json::Bool`，`string` 转为 `Json::Str`；
- class 字段递归调用该字段类型的 `toJson()`，因此嵌套 class 必须同样派生 `Json` 或手写 `toJson`；
- 数组 `T[]` 与 `Vec<T>` 生成 Json 数组，元素递归转换；
- `Option<T>` 的 `Some` 写入内层值的 Json，`None` 写 `null`；
- `Map<string, T>` 生成 Json 对象，键按 Map 的插入顺序输出；非 string 键暂不支持；
- 上述容器可以递归嵌套，例如 `Option<Vec<T>>`、`Vec<Option<T>>`、`Vec<Vec<T>>`、`Map<string, Vec<T>>`、`Option<Map<string, T>>`；
- enum 字段不会自动派生 `toJson`，需要该 enum 手写 `toJson()`；`Result` / interface / 非 string 键的 `Map` 暂不支持，派生直接报 `MYL102`；
- 泛型 class、非 class 目标、已有 `toJson` 方法都报 `MYL102`；
- 与 `ToString` 一样，派生只发生在编译期，运行时不保留字段名、偏移或类型表。

这不是通用的 `#[derive]` 宏系统：编译器不执行用户代码，也不提供字段枚举 / 属性读取 API；新增派生能力必须在编译器内实现。

测试覆盖：`tests/positive/DeriveToString.myl`（primitive / string / bool / 嵌套 class / class 数组字段）、`tests/positive/DeriveToStringContainers.myl`（`Option<Point>` / `Vec<Point>` / `Option<i64>` / `Vec<i64>` / 空 class）、`tests/positive/DeriveToStringNested.myl`（`Option<Vec<T>>` / `Vec<Option<T>>` / `Option<Option<T>>` / `Vec<Vec<T>>` / 数组 / `Map<string, T>` / `Map<i64, string>` / `Option<Map<string, T>>`）、`tests/positive/DeriveJson.myl`（嵌套 class / `Option` / `Vec` / 数组 / `Map<string, T>` / 多层容器 / 空对象）与 `tests/negative/{M039_derive_unknown,M040_derive_non_class,M041_derive_conflict,M042_derive_generic,M043_derive_json_conflict,M044_derive_json_non_class,M045_derive_json_generic,M046_derive_json_result,M047_derive_json_map_key}.myl`（均命中 `MYL102`）。

---

## 17.5 工程结构（`mylis.toml`）

一个目录 + 一份 `mylis.toml` 即一个工程；**无依赖解析、无锁文件、不联网**，依赖就是源码里的 `import`：

```toml
[project]
name = "myapp"

[build]
paths = ["src", "lib"]        # 模块搜索根（相对工程根；缺省即这两项）
backend = "llvm"              # 可选：llvm（默认）/ mir / ast / ir

[targets.myapp]
kind = "exe"                  # exe | static | shared
entry = "src/app.myl"
out = "build/myapp"           # 可省略，默认 build/<target 名>
```

命令：`mylc build [target]`、`mylc run [target]`、`mylc test`；单文件写法（`mylc app.myl`）不受影响。
`make all` 默认构建 LLVM 后端 + 自研 GC，把整套（`mylc`、`myls`、`std/`、`std-posix/` 或 `std-win32/`、`myrt/`、`gc/`、运行时库与六个 std native 库）收进 `bin/`，
拷贝 `bin/` 即可分发（`bin/` 自身就是 mylc 认定的 root；外部 C/clang 工具链仍需在目标机可用）。

## 17.6 工具链家族（gcc / clang / msvc，可扩展）

`mylc` 通过“家族”隔离不同 C 编译器的命令行方言：

| 家族 | 方言 |
| --- | --- |
| `gnu`（`gcc` / `clang`） | `-std=c11 -O2 -I<dir> -o out -c -shared -L<dir> -lname` |
| `msvc` | `/std:c11 /O2 /I <dir> /Fe:out /Fo<obj> /link /LIBPATH:<dir> name.lib` |

每个家族的二进制与参数由 `toolchains/*.toml` 描述（内置三份，可被同名文件覆盖），
查找目录为 `~/.mylis/toolchains`、`<安装根>/toolchains`、`<工程>/.mylis/toolchains` 与
`$MYLIS_TOOLCHAINS`。选择顺序为 `--cc` 推断 → `MYLIS_FAMILY` → 工程 `[toolchain] family`
→ 随包 `mylis-toolchain.json` 的 `family` → 平台默认（Windows 为 `msvc`，Linux/macOS 为 `gcc`）。新增编译器只需增加规格文件，不重编编译器。

运行时实现在 `mylrt-rs/`，通过 `Runtime` / `Gc` trait 选择 `own`（默认）/ `boehm` / `simple` 或第三方 `.rs` 实现；
GC 后端位于 `myrt/gc/`（`gc_boehm.c`，可选）、`mylgc/`（自研，Rust，**默认**）、`gc_simple.c`（调试），统一通过
`myrt/mylrt_gc.h` 的 C ABI 接入；自研 GC 与 IR 后端的结构预留见 `docs/后端与GC路线.md`。

MSVC 家族链接 Rust 运行时 `mylrt.lib`（由 `mylrt-rs` 生成），并链 MSVC 版自研 GC `mylgc.lib`；
`--static` / `--shared` 均支持。

## 18. 构建模式：可执行 / 静态库 / 动态库

Rust 里 `src/main.rs` 产出可执行、`src/lib.rs` 产出库，入口由 `main` 决定。Mylis 采用同样的两条线，
由编译开关决定，而不是靠文件目录约定：

| 模式 | 触发 | 入口 | 产物 |
| --- | --- | --- | --- |
| 可执行（默认） | `mylc foo.myl` | 入口文件必须声明 `pub fn main()`（否则报错） | 原生可执行文件 |
| 静态库 | `mylc --static foo.myl`（`--lib` 为别名） | **不要求** `main` | `lib<stem>.a` + `lib<stem>.h` |
| 动态库 | `mylc --shared foo.myl` | **不要求** `main` | `lib<stem>.so` / `<stem>.dll` + `<stem>.h` |
| 只要 C 源码 | `mylc --emit-c out.c foo.myl` | 同对应模式 | 单个 `.c` 文件 |
| 只检查 | `mylc --check foo.myl` | 任意 | 无 |

### 18.1 可执行模式的入口规则

- 入口文件 = 命令行传入的那个 `.myl`；
- 必须存在顶层、无所属类的 `main`，且**不能带参数**（`fn main(i32 argc)` → `MYL012`，C 侧按 `int main(void)` 调用）：
  - `fn main() -> i32`：返回值作为进程退出码；
  - `fn main() -> void`：等价于返回 0；
  - 其他返回类型：忽略返回值、按 0 退出；
- 生成的 C 里 `main` 固定为：`myrt_init()` → `myl_init_statics()` → 调用 Mylis 的 `main`；
- 入口文件缺少 `main`（且未加 `--lib` / `--check` / `--emit-c`）→ 编译器报错退出。

### 18.2 库模式

```bash
mylc --lib mathlib.myl -o libmathlib.a
# → libmathlib.a  +  libmathlib.h
```

- 生成的 C 里**没有 `main`**，取而代之的是一个导出函数 `void <stem>_init(void)`：
  它执行 `myrt_init()`（必要时再执行 `myl_init_statics()`），使用方必须先调用一次；
- `.a` 由 `cc -c` + `ar rcs` 生成，工具链由 §18.2.3 的解析规则决定；
- 头文件只列出**带 `@export` 的顶层 `pub fn`**，其余符号保持编译器内部命名，不建议外部依赖。

### 18.2.1 动态库模式

```bash
mylc --shared mathlib.myl -o libmathlib.so     # Linux/macOS
mylc --shared mathlib.myl -o mathlib.dll       # Windows
```

- Linux/macOS 用 `-shared -fPIC`；Windows MSVC 用 `cl /LD /Fe:<dll>`，MinGW 用 `gcc -shared` 并生成 `.dll.a` 导入库；两者都由 `__declspec(dllexport)` 导出；
- 同样导出 `void <stem>_init(void)`，使用方先调用一次；
- **运行时链接默认**：Linux/macOS 构建的 `mylc` 默认不把运行时链进 `.so`，宿主程序负责提供 `myrt_*`；Windows 构建的 `mylc` 默认把运行时链入 DLL（`cfg!(windows)`），`--embed-runtime` 用于在 Linux/macOS 上显式打开自包含。GNU 侧自研 GC 以 PIC 构建；Boehm 的 `libgc.a` 通常不是 PIC，直接链进 `.so` 可能报 `relocation R_X86_64_TPOFF32 ... recompile with -fPIC`。
- Windows 上调 `*.dll` 需要 **import lib**（`foo.lib` 或 `libfoo.dll.a`），不能直接给 `.dll`。

### 18.2.2 使用别人的库

| 选项 | 作用 | 透传给链接步骤 |
| --- | --- | --- |
| `-l <name>` | 链接名为 name 的库（可多次） | `-lname` |
| `-L <dir>` | 追加库搜索目录（可多次） | `-Ldir` |
| `--link <file>` | 直接链接一个 `.a` / `.o` / import lib（可多次） | 原样作为输入文件 |

外部 C 符号仍按 §5.3 声明：写在 `*_bind.myl` 的无体 `pub unsafe fn`。

```mylis
// cadd_bind.myl
pub unsafe fn c_add(i32 a, i32 b) -> i32;
```

```bash
mylc use.myl -L ./libs -l cadd -o use          # 静态库 libcadd.a
mylc use.myl -L ./libs -l cadddyn -o use_dyn   # 动态库 libcadddyn.so（运行时需 rpath/LD_LIBRARY_PATH）
```

### 18.2.3 工具链解析（不写死平台与随包路径）

`mylc` 不写死平台路径，CC/AR 与参数按以下顺序解析：

| 顺序 | 来源 | 键 |
| --- | --- | --- |
| 1 | 命令行 | `--cc`（`--ar` 未提供时由后续来源决定） |
| 2 | 环境变量 | `MYLIS_CC` / `MYLIS_AR` / `MYLIS_CFLAGS` / `MYLIS_LDFLAGS` / `MYLIS_FAMILY` |
| 3 | 工程配置 `<root>/mylis.toml` | `[toolchain]`：`cc` / `ar` / `cflags` / `ldflags` / `family` |
| 4 | 随包描述 `<root>/mylis-toolchain.json` | `{"family":"msvc","cc":"cl.exe","ar":"lib.exe"}`（相对 root） |
| 5 | 工具链规格 `toolchains/*.toml` | 规格里的 `cc` / `ar` 默认值 |
| 6 | PATH | Linux/macOS：`gcc` / `ar`；Windows：`cl` / `lib` |

- 随包工具链由打包脚本生成描述文件，编译器只认描述文件，因此换掉包内目录结构不影响 `mylc`；
- 安装目录含空格时，GNU 家族的 `gcc` 可能把路径拼进 `ld` 命令行后按空格拆开；`mylc` 会把工具链前缀与 `myrt/`、`gc/`、运行时 `.a` 复制到无空格缓存（`MYLIS_CACHE` → `LOCALAPPDATA` → `TEMP` → `TMP` → `USERPROFILE` → `SystemDrive`）。Windows 默认 MSVC 不经过这条路径；
- 后端边界：默认 LLVM 后端生成 LLVM IR → `clang`；MIR-C / AST-C 生成 C11 → `CC`/`AR`。语义层（lexer/parser/sema）与产物形态不受后端切换影响。

### 18.2.4 GC 后端抽象

运行时统一走 `myrt/mylrt_gc.h` 的四个接口，不直接调用具体 GC 实现：

| 接口 | 语义 |
| --- | --- |
| `myrt_gc_backend_init()` | 初始化（`main` 内、用户代码前调用） |
| `myrt_gc_backend_alloc(size_t)` | 分配，失败 abort |
| `myrt_gc_backend_collect()` | 主动回收，可为空实现 |
| `myrt_gc_backend_add_root_range(void*, size_t)` | 登记静态根区间，可为空实现 |

内置两份后端：

| 后端 | 源文件 | 说明 |
| --- | --- | --- |
| `own`（默认） | `mylgc/`（Rust staticlib） | 自研分代 mark-sweep（保守扫描）；GNU 用 `libmylgc.a`，MSVC 用 `mylgc.lib` |
| `boehm` | `myrt/gc_boehm.c` | Boehm-Demers-Weiser，保守式扫描；需 `libgc` |
| `simple` | `myrt/gc_simple.c` | `malloc` + 不回收；仅显式 `MYLIS_GC=simple` 调试用 |

生成的 C 只依赖运行时 ABI 头 `mylrt.h`，基本类型由 `myrt_i8..myrt_u64` 定义，不引入
C 标准库头；运行时为单线程语义（无 thread/spawn）。

自研 GC 的自动回收由 `MYLIS_GC_THRESHOLD`（字节，默认 `524288`，`0` 关闭）控制；运行时为
**单线程**语义，语言层不提供线程/并发原语。

```bash
make all                                  # 构建默认自研 GC 与运行库
MYLIS_GC=simple mylc hello.myl -o hello   # 链接不回收后端，不需要 libgc.a
MYLIS_GC=boehm mylc hello.myl -o hello    # 需先准备 libgc（make bootstrap）
```

`myrt_*` 的 ABI 不因换后端而改变（`myrt_alloc`、`myrt_register_root` 等签名不变）。

### 18.3 编译器的多语言消息

`mylc` 驱动与构建消息支持简体中文 / 英文：`--lang zh|en`、`MYLIS_LANG=zh|en`，
未指定时默认简体中文（系统 locale 仅当明确为 `zh*` 时生效）。生成 C 源码的头部注释固定为
`/* Generated by mylc -- do not edit. */`，不含任何品牌/宣传文案；诊断码 `MYLnnn` 与语言语义诊断文案
不随语言切换（后续迁移文案时直接换 `i18n::t` 的键即可，机制已就位）。

### 18.4 `@export("c_name")`：稳定 C ABI

Mylis 的符号名默认带模块前缀（如 `F_mathlib_add`），不适合作为库 ABI。给顶层 `pub fn` 加
`@export` 即固定 C 符号名：

```mylis
@export("mylis_add")
pub fn add(i32 a, i32 b) -> i32 {
    return a + b;
}

@export("mylis_greet")
pub fn greet(string name) -> string {
    return "hello, {name}!";
}
```

生成的头文件：

```c
#include "mylrt.h"

void mathlib_init(void);                 /* 库初始化，使用前调用一次 */
int32_t mylis_add(int32_t a, int32_t b);
MyStr* mylis_greet(MyStr* name);
```

C 侧用法（Linux 示例）：

```c
#include "libmathlib.h"
int main(void) {
    mathlib_init();
    printf("%d\n", mylis_add(2, 3));
    MyStr* g = mylis_greet(mylrt_str_lit("library", 7));
    printf("%s\n", myrt_str_ptr(g));
}
```

```bash
gcc -I<mylis>/myrt client.c libmathlib.a \
    <mylis>/build/libmylrt.a <mylis>/build/libmylgc.a -lm -lpthread -ldl -o client
# 若显式使用 Boehm，把 libmylgc.a 换成 <gc>/lib/libgc.a + libgc_boehm.a
```

规则：

- `@export` **只能**标注在顶层 `pub fn`（方法上、非 `pub` 函数上、类上 → `MYL102`）；
- 参数/返回类型必须能用 C 表达（数值、`bool`、`string` 以 `MyStr*` 传递、class 以 `void*`/不透明指针传递）；
- 同名 `@export` 冲突由 C 链接器报错，编译器不替你改名；
- 不加 `@export` 的 `pub fn` 仍然会被编译进 `.a`（Mylis 内部调用可用），但**不要**在头文件之外依赖其符号名。

### 18.5 库的可见性边界（与 Rust 一致）

- 库内模块之间：私有 `import` 不传递；要对外暴露子模块用 `pub import`（§7.1）；
- 库对外：只有 `pub` 声明能被其它库/程序看到；`@export` 进一步把它映射到稳定的 C 符号；
- 没有 `pub(crate)` 这类中间层级：**只有「文件内私有」与「pub」两级**（第 2 章红线）。

### 18.6 平台支持

当前验证基线：

| 平台 | 状态 | 工具链 / 说明 |
| --- | --- | --- |
| Windows x64 | 已验证 | Windows 10/11；MSVC `cl.exe` / `lib.exe`；Rust `x86_64-pc-windows-msvc`；默认 LLVM 需 LLVM 15+ `clang.exe` |
| Windows x64（MinGW） | 已验证 | Windows 10/11；MinGW-w64 `gcc.exe` / `ar.exe`；Rust `x86_64-pc-windows-gnu`；LLVM 需支持 `--target=x86_64-w64-windows-gnu` 的 clang |
| Linux x86_64 | 已验证 | `gcc` + `ar`；默认 LLVM 需 `clang`；POSIX `epoll` / `poll` |
| macOS / BSD | 按 POSIX 路径实现 | `poll` 后备；`kqueue` 优化尚未实现 |

Windows 支持 **MSVC x64** 与 **MinGW-w64 GNU x64** 两套隔离 ABI：`mylc.exe` 自身的 Rust target 决定链接 `.lib` 还是 `.a`，两套产物不能混用。MinGW 的默认 LLVM 后端用 clang 编 IR、MinGW `gcc` 链接，运行时/GC/std 侧车均为 GNU `.a`。`make test-win` 会按构建族检查工具链并运行回归；Windows 7 不在当前支持范围。

### 18.7 库模式的验证用例

`tests/lib/` 下有一组可脚本化的验收：

```bash
bash tests/test_lib.sh          # 编译库 → 链接 C 客户端 → 运行比对
```

它覆盖：`@export` 符号名、`<stem>_init`、静态字段初始化、`string` 返回值跨 ABI 传递。

---

## 附录 A：运算符与优先级

由低到高（`parse_expr` 的优先级爬升表）：

| 级别 | 运算符 | 结合性 | 说明 |
| --- | --- | --- | --- |
| 1 | `\|\|` | 左 | 逻辑或（两侧 `bool`，短路：左侧为真不再求右侧） |
| 2 | `&&` | 左 | 逻辑与（两侧 `bool`，短路：左侧为假不再求右侧） |
| 3 | `\|` | 左 | 按位或（同型整数） |
| 4 | `^` | 左 | 按位异或 |
| 5 | `&` | 左 | 按位与 |
| 6 | `==` `!=` | 左 | 相等（同型：数值 / string / class） |
| 7 | `<` `<=` `>` `>=` | 左 | 比较（同型数值或 string） |
| 8 | `<<` `>>` | 左 | 移位（同型整数） |
| 9 | `+` `-` | 左 | 加/减；`string + string` 为拼接 |
| 10 | `*` `/` `%` | 左 | 乘/除/取余；整数除零 → panic |
| — | `as` | 左 | 显式转换（低于一元运算、高于 `*` `/` `%`：`*p as i32` == `(*p) as i32`） |
| — | 一元 `!` `-` `~` `*` `&` | 右 | 逻辑非 / 取负 / 按位取反 / 解引用 / 取地址 |
| — | 后缀 `.` `?` `( )` `[ ]` `::` | 左 | 成员访问、`?` 传播、调用、下标、静态路径 |

注 1：`&` 作为一元运算符表示取地址（仅 unsafe），作为二元运算符表示按位与——由位置区分。
注 2：`=>`（match 分支箭头）不是运算符，不参与优先级；`::` 是静态路径访问后缀，与 `.` 同级。
注 3：操作数与实参按书写顺序**从左到右**求值：函数/构造函数/方法先求接收者，再从左到右求实参；二元运算先左后右；下标先容器后索引；赋值先求右值，再求左值位置。每个子表达式至多执行一次，接口装箱、写屏障等内部转换不得重复求值源表达式。

## 附录 B：字面量

| 形式 | 示例 | 说明 |
| --- | --- | --- |
| 十进制整数 | `42`、`0` | 默认 `i32`，可定向 |
| 十六进制整数 | `0xFF`、`0X10` | 同上 |
| 浮点 | `1.5`、`1e3`、`2.5E-4` | 默认 `f64`；必须有数字在 `.` 两侧之一 |
| 字符串 | `"abc"`、`"a\nb"` | 转义：`\n` `\t` `\r` `\0` `\\` `\"` `\'`；可含 UTF-8 |
| 布尔 | `true` `false` | `bool` |
| 空指针 | `null` | 仅指针 / class 类型位置 |

**不支持**：数字分隔符 `_`、二进制/八进制字面量、字符字面量、多行字符串、原始字符串。

## 附录 C：EBNF 全量语法

```ebnf
(* ---------- 词法 ---------- *)
IDENT      ::= (letter | "_") (letter | digit | "_")*
INT        ::= digit+ | "0x" hexdigit+
FLOAT      ::= digit+ "." digit+ (("e"|"E") ("+"|"-")? digit+)?
             | digit+ ("e"|"E") ("+"|"-")? digit+
STRING     ::= '"' (char | escape | "{" expr "}" | "{{" | "}}")* '"'   (* v0.2 插值 *)
escape     ::= "\\" ("n"|"t"|"r"|"0"|"\\"|'"'|"'")
comment    ::= "//" … newline | "/*" … "*/"

(* ---------- 模块 ---------- *)
module     ::= (import_decl | top_decl)* EOF
import_decl::= "pub"? "import" STRING ";"

(* ---------- 顶层 ---------- *)
top_decl   ::= annots "pub"? (class_decl | iface_decl | unsafe_fn | fn_decl)
annots     ::= ("@" IDENT ("(" annot_args? ")")?)*
annot_args ::= annot_arg ("," annot_arg)*
annot_arg  ::= INT | FLOAT | STRING | "true" | "false"

(* ---------- 类型 ---------- *)
type       ::= type_atom (("[]") | "*")*
type_atom  ::= "void"
             | "i8"|"i16"|"i32"|"i64"|"u8"|"u16"|"u32"|"u64"|"f32"|"f64"|"bool"|"string"
             | "Result" "<" type "," type ">"
             | "Option" "<" type ">"
             | IDENT

(* ---------- class / interface ---------- *)
class_decl ::= "class" IDENT "{" (class_item)* "}"
class_item ::= annots "pub"? (method | field | static_method | static_field)
field      ::= type IDENT ("=" expr)? ";"
static_method ::= "static" "fn" IDENT "(" params? ")" ("->" type)? block
static_field  ::= "static" type IDENT "=" const_lit ";"
const_lit  ::= INT | FLOAT | STRING | "true" | "false" | "null"
method     ::= "unsafe"? "fn" fn_tail
iface_decl ::= "interface" IDENT "{" (annots "fn" fn_tail_sig)* "}"
fn_tail_sig::= IDENT "(" params? ")" ("->" type)? ";"

(* ---------- 函数 ---------- *)
fn_decl    ::= "fn" fn_tail
unsafe_fn  ::= "unsafe" "fn" fn_tail
fn_tail    ::= IDENT "(" params? ")" ("->" type)? (block | ";")
params     ::= param ("," param)*
param      ::= type IDENT

(* ---------- 语句 ---------- *)
block      ::= "{" stmt* "}"
stmt       ::= block
             | "if" "(" expr ")" block ("else" (if_stmt | block))?
             | "while" "(" expr ")" block
             | "for" "(" for_init ";" expr? ";" for_step? ")" block
             | "return" expr? ";"
             | "break" ";" | "continue" ";"
             | "unsafe" block
             | var_decl
             | expr ("=" expr)? ";"
             | match_stmt
match_stmt ::= "match" "(" expr ")" "{" match_arm* "}"
match_arm  ::= pattern "=>" block
pattern    ::= "_" | 字面量 | "Ok" "(" bind ")" | "Err" "(" bind ")" | "Some" "(" bind ")"
             | "None" | "null" | IDENT "::" IDENT ("(" bind ("," bind)* ")")?
pattern    ::= "_" | "Ok" "(" ("_"|IDENT) ")" | "Err" "(" ("_"|IDENT) ")"
             | "Some" "(" ("_"|IDENT) ")" | "None"
             | INT | FLOAT | STRING | "true" | "false" | "null"
for_init   ::= var_decl | expr ("=" expr)? ";"
for_step   ::= expr ("=" expr)?

(* ---------- 表达式 ---------- *)
expr       ::= or_expr
or_expr    ::= and_expr ("||" and_expr)*
and_expr   ::= bor_expr ("&&" bor_expr)*
bor_expr   ::= bxor_expr ("|" bxor_expr)*
bxor_expr  ::= band_expr ("^" band_expr)*
band_expr  ::= eq_expr ("&" eq_expr)*
eq_expr    ::= rel_expr (("=="|"!=") rel_expr)*
rel_expr   ::= shift_expr (("<"|"<="|">"|">=") shift_expr)*
shift_expr ::= add_expr (("<<"|">>") add_expr)*
add_expr   ::= mul_expr (("+"|"-") mul_expr)*
mul_expr   ::= cast_expr (("*"|"/"|"%") cast_expr)*
cast_expr  ::= unary ("as" "?"? type)*
unary      ::= ("!"|"-"|"~"|"*"|"&") unary | postfix
postfix    ::= primary postfix_op*
postfix_op ::= "." IDENT ("(" args? ")")?
             | "?"
primary    ::= INT | FLOAT | STRING | "true" | "false" | "null" | "this"
             | "Ok" "(" expr ")" | "Err" "(" expr ")" | "Some" "(" expr ")" | "None"
             | "new" IDENT ("(" args? ")")?
             | "new" type_atom "[" expr "]"
             | "[" (expr ("," expr)*)? "]"
             | IDENT ("(" args? ")")?
             | "(" expr ")"
args       ::= expr ("," expr)*
```

## 附录 D：实现边界（本版未实现的语法面）

为避免“文档写了、代码没有”的落差，此处显式声明本版**不支持**且不属于永久红线的内容：

| 缺失项 | 现状 | 替代方案 |
| --- | --- | --- |
| 切片 / 数组追加等容器算法 | 不实现（`T[]` 只有字面量、`new`、下标、`len`） | 用 `new` 扩容 + 逐元素复制（见 `tests/positive/Arrays.myl` 的扩容写法） |
| 全局变量 / 顶层常量 | 不实现 | 用 `static` 成员（§17.4） |
| `match` 作为表达式返回值 | 不实现（`match` 是语句） | `if/else` 链或 `Result` + `?` |
| 插值格式说明符（`{x:.2f}`） | 不实现 | 先格式化成 `string` 再插入 |
| 运算符重载 | 不实现（红线 R12） | 显式方法调用 |
| 模块别名 / 选择性 import | 不实现 | 用文件划分即可避让重名 |
| 文档注释生成 | 不实现 | 注释不参与 AST |

这些限制与第 1 章红线一致：**宁可缺失能力，绝不引入隐式行为。**

## 19. 分层：语言内建 / core / std

Mylis 不分 `no_std` 与 `std` 两套语言（不存在“无 GC、无操作系统”的编译目标），但**依赖边界**必须清楚，
否则一个纯计算库会被 I/O、文件、网络能力污染。三层划分如下：

| 层 | 内容 | 依赖 | 说明 |
| --- | --- | --- | --- |
| **语言内建** | 数组 `T[]`、字符串插值、`match`、enum、`static` 成员、`?`、`new` | 无（编译期直接生成 C） | 不 import 任何 `.myl` 文件即可用 |
| **core**（`myrt` + `core/`） | 分配（GC）、字符串表示与拼接、UTF-8 字符长度/索引/截取/取字符、Unicode Trim、数字格式化、全整型溢出语义、panic、`ToString`、`Str`、`Math`、`fmt_*` | 仅 C 标准库头（`stdint/stdbool/stddef/stdlib/string/math`）+ `mylrt.h` | 库模式（`--lib`）与任何计算型程序只需要这一层 |
| **std**（`std/` 独立包） | `print` / `println`（走 `printf`）、`print_*` / `println_*`；将来的文件 / 目录 / 进程 / 网络 | core 层 + CRT | 必须显式 `import` 才链接；“不 import 就不进二进制” |

**目录约定**

```
core/                  # 编译器隐式提供的语言 core 层，禁止任何 OS 调用
├── mylis.toml         # 包清单（name = core）
├── lib.myl            # core 包入口（Fmt/Str/Math/ToString/panic/assert 及全部自由函数）
├── ToString.myl       # 预置协议（loader 隐式挂到所有模块，方法名 toString）
├── Panic.myl          # 安全 panic / assert
├── Mylrt_bind.myl     # myrt core ABI 绑定（分配 / 字符串 / GC / panic / 格式化）
├── Fmt.myl            # Fmt 类（全部数值类型、Prec / Width 系列、Format 统一入口）+ fmt_* 自由函数
├── FmtSpec.myl        # FmtSpec 类（width / precision / flags）
├── FmtArg.myl         # FmtArg 枚举（统一格式化参数）
├── Str.myl            # Str 类 + 字符语义 len / index_of / substring / char_at / char_code_at + Unicode trim + bytes / from_bytes 等
├── Ordering.myl       # Ordering 枚举（Less / Equal / Greater）
├── I8.myl             # i8 checked / wrapping / saturating 运算
├── I16.myl            # i16 checked / wrapping / saturating 运算
├── I32.myl            # i32 checked / wrapping / saturating 运算
├── I64.myl            # i64 checked / wrapping / saturating 运算
├── U8.myl             # u8 checked / wrapping / saturating 运算
├── U16.myl            # u16 checked / wrapping / saturating 运算
├── U32.myl            # u32 checked / wrapping / saturating 运算
└── U64.myl            # u64 checked / wrapping / saturating 运算

std/                   # 独立 std 包，通过依赖显式引入
├── mylis.toml         # name = std，依赖 core
└── src/
    ├── Io.myl         # Io 类 + print / println / print_*/println_*
    ├── Io_bind.myl    # std 层 C 绑定：myrt_print / myrt_println
    ├── Math.myl       # Math 类（含 PI/E）+ sqrt64 / pow64 / abs64 / min64 / max64 / clamp64 / floor64 / ceil64 / round64 / trunc64 / fmod64 / sign64
    └── Math_bind.myl  # std 层 libm 绑定
```

编译器只隐式加载 `core/`。`std` 是普通包，应用必须显式声明：

```toml
[dependencies]
std = { path = "../std" }
```

```mylis
import { Io } from "std/Io";
```

**core ABI 承诺**：`myrt/mylrt.h` 中 `myrt_*` 符号是 core 层的稳定 ABI；
`core/` 只允许依赖这些运行时符号，不绑定 C 标准库头，**禁止文件 / 目录 / 进程 / 线程 / socket / 环境变量**。

**自动检查**：`tools/check_core.sh` 扫描 `core/`，发现非白名单 `@header` 或反向依赖 `std/` 层即失败，
`make test` 与两个打包脚本都会先跑它。

**注解系统的边界**（与 Java / Rust 的对照结论）：Mylis 注解只做编译期元数据与少量静态语义
（`@export` / `@header` / `@deprecated` / `@inline`），外加一个受限的内置派生 `@derive(ToString)`
（编译器读取 class 实例字段并生成方法，见 §17.7）；没有通用宏系统，也没有运行时反射。
理由是同一条：**拒绝“代码形态取决于第三方宏或构建配置”的隐式行为**，派生能力由语言实现内置、可审计。


