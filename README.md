# Mylis

一门**自建语言**：C 风格语法、原生 `MIR → LLVM IR` 后端（默认）与 C 后端，引用语义对象由可替换 GC 托管（默认自研 GC），编译产物是原生可执行文件 / 静态库 / 动态库。
编译器 `mylc` 用 Rust 写成，零第三方依赖；配套语言服务器 `myls`（VS Code / Neovim 可用）。

## 文档导航

| 想了解 | 先读 |
| --- | --- |
| 安装、命令行、工程、std API | [`docs/使用手册.md`](docs/使用手册.md) |
| 语法与语义 | [`docs/Mylis语言规范.md`](docs/Mylis语言规范.md) |
| 编译器结构、扩展点、测试 | [`docs/开发者指南.md`](docs/开发者指南.md) |
| MIR、后端与 GC | [`docs/MIR.md`](docs/MIR.md)、[`docs/后端与GC路线.md`](docs/后端与GC路线.md) |
| std 平台侧车与异步模型 | [`docs/std平台与异步约定.md`](docs/std平台与异步约定.md) |
| 库格式、依赖、目录布局 | [`docs/Mylis库标准.md`](docs/Mylis库标准.md)、[`docs/包获取与目录结构.md`](docs/包获取与目录结构.md) |

---

## 1. 快速开始

```bash
make all            # 编译 mylc + myls + 运行时；默认 LLVM 后端 + 自研 GC
```

`make all` 之后 `bin/` 是自足的：

```
bin/
├── mylc          # 编译器（Windows 为 mylc.exe）
├── myls          # 语言服务器
├── core/         # 语言 core：GC/格式化/字符串/数学/ToString/panic+assert
├── std/          # 独立 std 包（Io 等），通过包依赖显式使用
├── std-posix/    # POSIX 的 std native 侧车（Linux/macOS）
├── std-win32/    # Windows MSVC 的 std native 侧车（shim + 头文件）
├── myrt/         # 运行时头文件与源码（mylrt.h / mylrt_gc.h ...）
├── gc/           # 可选 Boehm GC 的 include + lib
└── build/        # libmylrt.a / libmylgc.a（Windows MSVC 为 .lib；Windows 下还有 std-win32/native/mylis_*.lib）
```

**拷贝 `bin/` 这一个目录就能用**（Linux/macOS 需要 GCC 兼容 C 编译器；Windows 用 MSVC `cl.exe`；默认 LLVM 后端需要 LLVM 15+ 的 `clang`）：

```bash
bin/mylc hello.myl -o hello
bin/mylc hello.myl --run
```

默认不需要 Boehm GC。只有显式切换到 Boehm 时才需要：

```bash
make bootstrap                 # 下载并编译 Boehm GC（可选）
make all
MYLIS_GC=boehm mylc app.myl -o app
```

---

## 2. 命令行

```bash
# 单文件
mylc app.myl                    # → 可执行；默认 LLVM 后端
mylc app.myl -o out/app         # 指定产物
mylc app.myl --run              # 编译后立即运行
mylc app.myl --check            # 只做语法 + 语义检查
mylc app.myl --emit-c out.c     # 只生成 C 源码（MIR-C；审 C 后端用）
mylc app.myl --emit-ir out.ir   # 只生成 MIR（审前端）
mylc app.myl --emit-llvm out.ll # 只生成 LLVM IR（原生 MIR 后端）
mylc --emit-c out.c app.myl                 # 显式走 C 源码（MIR-C）
mylc --backend mir app.myl -o app           # 显式用 MIR-C 编译
mylc --backend ast --emit-c out.c app.myl   # 回退 AST → C
mylc --backend llvm app.myl -o app          # 原生 LLVM IR → clang（默认，需 LLVM 15+）

# 三种产物
mylc app.myl                    # 可执行
mylc --static libx.myl          # 静态库：libx.a / libx.h
mylc --shared libx.myl          # 动态库：libx.so / libx.dll + libx.h

# 消费别人的库
mylc app.myl -L ./libs -l cadd              # -l 名字
mylc app.myl --link ./libs/libfoo.a         # 直接给 .a / .o / import lib
mylc --std app.myl                         # 自动加入 std 源码与当前平台 native
mylc app.myl                               # 源码或清单依赖 std 时自动启用

# 工程（见第 3 节）
mylc build [target]             # 按 mylis.toml 构建全部/单个目标
mylc build --std [target]       # 构建时自动加入 std 源码与当前平台 native
mylc run [target]               # 构建后运行
mylc test                       # 编译并运行 tests/**/*.myl，比对 .expected

# 其他
mylc -v app.myl                 # 打印模块拓扑与 C 工具链命令行
mylc --lang en app.myl          # mylc 自身的消息用英文（默认简体中文）
```

环境变量：

