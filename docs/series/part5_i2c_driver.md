# StarryOS RK3588 边缘AI系统架构深度解析（五）：I2C驱动实现与embedded-hal规范

## 引言

在前面的文章中，我们探讨了StarryOS RK3588系统的设备树解析和硬件抽象层设计。本文将深入分析I2C驱动的具体实现，重点关注其如何遵循embedded-hal规范，以及在RK3588平台上的寄存器级实现细节。

I2C（Inter-Integrated Circuit）是一种广泛应用的串行通信协议，用于连接微控制器和低速外围设备。在嵌入式系统中，I2C驱动的质量直接影响着系统的稳定性和兼容性。

## embedded-hal规范概述

### 什么是embedded-hal？

embedded-hal是Rust嵌入式生态系统中的一个重要规范，它定义了一套通用的硬件抽象接口。通过遵循这一规范，驱动程序可以获得以下优势：

1. **可移植性**：驱动可以在不同的硬件平台上复用
2. **兼容性**：与其他遵循相同规范的库无缝集成
3. **类型安全**：利用Rust的类型系统保证接口使用的正确性

### I2C Trait定义

embedded-hal为I2C接口定义了几个核心trait：

```rust
// 简化的embedded-hal I2C trait示例
pub trait I2c {
    type Error;
    
    // 写操作
    fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error>;
    
    // 读操作
    fn read(&mut self, addr: u8, buffer: &mut [u8]) -> Result<(), Self::Error>;
    
    // 写后读操作
    fn write_read(
        &mut self, 
        addr: u8, 
        bytes: &[u8], 
        buffer: &mut [u8]
    ) -> Result<(), Self::Error>;
}
```

## StarryOS I2C驱动实现

### RK3588 I2C控制器架构

RK3588芯片内置了9个I2C控制器，每个控制器都有独立的寄存器空间。这些控制器支持标准模式（100kHz）和快速模式（400kHz）。

在StarryOS中，I2C控制器的基地址定义如下：

```rust
/// I2C基地址 (RK3588有9个I2C控制器)
pub const I2C0_BASE: u64 = 0xfea80000;
pub const I2C1_BASE: u64 = 0xfea90000;
pub const I2C2_BASE: u64 = 0xfeaa0000;
// ... 其他I2C控制器基地址
```

### 核心数据结构

I2C驱动的核心数据结构是[I2c]结构体：

```rust
/// I2C驱动结构体
pub struct I2c {
    base: u64,
    /// I2C总线频率 (标准: 100kHz, 快速: 400kHz)
    freq_khz: u32,
}
```

这个结构体包含了I2C控制器的基地址和工作频率，是所有操作的基础。

### 寄存器映射

I2C控制器通过内存映射I/O（MMIO）方式进行控制，关键寄存器包括：

```rust
// ============ I2C寄存器偏移 ============
/// I2C控制寄存器
const I2C_CON: u64 = 0x0;

/// I2C时钟分频寄存器
const I2C_CLKDIV: u64 = 0x4;

/// 主设备写/接收FIFO
const I2C_MRXADDR: u64 = 0x8;

/// 主设备读/发送FIFO
const I2C_MRXRADDR: u64 = 0xc;

/// 主设备发送数据寄存器
const I2C_MTXCNT: u64 = 0x10;

/// 主设备接收数据寄存器
const I2C_MRXCNT: u64 = 0x14;
```

### 初始化流程

I2C控制器的初始化是驱动工作的第一步：

```rust
/// 初始化I2C控制器
pub fn init(&self, apb_freq_mhz: u32) -> Result<(), I2cError> {
    unsafe {
        // 1. 禁用I2C
        write_volatile((self.base + I2C_CON) as *mut u32, 0);
        
        // 2. 计算时钟分频
        // I2C频率 = APB频率 / (2 * (CLKDIV + 1))
        // 例: 24MHz APB, 100kHz I2C => CLKDIV = (24000/100 - 2) / 2 = 119
        let div = ((apb_freq_mhz * 1000) / (2 * self.freq_khz)) - 1;
        
        if div > 0xFFFF {
            return Err(I2cError::InvalidAddr);
        }
        
        write_volatile((self.base + I2C_CLKDIV) as *mut u32, div as u32);
        
        // 3. 启用I2C控制器
        write_volatile((self.base + I2C_CON) as *mut u32, I2C_CON_EN);
        
        Ok(())
    }
}
```

初始化过程包括：
1. 禁用控制器以确保安全配置
2. 根据APB总线频率和期望的I2C频率计算时钟分频值
3. 配置时钟分频寄存器
4. 启用控制器

### 写操作实现

写操作是I2C通信的基本操作之一：

```rust
/// 发送数据 (内部函数)
fn write_internal(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
    self.wait_idle(1000)?;
    self.clear_irq();
    
    unsafe {
        // 1. 设置目标地址 (发送模式)
        write_volatile((self.base + I2C_MRXADDR) as *mut u32, (addr as u32) & 0x7f);
        
        // 2. 设置发送字节数
        write_volatile((self.base + I2C_MTXCNT) as *mut u32, data.len() as u32);
        
        // 3. 写入数据到FIFO
        for (i, &byte) in data.iter().enumerate() {
            if i < 32 {  // 最多32字节
                write_volatile(
                    (self.base + I2C_TXDATA0 + (i as u64)) as *mut u8,
                    byte,
                );
            }
        }
        
        // 4. 启动发送 (设置CON寄存器)
        let mut con = read_volatile((self.base + I2C_CON) as *const u32);
        con &= !I2C_CON_MODE_MASK;
        con |= I2C_CON_MODE_TX;
        con |= 1 << 4; // START_EN
        write_volatile((self.base + I2C_CON) as *mut u32, con);
        
        // 5. 等待发送完成
        self.wait_idle(1000)?;
    }
    
    Ok(())
}
```

