//! NS16550A 兼容串口驱动。
//!
//! QEMU `virt` 平台默认 UART（`serial@10000000`，compatible = `ns16550a`）。
//! 本驱动按设备树 `reg[0]` 取 MMIO 基址。
//!
//! 接收侧提供两种模式：
//! - **轮询**：`Serial::read() / has_data()` 直接读 RBR。
//! - **中断驱动**：probe 使能 FIFO 和 ERBFI（RX 中断），通过板级注册
//!   [`rx_handler`] 到 `platform::register_irq`，字节流入内建环形缓冲区。
//!   `Serial::rx_register_waker` 遵循 disable→register→recheck→enable 临界区模式
//!   （与 `apps/rt-async-app/src/uart_wait.rs` 一致），可被
//!   `SerialRx` Future 经 registry 调用。

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use core::task::Waker;

use fdt_parser::Node;
use tock_registers::interfaces::{Readable, Writeable};
use tock_registers::registers::{ReadOnly, ReadWrite};
use tock_registers::{register_bitfields, register_structs};

use crate::device::{Driver, Serial};

// ── 寄存器定义（tock-registers）────────────────────────────────────

register_bitfields![u8,
    /// 中断使能寄存器 IER。
    Ier [
        ERBFI OFFSET(0) NUMBITS(1) [],  // RX 数据可用中断使能
    ],
    /// FIFO 控制寄存器 FCR。
    Fcr [
        ENABLE OFFSET(0) NUMBITS(1) [],  // FIFO 使能
        CLR_RX OFFSET(1) NUMBITS(1) [],  // 清 RX FIFO
        CLR_TX OFFSET(2) NUMBITS(1) [],  // 清 TX FIFO
    ],
    /// 线路控制寄存器 LCR。
    Lcr [
        WLEN8 OFFSET(0) NUMBITS(2) [],   // 8 数据位（值 0b11）
        DLAB  OFFSET(7) NUMBITS(1) [],   // 除数锁存访问
    ],
    /// 线路状态寄存器 LSR。
    Lsr [
        DR   OFFSET(0) NUMBITS(1) [],   // 数据就绪
        THRE OFFSET(5) NUMBITS(1) [],   // 发送保持寄存器空
    ],
];

register_structs! {
    /// NS16550A 寄存器映射（u8 寄存器，stride = 1）。
    pub Ns16550aRegs {
        (0x00 => rbr_thr: ReadWrite<u8>),                       // 读: RBR, 写: THR
        (0x01 => ier:     ReadWrite<u8, Ier::Register>),
        (0x02 => iir_fcr: ReadWrite<u8, Fcr::Register>),        // 读: IIR, 写: FCR
        (0x03 => lcr:     ReadWrite<u8, Lcr::Register>),
        (0x04 => _reserved),
        (0x05 => lsr:     ReadOnly<u8, Lsr::Register>),
        (0x06 => _reserved1),
        (0x08 => @END),
    }
}

// ── 驱动实例 ─────────────────────────────────────────────────────────

/// NS16550A 串口单例（零大小）。
pub struct Ns16550a;

/// 全局单例，供 probe 注册进 registry。
pub static INSTANCE: Ns16550a = Ns16550a;

/// probe 写入的 MMIO 基址。0 表示尚未 probe。
static BASE: AtomicUsize = AtomicUsize::new(0);

/// 返回寄存器引用。probe 前调用为 panic。
fn regs() -> &'static Ns16550aRegs {
    let addr = BASE.load(Ordering::Acquire);
    assert!(addr != 0, "ns16550a: not probed");
    // SAFETY: addr 来自 probe 写入的 DT reg，指向已验证的 MMIO 区域。
    // 单 hart 串行访问，无别名引用（tock-registers 内部用 volatile）。
    unsafe { &*(addr as *const Ns16550aRegs) }
}

// ── Serial trait impl ────────────────────────────────────────────────

impl Serial for Ns16550a {
    fn write(&self, buf: &[u8]) {
        let r = regs();
        for &byte in buf {
            // 等发送保持寄存器空（LSR.THRE = 1），防止连续写入时 FIFO 溢出丢字节。
            while !r.lsr.is_set(Lsr::THRE) {
                core::hint::spin_loop();
            }
            r.rbr_thr.set(byte);
        }
    }

    fn read(&self) -> Option<u8> {
        let r = regs();
        if !r.lsr.is_set(Lsr::DR) {
            return None;
        }
        Some(r.rbr_thr.get())
    }

    fn has_data(&self) -> bool {
        regs().lsr.is_set(Lsr::DR)
    }

    /// 中断驱动 RX 的 poll 原语（override）。
    ///
    /// 遵循经典的 ISR/task 竞争修复模式：
    /// 1. 快速路径：环形缓冲区非空 → 立即返回字节
    /// 2. 关中断，注册 waker
    /// 3. **重检**环形缓冲区——若在注册 waker 的间隙 ISR 推入了字节，
    ///    则立即取出并拆回 waker
    /// 4. 开中断，返回 Pending
    ///
    /// 仅 riscv64 可用：依赖 `crate::arch` 关/开中断原语。host 桩经 trait
    /// 默认实现静默降级为 `Unsupported`。
    #[cfg(feature = "riscv64")]
    fn rx_register_waker(&self, cx: &mut core::task::Context<'_>) -> crate::device::SerialRxStatus {
        use crate::device::SerialRxStatus;
        // 快速路径。
        if let Some(byte) = rx_pop() {
            return SerialRxStatus::Ready(byte);
        }

        // 关中断后注册 waker。
        unsafe { crate::arch::disable_interrupts() };
        // SAFETY: 关中断临界区。
        unsafe {
            if RX.has_waker.load(Ordering::Relaxed) {
                (*RX.waker.get()).assume_init_drop();
            }
            (*RX.waker.get()).write(cx.waker().clone());
        }
        RX.has_waker.store(true, Ordering::Release);

        // 重检——ISR 可能在注册 waker 前已推入字节。
        if let Some(byte) = rx_pop() {
            RX.has_waker.store(false, Ordering::Relaxed);
            unsafe {
                (*RX.waker.get()).assume_init_drop();
            }
            unsafe { crate::arch::enable_interrupts() };
            return SerialRxStatus::Ready(byte);
        }

        unsafe { crate::arch::enable_interrupts() };
        SerialRxStatus::Pending
    }
}

