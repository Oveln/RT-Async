#![no_std]

use extern_trait::extern_trait;
#[extern_trait(pub ChipImpl)]
pub trait Chip {
    fn board_init();
    fn shutdown() -> !;
    fn put_str(s: &str);
    unsafe fn pend();
    unsafe fn clear_pend();
}

#[extern_trait(pub TimerChipImpl)]
pub trait TimerChip {
    fn freq_hz() -> u32;
    fn now_ticks() -> u64;
    fn set_deadline(tick: u64);
    unsafe fn enable_timer_irq();
}

#[cfg(feature = "riscv64")]
pub use arch::{disable_interrupts, enable_interrupts, idle};
#[cfg(feature = "riscv64")]
pub use riscv64_rt as arch;

pub mod dtb;
pub mod logger;
pub use logger::Logger;

static LOGGER: Logger = Logger::new();

pub fn init(max_level: log::LevelFilter) {
    let _ = LOGGER.init(max_level);

    #[cfg(feature = "riscv64")]
    arch::arch_init();

    ChipImpl::board_init();
}

#[cfg(feature = "riscv64")]
pub unsafe fn start() {
    unsafe {
        TimerChipImpl::enable_timer_irq();
        arch::enable_msi();
        arch::enable_mei();
        arch::enable_interrupts();
    }
}

pub static PEND_MARKER: portable_atomic::AtomicBool = portable_atomic::AtomicBool::new(false);

pub unsafe fn pend() {
    PEND_MARKER.store(true, portable_atomic::Ordering::Release);
    unsafe { ChipImpl::pend() };
}

pub unsafe fn clear_pend() -> bool {
    let is_system = PEND_MARKER.swap(false, portable_atomic::Ordering::AcqRel);
    unsafe { ChipImpl::clear_pend() };
    is_system
}
