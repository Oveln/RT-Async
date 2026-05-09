//! # Platform Traits
//!
//! Chip 抽象 trait 定义。

#![no_std]

/// Chip 平台抽象。
///
/// 每个 SoC / board 提供此 trait 的具体实现。
pub trait Chip {
    /// 关机（成功退出）。
    fn shutdown() -> !;

    /// 通过串口输出字符串。
    fn put_str(s: &str);

    /// 触发调度器软件中断。
    unsafe fn pend();

    /// 清除调度器软件中断挂起标志。
    unsafe fn clear_pend();
}
