/* StarryOS RK3588 链接脚本
 * AArch64裸机环境下的内存布局
 * 
 * 内存分布:
 * 0x00000000 - 0x00010000: 引导代码/异常向量
 * 0x00010000 - 0x20000000: 内核代码和数据
 * 0x20000000 - 0xFFFFFFFF: 用户空间/设备寄存器
 */

ENTRY(_start)

MEMORY
{
    /* LPDDR4X DDR内存 (8GB) */
    RAM (rwx) : ORIGIN = 0x00000000, LENGTH = 0x200000000
    
    /* 设备MMIO映射区域 */
    MMIO (rw) : ORIGIN = 0xfe000000, LENGTH = 0x02000000
}

SECTIONS
{
    /* 启动代码和异常向量表必须在最前面 */
    .text.boot :
    {
        KEEP(*(.text.boot))
        KEEP(*(.text.exceptions))
    } > RAM AT > RAM

    /* 内核代码段 */
    .text :
    {
        *(.text .text.*)
        *(.rodata .rodata.*)
    } > RAM AT > RAM

    /* 初始化数据段 */
    .data :
    {
        *(.data .data.*)
    } > RAM AT > RAM

    /* 未初始化数据段 */
    .bss :
    {
        *(.bss .bss.*)
        *(COMMON)
    } > RAM AT > RAM

    /DISCARD/ :
    {
        *(.note.GNU-stack)
        *(.gnu_debuglink)
        *(.gnu_debugdata)
    }
}
