//! 集成测试：验证 QEMU virt 平台启动与关机
//!
//! 通过 `cargo run --bin smoke --features qemu-virt` 运行。
//! 成功时 QEMU 以 exit code 0 退出（SiFive Test FINISHER_PASS）。

#![no_std]
#![no_main]

#[cfg(feature = "qemu-virt")]
extern crate qemu_virt;

use platform::Chip;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn __rust_main() -> ! {
    platform::init();
    log::info!("test/smoke: boot OK, shutting down");
    platform::ChipImpl::shutdown()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn MachineSoft(_trap_frame: &mut platform::arch::TrapFrame) {
    // smoke 测试不使用中断，不应到达此处
}
