//! SiFive PLIC 驱动（RISC-V 标准平台中断控制器）。
//!
//! QEMU `virt` 平台 PLIC（compatible = `riscv,plic0`）。
//! context 从 `mhartid` 派生（每 hart M/S 双 context，故 hart1 M-mode = 2）。
//!
//! 设计说明：
//! - 零大小单例 [`Plic`]，`'static` 生命周期。
//! - probe 得到的基址存全局 [`BASE`]（`AtomicUsize`）。
//! - 所有 MMIO 访问经 tock-registers 的 volatile 方法（`.get()`/`.set()`），
//!   不手写 `read_volatile`/`write_volatile`。借鉴 tgoskits
//!   的 `ContextLocal` 思路，但不硬编码巨型 PLICRegs 布局。
//! - ISR 上下文由调用者保证关中断单线访问。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;
use tock_registers::interfaces::{Readable, Writeable};
use tock_registers::registers::ReadWrite;
use tock_registers::register_structs;

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

const PRIORITY_BASE: usize = 0x0000;
const ENABLE_BASE: usize = 0x2000;
/// threshold 起点；claim/complete 在 +4（由 PlicContext 结构布局表达，
/// 故不单独定义 CLAIM_OFFSET 常量）。
const THRESHOLD_OFFSET: usize = 0x200000;

register_structs! {
    /// per-context 寄存器组：threshold（偏移 0）+ claim/complete（偏移 4）。
    /// 二者同在一个 0x1000 窗口内，构造时地址指向窗口起点（threshold 处）。
    pub PlicContext {
        (0x00 => threshold: ReadWrite<u32>),
        (0x04 => claim_complete: ReadWrite<u32>),
        (0x08 => @END),
    }
}

register_structs! {
    /// PLIC priority 寄存器（单 u32，散落在 base + irq*4）。
    pub PlicPriority {
        (0x00 => priority: ReadWrite<u32>),
        (0x04 => @END),
    }
}

register_structs! {
    /// PLIC enable 寄存器（单 u32，散落在 base + 0x2000 + ctx*0x80 + word*4）。
    pub PlicEnable {
        (0x00 => enable: ReadWrite<u32>),
        (0x04 => @END),
    }
}

impl Plic {
    /// per-context 寄存器组引用。地址 = base + THRESHOLD_OFFSET + ctx*0x1000。
    fn ctx_regs(&self) -> &'static PlicContext {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let addr = base + THRESHOLD_OFFSET + ctx * 0x1000;
        // SAFETY: addr 来自 probe 写入的 DT reg + 固定偏移计算，指向已验证的
        // MMIO 区域。单 hart 串行访问，无别名引用（tock-registers 内部用 volatile）。
        unsafe { &*(addr as *const PlicContext) }
    }

    /// 指定中断源的 priority 寄存器引用。地址 = base + irq*4。
    fn priority_regs(&self, irq: u32) -> &'static PlicPriority {
        let base = BASE.load(Ordering::Acquire);
        let addr = base + PRIORITY_BASE + irq as usize * 4;
        // SAFETY: 同上。
        unsafe { &*(addr as *const PlicPriority) }
    }

    /// 指定中断源的 enable 寄存器引用。地址 = base + 0x2000 + ctx*0x80 + (irq/32)*4。
    fn enable_regs(&self, irq: u32) -> &'static PlicEnable {
        let base = BASE.load(Ordering::Acquire);
        let ctx = CONTEXT.load(Ordering::Acquire);
        let addr = base + ENABLE_BASE + ctx * 0x80 + (irq as usize / 32) * 4;
        // SAFETY: 同上。
        unsafe { &*(addr as *const PlicEnable) }
    }
}

impl InterruptController for Plic {
    fn enable_irq(&self, irq: u32) {
        let bit = 1u32 << (irq % 32);
        // enable 寄存器无位域定义（与 tgoskits 一致，IRQ 位语义由 irq 参数表达），
        // 故用手动 get+set RMW（关中断上下文单线程串行，安全）。
        let r = self.enable_regs(irq);
        r.enable.set(r.enable.get() | bit);
    }

    fn disable_irq(&self, irq: u32) {
        let bit = 1u32 << (irq % 32);
        let r = self.enable_regs(irq);
        r.enable.set(r.enable.get() & !bit);
    }

    fn set_priority(&self, irq: u32, prio: u32) {
        self.priority_regs(irq).priority.set(prio);
    }

    fn set_threshold(&self, thr: u32) {
        self.ctx_regs().threshold.set(thr);
    }

    fn claim(&self) -> u32 {
        self.ctx_regs().claim_complete.get()
    }

    fn complete(&self, irq: u32) {
        self.ctx_regs().claim_complete.set(irq);
    }
}

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
        self.ctx_regs().threshold.set(0);

        crate::driver::INTC.set(&PLIC);
    }
}
