//! 抢占链：prio2 spawn prio0，prio0 再 spawn prio1，
//! 验证执行顺序 low_start → high → mid → low_end。

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
async fn task_mid() {
    unsafe {
        test::record("mid");
    }
}

#[executor::task]
async fn task_high(spawner: Pin<&'static Spawner<4>>) {
    unsafe {
        test::record("high");
    }
    spawner.spawn(Priority::new(1), task_mid().unwrap());
}

#[executor::task]
async fn task_low(spawner: Pin<&'static Spawner<4>>) {
    unsafe {
        test::record("low_start");
    }
    spawner.spawn(Priority::new(0), task_high(spawner).unwrap());
    unsafe {
        test::record("low_end");
    }

    test::assert_log(&["low_start", "high", "mid", "low_end"]);
    platform::ChipImpl::shutdown();
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(2), task_low(spawner).unwrap());
}
