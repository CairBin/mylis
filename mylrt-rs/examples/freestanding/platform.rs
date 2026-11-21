// Linux x86_64 的无 libc 平台实现：直接走 syscall。

use crate::plat::Platform;

pub struct BarePlatform;
static BARE: BarePlatform = BarePlatform;

impl Platform for BarePlatform {
    fn write_stdout(&self, bytes: &[u8]) { sys_write(1, bytes); }
    fn write_stderr(&self, bytes: &[u8]) { sys_write(2, bytes); }
    fn abort(&self) -> ! { sys_exit(1) }
    fn clock(&self) -> f64 { 0.0 }
    fn gc_threshold(&self) -> Option<usize> { None }
    fn format_f64(&self, _v: f64) -> ([u8; 64], usize) { ([0; 64], 0) }
}

fn sys_write(fd: i32, bytes: &[u8]) {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 1usize,
            in("rdi") fd,
            in("rsi") bytes.as_ptr(),
            in("rdx") bytes.len(),
            lateout("rcx") _,
            lateout("r11") _,
            lateout("rax") _,
        );
    }
}

fn sys_exit(code: i32) -> ! {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 60usize,
            in("rdi") code,
            options(noreturn),
        );
    }
}

pub fn platform() -> &'static dyn Platform { &BARE }
