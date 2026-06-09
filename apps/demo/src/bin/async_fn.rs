//! 演示 async fn 任务的手动 spawn 流程
//!
//! 展示如何在不使用宏的情况下，手动将 async fn 包装为 `SpawnToken`
//! 并通过 `Spawner` 调度执行。task1 使用手工展开的 `TaskTrait` 模式，
//! task2 使用 `#[executor::task]` 宏作为对比。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use core::mem::MaybeUninit;
use core::pin::Pin;

use executor::priority::Priority;
use executor::spawner::Spawner;
use executor::task::storage::TaskStorage;
use platform::arch::TrapFrame;

static mut SPAWNER: MaybeUninit<Spawner<4>> = MaybeUninit::uninit();

async fn __task1() {
    log::info!("hello from task 1");
}

fn task1() -> Result<
    executor::spawner::SpawnToken<impl Future<Output = ()> + 'static>,
    executor::task::storage::SpawnError,
> {
    trait TaskTrait {
        type Fut: Future<Output = ()> + 'static;
        fn construct() -> Self::Fut;
    }
    impl TaskTrait for () {
        type Fut = impl Future<Output = ()> + 'static;
        fn construct() -> Self::Fut {
            __task1()
        }
    }
    static TASK: TaskStorage<<() as TaskTrait>::Fut> = TaskStorage::new();
    TASK.spawn(move || <() as TaskTrait>::construct())
}

#[executor::task]
async fn task2() {
    log::info!("hello from task2");
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rust_main() -> ! {
    unsafe {
        platform::init(log::LevelFilter::Info);
        log::info!("demo: hello from rt-async on riscv64!");

        let ptr = core::ptr::addr_of_mut!(SPAWNER).cast::<Spawner<4>>();
        ptr.write(Spawner::new());
        Pin::new_unchecked(&mut *ptr).as_mut().init();

        let spawner = Pin::new_unchecked(&*ptr);

        spawner.spawn(Priority::new(0), task1().unwrap());
        spawner.spawn(Priority::new(1), task2().unwrap());
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
            // Enable interrupts so higher-priority tasks can preempt during run
            platform::enable_interrupts();
            spawner.run(rt);
            platform::disable_interrupts();
            spawner.complete_executor();
        }
    }
}
