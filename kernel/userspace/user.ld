/* PurityOS user-program linker script: bare ET_EXEC ELF64, no segments
   other than the two PT_LOAD the kernel loader understands. */

ENTRY(_start)

SECTIONS
{
    . = 0x400000;
    .text : {
        *(.text*)
        *(.rodata*)
    }
    .data : {
        *(.data*)
    }
    .bss : {
        *(.bss*)
    }
    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame)
        *(.rela*)
        *(.symtab)
        *(.strtab)
    }
}
