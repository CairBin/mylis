# std 平台分包与异步约定

> 状态：本文同时记录**当前实现**和**尚未落地的约定**；带“规划 / 未实现”标注的段落不是现状。
> 适用范围：`std` 的文件、网络、事件循环与平台原生后端。
> 不适用范围：`mylrt-core`、裸机运行时、GC 根扫描模型、`async/await` 语言语法。

## 1. 总原则

- `core` 与 `mylrt-core` 保持裸机约定：不依赖 libc、不依赖 OS、不做线程。
- 文件、网络、线程、事件循环全部属于 hosted 能力，只放在 `std` 侧。
- `Path` 是无 I/O 的纯 Mylis 工具，不进入平台后端。
- 平台差异只允许出现在 `std` 的平台包或其原生后端，不进入 `mylrt-core`。
- Rust 只作为 `std` 的 native backend，不作为运行时核心。
- 当前不引入 `async/await` 语法；异步能力由单线程事件循环和完成事件队列提供。

## 2. 分包结构

```text
std                    # 公共 Mylis API：File / TcpListener / TcpStream / EventLoop / IoError
std-posix              # POSIX 侧车包；native/ 下是 POSIX 与跨平台 Rust 实现
std-posix/native       # file.rs / io.rs / process.rs / time.rs / net.rs / event.rs
std-win32              # Windows MSVC / MinGW 侧车包；native/ 下是同名 shim 与 WSAPoll 事件后端
std-win32/native       # file/io/process/time/net.rs 复用 std-posix 实现；event.rs 独立
```

当前规则：

- 应用只 import 顶层 `std`，不直接 import 平台包。
- `std` 公共 API 不出现平台类型：不出现 `pollfd`、`HANDLE`、`sockaddr`、`SOCKET`。
- 平台包通过构建目标选择，不通过运行时探测。
- 平台包可以携带自己的 native 静态库/动态库；不得把平台库塞进 `mylrt-core`。
- 裸机工程不依赖 `std`，因此不会拉到任何 OS 符号。
- Windows 支持 **MSVC x64** 与 **MinGW-w64 GNU x64**：`std-win32` 的 shim 复用 `std-posix/native` 的跨平台代码，事件后端单独实现；MSVC 产出 `.lib`，MinGW 产出 `libmylis_*.a`。
- 纯 Mylis 的 `std` 公共 API 可以共享；带 native 载荷的平台包不能跨 ABI 共享。

**规划（未实现）**：把 Windows 两套 ABI 拆成独立的 `std-win32-msvc` / `std-win32-gnu` 包，以及把平台包拆成可独立分发的 `.mylib`。当前两套 ABI 都由同一个 `std-win32` 源码目录构建。

## 3. 公共 API

### 3.1 错误

统一使用 `Result<T, IoError>`，不引入异常。

```myl
enum IoErrorKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    InvalidInput,
    WouldBlock,
    TimedOut,
    Closed,
    Other,
}

class IoError {
    pub IoErrorKind kind;
    pub i32 code;
    pub string message;
}
```

### 3.2 文件

```myl
class File {
    pub static fn Open(string path) -> Result<File, IoError>;
    pub static fn Create(string path) -> Result<File, IoError>;
    pub static fn ReadAllBytesAsync(string path, FileHandler handler) -> Result<FileTask, IoError>;
    pub static fn WriteAllBytesAsync(string path, u8[] data, FileHandler handler) -> Result<FileTask, IoError>;
    pub fn readAllBytes() -> Result<u8[], IoError>;
    pub fn writeAllBytes(u8[] data) -> Result<i64, IoError>;
    pub fn readAllText() -> Result<string, IoError>;
    pub fn writeAllText(string text) -> Result<i64, IoError>;
    pub fn flush() -> Result<i64, IoError>;
    pub fn close() -> void;
}

interface FileHandler {
    fn onReadDone(i64 taskId, Result<u8[], IoError> result) -> void;
    fn onWriteDone(i64 taskId, Result<i64, IoError> result) -> void;
}

class FileTask {
    pub i64 id;
    pub i32 kind;
    pub fn cancel() -> bool;
}
```

约定：

