//! JoinHandle 集成测试：验证不同优先级抢占场景下 JoinHandle 的行为。
//!
//! 场景 1 — 中优先级 await 低优先级 worker：
//!   Pending → 低优先级 worker 执行完毕 → 唤醒 → Ready
//!
//! 场景 2 — 中优先级 spawn 高优先级 worker：
//!   高优先级立即抢占执行完毕 → await 直接 Ready
//!
//! 场景 3 — 中优先级同时 spawn 高 + 低优先级 worker：
//!   高优先级抢占完成 → await 高 handle: Ready
//!   await 低 handle: Pending → 低优先级 worker 执行完毕 → 唤醒 → Ready

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU32, Ordering},
    task::{Context, Poll},
};

use executor::priority::Priority;
use executor::spawner::Spawner;
use executor::task::storage::TaskStorage;
use platform::Chip;

// --- Worker futures ---

struct WorkerLow;
impl Future for WorkerLow {
    type Output = u32;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u32> {
        unsafe {
            test::record("w_low");
        }
        Poll::Ready(42)
    }
}

struct WorkerHigh;
impl Future for WorkerHigh {
    type Output = u32;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u32> {
        unsafe {
            test::record("w_high");
        }
        Poll::Ready(7)
    }
}

struct WorkerHigh2;
impl Future for WorkerHigh2 {
    type Output = u32;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u32> {
        unsafe {
            test::record("w_high2");
        }
        Poll::Ready(10)
    }
}

struct WorkerLow2;
impl Future for WorkerLow2 {
    type Output = u32;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<u32> {
        unsafe {
            test::record("w_low2");
        }
        Poll::Ready(20)
    }
}

static T_LOW: TaskStorage<WorkerLow> = TaskStorage::new();
static T_HIGH: TaskStorage<WorkerHigh> = TaskStorage::new();
static T_HIGH2: TaskStorage<WorkerHigh2> = TaskStorage::new();
static T_LOW2: TaskStorage<WorkerLow2> = TaskStorage::new();

static R1: AtomicU32 = AtomicU32::new(0);
static R2: AtomicU32 = AtomicU32::new(0);
static R3: AtomicU32 = AtomicU32::new(0);
static R4: AtomicU32 = AtomicU32::new(0);

#[executor::task]
async fn test_join(spawner: Pin<&'static Spawner<4>>) {
    // Scenario 1: prio 1 awaits prio 2 worker
    unsafe {
        test::record("s1");
    }
    let h1 = spawner.spawn(Priority::new(2), T_LOW.spawn(|| WorkerLow).unwrap());
    R1.store(h1.await, Ordering::Release);
    unsafe {
        test::record("s1_done");
    }

    // Scenario 2: prio 1 spawns prio 0 worker (preempts)
    unsafe {
        test::record("s2");
    }
    let h2 = spawner.spawn(Priority::new(0), T_HIGH.spawn(|| WorkerHigh).unwrap());
    R2.store(h2.await, Ordering::Release);
    unsafe {
        test::record("s2_done");
    }

    // Scenario 3: prio 1 spawns prio 0 + prio 2 workers
    unsafe {
        test::record("s3");
    }
    let h3 = spawner.spawn(Priority::new(0), T_HIGH2.spawn(|| WorkerHigh2).unwrap());
    let h4 = spawner.spawn(Priority::new(2), T_LOW2.spawn(|| WorkerLow2).unwrap());
    R3.store(h3.await, Ordering::Release);
    unsafe {
        test::record("s3_mid");
    }
    R4.store(h4.await, Ordering::Release);
    unsafe {
        test::record("s3_done");
    }

    test::assert_log(&[
        "s1", "w_low", "s1_done", "s2", "w_high", "s2_done", "s3", "w_high2", "s3_mid", "w_low2",
        "s3_done",
    ]);
    if R1.load(Ordering::Acquire) != 42 {
        test::fail("r1 != 42");
    }
    if R2.load(Ordering::Acquire) != 7 {
        test::fail("r2 != 7");
    }
    if R3.load(Ordering::Acquire) != 10 {
        test::fail("r3 != 10");
    }
    if R4.load(Ordering::Acquire) != 20 {
        test::fail("r4 != 20");
    }
    platform::ChipImpl::shutdown();
}

#[executor::main]
fn main(spawner: Pin<&'static Spawner<4>>) {
    spawner.spawn(Priority::new(1), test_join(spawner).unwrap());
}
