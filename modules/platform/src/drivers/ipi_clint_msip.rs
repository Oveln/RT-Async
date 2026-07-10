//! CLINT MSIP 核间中断（IPI）驱动。
//!
//! 匹配设备树 `compatible = "riscv,clint0-msip"` 的节点（QEMU virt 的
//! `ipi@2000000`）。从 `reg[0]` 取 CLINT 基址，MSIP 寄存器按 hart 编号排列：
//! - `msip`(hart N) = base + N*4
//!
//! hart id 取自 FDT header 的 `boot_cpuid_phys`（dtc 从 `/cpus` 节点推导）。
//! 这样同一份 driver 既能服务子模块 hart0（msip 在 base+0），也能服务
//! 主仓库 rt-async-amp 的 hart1（msip 在 base+4），无需硬编码偏移。
//!
//! `send` 写 1 触发机器软件中断，`clear` 写 0 清除。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;
use tock_registers::interfaces::Writeable;
use tock_registers::registers::ReadWrite;
use tock_registers::register_structs;

use crate::device::{Driver, Ipi};

/// 每个 hart 的 MSIP 寄存器步长（4 字节）。
const MSIP_STRIDE: usize = 4;

register_structs! {
    /// CLINT MSIP 寄存器（单 u32：写 1 触发 MSI，写 0 清除）。
    pub MsipReg {
        (0x00 => msip: ReadWrite<u32>),
        (0x04 => @END),
    }
}

/// CLINT MSIP 单例（零大小）。
pub struct ClintMsip;

/// 全局单例。
pub static INSTANCE: ClintMsip = ClintMsip;

/// probe 写入的 CLINT 基址。
static BASE: AtomicUsize = AtomicUsize::new(0);
/// probe 写入的 MSIP 偏移（按 hart id 计算：hart*4）。
/// 注意 hart0 的 MSIP 也在偏移 0，故用单独的 ready 标志区分"未 probe"与"hart0"。
static OFF_MSIP: AtomicUsize = AtomicUsize::new(0);
static READY: AtomicUsize = AtomicUsize::new(0);

/// 返回本 hart MSIP 寄存器引用。probe 前调用返回 None（静默跳过）。
fn regs() -> Option<&'static MsipReg> {
    if READY.load(Ordering::Acquire) == 0 {
        return None;
    }
    let addr = BASE.load(Ordering::Acquire) + OFF_MSIP.load(Ordering::Acquire);
    // SAFETY: addr 来自 probe 写入的 DT reg + hart 偏移，指向已验证的 MMIO
    // 区域。单 hart 串行访问，无别名引用（tock-registers 内部用 volatile）。
    Some(unsafe { &*(addr as *const MsipReg) })
}

impl Ipi for ClintMsip {
    unsafe fn send(&self) {
        if let Some(r) = regs() {
            // 写 MSIP（本 hart）= 1 触发 MSI。
            r.msip.set(1);
        }
    }

    unsafe fn clear(&self) {
        if let Some(r) = regs() {
            // 写 MSIP（本 hart）= 0 清除 pending。
            r.msip.set(0);
        }
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

        // hart id 取自 FDT header 的 boot_cpuid_phys（dtc 推导自 /cpus）。
        // msip = base + hart*4。
        let hart = node.fdt().boot_cpuid_phys() as usize;
        OFF_MSIP.store(hart * MSIP_STRIDE, Ordering::Release);
        READY.store(1, Ordering::Release);

        crate::driver::IPI.set(&INSTANCE);
    }
}
