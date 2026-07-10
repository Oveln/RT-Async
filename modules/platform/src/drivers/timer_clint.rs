//! CLINT 定时器驱动（mtime / mtimecmp）。
//!
//! 匹配设备树 `compatible = "riscv,clint0"` 的节点（QEMU virt 的 `timer@2000000`）。
//! 从 `reg[0]` 取 CLINT 基址，按 RISC-V CLINT 布局推算：
//! - `mtimecmp`(hart N) = base + 0x4000 + N*8
//! - `mtime`           = base + 0xBFF8（所有 hart 共用）
//!
//! hart id 取自 FDT header 的 `boot_cpuid_phys`（dtc 从 `/cpus` 节点推导）。
//! 这样同一份 driver 既能服务子模块 hart0（boot_cpuid_phys=0），也能服务
//! 主仓库 rt-async-amp 的 hart1（boot_cpuid_phys=1），无需硬编码偏移。
//!
//! 时钟频率取自设备树 `/cpus/timebase-frequency`；缺失时回退 10 MHz（QEMU virt 默认）。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;
use tock_registers::interfaces::{Readable, Writeable};
use tock_registers::registers::{ReadOnly, ReadWrite};
use tock_registers::register_structs;

use crate::device::{Driver, Timer};

/// CLINT `mtimecmp` 相对基址的偏移起点（hart 0）。
const OFF_MTIMECMP_BASE: usize = 0x4000;
/// 每个 hart 的 mtimecmp 寄存器步长（8 字节）。
const MTIMECMP_STRIDE: usize = 8;
/// CLINT `mtime` 相对基址的偏移（所有 hart 共用）。
const OFF_MTIME: usize = 0xBFF8;

register_structs! {
    /// CLINT mtimecmp 寄存器，拆成 hi/lo 两个 32 位字段。
    ///
    /// RISC-V 规范要求写 mtimecmp 应先写高 32 位再写低 32 位，避免中间出现
    /// 很小的临时值伪触发定时器中断。
    pub ClintMtimecmp {
        (0x00 => lo: ReadWrite<u32>),
        (0x04 => hi: ReadWrite<u32>),
        (0x08 => @END),
    }
}

register_structs! {
    /// CLINT mtime 寄存器（64 位全局单调计数器）。
    pub ClintMtime {
        (0x00 => count: ReadOnly<u64>),
        (0x08 => @END),
    }
}

/// CLINT timer 单例（零大小）。
pub struct ClintTimer;

/// 全局单例。
pub static INSTANCE: ClintTimer = ClintTimer;

/// probe 写入的 CLINT 基址。
static BASE: AtomicUsize = AtomicUsize::new(0);
/// probe 写入的 mtimecmp 偏移（按 hart id 计算：0x4000 + hart*8）。
static OFF_MTIMECMP: AtomicUsize = AtomicUsize::new(0);
/// probe 写入的时钟频率（Hz）。
static FREQ: AtomicUsize = AtomicUsize::new(0);

impl ClintTimer {
    fn base(&self) -> usize {
        BASE.load(Ordering::Acquire)
    }

    fn mtimecmp_off(&self) -> usize {
        OFF_MTIMECMP.load(Ordering::Acquire)
    }

    /// 返回 mtime 寄存器引用。
    fn mtime_regs(&self) -> &'static ClintMtime {
        let addr = self.base() + OFF_MTIME;
        // SAFETY: addr 来自 probe 写入的 DT reg + 固定偏移，指向已验证的 MMIO 区域。
        // 单 hart 串行访问，无别名引用（tock-registers 内部用 volatile）。
        unsafe { &*(addr as *const ClintMtime) }
    }

    /// 返回本 hart mtimecmp 寄存器引用。
    fn mtimecmp_regs(&self) -> &'static ClintMtimecmp {
        let addr = self.base() + self.mtimecmp_off();
        // SAFETY: 同上。
        unsafe { &*(addr as *const ClintMtimecmp) }
    }
}

impl Timer for ClintTimer {
    fn freq_hz(&self) -> u32 {
        // probe 完成后为真实 timebase-frequency（从 DT /cpus/timebase-frequency 读）。
        FREQ.load(Ordering::Acquire) as u32
    }

    fn now(&self) -> u64 {
        // 读 64 位 mtime（QEMU virt 上 64 位对齐读是原子的）。
        self.mtime_regs().count.get()
    }

    fn set_deadline(&self, tick: u64) {
        let cmp = self.mtimecmp_regs();
        // RISC-V 真板写 mtimecmp 应先写高 32 位再写低 32 位，避免中间出现一个
        // 很小的临时值伪触发定时器中断。
        cmp.hi.set((tick >> 32) as u32);
        cmp.lo.set(tick as u32);
    }
}

impl Driver for ClintTimer {
    fn compatible(&self) -> &'static [&'static str] {
        &["riscv,clint0"]
    }

    fn probe(&self, node: &Node<'_>) {
        let reg = node
            .reg()
            .expect("clint timer: missing reg property")
            .next()
            .expect("clint timer: empty reg");
        BASE.store(reg.address as usize, Ordering::Release);

        // hart id 取自 FDT header 的 boot_cpuid_phys（dtc 推导自 /cpus）。
        // mtimecmp = base + 0x4000 + hart*8。
        let hart = node.fdt().boot_cpuid_phys() as usize;
        let mtimecmp_off = OFF_MTIMECMP_BASE + hart * MTIMECMP_STRIDE;
        OFF_MTIMECMP.store(mtimecmp_off, Ordering::Release);

        // 时钟频率取自 /cpus/timebase-frequency。
        let freq = read_timebase_freq().unwrap_or(10_000_000);
        FREQ.store(freq as usize, Ordering::Release);

        crate::driver::TIMER.set(&INSTANCE);
    }
}

/// 从设备树 `/cpus` 节点读 `timebase-frequency`。
fn read_timebase_freq() -> Option<u32> {
    let fdt = crate::dtb::dt();
    let cpus = fdt.find_nodes("/cpus").next()?;
    let prop = cpus.find_property("timebase-frequency")?;
    Some(prop.u32())
}
