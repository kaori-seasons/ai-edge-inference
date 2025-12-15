/* StarryOS AArch64启动代码
 * 
 * 启动流程:
 * 1. 禁用cache和MMU
 * 2. 初始化页表
 * 3. 启用MMU
 * 4. 初始化GIC-500
 * 5. 跳转到Rust主程序
 */

.section .text.boot
.global _start
.align 4

_start:
    /* X0 = DTB物理地址 (由bootloader传入)
     * X1 = 入口魔数 (0x65584150)
     * X2-X3 = 保留
     * 
     * 此时系统状态:
     * - MMU: 关闭
     * - Cache: 关闭
     * - EL: EL1
     * - 大端/小端: 小端
     */

    /* 1. 初始化SCTLR_EL1 (系统控制寄存器)
     *    禁用所有缓存和指令预取 */
    mrs x1, sctlr_el1
    /* 清除cache控制位 */
    bic x1, x1, #0x4            /* C位: 数据cache禁用 */
    bic x1, x1, #0x1000         /* I位: 指令cache禁用 */
    bic x1, x1, #0x1            /* M位: MMU禁用 */
    msr sctlr_el1, x1
    isb                          /* 指令同步屏障 */

    /* 2. 初始化异常相关寄存器 */
    /* VBAR_EL1: 异常向量基地址 */
    adrp x1, exception_vectors
    msr vbar_el1, x1
    
    /* 3. 初始化栈指针 (kernel_stack定义在_linker脚本中)
     *    栈从高地址向低地址增长 */
    adrp x1, kernel_stack_top
    add x1, x1, :lo12:kernel_stack_top
    mov sp, x1

    /* 4. 保存DTB指针到x0 (作为Rust main的参数) */
    /* DTB地址将作为第一个参数传递给main() */
    mov x19, x0                  /* x19保存DTB地址 (callee-saved) */

    /* 5. 初始化BSS段 (清零) */
    adrp x1, __bss_start
    add x1, x1, :lo12:__bss_start
    adrp x2, __bss_end
    add x2, x2, :lo12:__bss_end
    
.clear_bss:
    cmp x1, x2
    b.ge .bss_done
    str xzr, [x1], #8           /* 写入零值, 并递增地址 */
    b .clear_bss

.bss_done:
    /* 6. 跳转到Rust主程序
     *    main(dtb_ptr: u64)
     *    x0 = DTB物理地址 */
    mov x0, x19                  /* 恢复DTB指针到x0 */
    
    /* 调用Rust的main函数 */
    adrp x1, main
    add x1, x1, :lo12:main
    blr x1

    /* 如果main返回, 进入无限循环 */
    b .

/* 异常向量表 */
.section .text.exceptions
.align 11                        /* 异常向量必须11位对齐 (2048字节) */

exception_vectors:
    /* 当前EL, SP0 (未使用) */
    .skip 0x80
    
    /* 当前EL, SPx */
    .skip 0x80
    
    /* 低EL (用户空间) 的异常处理
     * 这些是我们真正需要的 */
    
    /* 同步异常 (Synchronous) */
    .align 7
    b .                          /* 暂时忽略, 进入死循环 */
    
    /* 中断 (IRQ) */
    .align 7
    b .                          /* 在gic500中处理 */
    
    /* 快中断 (FIQ) */
    .align 7
    b .
    
    /* 系统错误 (SError) */
    .align 7
    b .

/* 内核栈 (512KB) */
.section .bss
.align 4

kernel_stack:
    .skip 0x80000               /* 512KB的栈空间 */

.global kernel_stack_top
kernel_stack_top:
    .align 4

/* BSS段标记符号 (链接器脚本会填充) */
.global __bss_start
.global __bss_end
