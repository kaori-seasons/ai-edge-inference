# StarryOS RK3588 边缘AI系统架构深度解析（二）：AArch64裸机启动与内存管理

## 引言

在上一篇文章中，我们整体介绍了StarryOS RK3588系统的架构设计理念和五层结构。从本文开始，我们将深入探讨每一层的具体实现细节。作为系统启动的第一步，AArch64裸机启动和内存管理是整个系统的基础，它们决定了系统能否正确运行以及后续功能的实现方式。

本文将详细解析StarryOS RK3588的启动流程和内存管理系统，包括汇编启动代码、页表结构设计以及MMU启用过程。

## AArch64启动流程详解

### 启动入口点

StarryOS RK3588的启动入口点位于`src/arch/aarch64/boot.s`文件中。当系统上电或复位时，处理器会跳转到这个入口点开始执行：

```assembly
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
```

### 系统状态初始化

启动代码首先需要确保系统处于一个已知的状态。这包括禁用缓存和MMU，设置异常向量表等：

```assembly
/* 1. 初始化SCTLR_EL1 (系统控制寄存器)
 *    禁用所有缓存和指令预取 */
mrs x1, sctlr_el1
/* 清除cache控制位 */
bic x1, x1, #0x4            /* C位: 数据cache禁用 */
bic x1, x1, #0x1000         /* I位: 指令cache禁用 */
bic x1, x1, #0x1            /* M位: MMU禁用 */
msr sctlr_el1, x1
isb                          /* 指令同步屏障 */
```

这段代码通过读取、修改和写回SCTLR_EL1寄存器来禁用MMU和缓存，确保系统在已知状态下运行。

### 异常向量表设置

AArch64架构使用异常向量表来处理各种异常情况，包括中断、系统调用等：

```assembly
/* 2. 初始化异常相关寄存器 */
/* VBAR_EL1: 异常向量基地址 */
adrp x1, exception_vectors
msr vbar_el1, x1
```

这里将异常向量表的基地址设置到VBAR_EL1寄存器中，使得处理器知道在发生异常时应该跳转到哪里处理。

### 栈指针初始化

在进入Rust代码之前，需要初始化栈指针，因为Rust代码可能会使用栈：

```assembly
/* 3. 初始化栈指针 (kernel_stack定义在_linker脚本中)
 *    栈从高地址向低地址增长 */
adrp x1, kernel_stack_top
add x1, x1, :lo12:kernel_stack_top
mov sp, x1
```

StarryOS为内核分配了512KB的栈空间，栈指针指向栈顶。

### BSS段清零

BSS段包含未初始化的全局变量，在程序启动时需要将其清零：

```assembly
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
```

这段代码遍历整个BSS段，将其内容清零。

### 跳转到Rust主程序

完成所有必要的初始化后，启动代码会跳转到Rust主程序：

```assembly
/* 6. 跳转到Rust主程序
 *    main(dtb_ptr: u64)
 *    x0 = DTB物理地址 */
mov x0, x19                  /* 恢复DTB指针到x0 */

/* 调用Rust的main函数 */
adrp x1, main
add x1, x1, :lo12:main
blr x1
```

这里将设备树的物理地址作为参数传递给Rust的main函数，并通过BLR指令跳转到Rust代码。

## 内存管理系统设计

### 页表结构

StarryOS RK3588采用AArch64标准的四级页表结构：

1. **L0 (PGD)**: Page Global Directory
2. **L1 (PUD)**: Page Upper Directory  
3. **L2 (PMD)**: Page Middle Directory
4. **L3 (PTE)**: Page Table Entry

这种四级结构可以支持48位虚拟地址空间，足以满足大多数应用场景的需求。

### 页表项设计

页表项(PTE)的设计直接影响内存访问的性能和安全性。StarryOS RK3588的PTE结构如下：

