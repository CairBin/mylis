#![no_std]
#![no_main]

use cortex_m_rt::entry;


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

#[entry]
fn main() -> ! {
    mylrt::myrt_init();
    let s = mylrt::mylrt_str_lit(b"qemu-arm freestanding ok\n".as_ptr(), 27);
    mylrt::myrt_println(s);
    semihost_exit(0)
}