写操作的流程包括：
1. 等待控制器空闲并清除中断标志
2. 设置目标设备地址
3. 设置要发送的数据长度
4. 将数据写入发送FIFO
5. 配置控制寄存器启动发送
6. 等待发送完成

### 读操作实现

读操作同样重要，特别是在传感器数据读取等场景中：

```rust
/// 读取数据 (内部函数)
fn read_internal(&mut self, addr: u8, len: u8) -> Result<Vec<u8>, I2cError> {
    self.wait_idle(1000)?;
    self.clear_irq();
    
    let mut data = Vec::new();
    
    unsafe {
        // 1. 设置目标地址 (读模式)
        let addr_val = ((addr as u32) & 0x7f) | (1 << 24); // 读标志
        write_volatile((self.base + I2C_MRXADDR) as *mut u32, addr_val);
        
        // 2. 设置读取字节数
        write_volatile((self.base + I2C_MRXCNT) as *mut u32, len as u32);
        
        // 3. 启动读取
        let mut con = read_volatile((self.base + I2C_CON) as *const u32);
        con &= !I2C_CON_MODE_MASK;
        con |= I2C_CON_MODE_RX;
        con |= 1 << 4; // START_EN
        write_volatile((self.base + I2C_CON) as *mut u32, con);
        
        // 4. 等待读取完成
        self.wait_idle(1000)?;
        
        // 5. 读取FIFO中的数据
        for i in 0..len {
            let byte = read_volatile(
                (self.base + I2C_RXDATA0 + (i as u64)) as *const u8
            );
            data.push(byte);
        }
    }
    
    Ok(data)
}
```

读操作的流程包括：
1. 等待控制器空闲并清除中断标志
2. 设置目标设备地址（读模式）
3. 设置要读取的数据长度
4. 配置控制寄存器启动读取
5. 等待读取完成
6. 从接收FIFO中读取数据

### 公共接口

为了便于使用，驱动提供了简洁的公共接口：

```rust
/// 执行写操作
pub fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
    if addr > 0x7f && addr < 0x78 {
        return Err(I2cError::InvalidAddr);
    }
    
    self.write_internal(addr, data)
}

/// 执行读操作
pub fn read(&mut self, addr: u8, len: u8) -> Result<Vec<u8>, I2cError> {
    if addr > 0x7f && addr < 0x78 {
        return Err(I2cError::InvalidAddr);
    }
    
    self.read_internal(addr, len)
}

/// 执行写后读操作
pub fn write_read(
    &mut self,
    addr: u8,
    bytes: &[u8],
    read_len: u8,
) -> Result<Vec<u8>, I2cError> {
    self.write(addr, bytes)?;
    self.read(addr, read_len)
}
```

这些接口进行了地址有效性检查，并调用相应的内部函数完成操作。

## 全局实例管理

为了方便在系统中使用，StarryOS为每个I2C控制器创建了全局实例：

```rust
/// 全局I2C实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref I2C0: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C0_BASE, 100));
    pub static ref I2C1: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C1_BASE, 100));
    // ... 其他I2C控制器
}
```

这种设计使用了[lazy_static]宏来创建静态全局变量，并通过[spin::Mutex]保证线程安全。

## 系统集成

在系统初始化阶段，所有I2C控制器都会被初始化：

```rust
/// 初始化所有I2C控制器
pub fn i2c_init_all(apb_freq_mhz: u32) {
    for i2c in [
        I2C0.lock(),
        I2C1.lock(),
        // ... 其他I2C控制器
    ] {
        let _ = i2c.init(apb_freq_mhz);
    }
}
```

这种集中初始化的方式确保了所有外设在系统启动时都处于正确的状态。

## 错误处理

良好的错误处理是高质量驱动的重要特征。StarryOS的I2C驱动定义了详细的错误类型：

```rust
/// I2C错误类型
#[derive(Debug, Clone, Copy)]
pub enum I2cError {
    /// 总线被占用
    BusBusy,
    /// 从设备应答失败
    NoAck,
    /// 数据冲突
    DataConflict,
    /// 超时
    Timeout,
    /// 地址错误
    InvalidAddr,
}
```

每种错误类型都有明确的含义，便于上层代码进行针对性处理。

## 测试验证

为了确保驱动的正确性，StarryOS包含了单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_i2c_new() {
        let i2c = I2c::new(I2C0_BASE, 100);
        assert_eq!(i2c.base, I2C0_BASE);
        assert_eq!(i2c.freq_khz, 100);
    }
    
    #[test]
    fn test_clock_div_calculation() {
        // APB: 24MHz, I2C: 100kHz
        // div = (24*1000 / (2*100)) - 1 = 119
        let div = ((24 * 1000) / (2 * 100)) - 1;
        assert_eq!(div, 119);
    }
}
```

这些测试验证了基本功能的正确性。

## 总结

本文深入分析了StarryOS RK3588系统中I2C驱动的实现细节。通过遵循embedded-hal规范，该驱动获得了良好的可移植性和兼容性。在硬件层面，驱动通过对RK3588 I2C控制器寄存器的精确控制，实现了稳定可靠的I2C通信。

I2C驱动的成功实现为系统中其他依赖I2C的组件（如传感器、EEPROM等）提供了坚实的基础。在下一文中，我们将探讨CAN总线驱动的实现及其在实时通信中的应用。