```rust
/// 页表项 (Page Table Entry)
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Pte(u64);

impl Pte {
    /// 表项有效标志
    const VALID: u64 = 1 << 0;
    
    /// 表项类型: 0=表, 1=块/页
    const TYPE: u64 = 1 << 1;
    
    /// 访问权限 (AP)
    const AP_MASK: u64 = 0x3 << 6;
    const AP_EL0_NONE: u64 = 0 << 6;      // EL0无访问权限
    const AP_EL0_RW: u64 = 1 << 6;        // EL0可读写
    const AP_KERN_RO: u64 = 2 << 6;       // 内核只读
    const AP_KERN_RW: u64 = 3 << 6;       // 内核可读写
    
    /// 缓存策略 (AttrIdx)
    const ATTR_IDX_MASK: u64 = 0x7 << 2;
    const ATTR_DEVICE: u64 = 0 << 2;      // 设备内存 (MMIO)
    const ATTR_NORMAL: u64 = 1 << 2;      // 普通内存 (缓存)
    const ATTR_NORMAL_NC: u64 = 2 << 2;   // 普通内存 (非缓存)
    
    /// 访问标志 (Access Flag)
    const AF: u64 = 1 << 10;
    
    /// 执行权限禁用标志
    const UXN: u64 = 1 << 54;             // User eXecute Never
    const PXN: u64 = 1 << 53;             // Privileged eXecute Never
}
```

### 内存映射策略

StarryOS RK3588采用以下内存映射策略：

1. **DDR内存**: 1GB @ 0x0，使用缓存属性(NORMAL)
2. **MMIO区域**: 256MB @ 0xfe000000，使用设备属性(DEVICE)

```rust
/// 初始化页表管理器
/// 
/// 设置:
/// - DDR内存: 1GB @ 0x0, 缓存可用 (NORMAL)
/// - MMIO区域: 256MB @ 0xfe000000, 非缓存 (DEVICE)
pub fn init() -> Self {
    unsafe {
        // L0表 (PGD)
        let l0_addr = 0x1000 as *mut PageTable;
        let l0 = &mut *l0_addr;
        *l0 = PageTable::new();
        
        // L1表
        let l1_addr = 0x2000 as *mut PageTable;
        let l1 = &mut *l1_addr;
        *l1 = PageTable::new();
        
        // L2表 (DDR: 1GB)
        let l2_addr = 0x3000 as *mut PageTable;
        let l2 = &mut *l2_addr;
        *l2 = PageTable::new();
        
        // L3表 (MMIO: 256MB)
        let l3_addr = 0x4000 as *mut PageTable;
        let l3 = &mut *l3_addr;
        *l3 = PageTable::new();
        
        // 配置L0 -> L1
        l0.set(0, Pte::table(l1_addr as u64));
        
        // 配置L1 -> L2 (DDR)
        l1.set(0, Pte::table(l2_addr as u64));
        
        // 配置L2 -> 1GB块 @ 0x0 (DDR内存, 缓存)
        for i in 0..512 {
            let phys_addr = (i as u64) * 0x200000; // 2MB块
            l2.set(i, Pte::block(phys_addr, Pte::ATTR_NORMAL, true));
        }
        
        // 配置L1 -> L3 (MMIO)
        l1.set(511, Pte::table(l3_addr as u64)); // 顶部地址空间
        
        // 配置L3 -> 256MB块 @ 0xfe000000 (设备寄存器, 非缓存)
        for i in 0..512 {
            let phys_addr = 0xfe000000 + ((i as u64) * 0x200000);
            l3.set(i, Pte::block(phys_addr, Pte::ATTR_DEVICE, false));
        }
        
        PageTableManager {
            l0,
        }
    }
}
```

### MMU启用过程

MMU的启用涉及多个寄存器的配置：

1. **TTBR0_EL1**: 设置页表基地址
2. **TCR_EL1**: 配置转换控制参数
3. **MAIR_EL1**: 设置内存属性
4. **SCTLR_EL1**: 启用MMU

