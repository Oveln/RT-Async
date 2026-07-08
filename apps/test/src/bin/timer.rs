//! 定时器集成测试：验证 platform::timer() 的基本行为。
//!
//! 测试项：
//! 1. `now_ticks()` 非零且单调递增
//! 2. `set_deadline()` 触发 MachineTimer 中断
//! 3. 中断触发时 ISR tick >= deadline

#![no_std]
#![no_main]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use platform::idle;

static TIMER_FIRED: AtomicBool = AtomicBool::new(false);
static ISR_TICK: AtomicU64 = AtomicU64::new(0);

#[executor::interrupt]
fn MachineTimer(_tf: &mut platform::arch::TrapFrame) {
    let now = platform::timer().now();
    ISR_TICK.store(now, Ordering::Relaxed);
    TIMER_FIRED.store(true, Ordering::Relaxed);
    platform::timer().set_deadline(u64::MAX);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MachineSoft(_tf: &mut platform::arch::TrapFrame) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rust_main() {
    platform::init(log::LevelFilter::Info);

    // 测试 1: now_ticks() 单调递增
    let t0 = platform::timer().now();
    let t1 = platform::timer().now();
    if t1 < t0 {
        platform::console().write(b"FAIL: now_ticks not monotonic\n");
        unsafe { core::ptr::write_volatile(0x100_000 as *mut u32, 0x3333 | (1 << 16)) };
        loop {}
    }

    // 测试 2: set_deadline 触发中断
    // 先设 deadline 再开中断，避免 mtimecmp 初始值导致立即触发。
    let ticks_per_ms = platform::timer().freq_hz() as u64 / 1_000;
    let deadline = platform::timer().now() + ticks_per_ms;
    platform::timer().set_deadline(u64::MAX);
    unsafe { platform::arch::enable_mtimer() };
    platform::timer().set_deadline(deadline);
    unsafe { platform::arch::enable_interrupts() };

    // wfi 等待中断唤醒。
    while !TIMER_FIRED.load(Ordering::Acquire) {
        idle();
    }

    // 测试 3: ISR tick >= deadline
    let isr_tick = ISR_TICK.load(Ordering::Relaxed);
    if isr_tick < deadline {
        platform::console().write(b"FAIL: ISR tick < deadline\n");
        unsafe { core::ptr::write_volatile(0x100_000 as *mut u32, 0x3333 | (1 << 16)) };
        loop {}
    }

    platform::console().write(b"timer test passed\n");
    platform::reset().shutdown();
}
