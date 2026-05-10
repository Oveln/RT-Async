#![no_std]

pub use platform_traits;
pub use platform_traits::timer;

#[cfg(feature = "riscv64")]
pub use riscv64_rt as arch;
#[cfg(feature = "riscv64")]
pub use arch::{enable_interrupts, disable_interrupts, idle};

use platform_traits::Chip;
use platform_traits::timer::TimerChip;

#[cfg(feature = "qemu-virt")]
use qemu_virt as chip;
#[cfg(feature = "qemu-virt")]
pub use chip::QemuVirt as ChipImpl;

#[cfg(feature = "std")]
use std_chip as chip;
#[cfg(feature = "std")]
pub use chip::StdChip as ChipImpl;

pub mod logger;
pub use logger::Logger;

static LOGGER: Logger = Logger::new();

pub fn init() {
    let _ = LOGGER.init(log::LevelFilter::Trace);
}

/// 使能 MSI、定时器中断并开全局中断，开始响应调度器 ISR。
#[cfg(feature = "qemu-virt")]
pub unsafe fn start() {
    unsafe {
        ChipImpl::enable_timer_irq();
        arch::enable_msi();
        arch::enable_interrupts();
    }
}

/// 系统调度器 pend 标记。
///
/// `pend()` 触发 MSI 前置为 true，`MachineSoft` ISR 据此区分
/// 系统调度触发与外部 MSI 触发。
pub static PEND_MARKER: portable_atomic::AtomicBool = portable_atomic::AtomicBool::new(false);

/// 触发调度器软件中断。
pub unsafe fn pend() {
    PEND_MARKER.store(true, portable_atomic::Ordering::Release);
    unsafe { ChipImpl::pend() };
}

/// 清除调度器软件中断挂起标志，并返回 PEND_MARKER 的先前值。
///
/// 返回 `true` 表示本次 MSI 由调度器 `pend()` 触发，`false` 表示外部触发。
pub unsafe fn clear_pend() -> bool {
    let is_system = PEND_MARKER.swap(false, portable_atomic::Ordering::AcqRel);
    unsafe { ChipImpl::clear_pend() };
    is_system
}