| 变量 | 含义 |
| --- | --- |
| `MYLIS_ROOT` | 覆盖工程根（默认识别 `myrt/mylrt.h` 所在目录） |
| `MYLIS_CC` / `MYLIS_AR` | C 编译器 / 归档器 |
| `MYLIS_CFLAGS` / `MYLIS_LDFLAGS` | 覆盖编译/链接参数（整体替换默认值） |
| `MYLIS_PATH` | 追加模块搜索路径（Windows 用 `;`，其他用 `:`） |
| `MYLIS_GC_ROOT` | 覆盖 Boehm GC 安装前缀 |
| `MYLIS_GC` | GC 后端：默认 `own`（自研分代 mark-sweep，GNU `libmylgc.a` / MSVC `mylgc.lib`）；`boehm`（需 `libgc`，先 `make bootstrap`）；`simple`（malloc 不回收，调试用） |
| `MYLIS_GC_THRESHOLD` | 自研 GC 自动回收阈值（字节，默认 `524288`，`0` 关闭；仍可显式 `myrt_gc_collect()`） |
| `MYLIS_SERVER` | 编辑器查找 `myls` 的额外位置 |
| `MYLIS_LANG=zh\|en` | 消息语言 |

---

## 3. 工程结构与包管理

一个目录 + 一份 `mylis.toml` 就是一个工程。基础单文件编译不需要包管理；包依赖支持本地 `path`、Git、本地/HTTP `.mylib` 与本地 registry，并通过 `mylis.lock` 锁定来源、版本和 checksum。

```
myapp/
├── mylis.toml
├── src/                 # 业务源码（模块搜索根 1）
│   ├── app.myl          # 含 fn main()
│   └── model/User.myl
├── lib/                 # 内部可复用库（搜索根 2）
│   └── text/Text.myl
├── tests/               # *.myl + *.expected（mylc test）
└── build/               # 产物（自动创建）
```

```toml
[project]
name = "myapp"

[build]
paths = ["src", "lib"]        # 模块搜索根（相对工程根，按序；缺省即这两项）
backend = "llvm"              # llvm（默认）/ mir / ast / ir

[targets.myapp]
kind = "exe"                  # exe | static | shared
entry = "src/app.myl"
out = "build/myapp"           # 可省略，默认 build/<target 名>

[targets.text]
kind = "static"
entry = "lib/text/Text.myl"

[toolchain]
family = "msvc"               # Windows 默认 msvc；Linux/macOS 默认 gcc
cc = "cl.exe"
ar = "lib.exe"
```

```bash
mylc build          # 全部目标（按声明顺序）
mylc build myapp    # 只编一个
mylc run myapp      # 编完直接跑
mylc test           # tests/ 下的用例
```

约定：`import` 一律从搜索根写（`import "model/User.myl"`）；`pub class User` 必须放在 `User.myl` 里（`MYL002`）。

---

## 4. 语言概览

```mylis
import { Io } from "std/Io";

// std 是普通包：应用在 mylis.toml 里声明
// [dependencies]
// std = { path = "../std" }

type Id = i32;                 // 类型重命名（纯别名）
pub type Ids = Id[];

class User {
    const i32 id;              // const 字段：只能在构造器里赋值一次
    string name;

    pub fn init(i32 id, string name) {
        this.id = id;
        this.name = name;
        println("hello, {this.name}");   // 字符串插值
    }
}

pub fn main() -> i32 {
    const i32 base = 7;        // const 绑定：不可重新赋值（MYL015）
    User u = new User(base, "mylis");
    return 0;
}
```

- **类型**：`i8..u64 / f32 f64 / bool / string`、`T*`、`T[]`、`Result<T,E>`、`Option<T>`、`class / interface / enum`；
- **类**：字段必须在声明处或构造器里初始化；静态字段/方法；无继承；
- **接口**：鸭子类型（无需 `impl`），支持内嵌组合（`interface Reader { Io2; fn read() -> string; }`）；
- **空接口与转换**：`interface Any {}` 可装箱 `i8..u64/f32/f64/bool/string`（隐式或 `as Any`），
  `as` 失败 panic、`as?` 失败返回 `Option<T>.None`；支持接口→类、接口→接口、接口→基本类型，
  也支持 `Option<Iface>`、`Result<Iface, E>`、`Result<T, Iface>`（含 `unwrap`、`?`、`match`）
  以及 `Option?` / `Result?` 传播
  和显式 `class as Interface`；
- **`match`**：穷尽检查，枚举载荷解构；
- **整数语义**：`+ - *` **回绕**（绕开 C 的有符号溢出 UB），`/ 0`、`% 0` **panic**；
- **不安全边界**：裸指针、C FFI（`*_bind.myl` 里的无体 `pub unsafe fn`）只在 `unsafe` 域内可用；
- **注解（生效的四个）**：`@export("c_name")`、`@header("x.h")`、`@deprecated`、`@inline`，拼错给 `MYLW01`。

