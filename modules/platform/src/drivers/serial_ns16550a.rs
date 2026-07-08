//! NS16550A 兼容串口驱动。
//!
//! QEMU `virt` 平台默认 UART（`serial@10000000`，compatible = `ns16550a`）。
//! 本驱动按设备树 `reg[0]` 取 MMIO 基址，每个字节直接写 `THR`（发送保持寄存器）。
//!
//! 设计说明：
//! - 实例是零大小单例 [`INSTANCE`]，`'static` 生命周期便于注册进 registry。
//! - probe 得到的基址存全局 [`BASE`]（`AtomicUsize`），`Serial::write` 时读取。
//!   单 hart 串行 probe 场景下无竞争。
//! - 不做 LSR 忙等：QEMU 的 16550A 模型即写即收，主机侧无背压；真板上需要补
//!   `while (LSR & THRE) == 0 {}` 轮询（届时可加 `wait_tx_ready` 配置项）。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;

use crate::device::{Driver, Serial};

/// NS16550A 串口单例（零大小）。
pub struct Ns16550a;

/// 全局单例，供 probe 注册进 registry。
pub static INSTANCE: Ns16550a = Ns16550a;

/// probe 写入的 MMIO 基址。0 表示尚未 probe。
static BASE: AtomicUsize = AtomicUsize::new(0);

impl Serial for Ns16550a {
    fn write(&self, buf: &[u8]) {
        let base = BASE.load(Ordering::Acquire) as *mut u8;
        if base.is_null() {
            // console 在 probe 完成前不应被调用；静默丢弃避免 panic 把日志通路打死。
            return;
        }
        for &byte in buf {
            // SAFETY: 写 THR 寄存器（基址偏移 0）。QEMU 即写即收。
            unsafe { core::ptr::write_volatile(base, byte) };
        }
    }
}

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
        BASE.store(reg.address as usize, Ordering::Release);
        crate::driver::set_console(&INSTANCE);
    }
}
