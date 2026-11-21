# std-win32

`std` 的 Windows MSVC native 侧车包。

- `native/file.rs`：文件后端，复用跨平台实现，产物 `mylis_file.lib`
- `native/io.rs`：stdio/argv/env/exit 后端，产物 `mylis_io.lib`
- `native/process.rs`：`cmd /C` shell 进程后端；Run/Spawn/Popen/Popen2 与 POSIX 同语义，管道读超时用 `PeekNamedPipe`，写超时用复制句柄 + 等待线程，产物 `mylis_process.lib`
- `native/time.rs`：wall clock、单调时钟与 sleep，产物 `mylis_time.lib`
- `native/net.rs`：TCP/UDP；`nativeFd()` 返回 64 位 Windows `SOCKET`，产物 `mylis_net.lib`
- `native/event.rs`：Winsock `WSAPoll` 事件后端，产物 `mylis_event.lib`
- `native/mylis_*.h`：对应 C header，签名与 `std-posix/native` 保持一致

构建（Windows + MSVC target）：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/build_std_native.ps1
```

或在 Linux 上交叉验证（需要安装 `x86_64-pc-windows-msvc` Rust target）：

```bash
TARGET=x86_64-pc-windows-msvc bash tools/build_std_native.sh
```

产物在 `build/std-win32/native/`。消费端用 `--std` 时会自动加入
`std-win32/native` 头文件目录和六个静态库；手工编译时显式加：

```bat
-I std-win32\native ^
--link build\std-win32\native\mylis_file.lib ^
--link build\std-win32\native\mylis_process.lib ^
--link build\std-win32\native\mylis_net.lib ^
--link build\std-win32\native\mylis_event.lib
```

Windows 侧车只支持 64 位 MSVC ABI；MSVC 链接时还需要 `ws2_32.lib`（`--std`/构建清单已包含）。
