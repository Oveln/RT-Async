#![no_std]

pub use platform_traits;

#[cfg(feature = "riscv64")]
pub use riscv64_rt as arch;
#[cfg(feature = "riscv64")]
pub use arch::{enable_interrupts, disable_interrupts, idle};

use platform_traits::Chip;

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

/// 使能 MSI 并开全局中断，开始响应调度器 ISR。
#[cfg(feature = "qemu-virt")]
pub unsafe fn start() {
    unsafe {
        arch::enable_msi();
        arch::enable_interrupts();
    }
}

/// 触发调度器软件中断。
pub unsafe fn pend() {
    unsafe { ChipImpl::pend() };
}