#![no_std]

use extern_trait::extern_trait;

/// 板级初始化钩子。
///
/// 每个板级 crate 实现该 trait（经 `#[extern_trait]` 做静态分发）。
/// `init()` 在 `platform::init()` 内部调用，负责 DTB 注入、driver 列表注册
/// 和 DT 遍历实例化 ([`crate::driver::boot`])。
#[extern_trait(pub BoardImpl)]
pub trait Board {
    fn init();
    /// 板级延迟初始化：在 app `main()` 之后、`platform::start()` 开中断之前调用。
    ///
    /// 用于需要推迟到全局中断开启前最后时刻的板级配置。典型场景：AMP 共享
    /// 中断控制器，需等待另一 hart 完成初始化后再配置本 hart 的中断源。
    /// 默认空实现，无此需求的板子无需实现。
    fn late_init() {}
}

#[cfg(feature = "riscv64")]
pub use arch::{disable_interrupts, enable_interrupts, enable_mtimer, idle};
#[cfg(feature = "riscv64")]
pub use riscv64_rt as arch;

pub mod device;
pub mod driver;
pub mod drivers;
pub mod dtb;
pub mod irq;
pub mod logger;
pub use logger::Logger;

// 便捷 re-export：上层（executor / futures / apps）直接取用。
pub use device::{Driver, InterruptController, Ipi, Reset, Serial, SerialRxStatus, Timer};
pub use driver::{boot, console, intctl, ipi, reset, timer, DeviceRegistry, Slot};
pub use irq::{dispatch_external, register_irq, IrqHandler};

static LOGGER: Logger = Logger::new();

pub fn init(max_level: log::LevelFilter) {
    let _ = LOGGER.init(max_level);

    #[cfg(feature = "riscv64")]
    arch::arch_init();

    BoardImpl::init();
}

#[cfg(feature = "riscv64")]
pub unsafe fn start() {
    unsafe {
        // 板级延迟初始化：在开中断前给板子最后一次配置机会（如 AMP 共享
        // 中断控制器需等待另一 hart 完成初始化）。默认空实现。
        BoardImpl::late_init();

        // 先把 deadline 推到最远，避免立即触发定时器中断。
        driver::timer().set_deadline(u64::MAX);
        arch::enable_mtimer();
        // MEIE 必须在 MSIE 之前：start() 被调用时 mip.MSIP 常已 pending
        // （hart0 的 IPI），若先开 MSIE 会立即被 MSI 抢占进 MachineSoft ISR
        // （该 ISR 内部直接跑抢占式调度器），旁路后续 enable_mei/
        // enable_interrupts，导致 MEIE=0、外部中断永不触发。
        arch::enable_mei();
        arch::enable_msi();
        arch::enable_interrupts();
    }
}

pub static PEND_MARKER: portable_atomic::AtomicBool = portable_atomic::AtomicBool::new(false);

pub unsafe fn pend() {
    PEND_MARKER.store(true, portable_atomic::Ordering::Release);
    // SAFETY: 调用者（executor wake path）保证上下文合适。
    unsafe { driver::ipi().send() };
}

pub unsafe fn clear_pend() -> bool {
    let is_system = PEND_MARKER.swap(false, portable_atomic::Ordering::AcqRel);
    // SAFETY: ISR 早期调用，关中断上下文。
    unsafe { driver::ipi().clear() };
    is_system
}
