# std-posix

`std` 的 POSIX native 侧车包。

- `native/file.rs`：文件后端，`libmylis_file.a`
- `native/io.rs`：stdio/argv/env/exit 后端，`libmylis_io.a`
- `native/process.rs`：shell 进程后端，`libmylis_process.a`；Run 同步捕获输出，Spawn 提供 pid/wait/kill/close，Popen/Popen2 提供单向或双向字节管道；ProcessPipe 还支持按行读取和超时读取，供 Worker 行协议使用
- `native/time.rs`：时间与 sleep 后端，`libmylis_time.a`
- `native/net.rs`：TCP/UDP 后端，`libmylis_net.a`
- `native/event.rs`：Linux epoll / 其他 POSIX poll 事件后端，`libmylis_event.a`
- `native/mylis_*.h`：对应 C header

构建：

```bash
bash tools/build_std_native.sh
```

产物在 `build/std-posix/native/`。消费端编译时加：

```bash
-I std-posix/native
--link build/std-posix/native/libmylis_file.a
--link build/std-posix/native/libmylis_process.a
--link build/std-posix/native/libmylis_net.a
--link build/std-posix/native/libmylis_event.a
```

非嵌入 native 不随源码包自动传播，最终链接者必须显式提供。

当前只在 Linux/POSIX 验证；Windows 需要单独的 `std-win32-*` 侧车包。
