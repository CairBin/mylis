# Agent 开发规范

本规范适用于所有使用 AI Agent（包括自动补全、单轮生成、多轮 Agent）修改本仓库的人。仓库**接受 AI 编写或辅助编写的代码**，但 AI 不能成为不可审计的“黑箱”：每一处改动都必须说明改了什么、为什么改、影响什么、如何验证、还有什么没有验证。

本规范不是建议，而是合并门槛。无法满足时，应先缩小改动范围或只提交调查结论，不应把未验证的代码合入。

## 1. 基本原则

1. **先读后写**：没有读过目标文件、调用方、测试和对应文档，不得生成“看起来合理”的 API。
2. **最小改动**：只改完成任务必需的代码；不做顺手的格式化、重命名、目录搬迁或无关重构。
3. **保持契约**：公开 API、C ABI、库名、文件路径、命令参数、错误码不得在未确认的情况下改变。
4. **双平台意识**：Linux/POSIX 与 Windows 是两个独立构建目标；只验证一端不能声称“跨平台通过”。
5. **证据优先**：结论必须来自源码、测试输出或命令结果；禁止编造测试通过、文件存在、链接可访问。
6. **用户文件优先**：不删除、不覆盖、不移动用户已有的文件；不清理不属于本任务的构建产物或日志。
7. **文档同步**：改 API、命令、环境变量、native 库或平台行为时，同一提交更新对应文档。
8. **可回滚**：提交应聚焦一个主题；大改动拆成可独立验证的提交，便于回滚。

## 2. 开始前必须读什么

按任务领域选择，不要只读根 README 就开始改代码。

| 顺序 | 文件 | 什么时候必读 |
| --- | --- | --- |
| 1 | `README.md` | 任何任务；确认构建方式、目录、基本约束 |
| 2 | `docs/README.md` | 任何任务；确认文档分层和单一事实源 |
| 3 | `docs/agent开发规范.md` | 任何 Agent 任务；本文件 |
| 4 | `docs/Mylis语言规范.md` 相关章节 | 改语法、类型、语义、代码生成、错误码 |
| 5 | `docs/开发者指南.md` | 改编译器、后端、运行时、C ABI、工具链 |
| 6 | `docs/std手册/README.md` + 对应章节 | 改 `std/src`、native 侧车、std 测试 |
| 7 | `docs/std平台与异步约定.md` | 改 `std-posix`、`std-win32`、事件循环、异步文件 |
| 8 | `docs/MIR.md` / `docs/后端与GC路线.md` | 改 MIR、LLVM/C 后端、GC、运行时布局 |
| 9 | `docs/Mylis库标准.md` / `docs/包获取与目录结构.md` | 改包格式、依赖、`.mylib`、native 载荷 |
| 10 | `docs/使用手册.md` 对应章节 | 改 CLI、`mylis.toml`、环境变量、用户可见行为 |
| 11 | `tools/check_naming.sh`、目标目录的 README/测试 | 写代码前确认命名、测试入口和既有约定 |

读完后在任务记录里写一句“已读文件清单”。不要把“读文件名”当成读过内容。

## 3. 仓库地图

| 路径 | 作用 | 改动风险 |
| --- | --- | --- |
| `mylc/src/` | 编译器：词法、AST、语义、MIR、C/LLVM 后端、包管理、LSP | 高；影响所有产物 |
| `mylrt-rs/` | Rust 运行时与 GC 后端，导出 `myrt_*` / `mylrt_*` C ABI | 高；ABI 或 GC 语义变化影响全部程序 |
| `core/` | 编译器隐式提供的 core 包：格式化、字符串、数学、GC 绑定 | 高；所有程序可见 |
| `std/src/` | 独立 std 包：容器、文本、文件、进程、网络、Worker 等 | 中高；公开 API 需同步 std 手册 |
| `std-posix/native/` | POSIX std native 侧车（Rust staticlib） | 中高；C ABI 与平台行为 |
| `std-win32/native/` | Windows std native 侧车（shim / 独立 event） | 中高；MSVC 与 MinGW 两套 ABI |
| `tests/` | 正向、反向、std native、LLVM、LSP、包、运行时测试 | 中；新增测试是最低验收 |
| `tools/` | 构建、命名、发布、Windows 批处理脚本 | 中；改脚本必须实跑 |
| `editors/` | VS Code / Neovim 客户端 | 中；需 `node --check` / `luac -p` |
| `docs/` | 用户、语言、std、平台、Agent 文档 | 中；必须与代码同步 |
| `toolchains/` | C 工具链规格 | 中；影响跨平台构建 |

