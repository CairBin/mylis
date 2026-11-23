// Windows std native 网络侧车：复用 std-posix/native/net 的跨平台实现。
// 平台差异由子模块内部的 #[cfg(windows)] 分支处理。
#[path = "../../std-posix/native/net/mod.rs"]
mod net_impl;
