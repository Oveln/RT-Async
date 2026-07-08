//! Mutex 竞争测试：holder 持锁跨 yield，两个 waiter 排队，验证数据正确传递。
//!
//! 验证：
//! 1. holder 持锁期间 waiter 被 Pending
//! 2. holder 释放后 waiter 依次获得锁
//! 3. 数据通过多次 lock/unlock 正确传递

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
static DATA: futures::mutex::Mutex<u32, 3> = futures::mutex::Mutex::new(0);

async fn yield_once() {
    let mut done = false;
    core::future::poll_fn(move |cx| {
        if done {
            core::task::Poll::Ready(())
        } else {
            done = true;
            cx.waker().wake_by_ref();
            core::task::Poll::Pending
        }
    })
    .await;
}

#[executor::task]
async fn holder() {
    let mut guard = DATA.lock().await;
    *guard = 10;
    yield_once().await;
}

#[executor::task]
async fn waiter_a() {
    let mut guard = DATA.lock().await;
    *guard += 1;
}

#[executor::task]
async fn waiter_b() {
    let guard = DATA.lock().await;
    if *guard != 11 {
        test::fail("expected 11");
    }
    unsafe { test::record("ok") };
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(0), holder().unwrap());
    spawner.spawn(Priority::new(0), waiter_a().unwrap());
    spawner.spawn(Priority::new(0), waiter_b().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    test::assert_log(&["ok"]);
    platform::reset().shutdown();
}
