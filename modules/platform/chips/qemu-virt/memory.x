ENTRY(__start);

MEMORY
{
    RAM : ORIGIN = 0x80000000, LENGTH = 64M
}

_max_hart_id = 0;
_hart_stack_size = 8192;