`string` 是 **UTF-8 字节串**。内建 `s.len()` 返回字节数；`Str::len(s)` 按 Unicode 码点计数，`Str::ByteLen(s)` 返回字节数；`==` / `<` 按字节比较（对合法 UTF-8 等价于码点序）。字符级操作由 core 的 `Str::CharAt` / `CharCodeAt` 提供，`Str::Bytes` / `Str::FromBytes` 负责字节视图。

容器与算法由泛型标准库提供：`Vec<T>`、`Map<K: Eq, V>`（线性表，含 `keys` / `values`）、`HashMap<K: Hash + Eq, V>`、`BTreeMap<K: Ord, V>`（B 树，O(log n) 查找/插入/删除）、`BTreeSet<T: Ord>`、`Set<T: Eq>`（含 `toVec`）与 `Algorithms`（`contains` / `index_of` / `sort` / `extend` / `binary_search` / `is_sorted`）。所有容器在编译期单态化，不做 `Any` 装箱混存。

`Text` 提供切分/拼接/按行拆分、空白切分、整数/32 位整数/浮点与布尔解析；`Path` 提供路径拼接、取文件名/目录/扩展名/词干与 `Normalize`；`Math` 提供 `Sqrt64/Pow64/Abs64/Min64/Max64/Clamp64/Floor64/Ceil64/Round64/Trunc64/Fmod64/Sign64` 与 `PI` / `E`，位于 `std` 并允许绑定 libm。

std 还提供显式迭代器 `Iterator<T>` 与各容器迭代器、`PriorityQueue`、`Csv`、`Base64`、`Json`、`Time`、`Random`、`Log`、`Process`、`Worker`、`WorkerServer`、`WorkerHandler`、`WorkerPool`、`WorkerTaskResult`、`WorkerReply`、`WorkerResultHandler`；hosted `Stdio`（stdin/argv/环境变量/退出）、`File`（读写、目录、元数据、seek/tell）、`IoError` / `TcpListener` / `TcpStream` / `UdpSocket` / `EventLoop`。这些 hosted 接口由 `std-posix/native` 与 Windows MSVC 的 `std-win32/native` 侧车提供（Rust staticlib，C ABI；Windows 事件循环为 Winsock `WSAPoll`），不进入 `core` 或 GC；`--std` 会自动加入头文件路径和六个 native 库。

---

标准库命名约定：类名/静态方法用大驼峰（`Io::PrintlnI32`），非类内函数用
`snake_case`（`println_i32`），类内实例方法和接口方法用小驼峰（`toString`、
`equals`），变量（局部/成员/静态）用小驼峰，编译期常量全大写 + 下划线（`LEFT`、`PI`）。

## 5. 分层与运行时

| 层 | 位置 | 规则 |
| --- | --- | --- |
| 内建 | 编译器 | 只认类型、控制流、内存模型 |
| core | `core/` | **编译器只隐式提供这一层**；不做 OS 调用，只依赖 `mylrt` 运行时 ABI，不绑定 C 标准库 |
| std | `std/` | **独立包**，允许 I/O；应用通过 `mylis.toml` 依赖或 `-I std/src` 显式引入 |

内存：所有 `new` 出来的对象都在 GC 堆上（引用语义，无拷贝）；值类型在栈上。运行时通过抽象接口分配：

```
myrt/mylrt_gc.h   ← myrt_gc_backend_init / alloc / collect / add_root_range
mylgc/             ← 默认：自研分代 mark-sweep（Rust staticlib）
myrt/gc_boehm.c    ← 可选：Boehm
myrt/gc_simple.c   ← 可选：malloc 不回收（调试）
```

```bash
make all                         # 默认自研 GC
MYLIS_GC=simple mylc app.myl -o app   # 不回收，调试用
make bootstrap && make all            # 可选：准备 Boehm
MYLIS_GC=boehm mylc app.myl -o app
```

`myrt_*` 的 ABI 不随后端变化。

core 当前对外 API（`core/lib.myl` 统一导出）：

- `Fmt::I8/I16/I32/I64/U8/U16/U32/U64/F32/F64/Bool`，以及 `Fmt::F64Prec/F32Prec`（固定小数位）；
- `Fmt::F64Width/F64WidthLeft/F64WidthZero/F64WidthPlus` 与 F32 同名变体（最小宽度、左右对齐、零填充、正号）；
- 统一格式化入口：`FmtSpec` + `FmtArg` + `Fmt::Format`，另有 `Fmt::FormatI8/.../FormatStr` 类型化快捷方法：
  ```mylis
  FmtSpec spec = new FmtSpec(8, 2, FmtSpec::ZERO);
  string s = Fmt::Format(FmtArg::F64(3.14159), spec);      // "00003.14"
  string t = Fmt::FormatI64(-42, new FmtSpec(6, -1, FmtSpec::ZERO)); // "-00042"
  ```
  `FmtSpec` 的 `width` 按字符计，`precision` 为小数位（`-1` 表示默认），`flags` 支持 `LEFT/PLUS/ZERO/SPACE`；