- `File` 显式 `close`；GC 不负责关闭文件描述符。
- 同步 API 仍是阻塞读写；异步 API 当前只有 `ReadAllBytesAsync` / `WriteAllBytesAsync` 两个入口。
- 异步任务返回 `FileTask`，必须 `EventLoop.addFile(task)` 注册；`FileHandler.onReadDone` / `onWriteDone` 在主线程执行。`task.cancel()` 是尽力取消，已进入阻塞 I/O 的任务可能仍完成，并以 `IoError(125)` 返回。
- 异步文件后端是固定 Rust 工作线程池（默认 2 个，`MYLIS_ASYNC_FILE_THREADS` 可配置 1～8）；完成结果只放在 Rust 堆，主线程再拷入 `u8[]`，工作线程不接触 Mylis GC。
- TCP 为阻塞/非阻塞 socket；`accept` / `connect` / `read` 在阻塞模式下占住当前线程，非阻塞模式返回 `WouldBlock`。
- 路径类型使用 `string`；Windows 后端负责 UTF-8 -> UTF-16。
- 二进制内容用 `u8[]`，文本用 UTF-8 `string`。
- 编译器目前不支持 `Result<void, E>`，所以 `writeAllText` / `flush` 返回 `Result<i64, IoError>`。
- POSIX native 实现在 `std-posix/native/file.rs`；Windows 的 `std-win32/native/file.rs` 是 shim，复用同一跨平台实现。`--std` 会自动加入对应头文件与 `libmylis_file.a`（MinGW）/ `mylis_file.lib`（MSVC）。
- EventLoop 的 POSIX `epoll`/`poll` 实现在 `std-posix/native/event.rs`，Windows 为 `std-win32/native/event.rs` 的 Winsock `WSAPoll`；`--std` 会自动链接 `libmylis_event.a` / `mylis_event.lib`。
- Process 的 POSIX 实现在 `std-posix/native/process.rs`，Windows 由 `std-win32/native/process.rs` shim 复用同一份代码；`Run` 同步捕获输出，`Spawn` 返回句柄并支持 pid/wait/kill/close，`Popen` 提供单向字节管道、`Popen2` 同时提供 stdin/stdout，`ProcessPipe::readLine` / `readLineTimeout` 支持不关闭 stdin 的逐行请求-响应。Windows 的读超时用 `PeekNamedPipe` + 分块 `ReadFile`；写超时用复制句柄 + 后台线程，超时后用 `CancelSynchronousIo` 回收。Worker 基于这套行协议做进程隔离消息层，WorkerPool 在其上做固定进程池，并用 `callAllResults` 对单任务失败做隔离重启；`callAllReplies` 把结构化响应接到池级，每个任务用 `errorCode` 区分进程/传输层失败、用 `reply.ok` 区分远端业务失败；`WorkerServer::RunResult` / `Worker::callResult` 用单行 JSON 区分业务失败与协议失败，但不从 native 线程回调 Mylis。

### 3.3 网络

```myl
class TcpListener {
    pub static fn Bind(string address) -> Result<TcpListener, IoError>;
    pub fn localPort() -> i64;
    pub fn accept() -> Result<TcpStream, IoError>;
    pub fn setNonblocking(bool enabled) -> Result<i64, IoError>;
    pub fn close() -> void;
}

class TcpStream {
    pub static fn Connect(string address) -> Result<TcpStream, IoError>;
    pub fn read(i64 maxBytes) -> Result<u8[], IoError>;
    pub fn writeAllBytes(u8[] data) -> Result<i64, IoError>;
    pub fn writeAllText(string text) -> Result<i64, IoError>;
    pub fn setNonblocking(bool enabled) -> Result<i64, IoError>;
    pub fn setTimeout(i64 milliseconds) -> Result<i64, IoError>;
    pub fn close() -> void;
}
```

约定：

- 地址先使用 `"host:port"` 字符串。
- 阻塞 API 与事件循环共用同一套句柄和错误模型。
- `WouldBlock` 是合法结果，不视为致命错误；事件循环据此重试。
- UDP 提供 `UdpSocket` 的 `Bind` / `localPort` / `sendTo` / `sendText` / `recv` / `recvFrom` / `close`；`recvFrom` 返回 `UdpPacket(data, from)`。
- 当前编译器不支持 `Result<void, E>`，`setNonblocking` / `setTimeout` 返回 `Result<i64, IoError>`。
- POSIX native 实现在 `std-posix/native/net.rs`；Windows 由 `std-win32/native/net.rs` shim 复用同一跨平台实现并走 Winsock。`--std` 会自动链接 `libmylis_net.a` / `mylis_net.lib` 与对应系统库（MinGW 为 `-lws2_32` 等）。

## 4. 异步模型

### 4.1 当前：单线程 Reactor + 事件队列

