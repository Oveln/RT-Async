//! NS16550A 兼容串口驱动。
//!
//! QEMU `virt` 平台默认 UART（`serial@10000000`，compatible = `ns16550a`）。
//! 本驱动按设备树 `reg[0]` 取 MMIO 基址。
//!
//! 接收侧提供两种模式：
//! - **轮询**：`Serial::read() / has_data()` 直接读 RBR。
//! - **中断驱动**：probe 使能 FIFO 和 ERBFI（RX 中断），通过板级注册
//!   [`rx_handler`] 到 `platform::register_irq`，字节流入内建环形缓冲区。
//!   [`rx_poll`] 遵循 disable→register→recheck→enable 临界区模式
//!   （与 `apps/rt-async-app/src/uart_wait.rs` 一致），可被
//!   `SerialRx` Future 调用。

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use core::task::{Context, Poll, Waker};

use fdt_parser::Node;

use crate::device::{Driver, Serial};

// ── NS16550A 寄存器偏移 ──────────────────────────────────────────────

const RBR_THR: usize = 0x00; // 读: RBR, 写: THR
const IER: usize = 0x01;
const IIR_FCR: usize = 0x02; // 读: IIR, 写: FCR
const LCR: usize = 0x03;
const LSR: usize = 0x05;

// IER 位
const IER_ERBFI: u8 = 1 << 0; // RX 数据可用中断使能

// FCR 位
const FCR_ENABLE: u8 = 1 << 0; // FIFO 使能
const FCR_CLR_RX: u8 = 1 << 1; // 清 RX FIFO
const FCR_CLR_TX: u8 = 1 << 2; // 清 TX FIFO

// LSR 位
const LSR_DR: u8 = 1 << 0;   // 数据就绪
const LSR_THRE: u8 = 1 << 5; // 发送保持寄存器空

// ── 驱动实例 ─────────────────────────────────────────────────────────

/// NS16550A 串口单例（零大小）。
pub struct Ns16550a;

/// 全局单例，供 probe 注册进 registry。
pub static INSTANCE: Ns16550a = Ns16550a;

/// probe 写入的 MMIO 基址。0 表示尚未 probe。
static BASE: AtomicUsize = AtomicUsize::new(0);

// ── Serial trait impl ────────────────────────────────────────────────

impl Serial for Ns16550a {
    fn write(&self, buf: &[u8]) {
        let base = BASE.load(Ordering::Acquire) as *mut u8;
        for &byte in buf {
            // 等发送保持寄存器空（LSR.THRE = 1），防止连续写入时 FIFO 溢出丢字节。
            unsafe {
                while core::ptr::read_volatile(base.add(LSR)) & LSR_THRE == 0 {
                    core::hint::spin_loop();
                }
                core::ptr::write_volatile(base, byte);
            }
        }
    }

    fn read(&self) -> Option<u8> {
        let base = BASE.load(Ordering::Acquire) as *mut u8;
        // SAFETY: 读 LSR 和 RBR 寄存器。
        unsafe {
            let lsr = core::ptr::read_volatile(base.add(LSR));
            if lsr & LSR_DR == 0 {
                return None;
            }
            Some(core::ptr::read_volatile(base.add(RBR_THR)))
        }
    }

    fn has_data(&self) -> bool {
        let base = BASE.load(Ordering::Acquire) as *mut u8;
        unsafe { core::ptr::read_volatile(base.add(LSR)) & LSR_DR != 0 }
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
        let base_ptr = base as *mut u8;
        unsafe {
            core::ptr::write_volatile(base_ptr.add(IIR_FCR), FCR_ENABLE | FCR_CLR_RX | FCR_CLR_TX);
            core::ptr::write_volatile(base_ptr.add(IER), IER_ERBFI);
            // 8N1 模式（DLAB=0 时 LCR 设为 0x03）。
            core::ptr::write_volatile(base_ptr.add(LCR), 0x03);
        }

        // 复位环形缓冲区索引。
        RX.head.store(0, Ordering::Release);
        RX.tail.store(0, Ordering::Release);

        crate::driver::set_console(&INSTANCE);
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

/// `SerialRx` Future 的 poll 原语。
///
/// 遵循经典的 ISR/task 竞争修复模式：
/// 1. 快速路径：环形缓冲区非空 → 立即返回字节
/// 2. 关中断，注册 waker
/// 3. **重检**环形缓冲区——若在注册 waker 的间隙 ISR 推入了字节，
///    则立即取出并拆回 waker
/// 4. 开中断，返回 Pending
#[cfg(feature = "riscv64")]
pub fn rx_poll(cx: &mut Context<'_>) -> Poll<u8> {
    // 快速路径。
    if let Some(byte) = rx_pop() {
        return Poll::Ready(byte);
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
        return Poll::Ready(byte);
    }

    unsafe { crate::arch::enable_interrupts() };
    Poll::Pending
}

/// Non-riscv64 stub: serial RX not available on host.
#[cfg(not(feature = "riscv64"))]
pub fn rx_poll(_cx: &mut Context<'_>) -> Poll<u8> {
    Poll::Pending
}
