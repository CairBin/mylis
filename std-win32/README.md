# std-win32

`std` 的 Windows native 侧车包，同时支持 MSVC 与 MinGW-w64 GNU ABI。

- `native/file.rs`：文件后端，复用跨平台实现，MSVC 产物 `mylis_file.lib`，MinGW 产物 `libmylis_file.a`
- `native/io.rs`：stdio/argv/env/exit 后端，产物 `mylis_io.lib` / `libmylis_io.a`
- `native/process.rs`：进程后端；命令为可执行文件路径（无参数）时直接 spawn，否则 `cmd /C`。Run/Spawn/Popen/Popen2 与 POSIX 同语义；读超时用 `PeekNamedPipe` + 分块 `ReadFile`，写超时用复制句柄 + 后台线程 + `CancelSynchronousIo` 回收，`Stdio::Exit` 用 `TerminateProcess`，产物 `mylis_process.lib` / `libmylis_process.a`
- `native/time.rs`：wall clock、单调时钟与 sleep，产物 `mylis_time.lib` / `libmylis_time.a`
- `native/net.rs`：TCP/UDP；`nativeFd()` 返回 64 位 Windows `SOCKET`，产物 `mylis_net.lib` / `libmylis_net.a`
- `native/event.rs`：Winsock `WSAPoll` 事件后端，产物 `mylis_event.lib` / `libmylis_event.a`
- `native/mylis_*.h`：对应 C header，签名与 `std-posix/native` 保持一致

构建（Windows，Rust 目标决定 ABI）：

```powershell
# MSVC
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build_std_native.ps1
# MinGW
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build_std_native.ps1 `
  -Target x86_64-pc-windows-gnu -Out build\std-win32\native
```

或在 Linux 上交叉验证（需要安装对应 Rust target）：

```bash
TARGET=x86_64-pc-windows-msvc bash tools/build_std_native.sh
TARGET=x86_64-pc-windows-gnu bash tools/build_std_native.sh
```

产物在 `build/std-win32/native/`。消费端用 `--std` 时会自动加入
`std-win32/native` 头文件目录和六个静态库；手工编译时显式加：

```bat
:: MSVC
-I std-win32\native ^
--link build\std-win32\native\mylis_file.lib ^
--link build\std-win32\native\mylis_process.lib ^
--link build\std-win32\native\mylis_net.lib ^
--link build\std-win32\native\mylis_event.lib

:: MinGW
-I std-win32\native ^
--link build\std-win32\native\libmylis_file.a ^
--link build\std-win32\native\libmylis_process.a ^
--link build\std-win32\native\libmylis_net.a ^
--link build\std-win32\native\libmylis_event.a
```

Windows 侧车只支持 64 位。MSVC 链接还需要 `ws2_32.lib` 等系统库；MinGW 用 `-lws2_32 -lntdll ...`（`--std`/构建清单已包含）。两套 ABI 的静态库不能混用。