禁止在 `build/`、`target/`、`bin/` 等生成目录里做“修复”；这些目录不是源码，重新构建会丢失。

## 4. 改动前：建立基线

在动任何源码前，先确认工作区状态并保存基线结果：

```bash
git status --short
git log --oneline -5
make test > /tmp/mylis-baseline-test.log 2>&1; echo exit=$?
```

如果基线已经失败，先判断是否与任务相关；不能把基线失败算作自己的通过结果。若工作区有用户未提交改动：

- 不清理、不 stash、不覆盖；
- 先读清楚改动内容，必要时向用户确认边界；
- 只修改与任务相关的文件，提交时排除无关改动。

Windows 基线在 MSVC 开发者环境中执行：

```powershell
cmd /c '"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" && powershell -NoProfile -ExecutionPolicy Bypass -File tests\run_tests_windows.ps1 -Backend std'
```

路径按实际 Visual Studio 版本调整；不能因为路径不同就跳过 Windows 验证。

## 5. 设计与实现规则

### 5.1 命名

- 类名、静态方法、编译期常量：大驼峰 / 全大写。
- 类内实例方法、接口方法、变量：小驼峰。
- 顶层自由函数：`snake_case`。
- 文件名与主类保持一致；`*_bind.myl` 只放 C ABI 声明，不放业务逻辑。
- 提交前必须跑：

```bash
bash tools/check_naming.sh
```

### 5.2 模块解耦

- `std` 只通过 `*_bind.myl` 调 native，不直接写平台调用。
- native 按库/协议/平台拆分：例如网络域是 `net/common.rs`、`net/tcp.rs`、`net/udp.rs`、`net/unix.rs`，不要把 TCP、UDP、Unix 混回一个文件。
- `EventLoop` 不认识 TCP/UDP/File；域适配器（`TcpEventLoop` 等）负责类型转换和 handler 注册。
- `core` 不做 OS 调用；`std` 才允许 I/O。
- 领域之间不互相 import 实现细节；需要共享的底层放独立文件，并明确所有权。

### 5.3 C ABI 与 native

- native 库当前固定六个：`file`、`io`、`time`、`net`、`event`、`process`；库名 `libmylis_<name>.a` / `mylis_<name>.lib` 不得随意改。
- 句柄是 opaque `i64`；0 通常表示无效；错误返回负 errno，成功返回 0/句柄/计数。
- `u8[]` 在 native 侧布局是 `{ u64 len; u8 items[]; }`，数据从 `+8` 开始；参数 `offset/count` 必须在数组边界内。
- C 头文件与 `*_bind.myl` 的 `@header(...)` 必须一致；`std-posix/native` 与 `std-win32/native` 各有一份，新增/改签名要同步两边。
- 改 native 网络时保持 `libmylis_net` 的导出符号集合不变；用 `nm -g` 对比拆分前后。
- 改 Windows 侧车 wrapper 时注意 `#[path]` 的模块解析目录；`std-win32/native/net.rs` 指向 `std-posix/native/net/mod.rs`，子模块相对 `net/` 目录解析。

### 5.4 编译器/运行时

- 改语法或语义必须同时更新 `docs/Mylis语言规范.md`；改错误码必须检查 `tests/negative`。
- 改 MIR 指令或 verifier 必须更新 `docs/MIR.md`，并跑 MIR-C/LLVM 相关测试。
- 改 `myrt` C ABI 或 GC 布局必须评估所有后端和 Windows/MSVC；不能只跑单后端。
- 未经用户明确指令，不引入线程模型、不改 GC/runtime 架构、不做大规模性能重构。

### 5.5 文档

- 公开 API 变化：更新 `docs/std手册` 对应表格 + 使用示例。
- CLI/环境变量/`mylis.toml` 变化：更新 `docs/使用手册.md` + 根 README。
- native/平台变化：更新 `docs/std平台与异步约定.md` + `docs/std手册/06-native-ABI与构建.md`。
- 文档中的命令必须可执行；无法验证的示例标注“示意”。

## 6. 测试规范

