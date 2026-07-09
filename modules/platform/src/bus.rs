//! Bus registry：多实例总线槽位 + DFS 遍历期间的活跃 bus 栈。
//!
//! controller driver（i2c/spi）的 probe 把实例注册进 `I2C_BUSES`/`SPI_BUSES`，
//! 返回的索引压入对应 bus 栈；child device 的 probe 经 [`current_i2c`]/
//! [`current_spi`] 取栈顶（即最近一个进入的 controller）收发。
//!
//! [`crate::driver::boot`] DFS 遍历设备树时维护栈：进入 controller 子树前
//! controller 先被 probe 并 push，离开子树时按 [`Node::level`] pop。这保证
//! child probe 时 `current_*()` 指向其父 bus。
//!
//! # 设计
//! bus 栈存的是 [`DeviceRegistry::register`] 返回的索引（`usize`），不是
//! `Node`（`Node` 无取 bus 实例的方法）。活跃 bus 栈用 `UnsafeCell<BusStack>`
//! 承载（与 [`crate::driver::Slot`] 同安全模型），单 hart 串行 boot 下无并发。

use core::cell::UnsafeCell;

use heapless::Vec;

use crate::device::{I2cBus, SpiBus};
use crate::driver::DeviceRegistry;

/// I2C bus 注册表（容量 4）。driver 的 probe 经 [`DeviceRegistry::register`]
/// 登记，child device 经 [`current_i2c`] 取栈顶收发。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub static I2C_BUSES: DeviceRegistry<&'static dyn I2cBus, 4> = DeviceRegistry::new();

/// SPI bus 注册表（容量 4）。driver 的 probe 经 [`DeviceRegistry::register`]
/// 登记，child device 经 [`current_spi`] 取栈顶收发。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub static SPI_BUSES: DeviceRegistry<&'static dyn SpiBus, 4> = DeviceRegistry::new();

/// DFS 遍历期间的活跃 bus 栈（存 bus 索引）。
///
/// 单 hart 串行 boot 下无并发；用 `heapless::Vec` 提供安全的有界 push/pop。
struct BusStack {
    i2c: Vec<usize, 8>,
    spi: Vec<usize, 8>,
}

/// `UnsafeCell` 承载 `BusStack`，提供 `&self` → 内部可变访问（同 [`Slot`] 模型）。
///
/// `Vec::new()` 是 const fn，故整个 `UnsafeCell::new(BusStack { .. })` 可在
/// const 上下文构造。
struct BusStackCell(UnsafeCell<BusStack>);

// SAFETY: BUS_STACK 仅在 boot() 单 hart 串行路径上经下面的封装函数访问，无并发。
unsafe impl Sync for BusStackCell {}

static BUS_STACK: BusStackCell = BusStackCell(UnsafeCell::new(BusStack {
    i2c: Vec::new(),
    spi: Vec::new(),
}));

impl BusStackCell {
    /// 取内部 `BusStack` 的可变引用。仅在单 hart 串行 boot 路径调用。
    fn get_mut(&self) -> &mut BusStack {
        // SAFETY: 仅 boot() 单 hart 串行路径经封装函数访问，无并发。
        unsafe { &mut *self.0.get() }
    }
}

/// 重置 bus 栈。boot() 开始时调用。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub fn bus_stack_reset() {
    let s = BUS_STACK.get_mut();
    s.i2c.clear();
    s.spi.clear();
}

/// 离开深度 ≤ level 的子树时，弹出所有 level 更深的 bus 索引。
///
/// boot() 在进入每个 node 前调用：若 node.level < prev_level，说明离开了
/// 之前的 controller 子树，把栈顶 level 更深的 bus 弹出。
///
/// 当前简化实现：bus 栈只存索引、不逐 bus 记 level，故按「level 回退到根
/// （≤1）则全清」处理。这对「i2c/spi controller 直接挂在总线根下、child 同层
/// 或更深」的单层场景正确；多层嵌套 controller 需给每个 bus 索引配 level，
/// 留作未来增强。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub fn bus_stack_pop_to(level: usize) {
    if level <= 1 {
        let s = BUS_STACK.get_mut();
        s.i2c.clear();
        s.spi.clear();
    }
}

/// controller probe 注册 bus 后调用，压入其索引。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub fn push_i2c(idx: usize) {
    // 满（>8 层嵌套）忽略——bare-metal 不会发生。
    let _ = BUS_STACK.get_mut().i2c.push(idx);
}

/// 取当前活跃 I2C bus（栈顶）。child device probe 调用。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub fn current_i2c() -> Option<&'static dyn I2cBus> {
    let idx = *BUS_STACK.get_mut().i2c.last()?;
    I2C_BUSES.get(idx).copied()
}

/// controller probe 注册 bus 后调用，压入其索引。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub fn push_spi(idx: usize) {
    let _ = BUS_STACK.get_mut().spi.push(idx);
}

/// 取当前活跃 SPI bus（栈顶）。child device probe 调用。
// 消费方：未来 i2c/spi controller + child driver。
#[allow(dead_code)]
pub fn current_spi() -> Option<&'static dyn SpiBus> {
    let idx = *BUS_STACK.get_mut().spi.last()?;
    SPI_BUSES.get(idx).copied()
}