- `Str::Concat/Equals/Compare/CompareOrdering/len/ByteLen/Bytes/FromBytes/isEmpty/StartsWith/EndsWith/Contains/IndexOf/Trim/Substring/CharAt/CharCodeAt/Repeat/Replace` 与对应自由函数；
  - `len` / `IndexOf` / `Substring` / `CharAt` / `CharCodeAt` 使用 **UTF-8 字符语义**（按 Unicode 码点计数）；
  - `CharAt` 返回单字符字符串，越界返回空串；`CharCodeAt` 返回 Unicode 码点，越界或非法序列返回 `0xFFFFFFFF`；
  - `Trim` 按 Unicode 空白集合裁剪（含全角空格 U+3000、NBSP、U+2000..U+200A 等）；
  - 字节操作先用 `Str::Bytes` 转成 `u8[]`，再用 `Str::FromBytes` 还原；
- `Eq` / `Ord` / `Hash` 接口与内置标量、字符串实现；`Hash` 提供 `hash() -> u64`，供 `HashMap` 使用；
- `Ordering` 枚举（`Less/Equal/Greater`）；
- 全部整型 `I8/I16/I32/I64/U8/U16/U32/U64` 提供 `CheckedAdd/CheckedSub/CheckedMul/CheckedDiv`、`WrappingAdd/Sub/Mul`、`SaturatingAdd/Sub/Mul`；
  - 整数字面量支持完整 `u64` 正范围（含 `18446744073709551615` 与 `0xFFFFFFFFFFFFFFFF`）；负数仍用一元 `-` 表达；
- `ToString`、`panic`、`assert`。

顶层 `pub const` 支持字面量（含一元负号）编译期替换；类内静态常量使用 `pub static const`。

---

## 6. 工具链家族（gcc / clang / msvc，可扩展）

“家族”指**命令行方言**：

| 家族 | 典型编译器 | 参数风格 |
| --- | --- | --- |
| `gcc`（gnu） | gcc、cc（Unix；Windows 不支持 MinGW 构建） | `-std=c11 -O2 -I<dir> -o out -c -shared -L<dir> -lname` |
| `clang`（gnu） | clang、llvm-ar | 同上 |
| `msvc` | cl.exe、lib.exe | `/std:c11 /O2 /I <dir> /Fe:out /Fo<obj> /link /LIBPATH:<dir> name.lib` |

每个家族的具体可执行文件与参数都写在 `toolchains/*.toml` 里，**新增一种编译器只要加一份 toml，不需要重编 `mylc`**。

### 6.1 选哪个家族（优先级从高到低）

1. `--cc <path>`：按文件名判（`cl.exe`/`clang-cl` → msvc，`clang` → clang，其余 → gnu）；
2. 环境变量 `MYLIS_FAMILY=<规格名>`；
3. 工程 `mylis.toml` 的 `[toolchain] family = "<规格名>"`；
4. 随包 `mylis-toolchain.json` 的 `"family"`；
5. 平台默认：Windows `msvc`，Linux/macOS `gcc`。

### 6.2 规格文件（插件目录）

查找顺序（同名后者覆盖前者）：

| 位置 | 用途 |
| --- | --- |
| 内置 | `gcc` / `clang` / `msvc` 三份 |
| `~/.mylis/toolchains/*.toml` | 全局自定义 |
| `<安装根>/toolchains/*.toml` | 随包（`make all` 会收进 `bin/toolchains`） |
| `<工程>/.mylis/toolchains/*.toml` | 工程私有 |
| `$MYLIS_TOOLCHAINS`（多个目录，`;`/`:` 分隔） | 临时/CI |

字段说明见 [`toolchains/README.md`](toolchains/README.md)，最小示例：

```toml
name = "zigcc"
style = "gnu"                 # gnu | msvc
cc = "C:/tools/zig.exe"
ar = "C:/tools/zig-ar.exe"
cflags = ["-std=c11", "-O2", "-fno-strict-aliasing"]
runtime = "archive"           # archive | sources
```

```bat
set MYLIS_FAMILY=zigcc
mylc hello.myl -o hello.exe
```

`mylc --verbose` 会打印实际选中的规格、cc、ar 与执行命令，排查首选它。

### 6.3 MSVC 家族的特殊之处

