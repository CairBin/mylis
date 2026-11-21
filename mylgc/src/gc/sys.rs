use super::*;
use super::api::*;
use super::heap::*;
use super::mark::*;
use super::page::*;
use super::roots::*;
use super::stats::*;

#[no_mangle]
pub extern "C" fn myrt_gc_backend_set_allocator(a: Option<SysAlloc>, f: Option<SysFree>) {
    unsafe { ALLOC_FN = a; FREE_FN = f; }
}

#[no_mangle]
pub extern "C" fn myrt_gc_backend_set_stack_range(_lo: *mut u8, hi: *mut u8) {
    unsafe { STACK_TOP_OVERRIDE = hi as usize; }
}

#[inline]
pub(crate) unsafe fn sys_alloc(n: usize) -> *mut u8 {
    match ALLOC_FN { Some(f) => f(n), None => malloc(n) }
}

#[inline]
pub(crate) unsafe fn sys_free(p: *mut u8) {
    match FREE_FN { Some(f) => f(p), None => free(p) }
}


/// 栈顶（高地址）。保守扫描范围是 [当前 sp, 栈顶]：
/// 不能用 init 时的局部变量地址做上界——init 帧比 main 的帧更低，
/// 会把 main 的局部变量挡在扫描区间之外（Windows 实测字符串被误回收）。
#[inline]
pub(crate) fn read_sp() -> usize {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let sp: usize;
        core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags));
        return sp;
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let sp: usize;
        core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
        return sp;
    }
    #[allow(unreachable_code)]
    {
        let local = 0u8;
        &local as *const u8 as usize
    }
}

pub(crate) fn stack_top() -> usize {
    unsafe {
        if STACK_TOP_OVERRIDE != 0 { return STACK_TOP_OVERRIDE; }
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    unsafe {
        let base: usize;
        core::arch::asm!("mov {}, gs:[0x08]", out(reg) base, options(nostack, preserves_flags));
        return base;
    }
    #[cfg(all(target_os = "linux", not(target_env = "musl")))]
    unsafe {
        extern "C" {
            static __libc_stack_end: *mut u8;
        }
        let base = __libc_stack_end as usize;
        if base != 0 {
            return base;
        }
    }
    #[cfg(target_os = "macos")]
    unsafe {
        extern "C" {
            fn pthread_get_stackaddr_np(t: usize) -> *mut u8;
            fn pthread_self() -> usize;
        }
        return pthread_get_stackaddr_np(pthread_self()) as usize;
    }
    #[allow(unreachable_code)]
    {
        let local = 0u8;
        &local as *const u8 as usize
    }
}

pub(crate) fn now_cycles() -> usize {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let lo: u32; let hi: u32;
        core::arch::asm!("lfence", "rdtsc", out("eax") lo, out("edx") hi, options(nostack, preserves_flags));
        return ((hi as usize) << 32) | lo as usize;
    }
    #[allow(unreachable_code)]
    { 0 }
}