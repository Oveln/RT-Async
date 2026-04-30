//! # RISC-V 64-bit Runtime
//!
//! 提供 RISC-V 64-bit 架构的基础运行时支持：
//! - Trap 处理（上下文保存/恢复 + 中断/异常分发）
//! - 弱符号中断/异常处理器
//!
//! ## 中断/异常处理器
//!
//! 平台通过定义同名 `#[no_mangle]` 函数覆盖默认处理器：
//!
//! ```rust,ignore
//! #[no_mangle]
//! pub unsafe extern "C" fn MachineTimer(trap_frame: &mut TrapFrame) {
//!     // 自定义定时器中断处理
//! }
//! ```

#![no_std]

mod trap;
mod handlers;
mod start;
mod panic;

pub use trap::{TrapFrame, CONTEXT_STACK_SIZE, __trap_entry, trap_handler};

#[no_mangle]
pub extern "C" fn _default_abort() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

#[no_mangle]
pub extern "C" fn _default_start_trap() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

#[no_mangle]
pub extern "C" fn _default_setup_interrupts() {}