### 6.1 测试金字塔

| 层级 | 位置 | 适用 |
| --- | --- | --- |
| 单元/语义 | `tests/positive`、`tests/negative` | 语言行为、错误码、类型检查 |
| std API | `tests/std_native` | 真实文件/进程/socket/时间/正则/压缩 |
| std native 构建 | `tests/run_std_native_tests.sh` | 六个 native 库、C ABI、跨后端 |
| 运行时 | `tests/test_runtime_*.sh` | GC、freestanding、no_std、QEMU |
| 包/工具链 | `tests/run_package_tests.sh`、`tests/test_toolchain.sh` | `mylis.toml`、`.mylib`、C 工具链 |
| 编辑器 | `tests/test_lsp.py`、`node --check`、`luac -p` | LSP 与客户端语法 |
| Windows | `tests/run_tests_windows.ps1` | MSVC/MinGW 真实构建 |

### 6.2 新增正向用例

1. 在 `tests/positive/` 建 `<Name>.myl`，只依赖稳定的 std/core API。
2. 在同目录建 `<Name>.expected`，内容为程序 stdout 的精确文本（含换行约定）。
3. 用例要覆盖正常路径和至少一个边界（空输入、越界、重复键等）。
4. 跑：

```bash
./build/mylc tests/positive/<Name>.myl --run
```

确认输出后再用测试脚本跑全套。

### 6.3 新增反向用例

1. 在 `tests/negative/` 建 `<Name>.myl`，故意触发一个明确错误。
2. 建 `<Name>.expect`，写期望错误码（如 `MYL002`）；不写就默认 `MYL`。
3. 一个用例只验证一个错误，避免第一个错误掩盖后续问题。
4. 错误码必须来自现有规范/实现，不能自造。

### 6.4 新增 std native 用例

1. 在 `tests/std_native/` 建 `<Name>.myl` + `<Name>.expected`。
2. 覆盖真实资源：临时文件、目录、子进程、TCP/UDP/Unix socket、timer 等；测试结束要清理。
3. 若新增用例需要被自动跑，更新 `tests/run_std_native_tests.sh` 或 `tests/run_tests_windows.ps1 -Backend std` 的用例列表。
4. 手工编译示例：

```bash
./build/mylc -I std/src -I std-posix/native \
  --link build/std-posix/native/libmylis_file.a \
  --link build/std-posix/native/libmylis_net.a \
  tests/std_native/<Name>.myl -o build/tests/<Name>
```

5. Windows 用 `std-win32/native` 与 `.lib`/`.a` 对应后缀；必须在 MSVC 环境中跑。

### 6.5 新增 native C ABI 的测试

- 先加 `*_bind.myl` 声明和 C 头，再写 native 实现，最后写 Mylis 调用用例。
- 正负路径都要测：成功、无效句柄、越界 offset/count、错误 errno。
- 用 `nm -g --defined-only build/.../libmylis_<name>.a` 记录导出符号；拆分/重构前后对比。
- 不改变库名的前提下，`tests/run_std_native_tests.sh` 可复用现有 `--link` 参数。

### 6.6 测试命名

- 正向/反向用例用大驼峰，和被测类型/行为对应：`TcpReuseApi`、`RegexUnicodeCaseApi`。
- `.expected` 必须与 `.myl` 同名；不要复制粘贴后忘记改内容。
- 一个测试文件只测一个主题；不要把文件、网络、进程混在一个用例里。

### 6.7 必须记录的结果

在变更记录中写命令和真实退出码，例如：

```text
Linux: make test -> exit 0
Linux: tests/run_std_native_tests.sh -> exit 0
Windows: tests/run_tests_windows.ps1 -Backend std -> 26/26 pass
Naming: tools/check_naming.sh -> pass
ABI: nm 对比 -> 43/43 符号一致
```

不能写“测试通过”而不给命令；不能把未运行的测试写成通过。

## 7. 跨平台验证矩阵