- 平台后端负责等待 I/O 就绪，产出事件。
- `std` 的 `EventLoop.run()` 在 Mylis 主线程消费事件并调用 handler。
- 它是一个 hosted 网络事件循环，不是通用事件总线：可注册对象只有 `TcpListener` / `TcpStream` / `UdpSocket`，另外带一个异步 `FileTask` 完成泵；没有定时器、信号、任意 fd 或自定义事件源接口。
- 已落地：`TcpListenerHandler` / `TcpStreamHandler` / `UdpHandler` 三个独立 interface + `EventLoop.add/addListener/addUdp/remove/removeListener/removeUdp/run/stop/close`；
  Linux 用 `epoll`，其他 POSIX 用 `poll` 后备，Windows 用 Winsock `WSAPoll`；Rust 只返回就绪 fd 与 `revents`，不执行 Mylis 回调。
- Rust 线程不直接调用 Mylis 代码，因此不需要为 Rust 工作线程注册 GC 根。
- 回调使用 interface，而不是闭包；注册哪类 socket 就实现对应的 interface，不需要为其他协议补空方法：

```myl
interface TcpListenerHandler {
    fn onAccept(TcpListener listener) -> void;
    fn onListenerError(TcpListener listener, IoError error) -> void;
}

interface TcpStreamHandler {
    fn onReadable(TcpStream stream) -> void;
    fn onWritable(TcpStream stream) -> void;
    fn onError(TcpStream stream, IoError error) -> void;
}

interface UdpHandler {
    fn onUdpReadable(UdpSocket socket) -> void;
    fn onUdpError(UdpSocket socket, IoError error) -> void;
}

class EventLoop {
    pub fn add(TcpStream stream, TcpStreamHandler handler) -> void;
    pub fn addListener(TcpListener listener, TcpListenerHandler handler) -> void;
    pub fn addUdp(UdpSocket socket, UdpHandler handler) -> void;
    pub fn remove(TcpStream stream) -> void;
    pub fn removeListener(TcpListener listener) -> void;
    pub fn removeUdp(UdpSocket socket) -> void;
    pub fn addFile(FileTask task) -> void;
    pub fn run() -> void;
    pub fn stop() -> void;
    pub fn close() -> void;
}
```

### 4.2 平台后端

- Linux：已实现 `epoll`；其他 POSIX 暂用 `poll` 后备。
- macOS / BSD：当前走 `poll` 后备，`kqueue` 优化待补。
- Windows：Winsock `WSAPoll`，实现在 `std-win32/native/event.rs`。
- Rust 侧只使用标准库和系统 API，不引入 `mio` / Tokio 等第三方运行时。
- 所有后端只产出事件，不执行 Mylis 回调。
- 默认只注册读就绪；显式 `EventLoop.setWriteInterest(stream, true)` 后注册写就绪并触发 `onWritable`。
- `onError` 分发 TCP 流错误；`onListenerError` 分发监听器错误；`onUdpReadable` 分发 UDP 读就绪；`onUdpError` 分发 UDP socket 错误。

### 4.3 文件异步（当前最小实现）

- Rust 固定工作线程池只执行文件 I/O；完成结果进入 native 完成队列，Mylis 主线程通过 `EventLoop.run()` 泵入并分发回调。
- 当前完成延迟最多约 100ms（沿用事件等待的超时）；暂不做 `eventfd`/本地 socket 唤醒。
- 工作线程不得分配或访问 Mylis GC 对象；读结果先放在 Rust `Vec<u8>`，主线程 `take` 时拷入 `u8[]`。
- 当前入口只有 `File.ReadAllBytesAsync` / `File.WriteAllBytesAsync`；不做异步 open/seek/append，也不做 IOCP / `io_uring`。
- 因为 `EventLoop` 现在带文件完成泵，手工链接 EventLoop 的程序也需要同时链接 `libmylis_file.a` / `mylis_file.lib`；`--std` 会自动包含。

### 4.4 明确不做

- 不引入 `async` / `await` 语法。
- 不做多线程 reactor。
- 不做绿色线程/栈式协程。
- 不做 GC 线程安全改造。
- 不做 IOCP / `io_uring` 的当前版本实现。

## 5. Rust native backend 约定

- Rust 侧只导出 `extern "C"` ABI，不导出 Rust ABI。
- 句柄使用不透明 `i64` 或 `void*`，不传 Rust `String` / `Vec` / `Result`。
- 分配与释放成对：`open/close`、`create/destroy`。
- Rust panic 不得跨越 FFI 边界；边界处捕获或使用 `panic=abort`。
- native backend 不引用 Mylis 符号，不调用 Mylis 代码。
- native backend 不负责 GC 对象分配。
- 平台包负责携带 native 库并配置链接；应用不需要知道 native 库路径。

