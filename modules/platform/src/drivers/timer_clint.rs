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

use crate::device::{Driver, Timer};

/// CLINT `mtimecmp` 相对基址的偏移起点（hart 0）。
const OFF_MTIMECMP_BASE: usize = 0x4000;
/// 每个 hart 的 mtimecmp 寄存器步长（8 字节）。
const MTIMECMP_STRIDE: usize = 8;
/// CLINT `mtime` 相对基址的偏移（所有 hart 共用）。
const OFF_MTIME: usize = 0xBFF8;

/// CLINT timer 单例（零大小）。
pub struct ClintTimer;

/// 全局单例。
pub static INSTANCE: ClintTimer = ClintTimer;

/// probe 写入的 CLINT 基址。
static BASE: AtomicUsize = AtomicUsize::new(0);
/// probe 写入的 mtimecmp 偏移（按 hart id 计算：0x4000 + hart*8）。
/// 0 表示尚未 probe。
static OFF_MTIMECMP: AtomicUsize = AtomicUsize::new(0);
/// probe 写入的时钟频率（Hz）。0 表示尚未 probe。
static FREQ: AtomicUsize = AtomicUsize::new(0);

impl ClintTimer {
    fn base(&self) -> usize {
        BASE.load(Ordering::Acquire)
    }

    fn mtimecmp_off(&self) -> usize {
        OFF_MTIMECMP.load(Ordering::Acquire)
    }
}

impl Timer for ClintTimer {
    fn freq_hz(&self) -> u32 {
        // 0（未 probe）回退 10 MHz，避免上层在 probe 前读取 panic。
        FREQ.load(Ordering::Acquire) as u32
    }

    fn now(&self) -> u64 {
        let base = self.base();
        // SAFETY: 读 64 位 mtime 寄存器。QEMU virt 上 64 位对齐读是原子的。
        unsafe { core::ptr::read_volatile((base + OFF_MTIME) as *const u64) }
    }

    fn set_deadline(&self, tick: u64) {
        let addr = self.base() + self.mtimecmp_off();
        // SAFETY: 写 mtimecmp（本 hart）。RISC-V 上应先写高 32 位再写低 32 位避免
        // 伪瞬态触发；QEMU 上单次 64 位写等价，真板可拆分。
        unsafe { core::ptr::write_volatile(addr as *mut u64, tick) };
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

        crate::driver::set_timer(&INSTANCE);
    }
}

/// 从设备树 `/cpus` 节点读 `timebase-frequency`。
fn read_timebase_freq() -> Option<u32> {
    let fdt = crate::dtb::dt();
    let cpus = fdt.find_nodes("/cpus").next()?;
    let prop = cpus.find_property("timebase-frequency")?;
    Some(prop.u32())
}
