#[allow(unreachable_code)]
use std::process::exit;

use extern_trait::extern_trait;
use platform::{Chip, TimerChip};

pub struct StdChip;

#[extern_trait]
impl Chip for StdChip {
    fn shutdown() -> ! {
        exit(0)
    }

    fn put_str(s: &str) {
        print!("{}", s);
    }

    unsafe fn pend() {}

    unsafe fn clear_pend() {}
}

#[extern_trait]
impl TimerChip for StdChip {
    fn freq_hz() -> u32 {
        1_000_000
    }

    fn now_ticks() -> u64 {
        todo!()
    }

    fn set_deadline(_tick: u64) {
        todo!()
    }

    unsafe fn enable_timer_irq() {
        todo!()
    }
}
