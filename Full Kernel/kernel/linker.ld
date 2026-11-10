/* The Nova kernel is linked in the top 2 GiB of the address space.
   NovaBoot maps each segment with its own permissions:
     text   -> read + execute
     rodata -> read only (no execute)
     data   -> read + write (no execute)
   Every segment starts on a 4 KiB page so the permissions never overlap. */

ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);  /* R-X */
    rodata PT_LOAD FLAGS(4);  /* R-- */
    data   PT_LOAD FLAGS(6);  /* RW- */
}

SECTIONS
{
    . = 0xFFFFFFFF80000000;
    __kernel_start = .;

    __text_start = .;
    .text : ALIGN(4K)
    {
        *(.text._start)
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    __text_end = .;
    __rodata_start = .;
    .rodata : ALIGN(4K)
    {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(4K);
    __rodata_end = .;
    __data_start = .;
    .data : ALIGN(4K)
    {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss : ALIGN(16)
    {
        *(.bss .bss.*)
        *(COMMON)
    } :data

    . = ALIGN(4K);
    __kernel_end = .;

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
