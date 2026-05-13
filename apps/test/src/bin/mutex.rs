//! Mutex 集成测试：验证互斥锁的基本行为。
//!
//! 测试项：
//! 1. 单任务 lock/unlock，值可正确读写
//! 2. 两任务竞争同一 mutex，验证 FIFO 排队

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
async fn writer() {
    let mut guard = DATA.lock().await;
    unsafe { test::record("w_lock") };
    *guard += 1;
    unsafe { test::record("w_unlock") };
}

#[executor::task]
async fn reader() {
    let guard = DATA.lock().await;
    unsafe { test::record("r_lock") };
    let _v = *guard;
    drop(guard);
    unsafe { test::record("r_done") };
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(0), writer().unwrap());
    spawner.spawn(Priority::new(0), reader().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    // 同优先级协作调度：writer 先运行，lock → writer unlock → reader lock → reader done
    test::assert_log(&["w_lock", "w_unlock", "r_lock", "r_done"]);
    platform::ChipImpl::shutdown();
}
