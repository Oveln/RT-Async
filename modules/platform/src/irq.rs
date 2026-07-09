//! IRQ 分发层——零开销的外部中断路由。
//!
//! MachineExternal 中断对应 PLIC 的多源汇总线。本模块提供：
//! - [`register_irq`]：板级在 `board_init` 中为每个外设 IRQ 注册 handler。
//! - [`dispatch_external`]：通用 MachineExternal ISR 入口，按 IRQ 号做
//!   O(1) 静态数组查找并分派到对应 handler。
//! - `#[no_mangle] fn __rt_machine_external`：arch 链接脚本通过
//!   `PROVIDE(MachineExternal = __rt_machine_external)` 将其设为默认
//!   MachineExternal handler（弱符号，可被 App 强符号覆盖）。
//!
//! ## 设计
//!
//! - 查找：[`IRQ_TABLE`] 是 `[AtomicUsize; MAX_IRQ]`，直接用 IRQ 号索引，
//!   无哈希、无链表、无排序。
//! - handler 类型是 `unsafe fn(u32)` 裸函数指针，无 trait object / vtable。
//! - 注册在 `board_init` 中关中断完成，分发在 ISR 中关中断读取，单写者单读者，
//!   仅需 `Release`/`Acquire` 内存序保证可见性。
//!
//! ## 范围
//!
//! 仅管理 **MachineExternal** 中断。MachineSoft（调度器）由 executor-macro
//! 强制生成强符号接管；MachineTimer（定时器队列）由 `#[executor::interrupt]`
//! 提供。两者均为单用途中断，无需 run-time 分发。

use core::mem;
use portable_atomic::{AtomicUsize, Ordering};

/// 最大 IRQ 号 + 1。QEMU virt PLIC 有 53 个源，64 留有安全余量。
pub const MAX_IRQ: usize = 64;

/// 外设 IRQ handler 类型。`irq` 参数是中断源 ID（claim 返回的值）。
pub type IrqHandler = unsafe fn(irq: u32);

/// 全局 IRQ handler 表。`IRQ_TABLE[irq]` 存 `handler as usize`，
/// 零值表示未注册 handler。分发层在 `claim` 后以 IRQ 号直接索引。
static IRQ_TABLE: [AtomicUsize; MAX_IRQ] =
    [const { AtomicUsize::new(0) }; MAX_IRQ];

/// 为一个外设 IRQ 注册 handler。
///
/// 在 `board_init` 中调用（`DRIVERS.set` + `boot` 之后，`platform::start`
/// 开全局中断之前）。重复注册会静默覆盖旧的 handler。
pub fn register_irq(irq: u32, handler: IrqHandler) {
    debug_assert!((irq as usize) < MAX_IRQ, "IRQ {} exceeds MAX_IRQ", irq);
    IRQ_TABLE[irq as usize].store(handler as usize, Ordering::Release);
}

/// 通用 MachineExternal 分发入口。
///
/// 流程：`intctl().claim()` → 查表 → 调 handler → `intctl().complete()`。
/// 虚假中断（claim 返回 0）仅 complete 不调 handler。
pub fn dispatch_external() {
    let intctl = crate::driver::intctl();
    let irq = intctl.claim();
    if irq == 0 {
        intctl.complete(0);
        return;
    }
    let ptr = IRQ_TABLE[irq as usize].load(Ordering::Acquire);
    if ptr != 0 {
        let handler: IrqHandler = unsafe { mem::transmute_copy::<usize, IrqHandler>(&ptr) };
        unsafe { handler(irq) };
    }
    intctl.complete(irq);
}

/// MachineExternal 中断默认 handler（强符号）。
///
/// 该符号经链接脚本 `PROVIDE(MachineExternal = __rt_machine_external)` 设为
/// 默认值，App 无需手写 `#[executor::interrupt] fn MachineExternal`。
/// 如需自定义 MachineExternal，App 仍可提供同名强符号覆盖。
#[cfg(feature = "riscv64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rt_machine_external(_tf: &mut crate::arch::TrapFrame) {
    dispatch_external();
}
