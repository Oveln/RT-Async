//! RISC-V 64-bit 启动入口
//!
//! 提供 `_start` 汇编入口点、MTVEC 初始化及可选的 BSS 清零。

use riscv::register::mtvec::{self, Mtvec, TrapMode};

use crate::__trap_entry;

/// Clear the BSS section (zero-initialized data)
#[cfg(feature = "clear_bss")]
pub fn clear_bss() {
    extern "C" {
        static __sbss: u8;
        static __ebss: u8;
    }
    unsafe {
        core::slice::from_raw_parts_mut(
            &__sbss as *const u8 as *mut u8,
            &__ebss as *const u8 as usize - &__sbss as *const u8 as usize,
        )
        .fill(0);
    }
}

#[unsafe(no_mangle)]
extern "C" fn __start_rust() {
    unsafe {
        mtvec::write(Mtvec::new(
            __trap_entry as *const () as usize,
            TrapMode::Direct,
        ))
    };
}

// Assembly entry point
core::arch::global_asm!(
    ".section .init",
    ".global __start",
    ".align 4",
    "__start:",
    "la gp, __global_pointer$",
    "la sp, __sstack",
    #[cfg(feature = "clear_bss")]
    "call __clear_bss",
    "call __start_rust",
    "j __rust_main"
);

// Export the clear_bss function for assembly to call when feature is enabled
#[cfg(feature = "clear_bss")]
#[no_mangle]
extern "C" fn __clear_bss() {
    clear_bss();
}