## 6. 工具链与 `.mylib` 兼容性

### 6.1 纯 MIR `.mylib`

`--emit-mylib` 默认产物包含：

- `api`：Mylis 源码/模板
- `mir`：MIR 文本
- `mir-json`：MIR JSON
- 头部 `abi = "mylis-1"`

**不包含** native 载荷时，`.mylib` 与编译器的 Rust 构建方式无关。  
`mylc` 自身的 Rust 工具链不改变 MIR/API 内容；只要消费端使用相同 `mylis-1` ABI 和可用运行时，纯 MIR `.mylib` 可通用。当前 Windows 发行同时提供 MSVC（`win64.zip`）与 MinGW（`win64-mingw.zip`）两套构建。

### 6.2 带 native 载荷的 `.mylib`

使用 `--embed-static` / `--embed-shared` 时，`.mylib` 内含平台二进制库。此时：

- **不可跨 ABI 通用**：Windows GNU（MinGW）静态库不能给 MSVC 链接路径使用，反之亦然。
- **不可跨平台通用**：Linux `.a` / `.so` 不能给 Windows 使用。
- native 载荷必须与消费端 `mylis-build.json` 的 `abi` / `target` / `cc` 一致。

### 6.3 `.mylib` 的原生目标元数据

`abi = "mylis-1"` 是 Mylis 归档格式 ABI，不是原生库 ABI。带 native 载荷的归档现在额外记录：

```text
native_target = "x86_64-unknown-linux-gnu"
native_abi = "gnu"
```

- 这两个字段只在 `--embed-static` / `--embed-shared` 时写入，纯 MIR 归档不写。
- 值来自编译器构建清单 `mylis-build.json` 的 `target` / `abi`。
- 字段参与 checksum body，不能静默篡改。
- 消费端读取归档后，用当前构建清单校验；不匹配直接拒绝，不允许“先链接再看错误”。
- 当前构建清单没有这些字段时（例如未安装构建清单的裸机工具链），只跳过校验，不伪造元数据。

### 6.4 `mylc` 自身构建方式的影响

`mylc` 是编译器进程，它的 Rust 工具链只影响：

- 编译器二进制自身；
- 编译器内部依赖；
- 编译器默认调用的 CC/AR（如果未显式指定）。

它不决定 Mylis 程序的 ABI。Mylis 产物 ABI 由以下因素决定：

- `mylis-build.json` 的 `abi` / `target` / `myrt` / `mylgc`
- 实际链接的运行时库
- 实际 C 编译器（MSVC 或 GNU）
- native `.mylib` 载荷

因此：

- 两个 `mylc` 都走 MIR-LLVM、都使用同一套 Windows 构建清单与同一 ABI 的运行时（同为 MSVC，或同为 MinGW）：
  - 纯 MIR `.mylib` 通用；
  - 带 native 载荷的 `.mylib` 只要 native 载荷同为该 ABI（MSVC 对 MSVC，MinGW 对 MinGW），也通用。
- 一个 `mylc` 的构建链是 GNU、运行在 GNU 平台；另一个是 MSVC、运行在 Windows：
  - 纯 MIR `.mylib` 仍可读；
  - 带 native 载荷的 `.mylib` 不可混用；
  - 消费端仍必须通过 `mylis-build.json` 的 ABI 校验。

### 6.5 必须保持的校验

- `mylc` 链接前继续校验 `mylis-build.json` 的 `abi` / `target` / `myrt` / `mylgc`。
- 纯 MIR `.mylib` 只校验 `abi = mylis-1` 和 MIR 版本。
- 带 native 载荷的 `.mylib` 增加目标 ABI 校验。
- 平台包只能链接同一 `abi` / `target` 的 native 库。

## 7. 测试约定

- `test_runtime_bare.sh` / `test_runtime_nostd.sh` 必须保持通过，证明裸机约定未受影响。
- `core` 边界检查必须继续通过：`core` 不 import `std`、不出现 OS 符号。
- `std` 平台包各自做：
  - 文件：临时目录、文本/二进制、缺失文件、权限错误。
  - 网络：回环监听、连接、读写、`WouldBlock`、关闭。
  - 事件循环：单线程回调、停止、错误事件。
- 三后端：MIR / LLVM / AST-C 对同一 `std` API 输出一致；平台 native 库只通过 C ABI。
- 包级测试：
  - 纯 MIR `.mylib` 跨编译器构建方式可消费；
  - native `.mylib` 的 ABI 不匹配必须被拒绝。