// ── Driver trait impl ────────────────────────────────────────────────

impl Driver for Ns16550a {
    fn compatible(&self) -> &'static [&'static str] {
        &["ns16550a"]
    }

    fn probe(&self, node: &Node<'_>) {
        let reg = node
            .reg()
            .expect("ns16550a: missing reg property")
            .next()
            .expect("ns16550a: empty reg");
        let base = reg.address as usize;
        BASE.store(base, Ordering::Release);

        // 使能 FIFO + 清 FIFO + 开 RX 中断。
        let r = regs();
        r.iir_fcr.write(Fcr::ENABLE::SET + Fcr::CLR_RX::SET + Fcr::CLR_TX::SET);
        r.ier.write(Ier::ERBFI::SET);
        // 8N1 模式（DLAB=0 时 LCR 设为 WLEN8）。
        r.lcr.write(Lcr::WLEN8::SET);

        // 复位环形缓冲区索引。
        RX.head.store(0, Ordering::Release);
        RX.tail.store(0, Ordering::Release);

        // 登记进多实例注册表；默认 console 由 boot() 的 try_derive_console
        // 据 chosen.stdout-path 选定（不再由 probe 自命）。
        crate::driver::SERIALS.register(&INSTANCE);
    }
}

// ── 接收环形缓冲区 + Waker 槽 ────────────────────────────────────────

const RX_BUF_SIZE: usize = 256;
const RX_BUF_MASK: u16 = (RX_BUF_SIZE - 1) as u16;

struct RxState {
    /// ISR 写索引。
    head: AtomicU16,
    /// Task 读索引。
    tail: AtomicU16,
    /// 环形缓冲区存储。
    buf: UnsafeCell<[u8; RX_BUF_SIZE]>,
    /// Waker 槽占用标记。
    has_waker: AtomicBool,
    /// Waker 槽。
    waker: UnsafeCell<MaybeUninit<Waker>>,
}

// SAFETY: ISR 仅写入 head + push 字节；task 仅写入 tail + pop 字节。
// Waker 槽由 task 在关中断临界区写入、ISR 在关中断下通过 swap 消费。
// 单 hart 场景下无数据竞争。
unsafe impl Sync for RxState {}

static RX: RxState = RxState {
    head: AtomicU16::new(0),
    tail: AtomicU16::new(0),
    buf: UnsafeCell::new([0; RX_BUF_SIZE]),
    has_waker: AtomicBool::new(false),
    waker: UnsafeCell::new(MaybeUninit::uninit()),
};

// Ns16550a 是零大小单例，impl Sync 满足 registry 的 Send + Sync 约束。
unsafe impl Sync for Ns16550a {}

// ── 公开 RX API ──────────────────────────────────────────────────────

/// ISR 压入字节。环形缓冲区满则静默丢弃。
fn rx_push(byte: u8) {
    let head = RX.head.load(Ordering::Acquire);
    let next = (head.wrapping_add(1)) & RX_BUF_MASK;
    let tail = RX.tail.load(Ordering::Acquire);
    if next == tail {
        // 缓冲区满，丢弃。
        return;
    }
    unsafe {
        let buf = &mut *RX.buf.get();
        buf[head as usize] = byte;
    }
    RX.head.store(next, Ordering::Release);
}

/// Task 从环形缓冲区取字节。
fn rx_pop() -> Option<u8> {
    let tail = RX.tail.load(Ordering::Acquire);
    let head = RX.head.load(Ordering::Acquire);
    if head == tail {
        return None;
    }
    let byte = unsafe {
        let buf = &*RX.buf.get();
        buf[tail as usize]
    };
    RX.tail.store((tail.wrapping_add(1)) & RX_BUF_MASK, Ordering::Release);
    Some(byte)
}

/// ISR 在 RX 完成时调用的唤醒通知。原子消费已注册的 Waker。
fn rx_wake() {
    if RX.has_waker.swap(false, Ordering::AcqRel) {
        // SAFETY: consume 已注册的 Waker。has_waker==true 确保 Waker 槽有效。
        unsafe {
            let waker = (*RX.waker.get()).assume_init_read();
            waker.wake();
        }
    }
}

/// 供 `platform::register_irq` 使用的 RX 中断 handler。
///
/// 从 UART FIFO 中排出所有可用字节，推入环形缓冲区，然后唤醒等待的 task。
/// 板级在 `board_init` 中注册：`platform::register_irq(UART1IRQ, rx_handler)`。
pub fn rx_handler(_irq: u32) {
    while let Some(byte) = INSTANCE.read() {
        rx_push(byte);
    }
    rx_wake();
}
