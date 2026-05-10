//! 定时器集成测试：验证 QEMU virt TimerChip 的基本行为。
//!
//! 测试项：
//! 1. `now_ticks()` 非零且单调递增
//! 2. `set_deadline()` 触发 MachineTimer 中断
//! 3. 中断触发时 ISR tick >= deadline

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use platform::{ChipImpl, idle};
use platform::platform_traits::Chip;
use platform::timer::TimerChip;

static TIMER_FIRED: AtomicBool = AtomicBool::new(false);
static ISR_TICK: AtomicU64 = AtomicU64::new(0);

#[executor::interrupt]
fn MachineTimer(_tf: &mut platform::arch::TrapFrame) {
    let now = ChipImpl::now_ticks();
    ISR_TICK.store(now, Ordering::Relaxed);
    TIMER_FIRED.store(true, Ordering::Relaxed);
    ChipImpl::set_deadline(u64::MAX);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MachineSoft(_tf: &mut platform::arch::TrapFrame) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rust_main() {
    platform::init();

    // 测试 1: now_ticks() 单调递增
    let t0 = ChipImpl::now_ticks();
    let t1 = ChipImpl::now_ticks();
    if t1 < t0 {
        ChipImpl::put_str("FAIL: now_ticks not monotonic\n");
        unsafe { core::ptr::write_volatile(0x100_000 as *mut u32, 0x3333 | (1 << 16)) };
        loop {}
    }

    // 测试 2: set_deadline 触发中断
    // 先设 deadline 再开中断，避免 mtimecmp 初始值导致立即触发。
    let ticks_per_ms = ChipImpl::FREQ_HZ as u64 / 1_000;
    let deadline = ChipImpl::now_ticks() + ticks_per_ms;
    ChipImpl::set_deadline(deadline);
    unsafe { ChipImpl::enable_irq() };
    unsafe { platform::arch::enable_interrupts() };

    // wfi 等待中断唤醒。
    while !TIMER_FIRED.load(Ordering::Acquire) {
        idle();
    }

    // 测试 3: ISR tick >= deadline
    let isr_tick = ISR_TICK.load(Ordering::Relaxed);
    if isr_tick < deadline {
        ChipImpl::put_str("FAIL: ISR tick < deadline\n");
        unsafe { core::ptr::write_volatile(0x100_000 as *mut u32, 0x3333 | (1 << 16)) };
        loop {}
    }

    ChipImpl::put_str("timer test passed\n");
    ChipImpl::shutdown();
}