## 8. 分发方式

### 8.1 当前状态

当前 `std` 是**源码包**，不是 `.mylib`：

- `std/mylis.toml` 指向 `src/lib.myl`；
- 依赖 `core = { path = "../core" }`；
- `make bin` 把整个 `std/` 目录复制到 `bin/std/`；
- Windows 打包脚本把 `std/` 与 `std-win32/` 复制到发布目录，并构建 `build/std-win32/native/mylis_*.lib`；
- 测试/示例通过 `MYLIS_PATH` 定位 `std/src`；
- 当前 `std` 直接用 `@header("mylrt.h")` 绑定运行时，例如 `std/src/Io_bind.myl`；
- File / TCP 的 native 后端在 `std-posix/native/file.rs` / `net.rs`（Rust staticlib），Windows 由 `std-win32/native` 的同名 shim 复用；`--std` 会自动选择当前平台的头文件目录和六个库，手工编译时才需要显式 `-I` / `--link`。

因此当前分发的本质是：**跟着编译器发布包一起分发源码目录**，没有独立的 `std.mylib`，也没有完整拆分的平台包。

当前已经落地的平台侧车是 POSIX 的 `std-posix/native` 和 Windows 的 `std-win32/native`（MSVC 与 MinGW 两套 ABI）：都是非嵌入 Rust staticlib，`--std` 会按编译器目标选择对应头文件目录和库（`.lib` 或 `libmylis_*.a`）。按平台拆分的独立 `.mylib` 仍未实现。

### 8.2 规划（未实现）：按平台拆成可分发包

以下内容不是当前构建方式；当前仍使用 `std` 源码包 + `std-posix` / `std-win32` 侧车。

目标布局：

```text
std                     # 公共 API 包：纯 Mylis / 纯 MIR，可跨平台共享
std-posix               # POSIX 平台实现包
std-win32-msvc          # Windows MSVC native backend（规划中的拆分名）
std-win32-gnu           # Windows GNU/MinGW native backend（规划中的拆分名）
```

分发规则：

- `std` 公共 API 包不带 native 载荷，保持纯 MIR/源码，可跨编译器构建方式共享。
- `std-posix` / `std-win32-*` 平台包可以带 native 载荷；带 native 时按目标 ABI 分发，不跨 ABI 共享。
- 平台包优先用 `.mylib` 携带 API 声明 + native 静态库；Mylis 包装层放公共 `std`，不要和 native 载荷塞进同一个 `.mylib`，否则 `has_native()` 会跳过 MIR。
- native 部分作为 `.mylib` 载荷，不散落到 `mylrt-core`。
- 应用只依赖 `std`；当前构建目标选择 `std-posix`（POSIX）或 `std-win32`（Windows，MSVC 或 MinGW 由 `mylc` ABI 决定）；按 ABI 拆分的独立 `.mylib` 平台包仍属规划。
- 发布包必须携带对应平台的 native 库和 `mylis-build.json`；不能只带 Mylis 源码却声称支持该平台。
- 纯 MIR 的 `std` 包可以只发布源码或纯 MIR `.mylib`；一旦带 native，就必须记录 `target` / `abi`。

如果未来把 `std` 后端拆成独立平台包，则：

- `std` 不能作为**单一预编译包**跨平台分发。
- 可以让 `std` 保留一份公共 API 源码，平台实现放在 `std/platform/{posix,win32-msvc,win32-gnu}`。
- 预编译分发时仍必须按目标拆成不同包或不同 native 载荷。
- 当前 `mylis.toml` 没有按 target 选择依赖的能力；单包源码可以靠构建脚本选择平台目录，规划中的预编译包必须显式选择 `std-win32-msvc` / `std-win32-gnu`。
- 公共 API 不能暴露平台类型；平台句柄保持不透明。

### 8.3 版本与锁定

- `std` 与平台包版本必须一致，例如 `std = 0.1.0` 对应 `std-posix = 0.1.0`。
- `mylis.lock` 记录平台包路径或 `.mylib` checksum。
- 平台 native 库的 ABI/target 必须与消费端 `mylis-build.json` 一致，否则拒绝链接。

### 8.4 import 不传递 native 载荷