- 命令行全部换成 `cl` 方言，不再传 `-std`/`-o`/`-Wl,...`；
- **运行时用 Rust 静态库**：`mylrt-rs` 在 MSVC 下生成 `mylrt.lib`，通过 C ABI 与生成的代码连接，不再把 C 运行时源码编进产物；
- GC 用 **自研 GC 的 MSVC 版静态库** `build/mylgc.lib`（Rust `x86_64-pc-windows-msvc` 目标，已随包）；`MYLIS_GC=boehm` 因没有 MSVC 版 Boehm 自动改走 own 并提示，不再回退成「不回收」；
- 只有显式 `MYLIS_GC=simple` 才使用源码内联的 `gc_simple.c`（调试用，不回收）；
- `--static` 使用 `cl /c /Fo` + `lib /OUT:`；`--shared` 使用 `/LD`，导出函数带 `dllexport`。

### 6.4 配置文件与环境变量总表

**文件**

| 文件 | 位置 | 作用 |
| --- | --- | --- |
| `mylis.toml` | 工程根（`mylc build` 从 cwd 向上找） | `[project]` 名称/版本、`[build] paths/backend`、`[targets.*]`、`[native] include/files/dirs/libs`、`[toolchain] cc/ar/cflags/ldflags/family` |
| `mylis-toolchain.json` | 安装根（随包） | `{"cc":"...","ar":"...","family":"...","runtime":"..."}`，描述随包工具链 |
| `mylis-build.json` | 安装根/构建根 | 本套 `build/` 的 `os/family/abi/target/cc/ar/myrt/mylgc/simple/system_libs`；`mylc` 链接前校验 |
| `toolchains/*.toml` | 见 6.2 | 编译器家族规格（插件） |
| `mylis.toml` 的 `[toolchain]` | 工程根**优先**，安装根兜底 | 项目级编译器覆盖 |

**环境变量**

| 变量 | 作用 | 示例 |
| --- | --- | --- |
| `MYLIS_FAMILY` | 选择工具链规格 | `msvc` / `clang` / `zigcc` |
| `MYLIS_CC` | 覆盖 C 编译器 | `cl.exe`（Windows）/ `gcc`（Linux） |
| `MYLIS_AR` | 覆盖归档器 | `llvm-ar` |
| `MYLIS_CFLAGS` | 覆盖编译参数（空格分隔） | `-O3 -march=native` |
| `MYLIS_LDFLAGS` | 追加链接参数 | `-v` |
| `MYLIS_TOOLCHAINS` | 额外规格目录（多个用 `;`/`:` 分隔） | `D:/ci/toolchains` |
| `MYLIS_ROOT` | 覆盖安装根（找 `std/`、`myrt/`、`gc/`） | `D:/mylis/bin` |
| `MYLIS_PATH` | 追加模块搜索根（同 `import` 路径） | `D:/shared/lib` |
| `MYLIS_GC` | GC 后端：`own`（默认，自研 Rust GC）/ `boehm` / `simple`（不回收） | `own` |
| `MYLIS_GC_ROOT` | 覆盖 GC 目录（含 `include/gc.h`、`lib/libgc.a`） | `D:/gc` |
| `MYLIS_CACHE` | 含空格路径时工具链/运行时的缓存目录 | `D:/tmp` |
| `MYLIS_LANG` | 驱动消息语言：`zh` / `en` | `en` |
| `MYLIS_SERVER` | 编辑器找 `myls` 的位置 | `D:/mylis/bin/myls.exe` |

**优先级**（以 cc 为例）：`--cc` > `MYLIS_CC` > 工程 `mylis.toml` > 随包 `mylis-toolchain.json` > 规格文件里的 `cc` > PATH。

### 6.5 安装路径含空格

GNU 家族的 `gcc` 会把 exec prefix 拼进 ld 命令行，空格会被 ld 拆成多个参数（`cannot find C:/Program`）。
因此路径含空格时，`mylc` 会把工具链前缀与 `myrt/`、`gc/`、两个 `.a` 复制到无空格缓存
（`MYLIS_CACHE` → `LOCALAPPDATA` → `TEMP` → `TMP` → `USERPROFILE` → `SystemDrive`）。
Windows 默认的 MSVC 家族不经过这条路径（`cl` 自己处理引号）。

### 6.6 运行时实现（Rust trait）

运行时实现已迁到 `mylrt-rs/`，对外仍然只导出 `myrt/mylrt.h` 里的 C ABI，因此生成的 C / LLVM 代码完全不用改。

```text
mylrt-rs/src/
├── lib.rs       # #[no_mangle] extern "C" 导出 myrt_* / mylrt_*
├── runtime.rs   # Runtime trait + 默认字符串/数组/打印实现
├── gc.rs        # Gc trait + BackendGc（委托 mylgc / Boehm / simple）
├── plat.rs      # Platform trait：I/O、abort、时钟、f64 格式化
├── platforms/host.rs  # 默认 platform（std / POSIX / Win32）
└── impls/
    ├── own.rs       # 默认：自研 GC
    ├── boehm.rs
    └── simple.rs
```

选择实现：

