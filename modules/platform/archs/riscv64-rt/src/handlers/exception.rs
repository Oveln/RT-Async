//! RISC-V 异常分发
//!
//! 异常处理器通过链接脚本弱符号定义，默认指向 `ExceptionHandler`。
//! 平台可提供同名 `#[no_mangle]` 函数覆盖。

use riscv::{interrupt::Exception, ExceptionNumber};

use crate::TrapFrame;

extern "C" {
    fn InstructionMisaligned(trap_frame: &mut TrapFrame);
    fn InstructionFault(trap_frame: &mut TrapFrame);
    fn IllegalInstruction(trap_frame: &mut TrapFrame);
    fn Breakpoint(trap_frame: &mut TrapFrame);
    fn LoadMisaligned(trap_frame: &mut TrapFrame);
    fn LoadFault(trap_frame: &mut TrapFrame);
    fn StoreMisaligned(trap_frame: &mut TrapFrame);
    fn StoreFault(trap_frame: &mut TrapFrame);
    fn UserEnvCall(trap_frame: &mut TrapFrame);
    fn SupervisorEnvCall(trap_frame: &mut TrapFrame);
    fn MachineEnvCall(trap_frame: &mut TrapFrame);
    fn InstructionPageFault(trap_frame: &mut TrapFrame);
    fn LoadPageFault(trap_frame: &mut TrapFrame);
    fn StorePageFault(trap_frame: &mut TrapFrame);
}

#[no_mangle]
pub unsafe extern "C" fn ExceptionHandler(trap_frame: &mut TrapFrame) {
    let mcause = riscv::register::mcause::read().bits();
    let is_interrupt = mcause & (1 << 63) != 0;
    let exception_code = mcause & !(1 << 63);

    if is_interrupt {
        panic!(
            "Unhandled interrupt: mcause={:#x}, mepc={:#x}, mstatus={:#x}",
            mcause, trap_frame.mepc, trap_frame.mstatus
        );
    } else {
        let exception = Exception::from_number(exception_code)
            .unwrap_or_else(|_| panic!("Unknown exception code {exception_code}"));
        panic!(
            "Unhandled exception: {exception:?}, mcause={:#x}, mepc={:#x}, mstatus={:#x}",
            mcause, trap_frame.mepc, trap_frame.mstatus
        );
    }
}

pub unsafe extern "C" fn dispatch_exception(trap_frame: &mut TrapFrame, code: usize) {
    match Exception::from_number(code)
        .unwrap_or_else(|_| panic!("Unknown exception code: {code:#x}"))
    {
        Exception::InstructionMisaligned => InstructionMisaligned(trap_frame),
        Exception::InstructionFault => InstructionFault(trap_frame),
        Exception::IllegalInstruction => IllegalInstruction(trap_frame),
        Exception::Breakpoint => Breakpoint(trap_frame),
        Exception::LoadMisaligned => LoadMisaligned(trap_frame),
        Exception::LoadFault => LoadFault(trap_frame),
        Exception::StoreMisaligned => StoreMisaligned(trap_frame),
        Exception::StoreFault => StoreFault(trap_frame),
        Exception::UserEnvCall => UserEnvCall(trap_frame),
        Exception::SupervisorEnvCall => SupervisorEnvCall(trap_frame),
        Exception::MachineEnvCall => MachineEnvCall(trap_frame),
        Exception::InstructionPageFault => InstructionPageFault(trap_frame),
        Exception::LoadPageFault => LoadPageFault(trap_frame),
        Exception::StorePageFault => StorePageFault(trap_frame),
    }
}