- 包 A 里 `import std` 只是依赖声明，不会把 std 的 native 库自动打包进 A。
- 默认 `--emit-mylib` 生成的 A 只含 A 的 `api + mir + mir-json`，不含 std 实现。
- 只有显式 `--embed-static` / `--embed-shared` 才会把指定 native 库嵌入 A。
- B 导入 A 时，A 里的 `import std` 由 B 的构建目标解析，链接 B 选定的 std 平台包。
- 因此纯 MIR 的 A 可以跨编译器构建方式共享；带 native 的 A 按 A 自己的 target ABI 分发。
- 应用最终生成的可执行文件/动态库会链接 std native，但这是最终产物的依赖，不是 A 包的内容。

### 8.5 外部 C 库依赖

`.mylib` L 依赖某个 C 库时，有两种打包方式：

**不嵌入 native**

- `--emit-mylib` 只生成 L 的 `api + mir + mir-json`。
- C 库不进入 L；消费方必须自己链接：
  - `--link libfoo.a` / `--link foo.lib` / `--link foo.dll`
  - 或 `-L <dir> -l foo`
- 消费方还需要 C 头文件路径（`-I` 或根清单 `[native] include`）和对应系统库。
- `mylis.toml` 的 `[native]` 现在支持 `include` / `files` / `dirs` / `libs`，编译时合并进 `-I` / `--link` / `-L` / `-l`；`import L` 仍不会自动传递 L 的 C 库路径。
- C 库自己的传递依赖（如 zlib、pthread、ws2_32）也必须由消费方链接；嵌入 L 只带 L 指定的库，不会自动带它的依赖。
- 优点：L 保持纯 MIR，可跨编译器构建方式共享；L 的 Mylis 实现仍会被合并。
- 缺点：消费方必须自己处理 C 库、头文件和链接顺序。

**嵌入 native**

```bash
mylc L.myl --emit-mylib L.mylib --embed-static libfoo.a
mylc L.myl --emit-mylib L.mylib --embed-shared libfoo.so
mylc L.myl --emit-mylib L.mylib --embed-shared foo.lib   # Windows 导入库
```

- L.mylib 内含 native 载荷；消费方链接 L 时自动提取并链接，不需要再手动 `--link`。
- 这是 ABI 相关的包：MSVC 与 MinGW、Linux 与 Windows 不能混用。
- 当前链接语义是：`has_native()` 为真时**优先使用 native 载荷，不再合并 L 的 MIR**。
- 因此嵌入只适合“L 是 C 库的 ABI 包装层，公开符号由 native 库实现”的情况。
- 如果 L 自己还有 Mylis 实现代码，不能只把 C 依赖嵌入 L；否则 L 的 MIR 会被跳过，运行时找不到 L 自己的函数。
- 归档中写入 `native_target` / `native_abi` 元数据并参与 checksum；读取/链接时按消费端 `mylis-build.json` 校验。Windows 支持与 API 名对应的 `.lib` 导入库（MSVC）或 `.a` / `.dll.a`（MinGW），DLL 的运行时部署仍需调用方负责。

### 8.6 链接参数写在哪里

链接参数可以来自**消费方的构建命令**，也可以来自当前包的 `mylis.toml`：

- 命令行：`--link`、`-l`、`-L`、`-I`、`--link-mylib`
- `mylis.toml` 的 `[native]`：`include`、`files`、`dirs`、`libs`
- `Makefile` / CI 脚本：把同样的参数拼进 `mylc` 调用
- `mylis-build.json`：只描述运行时和系统库（`myrt` / `mylgc` / `system_libs`），不是项目 C 依赖的通用链接配置
- `mylis.lock`：只记录依赖来源和 checksum，不记录链接参数

`[native]` 示例：

```toml
[native]
include = ["vendor/include"]
files = ["vendor/libfoo.a", "vendor/foo.def"]
dirs = ["vendor/foo/lib"]
libs = ["foo", "ws2_32"]
```

语义：

- `include` 映射到 `-I`，`files` 映射到 `--link`，`dirs` 映射到 `-L`，`libs` 映射到 `-l`。
- 相对路径相对 `mylis.toml` 所在目录解析，不相对当前工作目录。
- 清单参数与命令行参数合并；`include` 和命令行 `-I` 同时加入 Mylis 模块搜索根。
- 当前只作用于**当前编译的根包**；依赖包自己的 `[native]` 不会自动传递给消费方。
- `import L` 不会自动带出 L 的 C 库路径；L 若需要自动链接，仍应以带 native 载荷的 `.mylib` 分发。

### 8.7 std 平台后端的链接责任