```bash
make all RT_IMPL=own                 # 默认
make all RT_IMPL=boehm
make all RT_IMPL=simple
make all RT_IMPL=/path/to/my_runtime.rs
make all RT_IMPL=/path/to/rt_dir          # 读取该目录下的 runtime.rs
make all RT_IMPL=myimpl RT_IMPL_DIR=/path/to/impls  # 读取 /path/to/impls/myimpl.rs
make all RT_NO_STD=1                             # no_std Rust 运行时
make all RT_NO_STD=1 PLATFORM_IMPL=/path/to/platform.rs  # 裸机平台
make all RT_NO_STD=1 PLATFORM_IMPL=/path/to/platform_dir
make all RT_NO_STD=1 RT_NO_BACKEND_GC=1 PLATFORM_IMPL=/path/to/platform.rs
```

`RT_NO_STD=1` 使用 `#![no_std]` 构建，默认平台薄层覆盖 POSIX 与 Win32；数字/浮点格式化不依赖 `String`/`format!`。
`RT_NO_BACKEND_GC=1` 会把默认 `BackendGc`（mylgc/Boehm/simple 的委托实现）也从静态库里去掉，裸机运行时只保留自己的 `Gc`。
`PLATFORM_IMPL` 可以把 `Platform` 整体换成第三方实现；`mylrt-rs/examples/freestanding/` 提供了一个 Linux x86_64 无 libc 的完整样例（syscall I/O + 固定 arena GC），`mylrt-rs/examples/qemu-arm/` 提供了 Cortex-M3 + QEMU 的裸机启动样例（semihosting 输出，`make test` 会自动跑）。

实现文件的最小契约见 [`mylrt-rs/README.md`](mylrt-rs/README.md)。

第三方 freestanding 实现只需在任意目录写一个 `.rs` 文件，提供 `runtime()`：

```rust
use crate::{Gc, Runtime};

pub struct MyRuntime;
static GC: MyGc = MyGc;
static RT: MyRuntime = MyRuntime;

impl Runtime for MyRuntime {
    fn gc(&self) -> &dyn Gc { &GC }
}

pub fn runtime() -> &'static dyn Runtime { &RT }
```

`Runtime` 已带默认的字符串、数组、打印、panic 实现；freestanding 可以只实现自己的 `Gc`，也可以覆盖 `print` / `panic` / `alloc` 等钩子。`make` 会把 `RT_IMPL` 指向的文件通过 `MYLRT_IMPL_RS` 编译进 `libmylrt.a` / `mylrt.lib`。

---

## 7. 编辑器

- **VS Code**：`editors/vscode/`（无需 `npm install`，内置极简 LSP 客户端 + TextMate 语法）；
- **Neovim (0.8+)**：`editors/nvim/`，走内置 `vim.lsp.start`，无需 lspconfig。

`myls` 提供：诊断（与 `--check` 同源）、文档符号、悬停签名、跳转定义、补全。

---

## 8. Windows（win64）

Windows 侧的 mylc/myls 与运行时默认使用 **MSVC**（Rust 目标 `x86_64-pc-windows-msvc`，C 代码用 `cl.exe` + `lib.exe`）。Linux/macOS 默认使用 GCC。

发行包 `mylis-*-win64.zip` 的形态：

```text
mylc.exe
myls.exe
core/               # 隐式 core 模块；mylc 运行必需
std/
std-win32/          # Windows MSVC native 侧车源码与头文件
myrt/
gc/                 # 可选 Boehm；默认自研 GC 不需要
build/mylrt.lib
build/mylgc.lib
build/gc_simple.lib
build/std-win32/native/   # 六个 std native 库
docs/ mylc/src/ tests/ editors/
mylis-toolchain.json   # {"family":"msvc"}；cl.exe/lib.exe 从 PATH 解析
mylis-build.json       # 本套产物的 ABI/target/库路径，mylc 启动时校验
```

### 在 Windows 上从源码构建

```bat
:: 前提：Windows 10/11 x64；Rust（含 x86_64-pc-windows-msvc target）、
:: Visual Studio Build Tools（cl.exe/lib.exe）、GNU Make 4.x；
:: 默认 LLVM 后端还需要 LLVM 15+ 的 clang.exe
rustup target add x86_64-pc-windows-msvc
cd mylis
set CC=cl
set AR=lib
make all
```

Windows 分支默认强制 `CC=cl`、`AR=lib`，会忽略终端继承的 `CC=cc` / `AR=ar`；
命令行仍可用 `make CC=clang-cl`。如果显式传 `make CC=cc`，Makefile 会拒绝。
`make test-win` 也会锁定并在结束后恢复。

Windows 分支不需要 gcc。GNU Make 需要 4.x；GnuWin32 的 3.81 在 cmd 配方下会卡住，
可换用 ezwinports 的 `make-4.4.1-without-guile-w32-bin.zip`。

`make all` 默认：

