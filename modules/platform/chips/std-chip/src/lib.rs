use std::process::exit;

use platform_traits::{Chip, timer::TimerChip};

pub struct StdChip;

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

/// std-chip 模拟定时器频率：1 MHz（微秒精度）。
const STD_FREQ_HZ: u32 = 1_000_000;

impl TimerChip<STD_FREQ_HZ> for StdChip {
    fn now_ticks() -> u64 {
        todo!()
    }

    fn set_deadline(_tick: u64) {
        todo!()
    }

    unsafe fn enable_irq() {
        todo!()
    }
}
