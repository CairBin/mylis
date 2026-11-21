//! 平台接口：运行时只通过这里做 I/O、abort、时钟和浮点格式化。
//! `MYLRT_PLATFORM_RS` 可以把整个实现替换成裸机/第三方版本。

pub trait Platform: Sync {
    fn write_stdout(&self, bytes: &[u8]);
    fn write_stderr(&self, bytes: &[u8]);
    fn abort(&self) -> !;
    fn clock(&self) -> f64;
    fn gc_threshold(&self) -> Option<usize>;
    fn format_f64(&self, v: f64) -> ([u8; 64], usize);

    /// 固定小数位格式化；默认退回普通格式化，第三方平台可按需覆盖。
    fn format_f64_prec(&self, v: f64, _precision: i32) -> ([u8; 64], usize) {
        self.format_f64(v)
    }
}

mod selected {
    include!(concat!(env!("OUT_DIR"), "/selected_platform.rs"));
}

pub fn platform() -> &'static dyn Platform { selected::platform() }
