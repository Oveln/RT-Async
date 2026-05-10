//! 同优先级 yield 交替：两任务同优先级各 yield 一次，验证交错执行。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use platform::platform_traits::Chip;

async fn yield_once(label: &'static str) {
    unsafe {
        test::record(label);
    }
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
    unsafe {
        test::record(label);
    }
}

#[executor::task]
async fn task_a() {
    yield_once("a").await;
}

#[executor::task]
async fn task_b() {
    yield_once("b").await;
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(0), task_a().unwrap());
    spawner.spawn(Priority::new(0), task_b().unwrap());

    while let Some(rt) = spawner.try_preempt() {
        spawner.run(rt);
        spawner.complete_executor();
    }

    // a poll → a yield → b poll → b yield → a resume → b resume
    test::assert_log(&["a", "b", "a", "b"]);
    platform::ChipImpl::shutdown();
}
