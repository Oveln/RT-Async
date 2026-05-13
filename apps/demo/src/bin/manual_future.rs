//! 演示手动实现 `Future` trait
//!
//! 包含两个手写 Future：
//! - `CountTask`：立即完成的简单任务，演示最基础的 poll 语义
//! - `YieldTwice`：通过 `wake_by_ref()` 让自身挂起并重新调度两次，
//!   展示 Pending → wake → re-poll 的完整异步生命周期

#![no_std]
#![no_main]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::mem::MaybeUninit;
use core::pin::Pin;
use core::sync::atomic::{AtomicU32, Ordering};
use core::task::{Context, Poll};

use executor::priority::Priority;
use executor::spawner::Spawner;
use executor::task::storage::TaskStorage;
use platform::arch::TrapFrame;

static mut SPAWNER: MaybeUninit<Spawner<4>> = MaybeUninit::uninit();
static COUNTER: AtomicU32 = AtomicU32::new(0);

struct CountTask;

impl Future for CountTask {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let c = COUNTER.fetch_add(1, Ordering::Relaxed);
        log::debug!("CountTask: counter -> {}", c + 1);
        Poll::Ready(())
    }
}

static COUNT_TASK: TaskStorage<CountTask> = TaskStorage::new();

struct YieldTwice(u8);

impl Future for YieldTwice {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0 += 1;
        log::debug!("YieldTwice: poll #{}", self.0);
        if self.0 < 3 {
            cx.waker().wake_by_ref();
            log::debug!("YieldTwice: pending, waking self");
            Poll::Pending
        } else {
            log::debug!("YieldTwice: done");
            Poll::Ready(())
        }
    }
}

static YIELD_TASK: TaskStorage<YieldTwice> = TaskStorage::new();

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rust_main() -> ! {
    unsafe {
        platform::init();
        log::info!("demo: hello from rt-async on riscv64!");

        let ptr = core::ptr::addr_of_mut!(SPAWNER).cast::<Spawner<4>>();
        ptr.write(Spawner::new());
        Pin::new_unchecked(&mut *ptr).as_mut().init();

        let spawner = Pin::new_unchecked(&*ptr);

        let token = COUNT_TASK.spawn(|| CountTask).unwrap();
        spawner.spawn(Priority::new(0), token);

        let token = YIELD_TASK.spawn(|| YieldTwice(0)).unwrap();
        spawner.spawn(Priority::new(1), token);

        platform::start();
    }
    loop {
        platform::idle();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MachineSoft(_trap_frame: &mut TrapFrame) {
    unsafe {
        // Clear MSI before processing to prevent re-trigger
        core::ptr::write_volatile(0x2000000usize as *mut u32, 0);

        let spawner = Pin::new_unchecked(&*core::ptr::addr_of!(SPAWNER).cast::<Spawner<4>>());

        while let Some(rt) = spawner.try_preempt() {
            // Enable interrupts so higher-priority tasks can preempt during run
            platform::enable_interrupts();
            spawner.run(rt);
            platform::disable_interrupts();
            spawner.complete_executor();
        }
    }
}