- 当前 `std` 通过 `@header("mylrt.h")` 使用 `mylrt`；`mylc` 已经自动链接 `mylrt`，用户不需要额外链接。
- 使用 `--std` 时，`mylc` 会自动加入当前平台的头文件目录和六个 std native 库（POSIX `.a` / Windows `.lib`）；未用 `--std` 的手工编译仍需 `--link` / `-l`，或由构建脚本代劳。
- 平台包以带 native 载荷的 `.mylib` 分发时：`--link-mylib` / `mylis.toml` 的 `mylib` 依赖会自动提取并链接，不需要用户手写 `-l`。
- 平台包带 native 时，必须与消费端 ABI 匹配；自动链接不等于跨 ABI 可用。
- 动态库即使自动链接，运行时仍需能找到 DLL/SO；静态库最省事。

## 9. `.mylib` 依赖打包语义

### 9.1 推荐语义：薄归档 + 传递依赖

- `B.mylib` 默认只包含 B 自己的 API / MIR，不包含依赖 A 的实现。
- `B.mylib` 头部或元数据记录依赖列表：包名、版本、来源、checksum、目标 ABI。
- C 依赖 B 时，`mylc` 按 B 的依赖元数据递归解析 A；C 不需要手写 A 和 B 两套链接参数。
- 这样避免同一份 A 被每个库重复打包，也避免重复符号和版本冲突。
- 该语义已经在 `mylib::emit` / `mylib::read` 落地：`B.mylib` 的 `--- mir` 只包含 B 的函数，依赖 A 只出现在 `--- deps`。

当前 `--- deps` 格式：

```text
--- deps
dep = "A|0.1.0|mylib+../A.mylib|sha256-..."
```

字段依次为包名、版本、来源、checksum；checksum 为空时保留末尾空字段。`--- deps` 在 checksum body 内，篡改依赖元数据会导致校验失败。

消费端行为：

- `--link-mylib B.mylib` 会递归读取 B 的 `mylib+...` 依赖，先注入 A 的 API，再注入 B 的 API；
- MIR 合并顺序为依赖在前、B 在后；
- C 不需要再手写 `--link-mylib A.mylib`。

当前限制：

- 自动递归只处理来源为 `mylib+...` 的依赖；`path+...` / `registry+...` / `git+...` 仍由包解析器按 `mylis.lock` 处理，不能只靠 `.mylib` 元数据定位。
- 相对路径 `mylib+../A.mylib` 按归档所在目录解析；若依赖归档不在该目录，需把 A 与 B 放在一起，或改用包依赖。
- `decl_count` 仍记录编译期完整声明表长度，用于 MIR id 平移；它不是“归档内函数数”。

### 9.2 选择原则

- 标准库、通用库优先薄归档：B 只带 B，依赖由元数据传递。
- 只有明确需要分发的独立工具库，才考虑自包含归档。
- 自包含归档必须保证依赖 API 别名、类型身份、checksum、版本都一致，否则很容易重复定义。

### 9.3 C 的静态库与动态库

`.mylib` 薄归档是**包层语义**；最终 C 的 `.a` / `.so` / `.dll` 是**链接产物语义**，两者不强制相同。

**可执行文件**

- 递归解析 B 的依赖 A。
- 把 B、A 的 MIR 一起编译进最终可执行文件。
- 这是最常见的路径。

**动态库 `.so` / `.dll`**

- 推荐把纯 MIR 依赖 B/A 静态编译进 C 的动态库，产物自包含；消费方只需要 C.so / C.dll。
- 如果依赖是原生共享库，可以记录 DT_NEEDED / DLL 导入；消费方运行时要部署 B/A。
- 纯 MIR 依赖不能作为动态库依赖直接存在，必须先编译成对象或 native 库。

**静态库 `.a` / `.lib`**

静态库只是对象归档，链接器不会读 `.mylib` 的依赖元数据，所以只有两种选择：

1. **自包含静态库**
   - 构建 C.a 时把 B、A 的 MIR 编译出的对象一起合并进去。
   - 消费方只链接 C.a。
   - 代价是重复代码和重复符号风险。

2. **薄静态库 + 依赖清单**
   - C.a 只含 C 自己的对象。
   - 额外提供 C.mylib / C.deps 等元数据，列出 B、A 和链接参数。
   - 消费方必须链接 C.a + B + A（或让构建系统读清单）。
   - 标准 C 链接器不会自动读清单，不能承诺“一个 C.a 就够”。

**当前实现限制**

- `mylc --static` / `--shared` 在链接纯 MIR `.mylib` 时会拒绝：

