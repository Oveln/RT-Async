//! 演示 `#[executor::main]` / `#[executor::task]` / `#[executor::interrupt]` 宏的完整用法
//!
//! 使用声明式宏自动生成入口函数和中断处理，无需手写 `__rust_main`、
//! `MachineSoft` 等 unsafe 符号。task3 通过直接写 MSI 寄存器触发
//! 一次外部软件中断，由 `#[executor::interrupt]` 标记的处理函数捕获。

#![no_std]
#![no_main]
#![feature(impl_trait_in_assoc_type)]

use executor::priority::Priority;
use executor::task::storage::TaskStorage;

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

#[executor::task]
async fn task3() {
    unsafe {
        core::ptr::write_volatile(0x2000000usize as *mut u32, 1);
    }
}

#[executor::interrupt]
fn MachineSoft(_tf: &mut platform::arch::TrapFrame) {
    log::info!("external MSI triggered!");
}

#[executor::main]
fn main(spawner: core::pin::Pin<&'static executor::spawner::Spawner<4>>) {
    log::info!("demo: hello from rt-async on riscv64!");

    spawner.spawn(Priority::new(0), task1().unwrap());
    spawner.spawn(Priority::new(2), task2().unwrap());
    spawner.spawn(Priority::new(1), task3().unwrap());
}
