//! 单任务执行：spawn 一个任务，验证它运行后 shutdown。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use platform::platform_traits::Chip;

#[executor::task]
async fn task_a() {
    unsafe {
        test::record("a");
    }
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(0), task_a().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    test::assert_log(&["a"]);
    platform::ChipImpl::shutdown();
}
