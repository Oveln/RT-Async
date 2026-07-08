//! 演示 `futures::timer::after()` 的基本用法
//!
//! 每秒输出一条日志，循环 5 次后关机。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use executor::priority::Priority;
use fugit::ExtU64;

#[executor::task]
async fn tick_task() {
    for i in 1..=5 {
        futures::timer::after(1.secs()).await;
        log::info!("tick #{i}");
    }
    log::info!("timer_async demo done");
    platform::reset().shutdown();
}

#[executor::interrupt]
fn MachineTimer(_tf: &mut platform::arch::TrapFrame) {
    futures::timer::handle_timer_isr();
}

#[executor::main]
fn main(spawner: core::pin::Pin<&'static executor::spawner::Spawner<4>>) {
    spawner.spawn(Priority::new(0), tick_task().unwrap());
}
