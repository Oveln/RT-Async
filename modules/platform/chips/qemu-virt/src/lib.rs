//! # QEMU Virt Chip 实现
//!
//! 为 QEMU `virt` 平台（RISC-V 64）提供 [`Chip`] trait 的具体实现。

#![no_std]

use platform_traits::Chip;

/// QEMU virt 串口寄存器基址（NS16550A 兼容 UART）。
const UART_BASE: usize = 0x1000_0000;
/// QEMU virt 关机寄存器基址（SiFive Test 设备）。
const SIFIVE_TEST_BASE: usize = 0x100_000;

/// QEMU virt 平台的 Chip 实现。
pub struct QemuVirt;

impl Chip for QemuVirt {
    fn shutdown() -> ! {
        // 向 SiFive Test 设备写入 0x5555 触发 QEMU 正常退出
        unsafe {
            core::ptr::write_volatile(SIFIVE_TEST_BASE as *mut u32, 0x5555);
        }
        loop {}
    }

    fn put_str(s: &str) {
        for &byte in s.as_bytes() {
            unsafe {
                // THR（发送保持寄存器）偏移为 0
                core::ptr::write_volatile(UART_BASE as *mut u8, byte);
            }
        }
    }
}
