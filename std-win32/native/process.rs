// Windows std native 侧车：复用跨平台 Rust 实现。
// 平台差异由被 include 的实现内部 #[cfg(windows)] 分支处理。
include!("../../std-posix/native/process.rs");