```text
静态库/动态库导出只能链接带原生载荷的 .mylib；改用 --embed-static/--embed-shared 生成归档
```

- 因此要支持薄归档的 C 静态库/动态库，必须先改静态/动态导出路径：
  - 要么把纯 MIR 依赖编译进产物（自包含）；
  - 要么生成依赖清单，交给上层构建系统。

**推荐**

- `.mylib` 保持薄归档。
- 可执行文件：全部 MIR 依赖链入。
- 动态库：默认自包含，静态编译纯 MIR 依赖。
- 静态库：默认自包含，避免给标准 C 消费方制造“还要找 B/A”的隐式约定；如果坚持薄 C.a，就必须同时发布机器可读的依赖清单。

### 9.4 公共 C 头文件

`mylc --static` / `--shared` 生成的公共头文件现在是独立标准 C 头：

- 只依赖 `<stdint.h>`、`<stdbool.h>`、`<stddef.h>`；
- 不包含 `mylrt.h`；
- 基本类型映射为 `int8_t` / `int16_t` / `int32_t` / `int64_t` / `uint8_t` / ... / `bool` / `float` / `double`；
- 字符串参数/返回值使用前置声明的 `MyStr*`，调用方只需要指针，不需要知道运行时布局；
- 类、枚举、数组保持不透明前置声明；`Option` / `Result` 的 tag 使用 `int32_t`。

因此 C 客户端可以只 `#include "libmathlib.h"` 并使用标准 C 类型；如果它还要调用 `mylrt_str_lit`、`myrt_str_ptr` 等运行时 API，需要自己额外 `#include "mylrt.h"`。这是客户端显式使用运行时的选择，不属于公共头文件的依赖。

### 9.5 C 动态库的依赖部署

当前 `mylc --shared` **不会自动把依赖的 `.so` / `.dll` 复制到 C 的输出目录**。

- `--embed-shared` 的 `.mylib` 载荷会解到：
  ```text
  ~/.mylis/mylib/native/<hash>/
  ```
  链接时使用该路径，并给共享库加 `-Wl,-rpath,<该缓存目录>`。
- 该 `rpath` 是绝对缓存路径，不适合直接发布。
- `--link foo.so` 只是在链接参数里引用它，不会复制。
- 静态 `.a` 依赖会被链进 C.so，不需要复制。
- Windows 的 DLL 依赖不会自动复制到输出目录；运行时仍需在程序目录或 `PATH` 中找到。
- `--embed-runtime` 只处理 `mylrt`/GC，不处理任意第三方依赖。

标准 C 发布建议：

- 静态链接依赖，让 C.so 自包含；或
- 把依赖 `.so` / `.dll` 和 C.so 放在同一发布目录；
- 使用相对 `$ORIGIN` 的 rpath，而不是缓存绝对路径；
- 发布前用 `ldd` / `dumpbin /dependents` 检查实际依赖。

### 9.6 非嵌入链的交付清单

链路：

```text
C.mylib -> A.mylib -> a.so
```

薄归档、全部非嵌入时，C 的提供方应同时交付：

```text
C.mylib
A.mylib
lib/a.so
include/a.h        # 如果 A 的 API 用 @header 声明
deps.lock / 依赖清单
```

职责区分：

- `C.mylib`：只含 C 的 API/MIR。
- `A.mylib`：A 的 API/MIR，提供编译期声明；如果 A 不嵌入 native，它本身不包含 a.so。
- `a.so`：链接期和运行期依赖。
- `a.h`：A 的 `@header` API 需要时提供。
- 依赖清单：记录 `A.mylib`、`a.so` 的路径、checksum、链接参数。
- `a.so` 自己的依赖也要一并交付或由系统提供。

消费方视角：

- 如果 `C.mylib` 已携带完整的传递依赖元数据，消费方只声明 C；`mylc` 递归解析 A 和 a.so。
- 当前只有 `mylib+...` 来源的依赖会按归档元数据递归解析；`path+...` / `registry+...` / `git+...` 仍由包解析器按 `mylis.lock` 处理。非嵌入 native 文件（a.so/a.lib）仍要随包交付或由系统提供，链接参数不会从 `a.so` 自动推导。

运行时只关心 `a.so`；`A.mylib` 是编译期依赖。`C.mylib` 不应把 A 或 a.so 复制进自己。

## 10. 非目标

- 不在 `mylrt-core` 增加文件/网络/线程符号。
- 不让 GC 承担 native 资源生命周期。
- 不做 `async/await`。
- 不做多线程 GC。
- 不把平台差异写进 `core` 或语言层。
