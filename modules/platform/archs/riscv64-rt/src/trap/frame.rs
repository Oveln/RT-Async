//! RISC-V Trap 上下文帧
//!
//! 定义了在 trap 发生时保存的处理器上下文结构。

/// 上下文占用的字节数
/// TrapFrame 的大小：30 个通用寄存器 + mepc + mstatus = 256 字节
pub const CONTEXT_STACK_SIZE: usize = core::mem::size_of::<TrapFrame>();

/// 中断栈上的 TrapFrame 结构
///
/// 这个结构体与汇编入口点中的寄存器保存/恢复代码紧密对应。
/// 任何修改都需要同步更新汇编代码。
#[derive(Debug, Clone, Copy)]
#[repr(C, align(16))]
pub struct TrapFrame {
    pub ra: usize,
    pub gp: usize,
    pub tp: usize,
    pub t0: usize,
    pub t1: usize,
    pub t2: usize,
    pub s0: usize,
    pub s1: usize,
    pub a0: usize,
    pub a1: usize,
    pub a2: usize,
    pub a3: usize,
    pub a4: usize,
    pub a5: usize,
    pub a6: usize,
    pub a7: usize,
    pub s2: usize,
    pub s3: usize,
    pub s4: usize,
    pub s5: usize,
    pub s6: usize,
    pub s7: usize,
    pub s8: usize,
    pub s9: usize,
    pub s10: usize,
    pub s11: usize,
    pub t3: usize,
    pub t4: usize,
    pub t5: usize,
    pub t6: usize,
    pub mepc: usize,
    pub mstatus: usize,
}
