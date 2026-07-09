//! # std 芯片实现（host 单测桩）
//!
//! 为 host `std` 环境提供 [`Board`] 实现和最小 driver 注册，
//! 使 executor / futures 的 host 单测（`cargo test`）可正常运行。
//!
//! 所有 driver 均为桩：console 走 `print!`，timer 返回固定值，
//! reset 调 `exit(0)`，ipi 为空操作。

#![allow(unreachable_code)]
use std::process::exit;

use extern_trait::extern_trait;
use platform::{Board};

/// std 环境的板级实现。
pub struct StdChip;

// ── 桩驱动 ───────────────────────────────────────────────────────────

struct StdSerial;
impl platform::Serial for StdSerial {
    fn write(&self, buf: &[u8]) {
        let s = core::str::from_utf8(buf).unwrap_or("<non-utf8>");
        print!("{}", s);
    }
}

struct StdTimer;
impl platform::Timer for StdTimer {
    fn freq_hz(&self) -> u32 {
        1_000_000
    }
    fn now(&self) -> u64 {
        0
    }
    fn set_deadline(&self, _tick: u64) {}
}

struct StdReset;
impl platform::Reset for StdReset {
    fn shutdown(&self) -> ! {
        exit(0)
    }
}

struct StdIpi;
impl platform::Ipi for StdIpi {
    unsafe fn send(&self) {}
    unsafe fn clear(&self) {}
}

static STD_SERIAL: StdSerial = StdSerial;
static STD_TIMER: StdTimer = StdTimer;
static STD_RESET: StdReset = StdReset;
static STD_IPI: StdIpi = StdIpi;

#[extern_trait]
impl Board for StdChip {
    fn init() {
        platform::driver::set_console(&STD_SERIAL);
        platform::driver::set_timer(&STD_TIMER);
        platform::driver::set_reset(&STD_RESET);
        platform::driver::set_ipi(&STD_IPI);
    }
}
