//! 抢占调度：低优先级 spawn 高优先级，验证抢占行为。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use platform::platform_traits::Chip;

#[executor::task]
async fn task_high() {
    unsafe {
        test::record("high");
    }
}

#[executor::task]
async fn task_low(spawner: Pin<&'static Spawner<4>>) {
    unsafe {
        test::record("low_start");
    }
    spawner.spawn(Priority::new(0), task_high().unwrap());
    unsafe {
        test::record("low_end");
    }

    test::assert_log(&["low_start", "high", "low_end"]);
    platform::ChipImpl::shutdown();
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(2), task_low(spawner).unwrap());
}
