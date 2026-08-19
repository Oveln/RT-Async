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

use core::cell::UnsafeCell;
use core::mem::{self, MaybeUninit};
use core::task::{Context, Poll, Waker};

use portable_atomic::{AtomicU8, AtomicUsize, Ordering};

/// 最大 IRQ 号 + 1。K3 Mailbox 中断号最高 69（+ 安全余量 → 96）。
pub const MAX_IRQ: usize = 96;

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

// ── IrqLatch：通用的 ISR→async task 桥接原语 ──────────────────────

/// 通用的中断通知锁存器——任意驱动复用的 await-IRQ 基础设施。
///
/// 一个 `IrqLatch` 实例绑定一个中断源。ISR 侧调 [`IrqLatch::notify`] 置位
/// pending 并唤醒等待者；async 侧经 [`IrqLatch::poll`] 注册 waker，中断到达后
/// 被唤醒重新 poll 返回 `Ready`。
///
/// ## 竞态修复
///
/// 采用与 `serial_ns16550a::rx_register_waker` 相同的"关中断→注册→重检→
/// 开中断"模式，消除 ISR 在注册 waker 前后触发的竞态：
///
/// 1. 关中断（`disable_interrupts`）
/// 2. 写入 waker，置 `has_waker = true`
/// 3. 重检 pending——若 ISR 在关中断前已到达，立即消费
/// 4. 开中断（`enable_interrupts`），返回 `Pending`
///
/// ## 约束
///
/// 仅 riscv64 可用——依赖 `arch` 关/开中断原语。
pub struct IrqLatch {
    // AtomicU8（0/1）而非 AtomicBool：portable-atomic 的 critical-section
    // 后端（K3 专属 atomic-cas:false target）不为 AtomicBool 提供 RMW——
    // 定宽类型的 swap 走 mstatus MIE 屏蔽 + 普通访存。
    pending: AtomicU8,
    has_waker: AtomicU8,
    waker: UnsafeCell<MaybeUninit<Waker>>,
}

// SAFETY: 并发安全靠原子状态 + 关中断临界区保证：
// - pending/has_waker 是 AtomicU8（0/1 语义），原子读写。
// - waker 槽的写只发生在关中断临界区内（poll 侧）或 ISR 上下文（关中断），
//   读发生在 ISR 的 notify（关中断），单写者单读者互斥。
unsafe impl Sync for IrqLatch {}

impl IrqLatch {
    /// 创建空锁存器（pending=false，无 waker）。
    pub const fn new() -> Self {
        Self {
            pending: AtomicU8::new(0),
            has_waker: AtomicU8::new(0),
            waker: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// ISR 侧：置位 pending 并唤醒等待者（若有）。
    ///
    /// 在中断上下文调用（关中断执行）。`has_waker` 的 `swap(false, AcqRel)`
    /// 原子地消费 waker 槽，保证 poll 与 notify 不会并发访问 waker。
    pub fn notify(&self) {
        self.pending.store(1, Ordering::Release);
        if self.has_waker.swap(0, Ordering::AcqRel) != 0 {
            // SAFETY: has_waker=true 保证 waker 槽已初始化；
            // swap 已原子消费标志，此处独占访问。
            unsafe {
                let waker = (*self.waker.get()).assume_init_read();
                waker.wake();
            }
        }
    }

    /// async 侧：检查 pending，未就绪则注册 waker 等待下次中断。
    ///
    /// 返回 `Poll::Ready(())` 表示有中断到达（pending 被消费）；
    /// `Poll::Pending` 表示已注册 waker，等待 ISR 唤醒。
    ///
    /// 仅 riscv64 可用。
    #[cfg(feature = "riscv64")]
    pub fn poll(&self, cx: &mut Context<'_>) -> Poll<()> {
        // 快速路径：已有 pending。
        if self.pending.swap(0, Ordering::AcqRel) != 0 {
            return Poll::Ready(());
        }

        // 关中断 → 注册 waker → 重检 → 开中断。
        unsafe { crate::arch::disable_interrupts() };
        // SAFETY: 关中断临界区。
        unsafe {
            if self.has_waker.load(Ordering::Relaxed) != 0 {
                (*self.waker.get()).assume_init_drop();
            }
            (*self.waker.get()).write(cx.waker().clone());
        }
        self.has_waker.store(1, Ordering::Release);

        // 重检——ISR 可能在关中断前已调 notify。
        if self.pending.swap(0, Ordering::AcqRel) != 0 {
            self.has_waker.store(0, Ordering::Relaxed);
            unsafe {
                (*self.waker.get()).assume_init_drop();
            }
            unsafe { crate::arch::enable_interrupts() };
            return Poll::Ready(());
        }

        unsafe { crate::arch::enable_interrupts() };
        Poll::Pending
    }
}

/// `Future` 包裹一个 [`IrqLatch`] 的引用。
///
/// 每完成一次表示一次中断到达。可反复 poll（循环 await 收割中断）。
///
/// ```ignore
/// let latch = IrqLatch::new();
/// // ISR 中: latch.notify();
/// IrqFuture(&latch).await;
/// ```
#[cfg(feature = "riscv64")]
pub struct IrqFuture<'a>(&'a IrqLatch);

#[cfg(feature = "riscv64")]
impl<'a> IrqFuture<'a> {
    pub fn new(latch: &'a IrqLatch) -> Self {
        Self(latch)
    }
}

#[cfg(feature = "riscv64")]
impl<'a> core::future::Future for IrqFuture<'a> {
    type Output = ();

    fn poll(self: core::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        self.0.poll(cx)
    }
}
