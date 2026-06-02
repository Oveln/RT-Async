//! RISC-V Trap 汇编入口点

extern "C" {
    pub fn __trap_entry();
}

core::arch::global_asm!(
    ".section .trap.entry, \"ax\"",
    ".global __trap_entry",
    ".align 4",
    "__trap_entry:",
    // === 在当前栈上保存上下文 ===
    // 使用 t0 临时保存原始 sp，分配 272 字节（256 trap frame + 16 对齐余量），
    // 强制 16 字节对齐以满足 RISC-V 调用约定，然后将原始 sp 存入偏移 256。
    "mv t0, sp",              // t0 = 原始 sp
    "addi sp, sp, -272",      // 分配 256 + 16 字节
    "andi sp, sp, -16",       // 强制 16 字节对齐
    "sd t0, 256(sp)",         // 保存原始 sp 到 padding 区域
    "sd x1, 0(sp)",    // ra
    "sd x3, 8(sp)",    // gp
    "sd x4, 16(sp)",   // tp
    "sd x5, 24(sp)",   // t0
    "sd x6, 32(sp)",   // t1
    "sd x7, 40(sp)",   // t2
    "sd x8, 48(sp)",   // s0/fp
    "sd x9, 56(sp)",   // s1
    "sd x10, 64(sp)",  // a0
    "sd x11, 72(sp)",  // a1
    "sd x12, 80(sp)",  // a2
    "sd x13, 88(sp)",  // a3
    "sd x14, 96(sp)",  // a4
    "sd x15, 104(sp)", // a5
    "sd x16, 112(sp)", // a6
    "sd x17, 120(sp)", // a7
    "sd x18, 128(sp)", // s2
    "sd x19, 136(sp)", // s3
    "sd x20, 144(sp)", // s4
    "sd x21, 152(sp)", // s5
    "sd x22, 160(sp)", // s6
    "sd x23, 168(sp)", // s7
    "sd x24, 176(sp)", // s8
    "sd x25, 184(sp)", // s9
    "sd x26, 192(sp)", // s10
    "sd x27, 200(sp)", // s11
    "sd x28, 208(sp)", // t3
    "sd x29, 216(sp)", // t4
    "sd x30, 224(sp)", // t5
    "sd x31, 232(sp)", // t6
    "csrr t0, mepc",
    "sd t0, 240(sp)",
    "csrr t0, mstatus",
    "sd t0, 248(sp)",
    // === 调用 Rust trap handler ===
    "mv a0, sp",
    "call trap_handler",
    // === 从当前栈恢复上下文 ===
    // 先恢复 CSR（用 t0 做临时寄存器），再恢复通用寄存器，
    // 否则 t0 会被 CSR 值覆盖，导致恢复后的 t0 不正确。
    "ld t0, 240(sp)",
    "csrw mepc, t0",
    "ld t0, 248(sp)",
    "csrw mstatus, t0",
    "ld x1, 0(sp)",
    "ld x3, 8(sp)",
    "ld x4, 16(sp)",
    "ld x5, 24(sp)",
    "ld x6, 32(sp)",
    "ld x7, 40(sp)",
    "ld x8, 48(sp)",
    "ld x9, 56(sp)",
    "ld x10, 64(sp)",
    "ld x11, 72(sp)",
    "ld x12, 80(sp)",
    "ld x13, 88(sp)",
    "ld x14, 96(sp)",
    "ld x15, 104(sp)",
    "ld x16, 112(sp)",
    "ld x17, 120(sp)",
    "ld x18, 128(sp)",
    "ld x19, 136(sp)",
    "ld x20, 144(sp)",
    "ld x21, 152(sp)",
    "ld x22, 160(sp)",
    "ld x23, 168(sp)",
    "ld x24, 176(sp)",
    "ld x25, 184(sp)",
    "ld x26, 192(sp)",
    "ld x27, 200(sp)",
    "ld x28, 208(sp)",
    "ld x29, 216(sp)",
    "ld x30, 224(sp)",
    "ld x31, 232(sp)",
    // 从 padding 区域恢复原始 sp（而非 addi，因为对齐可能消耗了额外字节）
    "ld sp, 256(sp)",
    "mret",
);
