//! Timer chip abstraction.
//!
//! Platforms implement [`TimerChip`] to expose a monotonic tick counter
//! and an absolute-deadline compare register used by the executor's
//! timer queue.
//!
//! The associated constant [`TimerChip::FREQ_HZ`] encodes the tick frequency,
//! matching fugit's fraction representation:
//!
//! ```ignore
//! // QEMU virt at 10 MHz
//! use fugit::Duration;
//! // Duration<u64, 1, 10_000_000> — FREQ_HZ is the denominator
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
/// All methods are static (no `&self`) so they can be stored as plain
/// function pointers and called from ISR context.
pub trait TimerChip {
    /// Tick frequency in Hz, known at compile time.
    const FREQ_HZ: u32;

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
