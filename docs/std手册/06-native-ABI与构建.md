# native ABI 与构建

本章是 std native 后端的 ABI 参考：`*_bind.myl` 中每个 `pub unsafe fn` 都对应一个 C 导出符号，由 Rust staticlib 或 Windows shim 提供。普通库使用者不应直接调用这些函数，应使用上层的 `File`、`TcpStream`、`Process` 等类；只有实现新后端、写 C 调用方或排查符号问题时才需要读本章。

## ABI 总约定

### 库名与入口

六个 native 库名固定为 `file`、`io`、`time`、`net`、`event`、`process`：

- Linux/macOS：`build/std-posix/native/libmylis_<name>.a`
- Windows MSVC：`build/std-win32/native/mylis_<name>.lib`
- Windows MinGW：`build/std-win32/native/libmylis_<name>.a`

`net` 是**一个库**，不是四个库；TCP/UDP/Unix 是 `libmylis_net` 内部的子模块，导出符号仍以 `mylis_tcp_*`、`mylis_udp_*`、`mylis_unix_*` 区分。

### 参数布局

- 句柄：`i64`，opaque；`<=0` 通常无效。成功创建返回正句柄，失败返回负 errno。
- 字节数组：`u8[]` 在 C 侧是 `{ uint64_t len; uint8_t items[]; }`，数据从 `+8` 开始；`offset/count` 指定切片，必须满足 `0 <= offset <= offset+count <= len`。
- 字符串：`i8*` 是 NUL 结尾的 C 字符串，由调用方保证在调用期间有效。native 返回的字符串由调用方用对应的 `*_free_string` 释放；不得用 Mylis GC 回收。
- 布尔：C ABI 用 `i32` 的 0/1，不用 C99 `_Bool`。
- 返回值：句柄/计数成功为正或 0，失败为负 errno；检查类返回 1/0，参数错误返回负 errno；字符串返回指针，失败为 NULL；`void` 无返回。
- 回调：native 不直接调用 Mylis 闭包；只把完成结果放入队列，由 Mylis 主线程的泵 dispatch。

### 网络子模块布局

- `std-posix/native/net.rs` 只是 `#[path = "net/mod.rs"] mod net_impl;`。
- `std-posix/native/net/mod.rs` 装配 `common/tcp/udp/unix`。
- `std-win32/native/net.rs` 用 `#[path = "../../std-posix/native/net/mod.rs"]` 复用 POSIX 实现；Windows 特有逻辑在 `std-win32/native/event.rs` 与 shim。
- C 头按域拆分：`mylis_tcp.h`、`mylis_udp.h`、`mylis_unix.h`；`mylis_net.h` 仅聚合 include。
- 改网络 ABI 后必须对比 `libmylis_net` 的导出符号集合，不能只看 Mylis 测试通过。

### 构建与符号检查

Linux：

```bash
bash tools/build_std_native.sh
nm -g --defined-only build/std-posix/native/libmylis_net.a | rg 'mylis_(tcp|udp|unix)_'
```

Windows 必须在 MSVC 开发者环境：

```powershell
cmd /c '"C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" && powershell -NoProfile -ExecutionPolicy Bypass -File tools\build_std_native.ps1'
```

构建脚本按 `file/io/time/net/event/process` 六个入口名编译；`net` 入口再展开 `net/*.rs`。Makefile 的 `STD_WIN_STAMP`/`STD_POSIX_STAMP` 已把 `net/*.rs` 纳入依赖，新增子模块时检查这两处 wildcard 是否覆盖。

## 本章文件索引

1. `Event_bind.myl` — 自由函数
2. `File_bind.myl` — 自由函数
3. `Io_bind.myl` — 自由函数
4. `Math_bind.myl` — 自由函数
5. `Process_bind.myl` — 自由函数
6. `Stdio_bind.myl` — 自由函数
7. `Tcp_bind.myl` — 自由函数
8. `Time_bind.myl` — 自由函数
9. `Udp_bind.myl` — 自由函数
10. `Unix_bind.myl` — 自由函数

> 单文件小节按“文件职责 → 类型/字段 → 公开方法/函数”排列；`私有` 辅助函数不在手册中列出，需读源码。

