#![no_std]

pub use riscv64_rt;
pub use platform_traits;

use qemu_virt as chip;
pub use chip::QemuVirt as ChipImpl;
pub mod logger;

pub use logger::Logger;

static LOGGER: Logger = Logger::new();

pub fn init() {
    let _ = LOGGER.init(log::LevelFilter::Trace);
}
