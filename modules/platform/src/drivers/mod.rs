//! rt-async 内置 driver 集合。
//!
//! 每个子模块一个具体驱动，均为零大小单例 + 全局 `AtomicUsize` 存 probe 来的
//! MMIO 基址。[`default_drivers`] 汇总所有内置 driver 单例为 `&'static` 切片，
//! 供板级 `board_init` 经 [`crate::driver::set_drivers`] 注入后由
//! [`crate::driver::boot`] 按 DT 实例化。
//!
//! 加新驱动：实现子模块 → 在本文件 `pub mod` + `pub use` → 加入
//! [`default_drivers`] 的 `DEFAULT` 数组即可。

pub mod ipi_clint_msip;
pub mod reset_sifive_test;
pub mod serial_ns16550a;
pub mod timer_clint;

pub use ipi_clint_msip::ClintMsip;
pub use reset_sifive_test::SifiveTest;
pub use serial_ns16550a::Ns16550a;
pub use timer_clint::ClintTimer;

/// rt-async 内置 driver 默认列表。
///
/// 返回 `'static` 切片，板级 `board_init` 可直接传给
/// [`crate::driver::set_drivers`]。需要替换某驱动时，板级可自行组装数组覆盖。
///
/// [`crate::driver::set_drivers`]: crate::driver::set_drivers
pub fn default_drivers() -> &'static [&'static dyn crate::Driver] {
    &DEFAULT
}

/// 内置 driver 单例列表。`static` 保证 `'static` 生命周期。
static DEFAULT: &[&dyn crate::Driver] = &[
    &serial_ns16550a::INSTANCE,
    &timer_clint::INSTANCE,
    &ipi_clint_msip::INSTANCE,
    &reset_sifive_test::INSTANCE,
];
