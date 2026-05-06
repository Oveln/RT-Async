//! RISC-V 中断分发
//!
//! 中断处理器通过链接脚本弱符号定义，平台可提供同名 `#[no_mangle]` 函数覆盖。

use crate::TrapFrame;
use riscv::interrupt::{Interrupt, InterruptNumber};

extern "C" {
    fn SupervisorSoft(trap_frame: &mut TrapFrame);
    fn MachineSoft(trap_frame: &mut TrapFrame);
    fn SupervisorTimer(trap_frame: &mut TrapFrame);
    fn MachineTimer(trap_frame: &mut TrapFrame);
    fn SupervisorExternal(trap_frame: &mut TrapFrame);
    fn MachineExternal(trap_frame: &mut TrapFrame);
}

pub unsafe extern "C" fn dispatch_interrupt(trap_frame: &mut TrapFrame, code: usize) {
    let interrupt = Interrupt::from_number(code)
        .unwrap_or_else(|_| panic!("Unhandled interrupt: code={:#x}", code));
    match interrupt {
        Interrupt::SupervisorSoft => SupervisorSoft(trap_frame),
        Interrupt::MachineSoft => MachineSoft(trap_frame),
        Interrupt::SupervisorTimer => SupervisorTimer(trap_frame),
        Interrupt::MachineTimer => MachineTimer(trap_frame),
        Interrupt::SupervisorExternal => SupervisorExternal(trap_frame),
        Interrupt::MachineExternal => MachineExternal(trap_frame),
    }
}
