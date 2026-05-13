//! 三级嵌套抢占：low(2) spawn mid(1)，mid spawn high(0)，
//! 验证 low_start → mid_start → high → mid_end → low_end。

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
async fn task_high() {
    unsafe {
        test::record("high");
    }
}

#[executor::task]
async fn task_mid(spawner: Pin<&'static Spawner<4>>) {
    unsafe {
        test::record("mid_start");
    }
    spawner.spawn(Priority::new(0), task_high().unwrap());
    unsafe {
        test::record("mid_end");
    }
}

#[executor::task]
async fn task_low(spawner: Pin<&'static Spawner<4>>) {
    unsafe {
        test::record("low_start");
    }
    spawner.spawn(Priority::new(1), task_mid(spawner).unwrap());
    unsafe {
        test::record("low_end");
    }

    test::assert_log(&["low_start", "mid_start", "high", "mid_end", "low_end"]);
    platform::ChipImpl::shutdown();
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(2), task_low(spawner).unwrap());
}
