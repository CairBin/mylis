# Mylis std 手册

本手册是 `std/src` 的完整 API 参考：覆盖 **100 个源文件、74 个类、13 个接口、1 个枚举、179 个顶层公开函数、564 个类/接口方法**（合计 743 个公开 API 条目）。私有辅助函数与私有字段不列入参考表，需要理解实现时直接读源码。

> 版本基准：当前 Linux 仓库 `std/src` 源码；每次公开 API 变化必须同一提交更新本手册。生成/校对方法见第 8 节。

## 1. std 是什么

`std` 是独立包，不属于编译器隐式 `core`。纯计算项目可以只依赖 `core`；需要 I/O、文件、进程、网络、时间、正则、压缩时显式依赖 `std`。

```toml
# mylis.toml
[dependencies]
std = { path = "std" }
```

```mylis
import { Vec } from "std/Vec";
import { File } from "std/File";
import { TcpListener, TcpStream } from "std/TcpListener";
```

用 `-I std/src` 直接编译单文件时，也可写 `import { Vec } from "Vec.myl";`。两种写法指向同一份源码，不要在同一工程混用。

`std/src/lib.myl` 只是聚合入口，`pub import` 各模块；调用时仍需模块限定名（例如 `Vec`、`File`），没有全局扁平命名空间。`*_bind.myl` 不被 `lib.myl` 聚合，只由实现类按需导入。

`lib.myl` 本身没有可调用 API：它按固定顺序 `pub import` 约 80 个模块，保证包被加载时声明可见。新增文件如果需要对库使用者公开，必须同时加入 `lib.myl`；只被内部实现导入的 `*_bind.myl` 不加入。

## 2. 阅读顺序

1. 先读本章第 3 节的调用约定；90% 的误用来自把引用类型当值类型、忘记检查 `Result`、或忘记 `close()`。
2. 按领域进入章节：容器与算法、文本与数据、时间/系统/进程、网络与事件、哈希与压缩。
3. 需要写 native 后端、C 调用方或排查符号时读第 4 节列出的 [`06-native-ABI与构建.md`](06-native-ABI与构建.md)。
4. 每个文件小节的结构固定为：文件职责 → 类型/字段/常量 → 公开方法/函数表。

## 3. 参数传递与调用约定

### 3.1 值类型与引用类型

- `i8/i16/i32/i64/u8/u16/u32/u64/f32/f64/bool` 按值复制；被调用方修改形参不影响调用方。
- `string`、`T[]`、`Vec<T>`、`Map<K,V>`、`Set<T>`、`Json`、类实例按引用传递：形参与实参指向同一对象，方法内修改字段/元素对调用方可见。
- 需要独立副本时显式调用 `toVec()`、`toArray()`、`asArray()`、`asObject()` 或逐元素复制；不要假设赋值会自动深拷贝。
- `Option<T>` 表示“可能没有”，`Result<T, E>` 表示“成功或失败”；不要用哨兵值代替，除非 API 明确返回 `-1`/空串。

### 3.2 `Vec<T>` 与 `T[]`

- `Vec<T>` 是带 `len/cap` 的动态数组对象，提供 `push/pop/insert/remove/resize` 等边界处理。
- `T[]` 是底层数组视图；`u8[]` 传给 native 时布局固定为 `{ u64 len; u8 items[]; }`，数据从第 8 字节开始。
- native 函数接收 `u8[] data, i64 offset, i64 count` 时，只操作 `[offset, offset+count)`；调用方必须保证范围在 `data.len` 内。上层类通常已做边界检查，直接调 native 时要自己检查。
- `Vec.get(i)` 对越界沿用底层数组行为，不返回 `Option`；需要安全读取时先用 `len()` 检查，或用 `first()/last()`。

### 3.3 字符串与路径

- `string` 是 UTF-8 文本；`Text.Len` 数 Unicode 码点，`Text.ByteLen` 数字节。`Regex` 的 `start/end` 是字节偏移，不是码点下标。
- 传给 native 的 `i8*` 是 NUL 结尾的 C 字符串，由调用方保证生命周期；native 不会接管 Mylis 字符串所有权。
- `Path` 只做字符串处理，不访问文件系统；`Normalize` 不解析符号链接，也不处理 IANA 时区/历史 DST。

### 3.4 句柄与资源所有权

- 文件、进程、管道、socket、监听器、事件循环、目录监听器都用不透明 `i64` 句柄；`0` 或负值通常表示无效。
- 资源由创建者拥有：`File.Open/Create/Append`、`TcpListener.Bind`、`Process.Spawn/Popen`、`EventLoop.init` 等返回的对象必须显式 `close()`。GC 只回收 Mylis 堆对象，不替调用方关闭 OS 句柄。
- `nativeHandle()`/`nativeFd()` 只借出句柄用于诊断或事件循环注册；不要自行 `close` 借出的 fd，除非 API 明确说明所有权转移。

### 3.5 错误模型

- native 侧返回负 errno；Mylis 包装层把它转成 `Err(new IoError(-rc))`，`IoError.code` 是正的 errno。
- `IoError.kind` 是跨平台类别（NotFound/PermissionDenied/AlreadyExists/InvalidInput/WouldBlock/TimedOut/Closed/Other），分支判断优先看 kind，日志再看 code/message。
- `Result` 必须用 `match` 或 `?` 处理；不要只取 `Ok` 分支。非阻塞 socket 的 `WouldBlock` 是正常控制流，不是致命错误。

### 3.6 回调、泵与线程

- `EventLoop`、`FilePump`、`TimerPump`、`DirWatchPump` 都在 Mylis 主线程执行回调；Rust native 工作线程只把完成结果放入队列，不直接调用 Mylis 回调。
- 回调执行期间不要阻塞事件循环；需要长任务时用 `File` 异步入口或 `Worker` 进程隔离。
- `EventPump.nextTimeoutMillis()` 返回 -1 表示没有时间要求，EventLoop 使用默认等待；`DirWatchPump` 固定 100ms 轮询。

