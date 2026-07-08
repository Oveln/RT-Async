//! CLINT 定时器驱动（mtime / mtimecmp）。
//!
//! 匹配设备树 `compatible = "riscv,clint0"` 的节点（QEMU virt 的 `timer@2000000`）。
//! 从 `reg[0]` 取 CLINT 基址，按 RISC-V CLINT 布局推算：
//! - `mtimecmp`（hart 0）= base + 0x4000
//! - `mtime`         = base + 0xBFF8
//!
//! 时钟频率取自设备树 `/cpus/timebase-frequency`；缺失时回退 10 MHz（QEMU virt 默认）。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;

use crate::device::{Driver, Timer};

/// CLINT `mtimecmp` 相对基址的偏移（hart 0）。
const OFF_MTIMECMP: usize = 0x4000;
/// CLINT `mtime` 相对基址的偏移。
const OFF_MTIME: usize = 0xBFF8;

/// CLINT timer 单例（零大小）。
pub struct ClintTimer;

/// 全局单例。
pub static INSTANCE: ClintTimer = ClintTimer;

/// probe 写入的 CLINT 基址。
static BASE: AtomicUsize = AtomicUsize::new(0);
/// probe 写入的时钟频率（Hz）。0 表示尚未 probe。
static FREQ: AtomicUsize = AtomicUsize::new(0);

impl ClintTimer {
    fn base(&self) -> usize {
        BASE.load(Ordering::Acquire)
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
        let base = self.base();
        // SAFETY: 写 mtimecmp（hart 0）。RISC-V 上应先写高 32 位再写低 32 位避免
        // 伪瞬态触发；QEMU 上单次 64 位写等价，真板可拆分。
        unsafe { core::ptr::write_volatile((base + OFF_MTIMECMP) as *mut u64, tick) };
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
