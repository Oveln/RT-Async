//! 异步定时器集成测试：验证 futures::timer::after() 的基本行为。
//!
//! 测试项：
//! 1. after() 在指定时间后正确完成
//! 2. 多次 after() 顺序执行

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use core::sync::atomic::{AtomicUsize, Ordering};

use fugit::ExtU64;
use platform::platform_traits::Chip;
use platform::timer::TimerChip;

static STEP: AtomicUsize = AtomicUsize::new(0);

#[executor::task]
async fn timer_task() {
    // 测试 1: after(1ms) 正确完成
    futures::timer::after(1.millis()).await;
    STEP.store(1, Ordering::Relaxed);

    // 测试 2: 多次顺序等待
    futures::timer::after(1.millis()).await;
    STEP.store(2, Ordering::Relaxed);

    futures::timer::after(1.millis()).await;
    STEP.store(3, Ordering::Relaxed);

    platform::ChipImpl::put_str("timer_async test passed\n");
    platform::ChipImpl::shutdown();
}

#[executor::interrupt]
fn MachineTimer(_tf: &mut platform::arch::TrapFrame) {
    futures::timer::handle_timer_isr();
}

#[executor::main]
fn main(spawner: core::pin::Pin<&'static executor::spawner::Spawner<4>>) {
    unsafe { platform::ChipImpl::enable_irq() };

    spawner.spawn(
        executor::priority::Priority::new(0),
        timer_task().unwrap(),
    );
}
