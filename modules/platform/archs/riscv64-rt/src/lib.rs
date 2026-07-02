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

/// Enable machine external interrupt (sets mie.MEIE).
pub unsafe fn enable_mei() {
    unsafe { riscv::register::mie::set_mext() };
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

// ── init 钩子（供 platform::init() 调用）──────────────────────────────────

/// arch 级早期初始化钩子。默认空实现；arch crate 可按需扩展。
/// （mtvec 已在 `__start_rust` 中设置，故此处不重复。）
pub fn arch_init() {}

// chip 板级初始化钩子：原生弱符号（空函数体）。
//
// platform 不依赖任何 chip crate，故无法直接调用其函数；改用弱符号——
// chip crate（如 chip-k3-rt24）用 `#[no_mangle] extern "C" fn _board_init()`
// 强定义覆盖。不覆盖时（QEMU/std-chip）调用落到此空实现，无副作用。
//
// 链接行为已实测：强定义存在时 `nm` 显示 `T`（强）并解析到 chip 实现；
// 不存在时显示 "W"（弱）仍链接成功。
core::arch::global_asm!(
    ".section .text",
    ".weak _board_init",
    "_board_init:",
    "ret",
);
