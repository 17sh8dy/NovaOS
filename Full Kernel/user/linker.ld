/* User programs are linked at 0x400000, each permission class on its own
   pages (so the kernel can map text R-X, rodata R--, data RW-). */

ENTRY(_start)

PHDRS
{
    text   PT_LOAD FLAGS(5);  /* R-X */
    rodata PT_LOAD FLAGS(4);  /* R-- */
    data   PT_LOAD FLAGS(6);  /* RW- */
}

SECTIONS
{
    . = 0x400000;

    .text : ALIGN(4K)
    {
        *(.text._start)
        *(.text .text.*)
    } :text

    . = ALIGN(4K);
    .rodata : ALIGN(4K)
    {
        *(.rodata .rodata.*)
    } :rodata

    . = ALIGN(4K);
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

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.note .note.*)
        *(.comment)
    }
}
