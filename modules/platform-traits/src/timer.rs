//! Timer chip abstraction.
//!
//! Platforms implement [`TimerChip`] to expose a monotonic tick counter
//! and an absolute-deadline compare register used by the executor's
//! timer queue.
//!
//! The const generic `FREQ_HZ` encodes the tick frequency at compile
//! time, matching fugit's fraction representation:
//!
//! ```ignore
//! // QEMU virt at 10 MHz
//! use fugit::Duration;
//! type Dur = Duration<u64, 1, 10_000_000>;
//! //            tick type ──┘  └── FREQ_HZ
//! ```
//!
//! # RISC-V (CLINT)
//!
//! `now_ticks` reads `mtime`; `set_deadline` writes `mtimecmp`.
//!
//! # ARM Cortex-M (SysTick / GPT)
//!
//! The implementation maintains a software 64-bit counter (accumulated
//! in the ISR) and converts the absolute deadline to a relative reload
//! value, handling the 24-bit SysTick limit internally.

/// Platform timer interface.
///
/// `FREQ_HZ` is the tick frequency in Hz, known at compile time.
///
/// All methods are static (no `&self`) so they can be stored as plain
/// function pointers and called from ISR context.
pub trait TimerChip<const FREQ_HZ: u32> {
    /// Read the current tick count (monotonic).
    fn now_ticks() -> u64;

    /// Program an absolute deadline.
    ///
    /// A timer interrupt fires when `now_ticks() >= tick`.
    /// Writing a value in the past triggers an immediate interrupt.
    /// Passing `u64::MAX` signals "no deadline" — the implementation
    /// may disable the timer.
    fn set_deadline(tick: u64);

    /// Enable the timer interrupt source.
    ///
    /// Called once during initialisation, before the main loop starts.
    unsafe fn enable_irq();
}
