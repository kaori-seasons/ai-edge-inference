# StarryOS RK3588 边缘AI系统架构深度解析（三）：GIC-500中断控制器与多核调度

## 引言

在上一篇文章中，我们详细解析了StarryOS RK3588系统的AArch64裸机启动流程和内存管理系统。本文将深入探讨另一个关键组件——GIC-500中断控制器及其在多核调度中的作用。

中断是现代计算机系统中至关重要的机制，它允许外设在需要CPU关注时主动通知CPU，而不是让CPU不断地轮询检查外设状态。对于一个复杂的SoC芯片如RK3588来说，高效的中断管理直接影响着系统的响应性和实时性。

## GIC-500中断控制器架构

### GIC-500简介

GIC-500（Generic Interrupt Controller v3）是ARM公司设计的新一代通用中断控制器，专为多核ARM处理器设计。在RK3588芯片中，GIC-500负责管理整个系统的中断分发和处理。

GIC-500的主要组成部分包括：
1. **GICD (Distributor)** - 中断分发器，负责全局中断的管理和路由
2. **GICR (Redistributer)** - 重定向器，每个CPU核心都有自己的GICR，负责该核心的私有中断处理

### 中断类型

GIC-500支持三种类型的中断：

1. **SGI (Software Generated Interrupt)** - 软件生成中断，用于CPU核心间的通信
2. **PPI (Private Peripheral Interrupt)** - 私有外设中断，每个核心独享的中断源
3. **SPI (Shared Peripheral Interrupt)** - 共享外设中断，多个核心共享的中断源

### GIC-500驱动实现

让我们来看看StarryOS中GIC-500驱动的核心实现：

```rust
/// GIC-500基地址 (RK3588特定)
pub const GIC_BASE: u64 = 0xfe600000;

/// GICD (Distributor)基地址
pub const GICD_BASE: u64 = GIC_BASE + 0x00000;

/// GICR (Redistributer)基地址
pub const GICR_BASE: u64 = GIC_BASE + 0x100000;
```

GICD负责管理SPI中断，而GICR则处理每个核心的SGI和PPI中断。这种分离的设计使得中断处理更加高效和灵活。

在初始化过程中，系统会分别初始化GICD和GICR：

```rust
/// 初始化GICD (Distributor)
pub fn init_gicd(&self) {
    unsafe {
        // 1. 禁用GICD
        write_volatile((self.gicd_base + GICD_CTLR) as *mut u32, 0);
        
        // 2. 获取中断数量
        let typer = read_volatile((self.gicd_base + GICD_TYPER) as *const u32);
        let num_interrupts = ((typer & 0x1F) + 1) * 32;
        
        // 3. 禁用所有SPI (32-1019)
        for i in (32..num_interrupts).step_by(32) {
            write_volatile(
                (self.gicd_base + GICD_ICENABLER + (i / 32) as u64 * 4) as *mut u32,
                0xFFFFFFFF,
            );
        }
        // ... 更多初始化代码
    }
}

/// 初始化GICR (Redistributer) - 每个核心调用一次
pub fn init_gicr(&self, cpu_id: u32) {
    unsafe {
        // GICR基地址 = 基址 + (CPU_ID * 0x20000)
        let gicr_cpu_base = self.gicr_base + (cpu_id as u64) * 0x20000;
        
        // 1. 禁用所有SGI/PPI
        write_volatile(
            (gicr_cpu_base + GICR_SGI_ICENABLER0) as *mut u32,
            0xFFFFFFFF,
        );
        // ... 更多初始化代码
    }
}
```

## 多核调度机制

### RK3588的CPU架构

RK3588采用了big.LITTLE架构，拥有两种不同性能的CPU核心：
- **A76核心** (0-3): 高性能核心，适合计算密集型任务
- **A55核心** (4-7): 低功耗核心，适合后台任务和轻量级处理

### 多核启动流程

在StarryOS中，多核启动通过SGI中断来实现。CPU 0作为主核心，在完成自身初始化后，会向其他核心发送SGI中断来唤醒它们：

```rust
/// 启动CPU核心
pub fn start_cpu(cpu_id: u32) {
    if cpu_id >= 8 {
        return;
    }
    
    // 获取CPU信息
    let cpu_info = &CPU_INFO[cpu_id as usize];
    cpu_info.set_state(CpuState::Starting);
    
    // 通过发送SGI中断来唤醒目标CPU
    // SGI15 用作CPU启动信号
    unsafe {
        use crate::hal::gic500::GIC;
        let gic = GIC.lock();
        gic.send_sgi(15, 1 << cpu_id);  // 只给指定CPU发送
    }
}
```

### CPU间通信

CPU核心之间可以通过IPI (Inter-Processor Interrupt)进行通信：

```rust
/// 发送核心间中断 (IPI)
pub fn send_ipi(vector: u32, cpu_mask: u32) {
    unsafe {
        use crate::hal::gic500::GIC;
        let gic = GIC.lock();
        if vector < 16 {
            gic.send_sgi(vector, cpu_mask);
        }
    }
}
```

## 异构调度器

为了充分利用RK3588的异构计算能力，StarryOS实现了专门的HMP (Heterogeneous Multi-Processing) 调度器：

### 任务亲和性提示

调度器支持三种任务亲和性提示：

```rust
/// 任务亲和性提示
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum TaskHint {
    /// 高性能计算任务 - 分配给A76核心
    HighPerf = 0,
    
    /// 低功耗任务 - 优先分配给A55核心
    LowPower = 1,
    
    /// NPU前后处理 - 强制A76核心
    NpuPrePost = 2,
}
```

### 调度决策

调度器根据任务类型和系统负载做出调度决策：

```rust
/// 决策任务应该分配给哪个CPU
pub fn decide_cpu(&self, task: &Task) -> u32 {
    match task.hint {
        TaskHint::HighPerf | TaskHint::NpuPrePost => {
            // 高性能任务: 强制分配给A76
            let a76_idx = self.find_least_loaded_a76();
            a76_idx  // 返回0-3
        }
        TaskHint::LowPower => {
            // 低功耗任务: 优先分配给A55
            let avg_a55_load = self.get_a55_avg_load();
            
            if avg_a55_load < self.config.a55_load_threshold {
                // A55空闲, 优先使用
                let a55_idx = self.find_least_loaded_a55();
                4 + a55_idx  // 返回4-7
            } else {
                // A55繁忙, 降级到A76
                let a76_idx = self.find_least_loaded_a76();
                a76_idx  // 返回0-3
            }
        }
    }
}
```

## NPU协同调度

在AI推理场景中，NPU的使用需要与CPU任务协调配合：

```rust
/// NPU 任务类型
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NpuTaskType {
    /// 预处理 (数据准备)
    Preprocess = 0,
    /// NPU 推理
    Inference = 1,
    /// 后处理 (结果处理)
    Postprocess = 2,
}
```

NPU调度器能够根据不同任务类型做出最优的调度决策，确保整个推理流水线的高效运行。

## 总结

本文深入解析了StarryOS RK3588系统中的GIC-500中断控制器和多核调度机制。通过合理利用SGI中断实现多核启动和通信，结合异构调度器对不同类型任务的智能分配，系统能够充分发挥RK3588芯片的多核异构计算能力。

在下一文中，我们将探讨设备树解析和硬件抽象层的实现，这是连接硬件和软件的关键桥梁。