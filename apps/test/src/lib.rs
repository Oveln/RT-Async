//! 集成测试公共工具：事件记录、断言、QEMU 失败信号。

#![no_std]

use core::sync::atomic::{AtomicUsize, Ordering};

const LOG_CAP: usize = 32;

static mut LOG: [&'static str; LOG_CAP] = [""; LOG_CAP];
static LOG_LEN: AtomicUsize = AtomicUsize::new(0);

/// 记录测试事件到执行日志。
pub unsafe fn record(s: &'static str) {
    let idx = LOG_LEN.fetch_add(1, Ordering::Relaxed);
    if idx < LOG_CAP {
        unsafe { LOG[idx] = s; }
    }
}

/// 验证执行日志与预期顺序一致，不匹配时 QEMU 以 exit code 1 退出。
pub fn assert_log(expected: &[&'static str]) {
    let len = LOG_LEN.load(Ordering::Acquire);
    if len != expected.len() {
        fail("log length mismatch");
    }
    for i in 0..expected.len() {
        if unsafe { LOG[i] } != expected[i] {
            fail("log order mismatch");
        }
    }
}

/// SiFive Test FINISHER_FAIL (QEMU exit code 1)。
pub fn fail(msg: &str) -> ! {
    log::error!("{}", msg);
    unsafe {
        core::ptr::write_volatile(0x100_000 as *mut u32, 0x3333 | ((1) << 16));
    }
    loop {}
}
