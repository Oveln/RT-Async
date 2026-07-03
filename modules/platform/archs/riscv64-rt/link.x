/*
 * RISC-V 64-bit Linker Script
 *
 * 使用弱符号机制，平台可通过 `#[no_mangle]` 函数覆盖默认处理器。
 *
 * 需要 memory.x 提供：
 *   MEMORY { RAM : ORIGIN = ..., LENGTH = ... }
 *   _max_hart_id, _hart_stack_size
 */

EXTERN(_default_abort);
PROVIDE(abort = _default_abort);

PROVIDE(_pre_init_trap = _default_abort);

EXTERN(_default_start_trap);
PROVIDE(_start_trap = _default_start_trap);

EXTERN(_default_setup_interrupts);
PROVIDE(_setup_interrupts = _default_setup_interrupts);

PROVIDE(hal_main = main);

PROVIDE(ExceptionHandler = abort);
PROVIDE(DefaultHandler = abort);
PROVIDE(_start_DefaultHandler_trap = _start_trap);

PROVIDE(InstructionMisaligned = ExceptionHandler);
PROVIDE(InstructionFault = ExceptionHandler);
PROVIDE(IllegalInstruction = ExceptionHandler);
PROVIDE(Breakpoint = ExceptionHandler);
PROVIDE(LoadMisaligned = ExceptionHandler);
PROVIDE(LoadFault = ExceptionHandler);
PROVIDE(StoreMisaligned = ExceptionHandler);
PROVIDE(StoreFault = ExceptionHandler);
PROVIDE(UserEnvCall = ExceptionHandler);
PROVIDE(SupervisorEnvCall = ExceptionHandler);
PROVIDE(MachineEnvCall = ExceptionHandler);
PROVIDE(InstructionPageFault = ExceptionHandler);
PROVIDE(LoadPageFault = ExceptionHandler);
PROVIDE(StorePageFault = ExceptionHandler);

PROVIDE(SupervisorSoft = DefaultHandler);
PROVIDE(__Inner_MachineSoft = DefaultHandler);
PROVIDE(SupervisorTimer = DefaultHandler);
PROVIDE(MachineTimer = DefaultHandler);
PROVIDE(SupervisorExternal = DefaultHandler);
PROVIDE(MachineExternal = DefaultHandler);

PROVIDE(_stext = ORIGIN(RAM));
PROVIDE(_stack_start = ORIGIN(RAM) + LENGTH(RAM));

SECTIONS
{
    .text _stext :
    {
        __stext = .;

        KEEP(*(.init));

        . = ALIGN(4);
        KEEP(*(.trap.vector));
        KEEP(*(.trap.entry));
        KEEP(*(.trap.start));
        KEEP(*(.trap.start.*));
        KEEP(*(.trap.continue));
        KEEP(*(.trap.rust));
        KEEP(*(.trap .trap.*));

        *(.text.abort);
        *(.text .text.*);
        *(.text.switch .text.switch.*);
        *(.text.scheduler .text.scheduler.*);
        *(.text.hot .text.hot.*);

        . = ALIGN(4);
        __etext = .;
    } > RAM

    .rodata : ALIGN(4)
    {
        . = ALIGN(4);
        __srodata = .;

        *(.srodata .srodata.*);
        *(.rodata .rodata.*);

        . = ALIGN(8);
        __erodata = .;
    } > RAM

    .data : ALIGN(8)
    {
        . = ALIGN(8);
        __sdata = .;

        PROVIDE(__global_pointer$ = . + 0x800);
        *(.sdata .sdata.* .sdata2 .sdata2.*);
        *(.data .data.*);
        *(.data.scheduler .data.scheduler.*);
        *(.data.hot .data.hot.*);

    } > RAM

    . = ALIGN(8);
    __edata = .;
    __sidata = LOADADDR(.data);

    .bss (NOLOAD) : ALIGN(8)
    {
        . = ALIGN(8);
        __sbss = .;

        *(.sbss .sbss.* .bss .bss.*);
        *(.stack.tasks .stack.tasks.*);
    } > RAM

    . = ALIGN(8);
    __ebss = .;

    .uninit (NOLOAD) : ALIGN(8)
    {
        . = ALIGN(8);
        __suninit = .;
        *(.uninit .uninit.*);
        . = ALIGN(8);
        __euninit = .;
    } > RAM

    .stack (NOLOAD) :
    {
        __estack = .;
        . = ABSOLUTE(_stack_start);
        __sstack = .;
        __stack_size = __sstack - __estack;
    } > RAM

    .got (INFO) :
    {
        KEEP(*(.got .got.*));
    }
}

ASSERT(_stext % 4 == 0, "
错误: _stext 必须是 4 字节对齐的");

ASSERT(__sdata % 8 == 0 && __edata % 8 == 0, "
错误: .data 段必须是 8 字节对齐的");

ASSERT(__sidata % 8 == 0, "
错误: .data 段的 LMA 必须是 8 字节对齐的");

ASSERT(__sbss % 8 == 0 && __ebss % 8 == 0, "
错误: .bss 段必须是 8 字节对齐的");

ASSERT(SIZEOF(.stack) >= (_max_hart_id + 1) * _hart_stack_size, "
错误: .stack 段太小，无法为所有 hart 分配栈空间。
考虑修改 _max_hart_id 或 _hart_stack_size");

ASSERT(SIZEOF(.got) == 0, "
错误: 检测到 .got 段。不支持动态重定位。
如果链接到 C 代码，请在编译时禁用 -fPIC 标志。");
