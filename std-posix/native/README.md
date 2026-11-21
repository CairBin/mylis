# std native backend

当前有 File、stdio、Process、TCP/UDP 与事件后端：

- `file.rs`：Rust `staticlib`，导出 C ABI `mylis_file_*`；包含文件读写、目录枚举、目录创建/删除、元数据大小、seek/tell
- `io.rs`：stdin、argv、环境变量与进程退出，导出 C ABI `mylis_io_*`
- `process.rs`：shell 命令执行（`/bin/sh -c`，Windows 为 `cmd /C`）；`run` 同步捕获 stdout/stderr，`spawn` 返回子进程句柄并支持 pid/wait/kill/close，`popen` 提供单向字节管道，`popen2` 提供 stdin/stdout 双向管道；`pipe_read_line` 按行读取（1 读到一行、0 EOF、负数 `-errno`），`pipe_read_line_timeout` 增加毫秒超时（超时 `-110`）；导出 C ABI `mylis_process_*`；不做异步回调
- `time.rs`：wall clock、单调时钟与 sleep，导出 C ABI `mylis_time_*`
- `net.rs`：Rust `staticlib`，导出 C ABI：`mylis_tcp_*` / `mylis_udp_*`
- `event.rs`：事件后端；Linux `epoll`，其他 POSIX `poll`，导出 `mylis_event_new/add/remove/wait/close`
- `mylis_file.h` / `mylis_io.h` / `mylis_time.h` / `mylis_net.h` / `mylis_event.h` / `mylis_process.h`：对应的 C 头，供 `*_bind.myl` 的 `@header` 使用
- 构建：`bash tools/build_std_native.sh`，产物 `build/std-posix/native/libmylis_file.a`、`libmylis_io.a`、`libmylis_time.a`、`libmylis_net.a`、`libmylis_event.a` 与 `libmylis_process.a`
- 使用：编译时加 `-I std-posix/native`，并显式 `--link` 对应静态库；`--std` 会自动加入上述六个库

`mylis_io_read_line` / `mylis_io_read_all` 通过返回空串表示 EOF 或读错误，具体状态由
`mylis_io_last_status()` 给出（1 成功、0 EOF、负数错误）。`mylis_io_env_get` 对不存在的变量返回空串，
是否存在的准确判断用 `mylis_io_env_exists()`。

这不是嵌入 `.mylib` 的 native 载荷；按包约定，最终链接者必须显式提供非嵌入 native。
当前在 Linux/POSIX 上验证；Windows MSVC/GNU 的静态库 ABI 与打包尚未验证。