```rust
/// 启用MMU
pub fn enable(&self) {
    unsafe {
        // 1. 设置TTBR0_EL1 (转换表基地址寄存器)
        let l0_addr = self.l0 as *const _ as u64;
        asm!("msr ttbr0_el1, {}", in(reg) l0_addr);
        
        // 2. 配置TCR_EL1 (转换控制寄存器)
        let tcr: u64 = (1 << 32)     // IPS=01 (40-bit)
            | (0 << 14)               // TG0=00 (4KB)
            | (3 << 12)               // SH0=11 (Inner shareable)
            | (1 << 10)               // ORGN0=01 (Write-Back)
            | (1 << 8)                // IRGN0=01 (Write-Back)
            | (32);                   // T0SZ=32
        
        asm!("msr tcr_el1, {}", in(reg) tcr);
        
        // 3. 设置MAIR_EL1 (Memory Attribute Indirection Register)
        let mair: u64 = 0x00FF4400u64;
        asm!("msr mair_el1, {}", in(reg) mair);
        
        // 4. 启能MMU (SCTLR_EL1.M=1)
        let mut sctlr: u64;
        asm!("mrs {}, sctlr_el1", out(reg) sctlr);
        sctlr |= 1;  // M位
        sctlr |= 1 << 2;  // C位 (数据缓存)
        sctlr |= 1 << 12; // I位 (指令缓存)
        asm!("msr sctlr_el1, {}", in(reg) sctlr);
        
        // 指令同步屏障
        asm!("isb");
    }
}
```

## 性能优化考虑

### 页表缓存友好的设计

为了提高页表查找的性能，StarryOS RK3588在设计时考虑了缓存友好的因素：

1. **页表对齐**: 所有页表都按4KB边界对齐，符合缓存行大小
2. **连续内存分配**: 页表在物理内存中连续分配，减少TLB miss
3. **合理映射粒度**: 使用2MB的大页映射DDR内存，减少页表层级

### 内存属性优化

不同的内存区域使用不同的缓存属性：

1. **普通内存(NORMAL)**: DDR内存使用写回缓存，提高访问性能
2. **设备内存(DEVICE)**: MMIO区域禁用缓存，确保内存访问的顺序性

## 安全性考虑

### 内存保护

通过页表项的访问权限控制，StarryOS RK3588实现了基本的内存保护：

1. **内核只读区域**: 重要的代码和数据区域设置为只读
2. **执行权限控制**: 通过PXN和UXN位控制代码执行权限
3. **用户空间隔离**: 为未来的用户空间进程提供隔离机制

### 地址空间布局

合理的地址空间布局有助于提高系统安全性：

1. **内核空间**: 固定在低地址区域
2. **设备空间**: 映射在高地址区域
3. **用户空间**: 未来可映射在中间区域

## 调试与验证

### 启动日志

StarryOS RK3588在启动过程中会输出详细的日志信息：

```
[StarryOS] Booting...
[StarryOS] Initializing memory management...
[StarryOS] MMU enabled
[StarryOS] Initializing GIC-500...
[StarryOS] GIC initialized for CPU 0
[StarryOS] Parsing device tree...
[StarryOS] Device tree parsed successfully
```

这些日志有助于开发者了解系统启动的进度和状态。

### QEMU仿真验证

StarryOS RK3588可以在QEMU中进行仿真验证，确保启动代码的正确性：

```bash
# 启动QEMU模拟器
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a76 \
  -kernel target/aarch64-unknown-none-elf/release/starryos \
  -serial stdio
```

## 总结

AArch64裸机启动和内存管理是StarryOS RK3588系统的基础，它们为后续的所有功能提供了运行环境。通过精心设计的启动流程和内存管理系统，StarryOS RK3588实现了：

1. **可靠的启动**: 确保系统在已知状态下启动
2. **高效的内存访问**: 通过合理的页表设计和缓存策略提高性能
3. **基本的安全保护**: 通过内存保护机制提供基础安全保障
4. **良好的可调试性**: 详细的启动日志和QEMU仿真支持

在下一篇文章中，我们将探讨StarryOS RK3588的中断管理系统，包括GIC-500控制器的配置和多核调度机制。

敬请期待下一篇文章：《StarryOS RK3588 边缘AI系统架构深度解析（三）：GIC-500中断控制器与多核调度》。