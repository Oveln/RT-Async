//! # QEMU Virt Chip 实现
//!
//! 为 QEMU `virt` 平台（RISC-V 64）提供 [`Chip`] 和 [`TimerChip`] 的具体实现。

#![no_std]

use platform_traits::{Chip, timer::TimerChip};

/// QEMU virt 串口寄存器基址（NS16550A 兼容 UART）。
const UART_BASE: usize = 0x1000_0000;
/// QEMU virt 关机寄存器基址（SiFive Test 设备）。
const SIFIVE_TEST_BASE: usize = 0x100_000;
/// CLINT msip 寄存器（hart 0）。
const CLINT_MSIP: usize = 0x2000_000;
/// CLINT mtimecmp 寄存器（hart 0）。
const CLINT_MTIMECMP: usize = 0x200_4000;
/// CLINT mtime 寄存器。
const CLINT_MTIME: usize = 0x200_BFF8;

/// QEMU virt 平台的 Chip 实现。
pub struct QemuVirt;

impl Chip for QemuVirt {
    fn shutdown() -> ! {
        unsafe {
            core::ptr::write_volatile(SIFIVE_TEST_BASE as *mut u32, 0x5555);
        }
        loop {}
    }

    fn put_str(s: &str) {
        for &byte in s.as_bytes() {
            unsafe {
                core::ptr::write_volatile(UART_BASE as *mut u8, byte);
            }
        }
    }

    unsafe fn pend() {
        unsafe { core::ptr::write_volatile(CLINT_MSIP as *mut u32, 1) };
    }

    unsafe fn clear_pend() {
        unsafe { core::ptr::write_volatile(CLINT_MSIP as *mut u32, 0) };
    }
}

/// QEMU virt 定时器频率：10 MHz。
const QEMU_VIRT_FREQ_HZ: u32 = 10_000_000;

impl TimerChip for QemuVirt {
    const FREQ_HZ: u32 = QEMU_VIRT_FREQ_HZ;

    fn now_ticks() -> u64 {
        unsafe { core::ptr::read_volatile(CLINT_MTIME as *const u64) }
    }

    fn set_deadline(tick: u64) {
        unsafe { core::ptr::write_volatile(CLINT_MTIMECMP as *mut u64, tick) };
    }

    unsafe fn enable_timer_irq() {
        Self::set_deadline(u64::MAX);
        unsafe { riscv::register::mie::set_mtimer() };
    }
}