- 后端：`BACKEND=llvm`
- GC 标签：`GC_BACKEND=own`（仅构建提示；实际选择见 `MYLIS_GC`）
- 编译器：`CC=cl`、`AR=lib`、Rust target `x86_64-pc-windows-msvc`
- 构建族：`TOOLCHAIN_FAMILY=msvc`

可覆盖：

```bat
set MYLIS_GC=simple
mylc app.myl -o app
make all CC=clang-cl
make all CARGO_TARGET=x86_64-pc-windows-msvc
```

当前 Windows 构建族只允许 `TOOLCHAIN_FAMILY=msvc` 和 `CARGO_TARGET=*-windows-msvc`；
如果改成 GNU target 或 MinGW 编译器，Makefile 会直接报错，而不是生成混合 ABI 的产物。

### 构建一致性检查

```bat
make check-toolchain   :: 检查 cl/lib/cargo 是否可用
make check-rt          :: 用当前 CC 同时链接 myrt + mylgc，并运行探针
```

`make all` 会先执行 `check-toolchain`。`mylc` 在链接前会读取 `mylis-build.json`，
校验工具链 ABI、target 和运行时库路径；清单缺失时保持旧包兼容，一旦清单存在但不匹配就拒绝编译。

### Windows 冒烟测试

在 Windows 上可以直接跑：

```bat
make test-win
:: 或
powershell -NoProfile -ExecutionPolicy Bypass -File tools\test_windows.ps1
```

脚本会：

- 自动定位并加载 `vcvars64.bat`（也可用 `-VcVars` 指定）；
- 检查 Rust MSVC target、`cl.exe`、`lib.exe`、GNU make、`clang.exe`；
- 构建 `bin/`；
- 编译并运行 hello、`u64` 模式、`FmtSpec`，分别测试默认 LLVM、MIR-C、AST-C；
- 生成并消费一个本地 `.mylib`；
- 运行 Windows 原生 std 测试：`WindowsStdApi`；File；Windows Process；TCP / EventLoop（LLVM、MIR、AST）；UDP（MIR、AST）；Worker、WorkerPool、WorkerReply、WorkerReplyInvalid、WorkerPoolReply 协议；`WorkerFlakyApi` 重复 5 次；进程与 Worker 写超时（`WindowsWriteTimeoutApi`、`WindowsWorkerWriteTimeoutApi`）。写超时用 MSVC 现场编译一个不读 stdin 的 sleeper 子进程；
- 执行 `make dist`，解开 zip 后用包内 `mylc.exe` 再跑一次，验证 `core/`、`std/src/`、`build/mylrt.lib`、`build/mylgc.lib` 和 `build/std-win32/native/` 六个 std 库都在。

常用参数：

```bat
powershell -File tools\test_windows.ps1 -Mode quick
powershell -File tools\test_windows.ps1 -Gc simple
powershell -File tools\test_windows.ps1 -SkipLlvm   :: 没有 LLVM 时只测 MIR-C/AST-C
powershell -File tools\test_windows.ps1 -SkipPackage
```

### Linux/macOS

```bash
make all                 # CC=gcc / AR=ar，默认自研 GC + LLVM 后端
make all CC=clang        # 显式指定 C 编译器
make bootstrap           # 可选：准备 Boehm
MYLIS_GC=boehm mylc app.myl -o app
```

### 用 mylc 编译 .myl 的默认后端

- 默认：**LLVM**（`MIR → LLVM IR → clang`）
- 改回 C 后端：`mylc --backend mir app.myl -o app`，或 `--backend ast` / `--backend c`
- 工程级指定：`mylis.toml`

```toml
[build]
backend = "mir"          # llvm（默认）/ mir / ast / ir

[toolchain]
family = "msvc"          # Windows；Linux/macOS 用 gnu
cc = "cl.exe"
ar = "lib.exe"
```

`--emit-c` 本身仍固定生成 C 源码（默认 MIR-C）。`--emit-llvm` 只输出 `.ll`，不调用 C 工具链。

## 9. 路线：后端抽象 / 自研 GC / IR

代码结构已经按三件事分层（细节见 [`docs/后端与GC路线.md`](docs/后端与GC路线.md)）：

