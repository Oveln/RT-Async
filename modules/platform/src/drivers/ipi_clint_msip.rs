//! CLINT MSIP 核间中断（IPI）驱动。
//!
//! 匹配设备树 `compatible = "riscv,clint0-msip"` 的节点（QEMU virt 的
//! `ipi@2000000`）。从 `reg[0]` 取 CLINT 基址，MSIP 寄存器位于基址偏移 0
//! （hart 0）。`send` 写 1 触发机器软件中断，`clear` 写 0 清除。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;

use crate::device::{Driver, Ipi};

/// CLINT MSIP（hart 0）相对基址的偏移。
const OFF_MSIP: usize = 0x0;

/// CLINT MSIP 单例（零大小）。
pub struct ClintMsip;

/// 全局单例。
pub static INSTANCE: ClintMsip = ClintMsip;

/// probe 写入的 CLINT 基址。CLINT timer 与 msip 的 reg 地址相同（都指向完整
/// CLINT 区），各自读自己的 reg 节点即可。
static BASE: AtomicUsize = AtomicUsize::new(0);

impl Ipi for ClintMsip {
    unsafe fn send(&self) {
        let base = BASE.load(Ordering::Acquire);
        // SAFETY: 写 MSIP（hart 0）= 1 触发 MSI。调用者保证上下文合适。
        unsafe { core::ptr::write_volatile((base + OFF_MSIP) as *mut u32, 1) };
    }

    unsafe fn clear(&self) {
        let base = BASE.load(Ordering::Acquire);
        // SAFETY: 写 MSIP（hart 0）= 0 清除 pending。ISR 早期调用。
        unsafe { core::ptr::write_volatile((base + OFF_MSIP) as *mut u32, 0) };
    }
}

impl Driver for ClintMsip {
    fn compatible(&self) -> &'static [&'static str] {
        &["riscv,clint0-msip"]
    }

    fn probe(&self, node: &Node<'_>) {
        let reg = node
            .reg()
            .expect("clint msip: missing reg property")
            .next()
            .expect("clint msip: empty reg");
        BASE.store(reg.address as usize, Ordering::Release);
        crate::driver::set_ipi(&INSTANCE);
    }
}
