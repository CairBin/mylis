// Cortex-M 平台：semihosting 输出，QEMU lm3s6965evb 可直接运行。

use crate::plat::Platform;

pub struct ArmPlatform;
static ARM: ArmPlatform = ArmPlatform;

impl Platform for ArmPlatform {
    fn write_stdout(&self, bytes: &[u8]) { semihost_write(bytes); }
    fn write_stderr(&self, bytes: &[u8]) { semihost_write(bytes); }
    fn abort(&self) -> ! { semihost_exit(1) }
    fn clock(&self) -> f64 { 0.0 }
    fn gc_threshold(&self) -> Option<usize> { None }
    fn format_f64(&self, _v: f64) -> ([u8; 64], usize) { ([0; 64], 0) }
}

fn semihost_write(bytes: &[u8]) {
    let mut buf = [0u8; 128];
    for chunk in bytes.chunks(120) {
        let n = chunk.len();
        buf[..n].copy_from_slice(chunk);
        buf[n] = 0;
        unsafe {
            core::arch::asm!(
                "mov r0, #4",
                "bkpt 0xAB",
                in("r1") buf.as_ptr(),
                lateout("r0") _,
                options(nostack),
            );
        }
    }
}

fn semihost_exit(code: i32) -> ! {
    unsafe {
        core::arch::asm!(
            "mov r0, #0x18",
            "mov r1, {code}",
            "bkpt 0xAB",
            code = in(reg) code,
            options(noreturn),
        );
    }
}

pub fn platform() -> &'static dyn Platform { &ARM }