| 层 | 位置 | 现状 |
| --- | --- | --- |
| 前端 | `mylc/src/{lexer,parser,program,sema,ast}.rs` | 稳定，与后端无关 |
| 后端 | `mylc/src/backend/`（`Backend` trait + `c.rs` / `ir.rs` / `mir_c.rs` / `llvm.rs`），实现细节在 `codegen/`、`mir.rs` | 默认 LLVM（原生消费 MIR，可执行/静态库/动态库）；`--backend mir` 显式 MIR-C，`--backend ast` / `--backend c` 回退 AST-C；LLVM（已覆盖标量/字符串/类/静态字段/数组/Option/Result/enum/接口与模式匹配，含装箱/拆箱、`as`/`as?` 与 `Option<Iface>`/`Result<Iface>`；库模式（静态库/动态库 + 头文件）已支持；语言采用编译期单态化泛型（类/函数/方法/接口/枚举），支持 `T: Interface` bound，后端消费已实例化的具体声明；其他未支持结构会明确报错） |
| GC | `myrt/mylrt_gc.h` + `myrt/gc/{gc_boehm,gc_simple}.c` + `mylgc/`（Rust） | 三后端：自研（默认）/ Boehm / simple |
| 分配器契约 | [`docs/裸机分配器契约.md`](docs/裸机分配器契约.md) | 裸机/RTOS 接入时必须提供可 free 复用的 arena |
| 使用手册 | [`docs/使用手册.md`](docs/使用手册.md) | 构建编译器、make/命令行参数、工程与库模式 |
| std 平台与异步约定 | [`docs/std平台与异步约定.md`](docs/std平台与异步约定.md) | std 分包、Rust 原生后端、事件循环、`.mylib` ABI 兼容规则 |
| 开发者指南 | [`docs/开发者指南.md`](docs/开发者指南.md) | 编译器架构、扩展点、测试约定与 LLVM ABI |
| Mylis 库标准 | [`docs/Mylis库标准.md`](docs/Mylis库标准.md) | 原生包 / MIR 库 / C FFI 三层边界与版本规则；P0 源码包 + P1 函数/类型 .mylib + P2 本地 registry + Git + .mylib 依赖已实现 |
| 包获取与目录结构 | [`docs/包获取与目录结构.md`](docs/包获取与目录结构.md) | 本地/Git/HTTP `.mylib`/registry 的拉取、缓存、锁文件、工程与产物目录 |
| 外部工具链 | `toolchain.rs` + `toolchains/*.toml` | gnu / clang / msvc，数据驱动插件 |

`driver` 已改为只调 `Backend`，不再直接依赖 `codegen` 的 `generate*`；新增后端只需实现 `Backend` 的 `name / emit_exe / emit_lib / header`，需要消费 MIR 库时再实现 `emit_exe_mir`。MIR 设计与 `--emit-ir` 见
[`docs/MIR.md`](docs/MIR.md)。MIR v0 已能 lower/dump/校验；默认 `mylc` 已走原生 LLVM，`--backend mir` 走 MIR-C，
覆盖当前全部正向程序，并与 AST 后端输出逐字一致。`--backend ast` 保留为回退路径。

## 10. 测试与目录

```bash
make test        # 正向/反向 127 项 + 后端对拍 80 项 + std native/库模式/工具链/MIR/[native] include/LSP/裸机验收
bash tests/run_package_tests.sh  # 嵌套模块、std 包依赖（含复制到项目 lib/）、导入可见性、原生载荷 LLVM 声明/真实链接、mixed native/MIR、泛型 .mylib（需 clang 15+）
bash tools/check_std.sh         # std 包自洽检查
bash tests/run_std_native_tests.sh  # Stdio/File/FileDir/Iter/Container/HashMap/BTree/CSV/Base64/Json/Time/Tcp/Udp/EventLoop/Process/Worker，MIR/AST/LLVM 声明
MYLIS_BACKEND=ast bash tests/run_tests.sh  # 同一套正向/反向用例跑 AST-C 回退后端
mylc test        # 只跑 tests/ 下的 .expected 比对
```

```
mylc/src/       编译器（main/cli/driver/i18n/toolchain/lexer/ast/parser/program/derive/sema/codegen/lsp/diag）
myrt/           C ABI 头文件（mylrt.h / mylrt_gc.h）
mylrt-rs/       Rust 运行时（Runtime / Gc trait + 实现）
core/, std/     core 层与独立 std 包；std-posix/native、std-win32/native 放 Rust/C ABI native 后端
tests/          positive / negative / lib / emit / test_lsp.py
tools/          bootstrap_gc.sh、build_std_native.{sh,ps1}、package_windows.{sh,ps1}、test_windows.ps1、write_manifest.{sh,ps1} 等构建/打包/测试脚本
docs/           Mylis 语言规范
```

### 明确不做

`Vec<T>` 由标准库以泛型类提供，覆盖泛型类/函数/方法/接口/枚举，并支持接口 bound（`T: Show`）与简单类型实参推断；泛型模板源码（含函数/方法体）和原模块 `import` 可随 `.mylib` 导出，消费端据此实例化；库内已产生的具体实例同时进入 MIR 段；设计见 [`docs/泛型设计.md`](docs/泛型设计.md)。通用宏、运行时反射、通用的 `#[derive]` 式代码生成、继承、隐式类型转换、`let` 语法不做；仅内置 `@derive(ToString)` 与 `@derive(Json)` 编译期派生（读取 class 实例字段生成 `toString` / `toJson`，不保留运行时类型表），见语言规范 §17.7。包管理只覆盖本地 path / Git / 本地与 HTTP `.mylib` / 本地 registry，不做远程 registry 与 workspace 子包。
