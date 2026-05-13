//! 演示基于优先级的抢占调度
//!
//! 三个不同优先级的任务验证抢占行为：
//! - `low_prio_task`(优先级 2)：运行中 spawn 更高优先级任务，触发立即抢占
//! - `mid_prio_task`(优先级 1)：验证中间优先级任务的正确调度
//! - `high_prio_task`(优先级 0)：高优先级任务抢占低优先级任务执行
//!
//! 辅助函数 `yield_n` 让任务主动 yield 指定次数，观察调度器在不同
//! 优先级之间的切换顺序。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::mem::MaybeUninit;
use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use platform::arch::TrapFrame;

static mut SPAWNER: MaybeUninit<Spawner<4>> = MaybeUninit::uninit();

#[executor::task]
async fn low_prio_task(spawner: Pin<&'static Spawner<4>>) {
    log::info!("[low  prio 2] started, about to spawn high-prio task...");

    // spawn(0) 会立即触发抢占：high_prio_task 中断当前执行
    spawner.spawn(Priority::new(0), high_prio_task().unwrap());

    // high 完成后，mid (prio 1) 优先于 low 恢复运行
    log::info!("[low  prio 2] resumed after preemption, yielding twice...");

    yield_n::<2>("[low  prio 2]").await;

    log::info!("[low  prio 2] now spawning another high-prio task...");
    spawner.spawn(Priority::new(0), high_prio_task().unwrap());

    log::info!("[low  prio 2] final resume, done");
}

#[executor::task]
async fn high_prio_task() {
    log::info!("[HIGH prio 0] *** preempted! ***");

    yield_n::<2>("[HIGH prio 0]").await;

    log::info!("[HIGH prio 0] done");
}

#[executor::task]
async fn mid_prio_task() {
    log::info!("[mid  prio 1] started, yielding once...");

    yield_n::<1>("[mid  prio 1]").await;

    log::info!("[mid  prio 1] done");
}

async fn yield_n<const N: u32>(label: &'static str) {
    let mut count = 0u32;
    core::future::poll_fn(move |cx| {
        count += 1;
        if count < N {
            log::info!("{label} poll #{count}/{N}, yielding...");
            cx.waker().wake_by_ref();
            return core::task::Poll::Pending;
        }
        log::info!("{label} poll #{count}/{N}, ready");
        core::task::Poll::Ready(())
    })
    .await;
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rust_main() -> ! {
    unsafe {
        platform::init();
        log::info!("demo: preempt test — priority preemption & interleaving");
        log::info!("expected order:");
        log::info!("  1. mid(1) runs & completes (highest prio among initial ready tasks)");
        log::info!("  2. low(2) starts, spawns high(0) -> preempted");
        log::info!("  3. high(0) runs to completion (yields internally but stays on executor[0])");
        log::info!("  4. low(2) resumes, yields twice, spawns high(0) again -> preempted");
        log::info!("  5. high(0) second run to completion");
        log::info!("  6. low(2) final resume, done");

        let ptr = core::ptr::addr_of_mut!(SPAWNER).cast::<Spawner<4>>();
        ptr.write(Spawner::new());
        Pin::new_unchecked(&mut *ptr).as_mut().init();

        let spawner = Pin::new_unchecked(&*ptr);

        spawner.spawn(Priority::new(2), low_prio_task(spawner).unwrap());
        spawner.spawn(Priority::new(1), mid_prio_task().unwrap());

        platform::start();
    }
    loop {
        platform::idle();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MachineSoft(_trap_frame: &mut TrapFrame) {
    unsafe {
        platform::clear_pend();

        let spawner = Pin::new_unchecked(&*core::ptr::addr_of!(SPAWNER).cast::<Spawner<4>>());

        while let Some(rt) = spawner.try_preempt() {
            platform::enable_interrupts();
            spawner.run(rt);
            platform::disable_interrupts();
            spawner.complete_executor();
        }
    }
}