### 3.7 泛型约束

- `Map/Set` 只要求 `T: Eq`，线性查找；`HashMap` 要求 `K: Hash + Eq`；`BTreeMap/BTreeSet` 要求 `T: Ord`。
- `Vec` 第一版只支持值类型元素，不做 `Any` 装箱；需要多态时用枚举或 `Json` 等显式表示。
- 泛型在编译期单态化；`T` 的方法调用必须在约束内可见。

## 4. 领域章节

| 章节 | 内容 | 适用场景 |
| --- | --- | --- |
| [01-容器与算法](01-容器与算法.md) | Vec/Map/Set/HashMap/BTreeMap/Deque/Queue/Stack/PriorityQueue/迭代器/Algorithms | 选容器、遍历、排序、查找 |
| [02-文本与数据](02-文本与数据.md) | Text/StringBuilder/ByteBuilder/Base64/CSV/JSON/Path/Regex/Unicode | 解析、序列化、文本处理 |
| [03-时间系统与进程](03-时间系统与进程.md) | DateTime/TimeZone/File/DirWatcher/Io/Log/Random/Math/Process/Worker/Timer | 文件、时间、子进程、Worker |
| [04-网络与事件](04-网络与事件.md) | EventLoop/TCP/UDP/Unix/BufferedTcpStream/各 handler | socket、事件驱动、非阻塞 |
| [05-哈希与压缩](05-哈希与压缩.md) | CRC-32/MD5/SHA-1/SHA-256/gzip | 校验、摘要、压缩 |
| [06-native-ABI与构建](06-native-ABI与构建.md) | `*_bind.myl` 的 C 导出符号与数组/句柄约定 | 写 native 后端、C 调用、排查符号 |

## 5. 文件总索引

100 个文件全部列出；`类型` 为 `-` 表示只有顶层函数。

