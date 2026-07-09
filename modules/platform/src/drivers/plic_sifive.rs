//! SiFive PLIC 驱动（RISC-V 标准平台中断控制器）。
//!
//! QEMU `virt` 平台 PLIC（compatible = `riscv,plic0`）。
//! context 从 `mhartid` 派生（每 hart M/S 双 context，故 hart1 M-mode = 2）。
//!
//! 设计说明：
//! - 零大小单例 [`Plic`]，`'static` 生命周期。
//! - probe 得到的基址存全局 [`BASE`]（`AtomicUsize`）。
//! - 所有 MMIO 访问经 `read_volatile`/`write_volatile`；ISR 上下文由调用者
//!   保证关中断单线访问。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;

use crate::device::{Driver, InterruptController};

/// PLIC 驱动单例（零大小）。
pub struct Plic;

/// 全局单例，供 probe 注册进 registry。
pub static PLIC: Plic = Plic;

/// probe 写入的 MMIO 基址。
static BASE: AtomicUsize = AtomicUsize::new(0);

/// probe 计算的 context 索引（每 hart M 模式 = hart_id * 2）。
static CONTEXT: AtomicUsize = AtomicUsize::new(0);

// ── MMIO 偏移（SiFive PLIC 标准布局） ──────────────────────────────

/// priority 区域基址偏移。
const PRIORITY_BASE: usize = 0x0000;
/// enable 区域基址偏移（per context）。
const ENABLE_BASE: usize = 0x2000;
/// threshold 寄存器偏移（per context）。
const THRESHOLD_OFFSET: usize = 0x200000;
/// claim/complete 寄存器偏移（per context，读 = claim，写 = complete）。
const CLAIM_OFFSET: usize = 0x200004;

// ── InterruptController impl ───────────────────────────────────────

impl InterruptController for Plic {
    fn enable_irq(&self, irq: u32) {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let enable_addr = (base + ENABLE_BASE + ctx * 0x80 + (irq as usize / 32) * 4) as *mut u32;
        let bit = 1 << (irq % 32);
        // SAFETY: 读写 MMIO 寄存器。ISR 上下文已关中断。
        unsafe {
            let v = core::ptr::read_volatile(enable_addr);
            core::ptr::write_volatile(enable_addr, v | bit);
        }
    }

    fn disable_irq(&self, irq: u32) {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let enable_addr = (base + ENABLE_BASE + ctx * 0x80 + (irq as usize / 32) * 4) as *mut u32;
        let bit = 1 << (irq % 32);
        unsafe {
            let v = core::ptr::read_volatile(enable_addr);
            core::ptr::write_volatile(enable_addr, v & !bit);
        }
    }

    fn set_priority(&self, irq: u32, prio: u32) {
        let base = BASE.load(Ordering::Acquire);
        let addr = (base + PRIORITY_BASE + irq as usize * 4) as *mut u32;
        unsafe { core::ptr::write_volatile(addr, prio) };
    }

    fn set_threshold(&self, thr: u32) {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let addr = (base + THRESHOLD_OFFSET + ctx * 0x1000) as *mut u32;
        unsafe { core::ptr::write_volatile(addr, thr) };
    }

    fn claim(&self) -> u32 {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let addr = (base + CLAIM_OFFSET + ctx * 0x1000) as *const u32;
        unsafe { core::ptr::read_volatile(addr) }
    }

    fn complete(&self, irq: u32) {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let addr = (base + CLAIM_OFFSET + ctx * 0x1000) as *mut u32;
        unsafe { core::ptr::write_volatile(addr, irq) };
    }
}

// ── Driver impl ─────────────────────────────────────────────────────

impl Driver for Plic {
    fn compatible(&self) -> &'static [&'static str] {
        &["riscv,plic0"]
    }

    fn probe(&self, node: &Node<'_>) {
        let reg = node
            .reg()
            .expect("plic: missing reg property")
            .next()
            .expect("plic: empty reg");
        BASE.store(reg.address as usize, Ordering::Release);

        // hart0 占 M/S 两个 context，hart1 的 M-mode context = hart_id * 2。
        let hart_id: usize = riscv::register::mhartid::read();
        let ctx = hart_id * 2;
        CONTEXT.store(ctx, Ordering::Release);

        log::info!("PLIC probed: base={:#x}, mhartid={}, context={}", reg.address, hart_id, ctx);

        // PLIC 门槛清零（不屏蔽任何中断）。
        let thr_addr =
            (reg.address as usize + THRESHOLD_OFFSET + ctx * 0x1000) as *mut u32;
        unsafe { core::ptr::write_volatile(thr_addr, 0) };

        crate::driver::INTC.set(&PLIC);
    }
}
