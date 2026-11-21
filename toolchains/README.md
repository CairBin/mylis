# 工具链规格（插件目录）

`mylc` 的编译器家族由这里的数据文件描述，**新增编译器不需要重编 `mylc`**。
把 `*.toml` 放到以下任一目录即可被发现（后面的覆盖前面的同名规格）：

1. `~/.mylis/toolchains/`        全局
2. `<安装根>/toolchains/`          随包
3. `<工程>/.mylis/toolchains/`     工程私有
4. `$MYLIS_TOOLCHAINS` 指向的目录   临时/CI

选择哪个规格：`MYLIS_FAMILY=<name>` → 工程 `mylis.toml` 的 `[toolchain] family` →
`mylis-toolchain.json` 的 `family` → 默认 `gcc`。内置 `gcc`、`clang`、`msvc` 三份。

## 字段

| 键 | 说明 | 示例 |
| --- | --- | --- |
| `name` | 规格名（`MYLIS_FAMILY` 用它选择） | `mycc` |
| `style` | 命令行方言：`gnu` 或 `msvc` | `gnu` |
| `cc` | 编译器可执行文件（可绝对路径/相对安装根） | `gcc` |
| `ar` | 归档器 | `ar` |
| `cflags` | 编译参数数组 | `["-std=c11","-O2"]` |
| `ldflags` | 额外链接参数数组 | `[]` |
| `include` | 头文件目录模板，`{path}` 占位 | `-I{path}` |
| `libdir` | 库目录模板 | `-L{path}` |
| `link_lib` | 库名模板，`{name}` 占位 | `-l{name}` |
| `obj_suffix` | 目标文件后缀 | `.o` |
| `static_prefix` / `static_suffix` | 静态库前后缀 | `lib` / `.a` |
| `shared_prefix` / `shared_suffix` | 动态库前后缀 | `lib` / `.so` |
| `runtime` | `archive`（链 `libmylrt.a`）或 `sources`（把 myrt 源码编进产物） | `archive` |

`style = "msvc"` 时 `mylc` 会改用 `/I /Fe: /Fo: /link /LIBPATH:`，并自动以
`runtime = "sources"` 方式编入运行时（`gc_simple` 后端）。

## 例子：换一个 GCC 兼容编译器

```toml
name = "zigcc"
style = "gnu"
cc = "C:/tools/zig.exe"      # 或包一层 cc 的脚本
ar = "C:/tools/zig-ar.exe"
cflags = ["-std=c11", "-O2", "-fno-strict-aliasing"]
```

```bat
set MYLIS_FAMILY=zigcc
mylc hello.myl -o hello.exe
```