| 文件 | 类型 | 章节 | 文件职责 |
| --- | --- | --- | --- |
| `Algorithms.myl` | - | [01-容器与算法](01-容器与算法.md) | std 层：基于 Eq/Ord 的 Vec<T> 通用查询与排序，不引入额外约束。 |
| `BTreeMap.myl` | BTreeMap | [01-容器与算法](01-容器与算法.md) | BTreeMap<K, V>：有序 B 树映射（最小度数 2，单节点 1..3 个 key）。 节点存放在扁平数组中，children 只存节点下标，不生成… |
| `BTreeMapIter.myl` | BTreeMapIter | [01-容器与算法](01-容器与算法.md) | BTreeMap<K,V> 的显式迭代器；按 key 升序返回键值对。 |
| `BTreeSet.myl` | BTreeSet | [01-容器与算法](01-容器与算法.md) | BTreeSet<T>：基于 BTreeMap<T, bool> 的有序集合。 add/contains/remove 均为 O(log n)（底层 B 树… |
| `BTreeSetIter.myl` | BTreeSetIter | [01-容器与算法](01-容器与算法.md) | BTreeSet<T> 的显式迭代器；按元素升序返回。 |
| `Base64.myl` | Base64 | [02-文本与数据](02-文本与数据.md) | Base64：RFC 4648 标准字母表，编码固定补 `=`，解码接受标准字母表与 ASCII 空白。 Decode/Encode 面向 UTF-8 文本… |
| `BufferedTcpStream.myl` | BufferedTcpStream | [04-网络与事件](04-网络与事件.md) | TCP 缓冲包装：读侧预取 + 按行读取，写侧聚合后 flush。 不改变 TcpStream 的阻塞/非阻塞语义；EAGAIN 等错误原样返回。 |
| `ByteBuilder.myl` | ByteBuilder | [02-文本与数据](02-文本与数据.md) | 可变字节缓冲：供缓冲 I/O、哈希、正则等纯 std 代码复用。 |
| `Crc32.myl` | Crc32 | [05-哈希与压缩](05-哈希与压缩.md) | CRC-32（IEEE 802.3，多项式 EDB88320），供 gzip/zip 校验使用。 |
| `Csv.myl` | Csv | [02-文本与数据](02-文本与数据.md) | CSV：RFC 4180 风格的逗号分隔文本解析与序列化。 支持双引号字段、字段内逗号/换行、"" 转义、CRLF/LF 行结束；空输入返回空表。 解析结果… |
| `DateTime.myl` | DateTime | [03-时间系统与进程](03-时间系统与进程.md) | 日期时间：UTC 毫秒时间戳 + 固定偏移时区；支持常见格式化与 ISO 8601 解析。 不内建 IANA 时区数据库；TimeZone.Local() … |
| `Deque.myl` | Deque | [01-容器与算法](01-容器与算法.md) | Deque<T>：双端队列，用两个 Vec<T> 分别存放前端和后端。 pushFront/pushBack/popFront/popBack 均摊 O(1… |
| `DequeIter.myl` | DequeIter | [01-容器与算法](01-容器与算法.md) | Deque<T> 的显式迭代器；构造时做一次快照，迭代期间不受后续修改影响。 |
| `DirEvent.myl` | DirEvent | [03-时间系统与进程](03-时间系统与进程.md) | 目录监听事件：kind 为 CREATED / DELETED / MODIFIED。 |
| `DirWatchHandler.myl` | DirWatchHandler | [03-时间系统与进程](03-时间系统与进程.md) | 目录监听回调接口。 |
| `DirWatchPump.myl` | DirWatchPump | [03-时间系统与进程](03-时间系统与进程.md) | 把 DirWatcher 接入 EventLoop：每 100ms 轮询一次，在主线程派发 DirEvent。 |
| `DirWatcher.myl` | DirWatcher | [03-时间系统与进程](03-时间系统与进程.md) | 轮询式目录监听：基于名字、mtime 和大小快照做差异比较。 默认只看根目录；WatchRecursive 会递归到子目录（最多 32 层，跳过不可读子目录… |
| `EventLoop.myl` | EventLoop | [04-网络与事件](04-网络与事件.md) | 单线程通用 fd 事件循环：事件后端等待 fd 就绪 -> Mylis 主线程分发 FdHandler。 Linux 使用 epoll，其他 POSIX 使… |
| `EventPump.myl` | EventPump | [04-网络与事件](04-网络与事件.md) | EventLoop 的可插拔泵接口：文件完成、定时器等不基于 fd 的工作可以独立挂入。 |
| `Event_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std EventLoop 事件后端绑定（Linux epoll，其他 POSIX poll 后备，Windows Winsock WSAPoll）。 |
| `FdHandler.myl` | FdHandler | [04-网络与事件](04-网络与事件.md) | EventLoop 的底层 fd 回调：事件循环只关心 fd 就绪，不认识具体协议对象。 |
| `File.myl` | File | [03-时间系统与进程](03-时间系统与进程.md) | std 文件 API：阻塞文本/二进制读写；以及两个最小异步入口（读/写全部字节）。 异步任务通过 FilePump.add 注册，回调在主线程执行；Fil… |
| `FileHandler.myl` | FileHandler | [03-时间系统与进程](03-时间系统与进程.md) | 异步文件完成回调接口：所有回调都在 Mylis 主线程执行，不在 Rust 工作线程执行。 |
| `FilePump.myl` | FilePump | [03-时间系统与进程](03-时间系统与进程.md) | 异步文件完成泵：只负责 FileTask 的完成队列和回调，不依赖网络 EventLoop。 |
| `FileTask.myl` | FileTask | [03-时间系统与进程](03-时间系统与进程.md) | 异步文件任务：由 File.ReadAllBytesAsync / WriteAllBytesAsync 返回。 必须通过 FilePump.add 注册后… |
| `File_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std 文件 native 后端绑定（Rust staticlib，C ABI）。 句柄是不透明 i64；data 是 Mylis u8[]，native … |
| `Gzip.myl` | Gzip | [05-哈希与压缩](05-哈希与压缩.md) | Gzip（RFC 1952）压缩/解压。压缩端同时尝试 stored、fixed Huffman + LZ77 与 dynamic Huffman + LZ… |
| `HashMap.myl` | HashMap | [01-容器与算法](01-容器与算法.md) | HashMap<K, V>：开放寻址哈希表，要求 K: Hash + Eq。 采用线性探测、墓碑删除；负载因子超过 3/4 时扩容并重建。 不保证遍历顺序；… |
| `HashMapIter.myl` | HashMapIter | [01-容器与算法](01-容器与算法.md) | HashMap<K,V> 的显式迭代器；构造时快照 keys/values，遍历顺序不保证。 |
| `Io.myl` | Io | [03-时间系统与进程](03-时间系统与进程.md) | std 层：标准输出。core 层负责格式化、字符串、数学、GC 绑定。 纯计算项目可以只依赖 core；需要 I/O 时才依赖 std。 |
| `IoError.myl` | IoError | [03-时间系统与进程](03-时间系统与进程.md) | std 层统一 I/O 错误。错误码使用 POSIX errno 的正值；native 侧返回负 errno。 |
| `IoErrorKind.myl` | IoErrorKind | [03-时间系统与进程](03-时间系统与进程.md) | std 层统一 I/O 错误类别。 |
| `Io_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | stdout 写入的 C 绑定（myrt_print/myrt_println）；由 Io 类导入。 |
| `Iter.myl` | Iter | [01-容器与算法](01-容器与算法.md) | Iterator 构造入口；容器的迭代器类分别放在各自文件。 |
| `Iterator.myl` | Iterator | [01-容器与算法](01-容器与算法.md) | std 迭代器协议：显式 next()，不依赖 for-in 或闭包。 |
| `Json.myl` | Json | [02-文本与数据](02-文本与数据.md) | std JSON：第一版提供严格的对象/数组/字符串/数字/布尔/null 解析与紧凑序列化。 不依赖 OS，不引入 Any 装箱；对象使用 std 的线性… |
| `Log.myl` | Log | [03-时间系统与进程](03-时间系统与进程.md) | std 轻量日志：输出到标准输出，格式为 `[级别] 消息`。 |
| `Map.myl` | Map | [01-容器与算法](01-容器与算法.md) | Map<K, V>：基于线性查找的泛型键值表。 第一版不依赖 hash，只要求 K: Eq；适合小到中等规模数据。 查找/插入/删除均为 O(n)，grow… |
| `MapEntry.myl` | MapEntry | [01-容器与算法](01-容器与算法.md) | Map 迭代器使用的键值对。字段是值语义，修改它不会回写 Map。 |
| `MapIter.myl` | MapIter | [01-容器与算法](01-容器与算法.md) | Map<K,V> 的显式迭代器；按 Map 内部插入顺序返回键值对。 |
| `Math.myl` | Math | [03-时间系统与进程](03-时间系统与进程.md) | std 层：数学工具（安全封装）。 依赖 libm；core 不再直接绑定 C 标准库。 |
| `Math_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | libm 标量数学函数的 C 绑定；由 Math 类封装，直接调用需在 unsafe 块内。 |
| `Md5.myl` | Md5 | [05-哈希与压缩](05-哈希与压缩.md) | MD5（纯 Mylis，按字节处理，返回小写十六进制）。 MD5 仅用于兼容既有校验值，不应作为新场景的安全哈希；安全场景请用 SHA-256。 |
| `Path.myl` | Path | [02-文本与数据](02-文本与数据.md) | 轻量路径工具：不访问文件系统，只做字符串处理。 同时接受 '/' 和 '\\' 作为分隔符；不做完整 normalize。 |
| `PriorityQueue.myl` | PriorityQueue | [01-容器与算法](01-容器与算法.md) | PriorityQueue<T>：基于 Vec<T> 的大顶堆，要求 T: Ord。 push/pop 均为 O(log n)，peek 为 O(1)。 |
| `Process.myl` | Process | [03-时间系统与进程](03-时间系统与进程.md) | std 进程接口：Run 对齐 Lua 的 os.execute（同步、捕获输出）， Spawn 返回轻量句柄，提供 pid/wait/kill/close… |
| `ProcessOutput.myl` | ProcessOutput | [03-时间系统与进程](03-时间系统与进程.md) | std 进程执行结果：同步命令结束后复制的 stdout/stderr 与退出状态。 子进程不共享 Mylis 堆；本对象只是父进程里的普通值对象。 |
| `ProcessPipe.myl` | ProcessPipe | [03-时间系统与进程](03-时间系统与进程.md) | Popen/Popen2 管道句柄：Popen 为单向，Popen2 同时有 stdin/stdout。 管道 fd 和子进程都由 native 侧管理；显… |
| `ProcessStatus.myl` | ProcessStatus | [03-时间系统与进程](03-时间系统与进程.md) | 等待子进程结束后的状态：正常退出时 signal 为 0；被信号终止时 exitCode 为 -1。 这只是父进程里的值对象，不持有 OS 资源。 |
| `Process_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std 进程 native 后端绑定（Rust staticlib，C ABI）。 Run 同步执行并捕获输出；Spawn 返回句柄，wait/kill/c… |
| `Queue.myl` | Queue | [01-容器与算法](01-容器与算法.md) | Queue<T>：基于 Vec<T> 的先进先出队列，用一个 head 下标避免每次出队搬移。 均摊 O(1)；head 过大时压缩一次，避免已出队元素长期… |
| `QueueIter.myl` | QueueIter | [01-容器与算法](01-容器与算法.md) | Queue<T> 的显式迭代器；只读当前队列，不改变 head。 |
| `Random.myl` | Random | [03-时间系统与进程](03-时间系统与进程.md) | std 伪随机数：xorshift64*，只用于非密码学场景。 |
| `RangeIter.myl` | RangeIter | [01-容器与算法](01-容器与算法.md) | 半开区间 [start, end)，step 必须为正；空区间直接返回 None。 |
| `Regex.myl` | Regex | [02-文本与数据](02-文本与数据.md) | 纯 Mylis 正则：支持字面量、`.`、`^`/`$`、字符类、`\d/\w/\s`、`\b/\B`、 内联选项 `(?i)` / `(?s)` / `(… |
| `RegexCaptures.myl` | RegexCaptures | [02-文本与数据](02-文本与数据.md) | 正则捕获结果：组 0 为整个匹配，组 1..N 为显式捕获组。 |
| `RegexMatch.myl` | RegexMatch | [02-文本与数据](02-文本与数据.md) | 正则匹配结果：字节偏移区间。 |
| `Set.myl` | Set | [01-容器与算法](01-容器与算法.md) | Set<T>：基于数组的线性集合，只要求 T: Eq。 add/remove/contains 均为 O(n)，第一版不依赖 hash。 |
| `SetIter.myl` | SetIter | [01-容器与算法](01-容器与算法.md) | Set<T> 的显式迭代器；按集合内部顺序返回值。 |
| `Sha1.myl` | Sha1 | [05-哈希与压缩](05-哈希与压缩.md) | SHA-1（纯 Mylis，按字节处理，返回小写十六进制）。 |
| `Sha256.myl` | Sha256 | [05-哈希与压缩](05-哈希与压缩.md) | SHA-256（纯 Mylis，按字节处理，返回小写十六进制）。 支持 UTF-8 文本与任意字节数组；不依赖 native 加密库。 |
| `Stack.myl` | Stack | [01-容器与算法](01-容器与算法.md) | Stack<T>：基于 Vec<T> 的后进先出容器。 push/pop/peek 均摊 O(1)；不做 Any 装箱。 |
| `Stdio.myl` | Stdio | [03-时间系统与进程](03-时间系统与进程.md) | std 标准输入/进程接口：阻塞 stdin、命令行参数、环境变量和进程退出。 本模块单独放在 Stdio 中，避免所有使用 Io 输出 API 的程序都被… |
| `Stdio_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std 标准输入/命令行参数/环境变量/退出码的 C 绑定；由 Stdio 类导入，不直接暴露给库使用者。 |
| `StringBuilder.myl` | StringBuilder | [02-文本与数据](02-文本与数据.md) | StringBuilder：用 Vec<string> 累积片段，最后一次性拼接。 适用于少量到中等量的文本构建；toString() 会分配新字符串并逐段… |
| `TcpEventLoop.myl` | TcpEventLoop | [04-网络与事件](04-网络与事件.md) | TCP 域的事件循环适配：组合通用 EventLoop + TCP handler，不让 EventLoop 认识 TCP 类型。 |
| `TcpListener.myl` | TcpListener | [04-网络与事件](04-网络与事件.md) | std TCP 监听器：接受连接，不引入线程或事件循环。 |
| `TcpListenerHandler.myl` | TcpListenerHandler | [04-网络与事件](04-网络与事件.md) | EventLoop 的 TCP 监听器回调：只处理 onAccept / onListenerError。 |
| `TcpStream.myl` | TcpStream | [04-网络与事件](04-网络与事件.md) | std TCP 流：第一版为阻塞/非阻塞 socket，不做 async/await。 |
| `TcpStreamHandler.myl` | TcpStreamHandler | [04-网络与事件](04-网络与事件.md) | EventLoop 的 TCP 连接回调：只处理 onReadable / onWritable / onError。 |
| `Tcp_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std TCP native 后端绑定（Rust staticlib，C ABI）。 句柄是不透明 i64；u8[] 与 File 后端使用同一数组布局。 |
| `Text.myl` | Text | [02-文本与数据](02-文本与数据.md) | std 层：文本拆分、拼接、行处理与整数解析。 只依赖 core/Str 与 Vec，不访问文件系统。 |
| `Time.myl` | Time | [03-时间系统与进程](03-时间系统与进程.md) | std 时间：wall clock、单调时钟与 sleep；实际实现由 hosted native 后端提供。 |
| `TimeZone.myl` | TimeZone | [03-时间系统与进程](03-时间系统与进程.md) | 固定偏移时区。Local() 读取操作系统当前偏移；历史时间/DST 切换请显式用 Fixed。 |
| `Time_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std wall clock、单调时钟、sleep 与本地时区偏移的 C 绑定；由 Time 类导入。 |
| `TimerHandler.myl` | TimerHandler | [03-时间系统与进程](03-时间系统与进程.md) | 定时器回调接口；回调在挂载 TimerPump 的 EventLoop 主线程执行。 |
| `TimerPump.myl` | TimerPump | [03-时间系统与进程](03-时间系统与进程.md) | 定时器泵：把一次性/周期定时器作为 EventPump 挂入通用 EventLoop。 只依赖单调时钟，不依赖 fd；回调在主线程执行。 |
| `UdpEventLoop.myl` | UdpEventLoop | [04-网络与事件](04-网络与事件.md) | UDP 域的事件循环适配：组合通用 EventLoop + UDP handler。 |
| `UdpHandler.myl` | UdpHandler | [04-网络与事件](04-网络与事件.md) | EventLoop 的 UDP 回调：只处理读就绪与 socket 错误。 |
| `UdpPacket.myl` | UdpPacket | [04-网络与事件](04-网络与事件.md) | UDP 接收结果：payload + 来源地址字符串。 |
| `UdpSocket.myl` | UdpSocket | [04-网络与事件](04-网络与事件.md) | std UDP socket：第一版阻塞收发，不做 async/await。 |
| `Udp_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std UDP native 后端绑定。 |
| `UnicodeData.myl` | UnicodeData | [02-文本与数据](02-文本与数据.md) | Unicode 字符类范围表：每项是 [start, end] 成对的码点区间。 由 Unicode 通用类别生成，覆盖常用类别与常见文字。 |
| `UnixEventLoop.myl` | UnixEventLoop | [04-网络与事件](04-网络与事件.md) | Unix domain socket 的事件循环适配：组合通用 EventLoop + UDS handler。 POSIX 与 Windows AF_UN… |
| `UnixListener.myl` | UnixListener | [04-网络与事件](04-网络与事件.md) | Unix domain listener；POSIX 与 Windows AF_UNIX 可用。 |
| `UnixListenerHandler.myl` | UnixListenerHandler | [04-网络与事件](04-网络与事件.md) | EventLoop 的 Unix 监听器回调：只处理 onAccept / onListenerError。 |
| `UnixStream.myl` | UnixStream | [04-网络与事件](04-网络与事件.md) | Unix domain stream socket；POSIX 与 Windows AF_UNIX 可用。 |
| `UnixStreamHandler.myl` | UnixStreamHandler | [04-网络与事件](04-网络与事件.md) | EventLoop 的 Unix 连接回调：只处理 onReadable / onWritable / onError。 |
| `Unix_bind.myl` | - | [06-native-ABI与构建](06-native-ABI与构建.md) | std Unix domain socket native 后端绑定（POSIX / Windows AF_UNIX）。 |
| `Vec.myl` | Vec | [01-容器与算法](01-容器与算法.md) | Vec<T>：编译期单态化的动态数组，不做 Any 装箱。 第一版只支持值类型元素；push/pop 均摊 O(1)，get/set 为 O(1)。 索引边… |
| `VecIter.myl` | VecIter | [01-容器与算法](01-容器与算法.md) | Vec<T> 的显式迭代器；构造时持有 Vec 引用，不复制元素。 |
| `Worker.myl` | Worker | [03-时间系统与进程](03-时间系统与进程.md) | Worker：基于 Popen2 的进程隔离通信层。 每行一条消息，调用方约定用 JSON 序列化；不共享 Mylis 对象、不共享 GC 堆。 子进程 s… |
| `WorkerHandler.myl` | WorkerHandler | [03-时间系统与进程](03-时间系统与进程.md) | Worker 侧请求处理接口；WorkerServer 每收到一行调用一次。 |
| `WorkerPool.myl` | WorkerPool | [03-时间系统与进程](03-时间系统与进程.md) | WorkerPool：固定数量的独立 worker 进程，做同步 fan-out/fan-in。 callAll 保持全有或全无语义；callAllResu… |
| `WorkerReply.myl` | WorkerReply | [03-时间系统与进程](03-时间系统与进程.md) | Worker 结构化响应：成功携带 value，失败携带 code/message。 线上格式是单行 JSON 对象，由 WorkerServer::Run… |
| `WorkerReplyResult.myl` | WorkerReplyResult | [03-时间系统与进程](03-时间系统与进程.md) | WorkerPool 的结构化单任务结果。 errorCode == 0 表示传输层成功，reply 是远端返回的 WorkerReply； errorCo… |
| `WorkerResultHandler.myl` | WorkerResultHandler | [03-时间系统与进程](03-时间系统与进程.md) | Worker 结构化响应处理器；返回 WorkerReply，由 WorkerServer::RunResult 序列化。 |
| `WorkerServer.myl` | WorkerServer | [03-时间系统与进程](03-时间系统与进程.md) | Worker 侧行协议服务端：读一行、交给 handler、把返回值打印成一行。 收到保留控制行 "__shutdown__" 时直接退出，Worker::… |
| `WorkerTaskResult.myl` | WorkerTaskResult | [03-时间系统与进程](03-时间系统与进程.md) | WorkerPool 的单任务结果：失败不会终止其他任务。 |
| `lib.myl` | - | - | std 包入口：按固定顺序 pub import 各模块；本身没有可调用 API。 |

## 6. 类型速查

按类型名查所在文件和章节；每个类型的字段、方法、参数说明在对应章节的文件小节。

| 类型 | 种类 | 文件 | 章节 | 用途 |
| --- | --- | --- | --- | --- |
| `BTreeMap` | class | `BTreeMap.myl` | [01-容器与算法](01-容器与算法.md) | BTreeMap<K, V>：有序 B 树映射（最小度数 2，单节点 1..3 个 key）。 |
| `BTreeMapIter` | class | `BTreeMapIter.myl` | [01-容器与算法](01-容器与算法.md) | 有序映射的显式迭代器；构造时保存 map 引用，`next()` 按 key 升序返回 `MapEntry`，耗尽返回 None。 |
| `BTreeSet` | class | `BTreeSet.myl` | [01-容器与算法](01-容器与算法.md) | BTreeSet<T>：基于 BTreeMap<T, bool> 的有序集合。 |
| `BTreeSetIter` | class | `BTreeSetIter.myl` | [01-容器与算法](01-容器与算法.md) | 有序集合的显式迭代器；按元素升序返回，耗尽返回 None。 |
| `Base64` | class | `Base64.myl` | [02-文本与数据](02-文本与数据.md) | Base64：RFC 4648 标准字母表，编码固定补 `=`，解码接受标准字母表与 ASCII 空白。 |
| `BufferedTcpStream` | class | `BufferedTcpStream.myl` | [04-网络与事件](04-网络与事件.md) | TCP 缓冲包装：读侧预取 + 按行读取，写侧聚合后 flush。 |
| `ByteBuilder` | class | `ByteBuilder.myl` | [02-文本与数据](02-文本与数据.md) | 可变字节缓冲：供缓冲 I/O、哈希、正则等纯 std 代码复用。 |
| `Crc32` | class | `Crc32.myl` | [05-哈希与压缩](05-哈希与压缩.md) | CRC-32（IEEE 802.3，多项式 EDB88320），供 gzip/zip 校验使用。 |
| `Csv` | class | `Csv.myl` | [02-文本与数据](02-文本与数据.md) | CSV：RFC 4180 风格的逗号分隔文本解析与序列化。 |
| `DateTime` | class | `DateTime.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 日期时间：UTC 毫秒时间戳 + 固定偏移时区。 |
| `Deque` | class | `Deque.myl` | [01-容器与算法](01-容器与算法.md) | Deque<T>：双端队列，用两个 Vec<T> 分别存放前端和后端。 |
| `DequeIter` | class | `DequeIter.myl` | [01-容器与算法](01-容器与算法.md) | 双端队列的显式迭代器；构造时做快照，迭代期间修改原 deque 不影响本次遍历。 |
| `DirEvent` | class | `DirEvent.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 目录监听事件值：kind 为 CREATED/DELETED/MODIFIED，name 是条目名，path 是完整路径。 |
| `DirWatchHandler` | interface | `DirWatchHandler.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 目录监听回调接口；`onDirEvent` 由 DirWatchPump 在主线程调用。 |
| `DirWatchPump` | class | `DirWatchPump.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 把 DirWatcher 接入 EventLoop：每 100ms 轮询一次，在主线程派发 DirEvent。 |
| `DirWatcher` | class | `DirWatcher.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 轮询式目录监听：基于名字、mtime 和大小快照做差异比较。 |
| `EventLoop` | class | `EventLoop.myl` | [04-网络与事件](04-网络与事件.md) | 单线程通用 fd 事件循环：事件后端等待 fd 就绪 -> Mylis 主线程分发 FdHandler。 |
| `EventPump` | interface | `EventPump.myl` | [04-网络与事件](04-网络与事件.md) | 可挂入 EventLoop 的泵接口；文件完成、定时器等非 fd 工作通过它参与事件循环。 |
| `FdHandler` | interface | `FdHandler.myl` | [04-网络与事件](04-网络与事件.md) | EventLoop 的底层 fd 回调；只报告可读/可写/错误，不解释协议。 |
| `File` | class | `File.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 文件 API：阻塞文本/二进制读写。 |
| `FileHandler` | interface | `FileHandler.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 异步文件完成回调；所有回调在主线程执行，不在 Rust 工作线程执行。 |
| `FilePump` | class | `FilePump.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 异步文件完成泵：只负责 FileTask 的完成队列和回调，不依赖网络 EventLoop。 |
| `FileTask` | class | `FileTask.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 异步文件任务句柄；必须交给 `FilePump.add` 后才会在主线程收到完成回调。 |
| `Gzip` | class | `Gzip.myl` | [05-哈希与压缩](05-哈希与压缩.md) | Gzip（RFC 1952）压缩/解压。 |
| `HashMap` | class | `HashMap.myl` | [01-容器与算法](01-容器与算法.md) | HashMap<K, V>：开放寻址哈希表，要求 K: Hash + Eq。 |
| `HashMapIter` | class | `HashMapIter.myl` | [01-容器与算法](01-容器与算法.md) | 哈希映射的显式迭代器；构造时快照 keys/values，遍历顺序不保证。 |
| `Io` | class | `Io.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 层：标准输出。 |
| `IoError` | class | `IoError.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 层统一 I/O 错误。 |
| `IoErrorKind` | enum | `IoErrorKind.myl` | [03-时间系统与进程](03-时间系统与进程.md) | I/O 错误类别枚举；具体 errno 在 `IoError.code`，类别用于分支判断。 |
| `Iter` | class | `Iter.myl` | [01-容器与算法](01-容器与算法.md) | 迭代器工厂：把整数区间或容器适配成显式 `next()` 迭代器，供手写循环使用。 |
| `Iterator` | interface | `Iterator.myl` | [01-容器与算法](01-容器与算法.md) | std 的迭代器协议；实现者只需提供 `next()`，不依赖 for-in 或闭包。 |
| `Json` | class | `Json.myl` | [02-文本与数据](02-文本与数据.md) | std JSON：第一版提供严格的对象/数组/字符串/数字/布尔/null 解析与紧凑序列化。 |
| `Log` | class | `Log.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 轻量日志：输出到标准输出，格式为 `[级别] 消息`。 |
| `Map` | class | `Map.myl` | [01-容器与算法](01-容器与算法.md) | Map<K, V>：基于线性查找的泛型键值表。 |
| `MapEntry` | class | `MapEntry.myl` | [01-容器与算法](01-容器与算法.md) | Map 迭代器返回的键值对；字段是值对象，修改它不会回写 Map。 |
| `MapIter` | class | `MapIter.myl` | [01-容器与算法](01-容器与算法.md) | 线性 Map 的显式迭代器；按内部插入顺序返回键值对。 |
| `Math` | class | `Math.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 层：数学工具（安全封装）。 |
| `Md5` | class | `Md5.myl` | [05-哈希与压缩](05-哈希与压缩.md) | MD5（纯 Mylis，按字节处理，返回小写十六进制）。 |
| `Path` | class | `Path.myl` | [02-文本与数据](02-文本与数据.md) | 轻量路径工具：不访问文件系统，只做字符串处理。 |
| `PriorityQueue` | class | `PriorityQueue.myl` | [01-容器与算法](01-容器与算法.md) | PriorityQueue<T>：基于 Vec<T> 的大顶堆，要求 T: Ord。 |
| `Process` | class | `Process.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 进程接口：Run 对齐 Lua 的 os.execute（同步、捕获输出）， Spawn 返回轻量句柄，提供 pid/wait/kill/close。 |
| `ProcessOutput` | class | `ProcessOutput.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 同步进程执行结果的值对象；stdout/stderr 已复制到父进程，不持有子进程句柄。 |
| `ProcessPipe` | class | `ProcessPipe.myl` | [03-时间系统与进程](03-时间系统与进程.md) | Popen/Popen2 管道句柄：Popen 为单向，Popen2 同时有 stdin/stdout。 |
| `ProcessStatus` | class | `ProcessStatus.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 子进程结束状态；正常退出 signal=0，被信号终止时 exitCode=-1。 |
| `Queue` | class | `Queue.myl` | [01-容器与算法](01-容器与算法.md) | Queue<T>：基于 Vec<T> 的先进先出队列，用一个 head 下标避免每次出队搬移。 |
| `QueueIter` | class | `QueueIter.myl` | [01-容器与算法](01-容器与算法.md) | 队列的显式迭代器；从当前 head 开始只读遍历，不改变队列。 |
| `Random` | class | `Random.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 伪随机数：xorshift64*，只用于非密码学场景。 |
| `RangeIter` | class | `RangeIter.myl` | [01-容器与算法](01-容器与算法.md) | 半开区间 [start, end)，step 必须为正。 |
| `Regex` | class | `Regex.myl` | [02-文本与数据](02-文本与数据.md) | 纯 Mylis 正则：支持字面量、`.`、`^`/`$`、字符类、`\d/\w/\s`、`\b/\B`、 内联选项 `(?i)` / `(?s)` / `(… |
| `RegexCaptures` | class | `RegexCaptures.myl` | [02-文本与数据](02-文本与数据.md) | 捕获组结果；组 0 是整个匹配，组 1..N 是显式捕获组，未参与匹配的组为 None。 |
| `RegexMatch` | class | `RegexMatch.myl` | [02-文本与数据](02-文本与数据.md) | 一次匹配的字节偏移区间 `[start, end)`；`text(source)` 按偏移截取原文。 |
| `Set` | class | `Set.myl` | [01-容器与算法](01-容器与算法.md) | Set<T>：基于数组的线性集合，只要求 T: Eq。 |
| `SetIter` | class | `SetIter.myl` | [01-容器与算法](01-容器与算法.md) | 线性集合的显式迭代器；按内部存储顺序返回值。 |
| `Sha1` | class | `Sha1.myl` | [05-哈希与压缩](05-哈希与压缩.md) | SHA-1（纯 Mylis，按字节处理，返回小写十六进制）。 |
| `Sha256` | class | `Sha256.myl` | [05-哈希与压缩](05-哈希与压缩.md) | SHA-256（纯 Mylis，按字节处理，返回小写十六进制）。 |
| `Stack` | class | `Stack.myl` | [01-容器与算法](01-容器与算法.md) | Stack<T>：基于 Vec<T> 的后进先出容器。 |
| `Stdio` | class | `Stdio.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 标准输入/进程接口：阻塞 stdin、命令行参数、环境变量和进程退出。 |
| `StringBuilder` | class | `StringBuilder.myl` | [02-文本与数据](02-文本与数据.md) | StringBuilder：用 Vec<string> 累积片段，最后一次性拼接。 |
| `TcpEventLoop` | class | `TcpEventLoop.myl` | [04-网络与事件](04-网络与事件.md) | TCP 域的事件循环适配：组合通用 EventLoop + TCP handler，不让 EventLoop 认识 TCP 类型。 |
| `TcpListener` | class | `TcpListener.myl` | [04-网络与事件](04-网络与事件.md) | std TCP 监听器：接受连接，不引入线程或事件循环。 |
| `TcpListenerHandler` | interface | `TcpListenerHandler.myl` | [04-网络与事件](04-网络与事件.md) | TCP 监听器回调；`onAccept` 在监听 socket 可读时调用，`onListenerError` 报告 accept 错误。 |
| `TcpStream` | class | `TcpStream.myl` | [04-网络与事件](04-网络与事件.md) | std TCP 流：第一版为阻塞/非阻塞 socket，不做 async/await。 |
| `TcpStreamHandler` | interface | `TcpStreamHandler.myl` | [04-网络与事件](04-网络与事件.md) | TCP 连接回调；只报告可读/可写/错误，读写仍由调用方执行。 |
| `Text` | class | `Text.myl` | [02-文本与数据](02-文本与数据.md) | std 层：文本拆分、拼接、行处理与整数解析。 |
| `Time` | class | `Time.myl` | [03-时间系统与进程](03-时间系统与进程.md) | std 时间：wall clock、单调时钟与 sleep。 |
| `TimeZone` | class | `TimeZone.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 固定偏移时区。 |
| `TimerHandler` | interface | `TimerHandler.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 定时器回调接口；`onTimer(id)` 在挂载 TimerPump 的 EventLoop 主线程执行。 |
| `TimerPump` | class | `TimerPump.myl` | [03-时间系统与进程](03-时间系统与进程.md) | 定时器泵：把一次性/周期定时器作为 EventPump 挂入通用 EventLoop。 |
| `UdpEventLoop` | class | `UdpEventLoop.myl` | [04-网络与事件](04-网络与事件.md) | UDP 域的事件循环适配：组合通用 EventLoop + UDP handler。 |
| `UdpHandler` | interface | `UdpHandler.myl` | [04-网络与事件](04-网络与事件.md) | UDP socket 回调；只报告可读与 socket 错误。 |
| `UdpPacket` | class | `UdpPacket.myl` | [04-网络与事件](04-网络与事件.md) | UDP 接收结果：`data` 是负载，`from` 是来源地址字符串。 |
| `UdpSocket` | class | `UdpSocket.myl` | [04-网络与事件](04-网络与事件.md) | std UDP socket：第一版阻塞收发，不做 async/await。 |
| `UnicodeData` | class | `UnicodeData.myl` | [02-文本与数据](02-文本与数据.md) | Unicode 字符类范围表：每项是 [start, end] 成对的码点区间。 |
| `UnixEventLoop` | class | `UnixEventLoop.myl` | [04-网络与事件](04-网络与事件.md) | Unix domain socket 的事件循环适配：组合通用 EventLoop + UDS handler。 |
| `UnixListener` | class | `UnixListener.myl` | [04-网络与事件](04-网络与事件.md) | Unix domain listener。 |
| `UnixListenerHandler` | interface | `UnixListenerHandler.myl` | [04-网络与事件](04-网络与事件.md) | Unix domain 监听器回调；语义与 TCP listener handler 相同。 |
| `UnixStream` | class | `UnixStream.myl` | [04-网络与事件](04-网络与事件.md) | Unix domain stream socket。 |
| `UnixStreamHandler` | interface | `UnixStreamHandler.myl` | [04-网络与事件](04-网络与事件.md) | Unix domain 连接回调；语义与 TCP stream handler 相同。 |
| `Vec` | class | `Vec.myl` | [01-容器与算法](01-容器与算法.md) | Vec<T>：编译期单态化的动态数组，不做 Any 装箱。 |
| `VecIter` | class | `VecIter.myl` | [01-容器与算法](01-容器与算法.md) | Vec 的显式迭代器；构造时持有 Vec 引用，不复制元素。 |
| `Worker` | class | `Worker.myl` | [03-时间系统与进程](03-时间系统与进程.md) | Worker：基于 Popen2 的进程隔离通信层。 |
| `WorkerHandler` | interface | `WorkerHandler.myl` | [03-时间系统与进程](03-时间系统与进程.md) | Worker 行协议处理器；`handle(request)` 返回一行响应字符串。 |
| `WorkerPool` | class | `WorkerPool.myl` | [03-时间系统与进程](03-时间系统与进程.md) | WorkerPool：固定数量的独立 worker 进程，做同步 fan-out/fan-in。 |
| `WorkerReply` | class | `WorkerReply.myl` | [03-时间系统与进程](03-时间系统与进程.md) | Worker 结构化响应；成功携带 value，失败携带 code/message，线上格式是单行 JSON。 |
| `WorkerReplyResult` | class | `WorkerReplyResult.myl` | [03-时间系统与进程](03-时间系统与进程.md) | WorkerPool 单任务的结构化结果；`errorCode==0` 表示传输成功，`reply` 才是远端响应。 |
| `WorkerResultHandler` | interface | `WorkerResultHandler.myl` | [03-时间系统与进程](03-时间系统与进程.md) | Worker 结构化响应处理器；返回 `WorkerReply`，由 `WorkerServer.RunResult` 序列化。 |
| `WorkerServer` | class | `WorkerServer.myl` | [03-时间系统与进程](03-时间系统与进程.md) | Worker 侧行协议服务端：读一行、交给 handler、把返回值打印成一行。 |
| `WorkerTaskResult` | class | `WorkerTaskResult.myl` | [03-时间系统与进程](03-时间系统与进程.md) | WorkerPool 单任务结果；失败只影响该槽位，不终止其他任务。 |

## 7. 约定与限制

- **非并发安全**：容器、EventLoop、TimerPump、WorkerPool 都不保证多线程安全；当前 std 不提供线程 API。
- **单线程回调**：所有 handler 回调在主线程执行；不要在回调里再阻塞等待同一事件循环。
- **时区限制**：`TimeZone.Local()` 只取当前系统偏移；历史 DST 需要显式 `TimeZone.Fixed`。
- **正则限制**：`find/findAll` 默认 Thompson NFA；含断言、反向引用或 `findCaptures` 时走回溯，复杂模式可能有性能风险。
- **异步文件限制**：只有 `ReadAllBytesAsync`/`WriteAllBytesAsync` 两个入口；不是通用 async/await。
- **Worker 协议**：每行一条消息，调用方约定 JSON；不共享 Mylis 堆，只做同步行协议。

## 8. 维护与校对

本手册由 `std/src` 源码的公开声明生成初稿，再人工校对语义。修改公开 API 时：

1. 先在源码中改签名/字段/常量；
2. 重新生成对应章节，确认没有丢失重载或接口方法；
3. 人工检查新增/删除条目的说明、参数类型和返回类型；
4. 更新第 5/6 节索引和根 `README.md` 的文档导航；
5. 在提交说明中写“std 手册已同步”，并列出覆盖的文件数/类型数。

校对重点：`Json`、`Regex`、`Process`、`Worker*`、TCP/UDP/Unix 这几组参数多、错误分支多，不能只看签名。

## 9. 相关文档

- 文档总览：[`../README.md`](../README.md)
- 使用手册：[`../使用手册.md`](../使用手册.md)
- 平台与异步约定：[`../std平台与异步约定.md`](../std平台与异步约定.md)
- Agent 开发规范：[`../agent开发规范.md`](../agent开发规范.md)
- 库标准：[`../Mylis库标准.md`](../Mylis库标准.md)
