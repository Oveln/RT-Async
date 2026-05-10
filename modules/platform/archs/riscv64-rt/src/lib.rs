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

mod handlers;
mod panic;
mod start;
mod trap;

pub use trap::TrapFrame;
#[doc(hidden)]
pub use trap::{__trap_entry, trap_handler, CONTEXT_STACK_SIZE};

/// Enable global machine interrupts (sets mstatus.MIE).
pub unsafe fn enable_interrupts() {
    unsafe { riscv::register::mstatus::set_mie() };
}

/// Disable global machine interrupts (clears mstatus.MIE).
pub unsafe fn disable_interrupts() {
    unsafe { riscv::register::mstatus::clear_mie() };
}

/// Enable machine software interrupt (sets mie.MSIE).
pub unsafe fn enable_msi() {
    unsafe { riscv::register::mie::set_msoft() };
}

pub fn idle() {
    riscv::asm::wfi();
}

#[doc(hidden)]
#[no_mangle]
pub extern "C" fn _default_abort() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

#[doc(hidden)]
#[no_mangle]
pub extern "C" fn _default_start_trap() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

#[doc(hidden)]
#[no_mangle]
pub extern "C" fn _default_setup_interrupts() {}
