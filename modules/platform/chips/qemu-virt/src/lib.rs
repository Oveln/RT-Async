//! # QEMU Virt Chip 实现（转发 shim）
//!
//! 为 QEMU `virt` 平台（RISC-V 64）提供 [`Chip`] 和 [`TimerChip`] 的实现。
//!
//! 本 crate 已不再硬编码 MMIO 逻辑——具体外设驱动（NS16550A / CLINT timer /
//! CLINT msip / sifive test）抽到 `platform::drivers` 内部模块，由设备树 probe
//! 实例化。这里仅保留 `extern_trait` 静态分发入口，方法体转发到
//! `platform::driver` registry（console / timer / ipi / reset），保持上层
//! （executor / futures / apps）调用 `ChipImpl::*` / `TimerChipImpl::*` 零改动。
//!
//! [`Chip`] / [`TimerChip`] trait 定义在 `platform::lib`，不在此改动。

#![no_std]
#![allow(unreachable_code)]

use extern_trait::extern_trait;
use platform::{Chip, TimerChip};

/// QEMU virt 平台的 Chip 实现（转发到 driver registry）。
pub struct QemuVirt;

#[extern_trait]
impl Chip for QemuVirt {
    fn board_init() {
        // 1. 注入 rt-async 专属 DTB（子模块自包含模式）。
        //    路径：src -> qemu-virt -> chips -> platform -> modules -> rt-async 根（5 级 ../）。
        static RT_ASYNC_DTB: &[u8] =
            include_bytes!("../../../../../its/rt-async-qemu-virt.dtb");
        platform::dtb::init_dtb(RT_ASYNC_DTB);

        // 2. 注册板级 driver 列表（用 platform 内置默认列表）。
        //    未来 K3 等板可在此替换为自定义 driver 列表。
        let drivers = platform::drivers::default_drivers();
        platform::driver::set_drivers(drivers);

        // 3. 遍历 DT 实例化 driver（probe 各节点 → 填充 registry 槽位）。
        platform::driver::boot();
    }

    fn shutdown() -> ! {
        platform::driver::reset().shutdown()
    }

    fn put_str(s: &str) {
        platform::driver::console().write(s.as_bytes());
    }

    unsafe fn pend() {
        // SAFETY: 调用者（platform::pend）保证上下文合适。
        unsafe { platform::driver::ipi().send() };
    }

    unsafe fn clear_pend() {
        // SAFETY: ISR 早期调用，关中断上下文。
        unsafe { platform::driver::ipi().clear() };
    }
}

#[extern_trait]
impl TimerChip for QemuVirt {
    fn freq_hz() -> u32 {
        platform::driver::timer().freq_hz()
    }

    fn now_ticks() -> u64 {
        platform::driver::timer().now()
    }

    fn set_deadline(tick: u64) {
        platform::driver::timer().set_deadline(tick)
    }

    unsafe fn enable_timer_irq() {
        // 先把 deadline 推到最远，避免立刻触发；再开 mie.MTIE。
        // MTIE 属 arch 级配置，不属于 driver model，保留在此。
        Self::set_deadline(u64::MAX);
        unsafe { riscv::register::mie::set_mtimer() };
    }
}
