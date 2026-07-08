//! SiFive Test 复位/关机驱动。
//!
//! 匹配设备树 `compatible = "sifive,test1"` 的节点（QEMU virt 的
//! `reset@100000`）。从 `reg[0]` 取基址，`shutdown` 写 `0x5555` 触发 QEMU
//! 退出（FINISHER_PASS）。

use core::sync::atomic::{AtomicUsize, Ordering};

use fdt_parser::Node;

use crate::device::{Driver, Reset};

/// `sifive,test` finsh pass 退出码（QEMU 据此干净退出）。
const FINISHER_PASS: u32 = 0x5555;

/// SiFive Test 单例（零大小）。
pub struct SifiveTest;

/// 全局单例。
pub static INSTANCE: SifiveTest = SifiveTest;

/// probe 写入的 finsh 寄存器基址。
static BASE: AtomicUsize = AtomicUsize::new(0);

impl Reset for SifiveTest {
    fn shutdown(&self) -> ! {
        let base = BASE.load(Ordering::Acquire);
        // SAFETY: 写 finsh 寄存器触发 QEMU 退出。永不返回。
        unsafe { core::ptr::write_volatile(base as *mut u32, FINISHER_PASS) };
        loop {
            // 防优化空循环；非 QEMU 环境不会退出，原地暂停省电。
            core::hint::spin_loop();
        }
    }
}

impl Driver for SifiveTest {
    fn compatible(&self) -> &'static [&'static str] {
        &["sifive,test1"]
    }

    fn probe(&self, node: &Node<'_>) {
        let reg = node
            .reg()
            .expect("sifive test: missing reg property")
            .next()
            .expect("sifive test: empty reg");
        BASE.store(reg.address as usize, Ordering::Release);
        crate::driver::set_reset(&INSTANCE);
    }
}