| 改动类型 | Linux | Windows MSVC | Windows MinGW | 其他 |
| --- | --- | --- | --- | --- |
| 纯 Mylis/core | `make test` | `-Backend std` 或 `-Backend mir` | 如环境可用 | — |
| `std/src` | `make test` + `run_std_native_tests.sh` | `-Backend std` | 视改动 | — |
| `std-posix/native` | 重建六个库 + native 测试 | 重建 `.lib` + std 测试 | 若改跨平台逻辑 | macOS 若可用 |
| `std-win32/native` | 至少编译 POSIX 对照 | 必须跑 | 若支持 | — |
| 编译器/运行时 | `make test` | `make myls` / 全量 | 若改 ABI | LLVM 若可用 |
| 编辑器 | `python3 tests/test_lsp.py` | `node --check` | — | Neovim `luac -p` |

只改文档时仍要检查链接和旧名称残留；不需要重跑全部编译测试，但要在记录中说明“仅文档”。

## 8. 变更记录与提交模板

每个提交/PR 必须包含以下五段。缺少任何一段都视为不可审计。

```markdown
## 改了什么
- 文件/模块：
- 行为变化：
- 公开 API / ABI 变化：

## 为什么
- 问题/需求：
- 不这样改的替代方案与取舍：

## 影响面
- 用户可见：
- std/core：
- native/平台：
- Windows/Linux：
- 兼容性/迁移：

## 验证
- 基线：命令 + 结果
- Linux：命令 + 结果
- Windows：命令 + 结果
- 专项：命名/ABI/链接/文档检查 + 结果

## 未验证 / 风险
- 未覆盖的平台或后端：
- 已知限制：
- 需要用户确认的事项：
```

提交标题推荐：

```text
<type>(<scope>): <一句话结果>
```

`type` 用 `feat`、`fix`、`refactor`、`docs`、`test`、`build`；`scope` 用 `std`、`native`、`mylc`、`runtime`、`docs`、`editors`。

示例：

```text
refactor(std-native): 拆分网络后端为 TCP/UDP/Unix 子模块
docs(std): 新增完整 std API 手册与 Agent 开发规范
```

## 9. Agent 工作流

1. **接任务**：明确目标、非目标、验收标准；缺失关键输入时先问，不猜。
2. **读文档**：按第 2 节列清单；读目标源码、调用方、测试。
3. **建基线**：跑当前平台的最小相关测试，记录结果。
4. **定方案**：写出改动文件清单、API/ABI 影响、测试计划；大改动先给用户确认。
5. **实现**：最小 diff；一次只解决一个主题。
6. **自测**：先跑最小用例，再跑全套；修失败时不要删除测试来“通过”。
7. **跨平台**：按矩阵验证；无法验证时明确写“未验证”，不能声称通过。
8. **文档**：同步 std 手册/使用手册/平台文档；检查旧路径。
9. **变更记录**：按第 8 节写五段；核对每条命令和结果。
10. **交付**：给文件路径、提交 ID、测试结果、风险和下一步；不隐藏失败。

## 10. 禁止事项

- 禁止编造 API、参数、错误码、测试输出或文件链接。
- 禁止用 `--no-verify`、跳过测试、删除/弱化断言来让结果变绿。
- 禁止把未实现的规划写成“已支持”。
- 禁止未经用户同意改 GC/runtime 架构、线程模型、公开 ABI 或库名。
- 禁止在任务范围外批量重命名、移动目录、改格式。
- 禁止提交用户的日志、临时文件、构建产物或与本任务无关的改动。
- 禁止把“我读过了”当成“我验证过了”。

## 11. 完成定义（Definition of Done）

一个 Agent 任务只有同时满足以下条件才算完成：

- [ ] 目标与非目标明确，改动文件清单可核对。
- [ ] 公开 API/ABI 变化已说明，并有兼容性结论。
- [ ] 至少一个能复现问题的测试或可执行验证；修 bug 时先有失败证据。
- [ ] Linux 与 Windows 的验证结果分别记录；未验证项明确列出。
- [ ] `tools/check_naming.sh`、`git diff --check` 通过（涉及代码时）。
- [ ] std/CLI/平台/native 变化已同步文档。
- [ ] 变更记录五段完整，命令与结果可复制。
- [ ] 工作区没有遗留临时文件、日志或构建产物。

## 12. 相关文档

- 文档总览：[`README.md`](README.md)
- std 手册：[`std手册/README.md`](std手册/README.md)
- 平台与异步约定：[`std平台与异步约定.md`](std平台与异步约定.md)
- 开发者指南：[`开发者指南.md`](开发者指南.md)
- 语言规范：[`Mylis语言规范.md`](Mylis语言规范.md)
