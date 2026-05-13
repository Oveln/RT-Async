//! 优先级调度顺序：三个任务不同优先级，验证 high → mid → low。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use platform::Chip;
#[executor::task]
async fn task_low() {
    unsafe {
        test::record("low");
    }
}

#[executor::task]
async fn task_mid() {
    unsafe {
        test::record("mid");
    }
}

#[executor::task]
async fn task_high() {
    unsafe {
        test::record("high");
    }
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    // spawn 顺序 low → mid → high，验证执行仍为 high → mid → low
    spawner.spawn(Priority::new(2), task_low().unwrap());
    spawner.spawn(Priority::new(1), task_mid().unwrap());
    spawner.spawn(Priority::new(0), task_high().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    test::assert_log(&["high", "mid", "low"]);
    platform::ChipImpl::shutdown();
}
