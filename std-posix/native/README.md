# std native backend

当前有 File、stdio、Process、TCP/UDP/Unix 与事件后端：

- `file.rs`：Rust `staticlib`，导出 C ABI `mylis_file_*`；包含文件读写、目录枚举、目录创建/删除、元数据大小、seek/tell，以及 `ReadAllBytesAsync` / `WriteAllBytesAsync` 的固定工作线程池与完成队列
- `io.rs`：stdin、argv、环境变量与进程退出，导出 C ABI `mylis_io_*`
- `process.rs`：shell 命令执行（POSIX 用 `/bin/sh -c`；Windows 下可执行文件路径直接 spawn，否则 `cmd /C`）；`run` 同步捕获 stdout/stderr，`spawn` 返回子进程句柄并支持 pid/wait/kill/close，`popen` 提供单向字节管道，`popen2` 提供 stdin/stdout 双向管道；`pipe_read_line` 按行读取（1 读到一行、0 EOF、负数 `-errno`），`pipe_read_line_timeout` 增加毫秒超时（超时 `-110`），`pipe_write_timeout` 增加毫秒写超时（超时 `-110`，可能部分写入），`pipe_terminate` 强制结束并回收子进程；导出 C ABI `mylis_process_*`；不做异步回调。Windows 读侧用 `PeekNamedPipe` + 分块 `ReadFile`，写超时用复制句柄 + 后台线程 + `CancelSynchronousIo` 回收，`Stdio::Exit` 用 `TerminateProcess` 绕开 teardown 死锁
- `time.rs`：wall clock、单调时钟与 sleep，导出 C ABI `mylis_time_*`
- `net.rs`：Rust `staticlib`，导出 C ABI `mylis_tcp_*` / `mylis_udp_*` / `mylis_unix_*`；入口只负责模块装配，具体实现按协议拆分在 `net/`：
  - `net/common.rs`：错误码、句柄抽象、数组布局、socket option 常量与原始 socket 声明
  - `net/tcp.rs`：TCP 监听/连接/读写与 TCP socket option
  - `net/udp.rs`：UDP 绑定/收发/广播/TTL 与 UDP socket option
  - `net/unix.rs`：Unix domain socket 的 POSIX 实现与 Windows AF_UNIX 适配
- `event.rs`：事件后端；Linux `epoll`，其他 POSIX `poll`，导出 `mylis_event_new/add/remove/wait/close`
- `mylis_file.h` / `mylis_io.h` / `mylis_time.h` / `mylis_event.h` / `mylis_process.h`：对应的 C 头，供 `*_bind.myl` 的 `@header` 使用
- `mylis_tcp.h` / `mylis_udp.h` / `mylis_unix.h`：网络域按协议拆分的 C 头；`mylis_net.h` 保留为兼容聚合头
- 构建：`bash tools/build_std_native.sh`，产物 `build/std-posix/native/libmylis_file.a`、`libmylis_io.a`、`libmylis_time.a`、`libmylis_net.a`、`libmylis_event.a` 与 `libmylis_process.a`
- 使用：编译时加 `-I std-posix/native`，并显式 `--link` 对应静态库；`--std` 会自动加入上述六个库

`mylis_io_read_line` / `mylis_io_read_all` 通过返回空串表示 EOF 或读错误，具体状态由
`mylis_io_last_status()` 给出（1 成功、0 EOF、负数错误）。`mylis_io_env_get` 对不存在的变量返回空串，
是否存在的准确判断用 `mylis_io_env_exists()`。

这不是嵌入 `.mylib` 的 native 载荷；非嵌入 native 不随源码包/`.mylib` 自动传播。使用 `--std` 时会自动加入当前平台的头文件与六个库，手工编译才需要显式 `--link`。
Linux/POSIX 与 Windows MSVC 侧车均已验证（Windows 共用 `file/io/process/time` 与 `net.rs` 入口，`event.rs` 独立 WSAPoll）。
