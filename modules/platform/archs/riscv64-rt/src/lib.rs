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

/// Enable machine timer interrupt (sets mie.MTIE).
pub unsafe fn enable_mtimer() {
    unsafe { riscv::register::mie::set_mtimer() };
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

/// 默认中断处理桩——空操作并返回。
///
/// 供链接脚本 `PROVIDE` 作「用户未覆盖即安全忽略」的兜底，区别于
/// [`_default_abort`]（死循环）。典型用户是 `__Inner_MachineSoft`：它由
/// `#[executor::main]` 生成的 `MachineSoft` ISR 在进入调度器前调用；用户未提供
/// `#[executor::interrupt] fn MachineSoft` 时若兜底到 `abort`（wfi 死循环），首个
/// MSI 一来就把整个调度器锁死（MIE=0 + wfi，再无中断能唤醒）。此处改空返回，
/// 让 ISR 继续走到 `clear_pend()` + 调度循环。
#[doc(hidden)]
#[no_mangle]
pub extern "C" fn _default_interrupt_handler(_tf: &mut TrapFrame) {}

// ── init 钩子（供 platform::init() 调用）──────────────────────────────────

/// arch 级早期初始化钩子。默认空实现；arch crate 可按需扩展。
/// （mtvec 已在 `__start_rust` 中设置，故此处不重复。）
pub fn arch_init() {}