## `Event_bind.myl`

**文件职责**：std EventLoop 事件后端绑定（Linux epoll，其他 POSIX poll 后备，Windows Winsock WSAPoll）。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_event_new() -> i64` | 无 | `i64` | native C ABI：创建事件后端句柄。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_event_add(i64 handle, i64 fd) -> i64` | i64 handle, i64 fd | `i64` | native C ABI：注册 fd 与事件掩码。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_event_modify(i64 handle, i64 fd, i32 write_enabled) -> i64` | i64 handle, i64 fd, i32 write_enabled | `i64` | native C ABI：修改 fd 的事件掩码。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_event_remove(i64 handle, i64 fd) -> i64` | i64 handle, i64 fd | `i64` | native C ABI：注销 fd。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_event_wait(i64 handle, i32 timeout_ms, i64[] fds, i64[] revents, i32 max) -> i32` | i64 handle, i32 timeout_ms, i64[] fds, i64[] revents, i32 max | `i32` | native C ABI：等待 fd 事件或超时，填充 fds/revents。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_event_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭事件后端句柄。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |

## `File_bind.myl`

**文件职责**：std 文件 native 后端绑定（Rust staticlib，C ABI）。 句柄是不透明 i64；data 是 Mylis u8[]，native 侧按 { u64 len; u8 items[]; } 读取。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_file_open(i8* path, i32 mode) -> i64` | i8* path, i32 mode | `i64` | native C ABI：按 mode 打开文件，返回句柄。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭文件句柄。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_exists(i8* path) -> i64` | i8* path | `i64` | native C ABI：检查路径是否存在。参数按 C ABI 传递；成功返回 1/0；参数错误返回负 errno。 |
| `mylis_file_remove(i8* path) -> i64` | i8* path | `i64` | native C ABI：删除文件。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_rename(i8* from, i8* to) -> i64` | i8* from, i8* to | `i64` | native C ABI：重命名文件。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_size(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回文件大小。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_read(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：从文件读入 u8[] 的 offset/count 区间。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_write(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：把 u8[] 的 offset/count 区间写入文件。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_seek(i64 handle, i64 offset) -> i64` | i64 handle, i64 offset | `i64` | native C ABI：设置文件偏移。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_flush(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：刷新文件缓冲。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_is_dir(i8* path) -> i64` | i8* path | `i64` | native C ABI：检查路径是否为目录。参数按 C ABI 传递；成功返回 1/0；参数错误返回负 errno。 |
| `mylis_file_mkdir(i8* path) -> i64` | i8* path | `i64` | native C ABI：创建目录。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_mkdir_all(i8* path) -> i64` | i8* path | `i64` | native C ABI：递归创建目录。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_remove_dir(i8* path) -> i64` | i8* path | `i64` | native C ABI：删除空目录。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_remove_dir_all(i8* path) -> i64` | i8* path | `i64` | native C ABI：递归删除目录。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_metadata_size(i8* path) -> i64` | i8* path | `i64` | native C ABI：返回路径对应文件大小。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_read_dir_count(i8* path) -> i64` | i8* path | `i64` | native C ABI：返回目录项数量。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_read_dir_entry(i8* path, i64 index) -> i8*` | i8* path, i64 index | `i8*` | native C ABI：返回目录项名称字符串。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_file_free_string(i8* text) -> void` | i8* text | `void` | native C ABI：释放 native 返回的字符串。参数按 C ABI 传递；无返回值。 |
| `mylis_file_tell(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回当前文件偏移。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_read_all_bytes_async(i8* path) -> i64` | i8* path | `i64` | native C ABI：提交异步读全部字节任务。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_write_all_bytes_async(i8* path, u8[] data) -> i64` | i8* path, u8[] data | `i64` | native C ABI：提交异步写全部字节任务。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_async_cancel(i64 id) -> i64` | i64 id | `i64` | native C ABI：取消异步文件任务。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_async_poll(i64[] meta) -> i64` | i64[] meta | `i64` | native C ABI：轮询异步任务元数据。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_async_take(i64 id, u8[] data) -> i64` | i64 id, u8[] data | `i64` | native C ABI：取走异步任务结果字节。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_mode(i8* path) -> i64` | i8* path | `i64` | native C ABI：返回文件权限位。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_chmod(i8* path, i32 mode) -> i64` | i8* path, i32 mode | `i64` | native C ABI：设置文件权限位。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_file_modified_millis(i8* path) -> i64` | i8* path | `i64` | native C ABI：返回文件修改时间（毫秒）。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |

## `Io_bind.myl`

**文件职责**：stdout 写入的 C 绑定（myrt_print/myrt_println）；由 Io 类导入。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `myrt_print(string text) -> void` | string text | `void` | 运行时导出：写 stdout。 |
| `myrt_println(string text) -> void` | string text | `void` | 运行时导出：写 stdout 并换行。 |

## `Math_bind.myl`

**文件职责**：libm 标量数学函数的 C 绑定；由 Math 类封装，直接调用需在 unsafe 块内。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `sqrt(f64 value) -> f64` | f64 value | `f64` | libm `sqrt` 的原始绑定；通过 `Math.Sqrt64` 使用。 |
| `pow(f64 base, f64 exponent) -> f64` | f64 base, f64 exponent | `f64` | libm `pow` 的原始绑定；通过 `Math.Pow64` 使用。 |
| `fabs(f64 value) -> f64` | f64 value | `f64` | libm `fabs` 的原始绑定；通过 `Math.Abs64` 使用。 |
| `floor(f64 value) -> f64` | f64 value | `f64` | libm `floor` 的原始绑定；通过 `Math.Floor64` 使用。 |
| `ceil(f64 value) -> f64` | f64 value | `f64` | libm `ceil` 的原始绑定；通过 `Math.Ceil64` 使用。 |
| `round(f64 value) -> f64` | f64 value | `f64` | libm `round` 的原始绑定；通过 `Math.Round64` 使用。 |
| `trunc(f64 value) -> f64` | f64 value | `f64` | libm `trunc` 的原始绑定；通过 `Math.Trunc64` 使用。 |
| `fmod(f64 left, f64 right) -> f64` | f64 left, f64 right | `f64` | libm `fmod` 的原始绑定；通过 `Math.Fmod64` 使用。 |

## `Process_bind.myl`

**文件职责**：std 进程 native 后端绑定（Rust staticlib，C ABI）。 Run 同步执行并捕获输出；Spawn 返回句柄，wait/kill/close 管理生命周期。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_process_run(i8* command) -> i64` | i8* command | `i64` | native C ABI：同步执行命令并捕获输出。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_spawn(i8* command) -> i64` | i8* command | `i64` | native C ABI：启动子进程并返回句柄。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_popen(i8* command, i8* mode) -> i64` | i8* command, i8* mode | `i64` | native C ABI：打开单向管道。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_popen2(i8* command) -> i64` | i8* command | `i64` | native C ABI：打开双向管道。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_pid(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回管道子进程 PID。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_read_all(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：从管道读到 EOF。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_read_line(i64 handle) -> i8*` | i64 handle | `i8*` | native C ABI：从管道读一行。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_process_pipe_read_line_timeout(i64 handle, i64 timeout_ms) -> i8*` | i64 handle, i64 timeout_ms | `i8*` | native C ABI：带超时从管道读一行。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_process_pipe_last_status() -> i64` | 无 | `i64` | native C ABI：读取管道最后退出状态。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_read_copy(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：把管道数据复制到 u8[]。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_write(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：向管道写字节。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_write_timeout(i64 handle, u8[] data, i64 offset, i64 count, i64 timeout_ms) -> i64` | i64 handle, u8[] data, i64 offset, i64 count, i64 timeout_ms | `i64` | native C ABI：带超时向管道写字节。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_close_input(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭管道 stdin。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭管道并回收子进程。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pipe_terminate(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：终止管道子进程。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_pid(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回子进程 PID。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_wait(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：等待子进程结束。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_last_wait_exit_code() -> i64` | 无 | `i64` | native C ABI：返回上次 wait 的退出码。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_last_wait_signal() -> i64` | 无 | `i64` | native C ABI：返回上次 wait 的信号。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_kill(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：向子进程发送终止信号。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭进程句柄。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_last_exit_code() -> i64` | 无 | `i64` | native C ABI：返回上次同步执行退出码。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_last_signal() -> i64` | 无 | `i64` | native C ABI：返回上次同步执行信号。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_process_last_stdout() -> i8*` | 无 | `i8*` | native C ABI：返回上次同步执行的 stdout。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_process_last_stderr() -> i8*` | 无 | `i8*` | native C ABI：返回上次同步执行的 stderr。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_process_free_string(i8* text) -> void` | i8* text | `void` | native C ABI：释放进程字符串。参数按 C ABI 传递；无返回值。 |

## `Stdio_bind.myl`

**文件职责**：std 标准输入/命令行参数/环境变量/退出码的 C 绑定；由 Stdio 类导入，不直接暴露给库使用者。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_io_read_line() -> i8*` | 无 | `i8*` | native C ABI：从 stdin 读一行。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_io_read_all() -> i8*` | 无 | `i8*` | native C ABI：从 stdin 读到 EOF。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_io_free_string(i8* text) -> void` | i8* text | `void` | native C ABI：释放 stdio 字符串。参数按 C ABI 传递；无返回值。 |
| `mylis_io_last_status() -> i64` | 无 | `i64` | native C ABI：返回 stdio 最近状态。参数按 C ABI 传递；返回最近一次 stdio 状态。 |
| `mylis_io_arg_count() -> i64` | 无 | `i64` | native C ABI：返回命令行参数数量。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_io_arg(i64 index) -> i8*` | i64 index | `i8*` | native C ABI：返回指定命令行参数。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_io_env_exists(i8* key) -> i64` | i8* key | `i64` | native C ABI：检查环境变量是否存在。参数按 C ABI 传递；成功返回 1/0；参数错误返回负 errno。 |
| `mylis_io_env_get(i8* key) -> i8*` | i8* key | `i8*` | native C ABI：读取环境变量。参数按 C ABI 传递；成功返回 native 分配的 NUL 结尾字符串，失败 NULL；调用方用对应的 `*_free_string` 释放。 |
| `mylis_io_exit(i32 code) -> void` | i32 code | `void` | native C ABI：以状态码退出进程。参数按 C ABI 传递；无返回值。 |

## `Tcp_bind.myl`

**文件职责**：std TCP native 后端绑定（Rust staticlib，C ABI）。 句柄是不透明 i64；u8[] 与 File 后端使用同一数组布局。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_tcp_bind(i8* address) -> i64` | i8* address | `i64` | native C ABI：绑定 TCP 监听地址。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_bind_reuse(i8* address) -> i64` | i8* address | `i64` | native C ABI：设置地址复用后绑定 TCP 监听地址。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_listener_port(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回监听端口。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_accept(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：接受 TCP 连接。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_connect(i8* address) -> i64` | i8* address | `i64` | native C ABI：连接 TCP 地址。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_listener_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭 TCP 监听器。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭 TCP 连接。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_fd(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回 TCP 连接 fd。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_listener_fd(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回监听器 fd。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_listener_set_nonblocking(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置监听器非阻塞。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_set_nonblocking(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 TCP 连接非阻塞。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_set_timeout(i64 handle, i64 milliseconds) -> i64` | i64 handle, i64 milliseconds | `i64` | native C ABI：设置 TCP 连接超时。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_read(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：从 TCP 连接读入 u8[] 的 offset/count 区间。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_write(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：把 u8[] 的 offset/count 区间写入 TCP 连接。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_set_nodelay(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 TCP_NODELAY。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_set_keepalive(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 TCP keepalive。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_set_recv_buffer(i64 handle, i32 bytes) -> i64` | i64 handle, i32 bytes | `i64` | native C ABI：设置接收缓冲。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_stream_set_send_buffer(i64 handle, i32 bytes) -> i64` | i64 handle, i32 bytes | `i64` | native C ABI：设置发送缓冲。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_tcp_listener_set_reuse_address(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置监听器地址复用。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |

## `Time_bind.myl`

**文件职责**：std wall clock、单调时钟、sleep 与本地时区偏移的 C 绑定；由 Time 类导入。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_time_now_millis() -> i64` | 无 | `i64` | native C ABI：返回 wall clock 毫秒。参数按 C ABI 传递；返回时间值（毫秒/纳秒/分钟，按函数名）；失败通常返回负 errno。 |
| `mylis_time_now_nanos() -> i64` | 无 | `i64` | native C ABI：返回 wall clock 纳秒。参数按 C ABI 传递；返回时间值（毫秒/纳秒/分钟，按函数名）；失败通常返回负 errno。 |
| `mylis_time_monotonic_millis() -> i64` | 无 | `i64` | native C ABI：返回单调时钟毫秒。参数按 C ABI 传递；返回时间值（毫秒/纳秒/分钟，按函数名）；失败通常返回负 errno。 |
| `mylis_time_now_seconds() -> f64` | 无 | `f64` | native C ABI：返回 wall clock 秒。参数按 C ABI 传递；返回秒数（f64）；失败通常返回负值。 |
| `mylis_time_sleep_millis(i64 milliseconds) -> void` | i64 milliseconds | `void` | native C ABI：睡眠指定毫秒。参数按 C ABI 传递；无返回值。 |
| `mylis_time_local_offset_minutes() -> i64` | 无 | `i64` | native C ABI：返回本地时区偏移分钟。参数按 C ABI 传递；返回时间值（毫秒/纳秒/分钟，按函数名）；失败通常返回负 errno。 |

## `Udp_bind.myl`

**文件职责**：std UDP native 后端绑定。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_udp_bind(i8* address) -> i64` | i8* address | `i64` | native C ABI：绑定 UDP 地址。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_port(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回 UDP 本地端口。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_send(i64 handle, i8* address, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, i8* address, u8[] data, i64 offset, i64 count | `i64` | native C ABI：向指定地址发送数据报。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_recv(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：接收一个数据报。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_recv_from(i64 handle, u8[] data, i64 offset, i64 count, u8[] from, i64[] from_meta) -> i64` | i64 handle, u8[] data, i64 offset, i64 count, u8[] from, i64[] from_meta | `i64` | native C ABI：接收数据报并返回来源地址。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_set_nonblocking(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 UDP 非阻塞。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_fd(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回 UDP fd。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭 UDP socket。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_set_reuse_address(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 UDP 地址复用。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_set_broadcast(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 UDP 广播。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_udp_set_ttl(i64 handle, i32 ttl) -> i64` | i64 handle, i32 ttl | `i64` | native C ABI：设置 UDP TTL。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |

## `Unix_bind.myl`

**文件职责**：std Unix domain socket native 后端绑定（POSIX / Windows AF_UNIX）。

**公开函数**：

| 签名 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mylis_unix_bind(i8* path) -> i64` | i8* path | `i64` | native C ABI：绑定 Unix domain 监听路径。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_accept(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：接受 Unix domain 连接。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_connect(i8* path) -> i64` | i8* path | `i64` | native C ABI：连接 Unix domain 路径。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_listener_fd(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回 Unix 监听器 fd。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_stream_fd(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：返回 Unix 流 fd。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_listener_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭 Unix 监听器。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_stream_close(i64 handle) -> i64` | i64 handle | `i64` | native C ABI：关闭 Unix 流。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_listener_set_nonblocking(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 Unix 监听器非阻塞。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_stream_set_nonblocking(i64 handle, i32 enabled) -> i64` | i64 handle, i32 enabled | `i64` | native C ABI：设置 Unix 流非阻塞。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_stream_set_timeout(i64 handle, i64 milliseconds) -> i64` | i64 handle, i64 milliseconds | `i64` | native C ABI：设置 Unix 流超时。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_stream_read(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：从 Unix 流读入 u8[] 的 offset/count 区间。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_stream_write(i64 handle, u8[] data, i64 offset, i64 count) -> i64` | i64 handle, u8[] data, i64 offset, i64 count | `i64` | native C ABI：把 u8[] 的 offset/count 区间写入 Unix 流。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
| `mylis_unix_remove(i8* path) -> i64` | i8* path | `i64` | native C ABI：删除 Unix socket 路径。参数按 C ABI 传递；成功返回 0、正句柄或正计数；失败返回负 errno。 |
