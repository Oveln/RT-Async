//! Mutex 跨优先级测试：高优先级写入，低优先级读取验证。
//!
//! 即使低优先级先 spawn，高优先级仍然先运行，完成后低优先级获得锁。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use platform::Chip;
static DATA: futures::mutex::Mutex<u32, 2> = futures::mutex::Mutex::new(0);

#[executor::task]
async fn high_writer() {
    let mut guard = DATA.lock().await;
    *guard = 42;
}

#[executor::task]
async fn low_reader() {
    let guard = DATA.lock().await;
    if *guard != 42 {
        test::fail("expected 42");
    }
    unsafe { test::record("ok") };
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    // 低优先级先 spawn，但高优先级先执行
    spawner.spawn(Priority::new(1), low_reader().unwrap());
    spawner.spawn(Priority::new(0), high_writer().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    test::assert_log(&["ok"]);
    platform::ChipImpl::shutdown();
}
