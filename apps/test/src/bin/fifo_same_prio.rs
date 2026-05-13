//! 同优先级 FIFO 顺序：三个任务同一优先级，验证 a → b → c。

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
async fn task_a() {
    unsafe {
        test::record("a");
    }
}

#[executor::task]
async fn task_b() {
    unsafe {
        test::record("b");
    }
}

#[executor::task]
async fn task_c() {
    unsafe {
        test::record("c");
    }
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(0), task_a().unwrap());
    spawner.spawn(Priority::new(0), task_b().unwrap());
    spawner.spawn(Priority::new(0), task_c().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    test::assert_log(&["a", "b", "c"]);
    platform::ChipImpl::shutdown();
}
