//! # QEMU Virt Chip 实现（子仓库自包含模式）
//!
//! 为 QEMU `virt` 平台（RISC-V 64）提供 [`Board`] 的实现。
//! 负责 DTB 注入、driver 列表注册和 DT 遍历实例化。
//!
//! 子仓库的 demo / test 均为 TX 单测，不依赖外部中断，故此处不注册
//! UART RX IRQ handler。中断驱动 RX 由主仓库 chip crate 配置。

#![no_std]
#![allow(unreachable_code)]

use extern_trait::extern_trait;
use platform::Board;

/// QEMU virt 平台的板级实现。
pub struct QemuVirt;

#[extern_trait]
impl Board for QemuVirt {
    fn init() {
        // 1. 注入 rt-async 专属 DTB（内嵌模式）。
        //    .dtb 由 build.rs 在编译期用 dtc 从 .dts 生成到 OUT_DIR，路径经
        //    cargo:rustc-env=QEMU_VIRT_DTB_PATH 传入（不再追踪 .dtb 产物）。
        static RT_ASYNC_DTB: &[u8] = include_bytes!(env!("QEMU_VIRT_DTB_PATH"));
        platform::dtb::init_dtb(RT_ASYNC_DTB);

        // 2. 注册板级 driver 列表（platform 内置默认列表）。
        let drivers = platform::drivers::default_drivers();
        platform::driver::DRIVERS.set(drivers);

        // 3. 遍历 DT 实例化 driver（probe 各节点 → 填充 registry 槽位）。
        platform::driver::boot();
    }
}
