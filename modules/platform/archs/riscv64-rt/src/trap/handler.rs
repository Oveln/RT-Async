//! Trap 分发

use crate::TrapFrame;

#[no_mangle]
pub extern "C" fn trap_handler(trap_frame: &mut TrapFrame) {
    match riscv::register::mcause::read().cause() {
        riscv::interrupt::Trap::Interrupt(code) => {
            unsafe { crate::handlers::interrupt::dispatch(trap_frame, code) }
        }
        riscv::interrupt::Trap::Exception(code) => {
            unsafe { crate::handlers::exception::dispatch(trap_frame, code) }
        }
    }
}
