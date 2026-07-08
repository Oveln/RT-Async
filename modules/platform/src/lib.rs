#![no_std]

use extern_trait::extern_trait;

/// 板级初始化钩子。
///
/// 每个板级 crate 实现该 trait（经 `#[extern_trait]` 做静态分发）。
/// `init()` 在 `platform::init()` 内部调用，负责 DTB 注入、driver 列表注册
/// 和 DT 遍历实例化 ([`crate::driver::boot`])。
#[extern_trait(pub BoardImpl)]
pub trait Board {
    fn init();
}

#[cfg(feature = "riscv64")]
pub use arch::{disable_interrupts, enable_interrupts, enable_mtimer, idle};
#[cfg(feature = "riscv64")]
pub use riscv64_rt as arch;

pub mod device;
pub mod driver;
pub mod drivers;
pub mod dtb;
pub mod irq;
pub mod logger;
pub use logger::Logger;

// 便捷 re-export：上层（executor / futures / apps）直接取用。
pub use device::{Driver, InterruptController, Ipi, Reset, Serial, Timer};
pub use driver::{
    boot, console, intctl, ipi, reset, set_console, set_drivers, set_intctl, set_ipi, set_reset,
    set_timer, timer,
};
pub use irq::{dispatch_external, register_irq, IrqHandler};

static LOGGER: Logger = Logger::new();

pub fn init(max_level: log::LevelFilter) {
    let _ = LOGGER.init(max_level);

    #[cfg(feature = "riscv64")]
    arch::arch_init();

    BoardImpl::init();
}

#[cfg(feature = "riscv64")]
pub unsafe fn start() {
    unsafe {
        // 先把 deadline 推到最远，避免立即触发定时器中断。
        driver::timer().set_deadline(u64::MAX);
        arch::enable_mtimer();
        arch::enable_msi();
        arch::enable_mei();
        arch::enable_interrupts();
    }
}

pub static PEND_MARKER: portable_atomic::AtomicBool = portable_atomic::AtomicBool::new(false);

pub unsafe fn pend() {
    PEND_MARKER.store(true, portable_atomic::Ordering::Release);
    // SAFETY: 调用者（executor wake path）保证上下文合适。
    unsafe { driver::ipi().send() };
}

pub unsafe fn clear_pend() -> bool {
    let is_system = PEND_MARKER.swap(false, portable_atomic::Ordering::AcqRel);
    // SAFETY: ISR 早期调用，关中断上下文。
    unsafe { driver::ipi().clear() };
    is_system
}
