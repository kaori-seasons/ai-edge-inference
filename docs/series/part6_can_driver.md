# StarryOS RK3588 边缘AI系统架构深度解析（六）：CAN总线驱动与实时通信

## 引言

在前面的文章中，我们探讨了I2C驱动的实现及其如何遵循embedded-hal规范。本文将深入分析CAN总线驱动的实现细节，重点关注其在实时通信中的应用和优化。

CAN（Controller Area Network）总线是一种广泛应用在汽车和工业控制领域的串行通信协议。它以其高可靠性、抗干扰能力和实时性而著称，特别适用于对时间敏感的应用场景。

## CAN总线基础概念

### CAN协议特点

CAN总线具有以下关键特性：

1. **多主机架构**：总线上任何节点都可以主动发送数据
2. **仲裁机制**：通过ID优先级解决总线冲突
3. **错误检测与纠正**：内置多种错误检测机制
4. **实时性**：支持高优先级消息的快速传输

### CAN帧结构

CAN协议定义了两种帧格式：
1. **标准帧**：11位标识符
2. **扩展帧**：29位标识符

每帧包含以下部分：
- 帧起始位
- 仲裁段（包含标识符和远程传输请求位）
- 控制段（包含数据长度码）
- 数据段（0-8字节）
- CRC段
- 应答段
- 帧结束位

## StarryOS CAN驱动实现

### RK3588 CAN控制器架构

RK3588芯片集成了FlexCAN控制器，这是一种高性能的CAN控制器，支持标准和扩展帧格式。控制器提供了16个消息缓冲区，其中：
- MB0-MB7：用于接收消息
- MB8-MB15：用于发送消息

### 核心数据结构

CAN驱动的核心数据结构包括[Can]控制器结构体和[CanFrame]帧结构体：

```rust
/// CAN驱动
pub struct Can {
    base: u64,
    /// 波特率 (kbps)
    bitrate: u32,
}

/// CAN帧结构
#[derive(Debug, Clone)]
pub struct CanFrame {
    /// 帧标识符
    pub id: u32,
    /// 帧类型
    pub frame_type: CanFrameType,
    /// 数据长度 (0-8)
    pub dlc: u8,
    /// 数据字节
    pub data: [u8; 8],
    /// 是否为远程帧
    pub is_rtr: bool,
}
```

### 寄存器映射

CAN控制器通过内存映射I/O（MMIO）方式进行控制，关键寄存器包括：

```rust
// ============ CAN寄存器偏移 ============
/// 模块控制寄存器
const CAN_MCR: u64 = 0x0;

/// 控制寄存器
const CAN_CTRL1: u64 = 0x4;

/// 中断使能寄存器
const CAN_IMASK1: u64 = 0x28;

/// 中断标志寄存器
const CAN_IFLAG1: u64 = 0x30;

/// CAN消息缓冲起始地址
const CAN_MB_START: u64 = 0x80;
```

### 初始化流程

CAN控制器的初始化是驱动工作的第一步：

```rust
/// 初始化CAN控制器
pub fn init(&self, clk_freq_mhz: u32) -> Result<(), CanError> {
    unsafe {
        // 1. 进入配置模式 (MCR[HALT] = 1)
        let mut mcr = read_volatile((self.base + CAN_MCR) as *const u32);
        mcr |= 1 << 28; // HALT
        write_volatile((self.base + CAN_MCR) as *mut u32, mcr);
        
        // 等待进入冻结模式
        let mut timeout = 10000;
        loop {
            let esr = read_volatile((self.base + CAN_ESR1) as *const u32);
            if (esr & (1 << 24)) != 0 {  // FRZ_ACK
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                return Err(CanError::Timeout);
            }
        }
        
        // 2. 计算波特率分频
        let time_quanta = 10u32;
        let baudrate_div = clk_freq_mhz * 1000 / (self.bitrate * time_quanta);
        
        // 3. 配置CTRL1寄存器
        let mut ctrl1: u32 = 0;
        // PRESDIV: 波特率分频-1
        ctrl1 |= (baudrate_div - 1) & 0xFF;
        // RJW: 重新同步宽度
        ctrl1 |= 2 << 10;
        // PSEG1: 段1时间量
        ctrl1 |= 5 << 16;
        // PSEG2: 段2时间量
        ctrl1 |= 2 << 20;
        // BOFF_MSK: 总线关闭中断使能
        ctrl1 |= 1 << 15;
        // ERR_MSK: 错误中断使能
        ctrl1 |= 1 << 14;
        
        write_volatile((self.base + CAN_CTRL1) as *mut u32, ctrl1);
        
        // 4. 配置消息缓冲 (MB0-MB15)
        // MB0-7: 接收
        // MB8-15: 发送
        for i in 0..16 {
            let mb_addr = self.base + CAN_MB_START + (i as u64) * MB_SIZE;
            
            if i < 8 {
                // 接收消息缓冲
                // CODE: 0100 = 接收空闲
                write_volatile((mb_addr + MB_CS) as *mut u32, 0x40000000);
            } else {
                // 发送消息缓冲
                // CODE: 1000 = 发送非活跃
                write_volatile((mb_addr + MB_CS) as *mut u32, 0x80000000);
            }
        }
        
        // 5. 启用所有消息缓冲中断
        write_volatile((self.base + CAN_IMASK1) as *mut u32, 0xFFFF);
        
        // 6. 清除所有中断标志
        write_volatile((self.base + CAN_IFLAG1) as *mut u32, 0xFFFF);
        
        // 7. 退出配置模式
        mcr = read_volatile((self.base + CAN_MCR) as *const u32);
        mcr &= !(1 << 28); // 清除HALT
        write_volatile((self.base + CAN_MCR) as *mut u32, mcr);
    }
    
    Ok(())
}
```

初始化过程包括：
1. 进入配置模式
2. 计算并配置波特率分频
3. 配置控制寄存器
4. 初始化消息缓冲区
5. 启用中断
6. 退出配置模式

### 发送操作实现

发送操作是CAN通信的核心功能之一：

```rust
/// 发送CAN帧
pub fn send(&self, frame: &CanFrame) -> Result<(), CanError> {
    unsafe {
        // 查找空闲的发送消息缓冲 (MB8-15)
        let mut mb_index = None;
        for i in 8..16 {
            let mb_addr = self.base + CAN_MB_START + (i as u64) * MB_SIZE;
            let cs = read_volatile((mb_addr + MB_CS) as *const u32);
            
            // CODE: 1000 = 发送非活跃 (准备好)
            if (cs >> 24) == 0x08 {
                mb_index = Some(i);
                break;
            }
        }
        
        let mb_index = mb_index.ok_or(CanError::BusBusy)?;
        let mb_addr = self.base + CAN_MB_START + (mb_index as u64) * MB_SIZE;
        
        // 1. 写入ID
        let mut can_id: u32 = 0;
        match frame.frame_type {
            CanFrameType::Standard => {
                can_id = (frame.id & 0x7FF) << 18;  // 11-bit ID in bits [28:18]
            }
            CanFrameType::Extended => {
                can_id = frame.id & 0x1FFFFFFF;     // 29-bit ID
                can_id |= 1 << 31;                  // IDE标志
            }
        }
        write_volatile((mb_addr + MB_ID) as *mut u32, can_id);
        
        // 2. 写入数据
        for i in 0..frame.dlc.min(8) as u64 {
            write_volatile(
                (mb_addr + MB_DATA + i) as *mut u8,
                frame.data[i as usize],
            );
        }
        
        // 3. 写入控制/状态字
        let mut cs: u32 = 0;
        // CODE: 1100 = 发送数据或远程帧
        cs |= 0xC << 24;
        // DLC: 数据长度
        cs |= (frame.dlc as u32 & 0xF) << 16;
        // RTR: 远程传输请求
        if frame.is_rtr {
            cs |= 1 << 20;
        }
        
        write_volatile((mb_addr + MB_CS) as *mut u32, cs);
    }
    
    Ok(())
}
```

发送操作的流程包括：
1. 查找空闲的发送消息缓冲区
2. 构造并写入帧标识符
3. 写入数据字段
4. 配置控制/状态字以启动发送

### 接收操作实现

接收操作同样重要，特别是在实时控制系统中：

```rust
/// 接收CAN帧
pub fn recv(&self) -> Option<CanFrame> {
    unsafe {
        // 查找包含数据的接收消息缓冲 (MB0-7)
        for i in 0..8 {
            let mb_addr = self.base + CAN_MB_START + (i as u64) * MB_SIZE;
            let cs = read_volatile((mb_addr + MB_CS) as *const u32);
            
            // CODE: 0010 = 接收已填充数据
            if (cs >> 24) == 0x02 {
                let can_id = read_volatile((mb_addr + MB_ID) as *const u32);
                let dlc = ((cs >> 16) & 0xF) as u8;
                let is_rtr = ((cs >> 20) & 1) != 0;
                
                // 读取数据
                let mut data = [0u8; 8];
                for j in 0..dlc.min(8) as u64 {
                    data[j as usize] =
                        read_volatile((mb_addr + MB_DATA + j) as *const u8);
                }
                
                // 清除中断标志
                write_volatile((self.base + CAN_IFLAG1) as *mut u32, 1 << i);
                
                // 确定帧类型
                let frame_type = if (can_id >> 31) & 1 != 0 {
                    CanFrameType::Extended
                } else {
                    CanFrameType::Standard
                };
                
                let id = if matches!(frame_type, CanFrameType::Extended) {
                    can_id & 0x1FFFFFFF
                } else {
                    (can_id >> 18) & 0x7FF
                };
                
                return Some(CanFrame {
                    id,
                    frame_type,
                    dlc,
                    data,
                    is_rtr,
                });
            }
        }
    }
    
    None
}
```

接收操作的流程包括：
1. 遍历接收消息缓冲区查找已接收的数据
2. 读取帧标识符、数据长度和数据字段
3. 清除中断标志
4. 构造并返回CAN帧对象

## 实时性优化

### 中断驱动架构

为了确保实时性，StarryOS的CAN驱动采用了中断驱动架构：

```rust
/// 启用所有消息缓冲中断
write_volatile((self.base + CAN_IMASK1) as *mut u32, 0xFFFF);

/// 清除所有中断标志
write_volatile((self.base + CAN_IFLAG1) as *mut u32, 0xFFFF);
```

通过启用中断，系统可以在CAN消息到达时立即得到通知，而不需要轮询检查。

### 优先级管理

在RK3588平台上，CAN驱动的中断优先级被设置为高位：

```rust
// 在GIC初始化时设置CAN中断优先级为HIGH (GIC优先级=16)
```

这种配置确保了高优先级的CAN消息能够得到及时处理。

### 环形缓冲区

为了进一步提高实时性，驱动使用环形缓冲区管理消息队列：

```rust
// 在实际应用中，可以使用类似以下的环形缓冲区实现
struct RingBuffer<T, const N: usize> {
    buffer: [Option<T>; N],
    head: usize,
    tail: usize,
    count: usize,
}
```

环形缓冲区提供了高效的FIFO操作，减少了内存分配的开销。

## 全局实例管理

为了方便在系统中使用，StarryOS为每个CAN控制器创建了全局实例：

```rust
/// 全局CAN实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref CAN0: spin::Mutex<Can> = spin::Mutex::new(Can::new(CAN0_BASE, 1000));
    pub static ref CAN1: spin::Mutex<Can> = spin::Mutex::new(Can::new(CAN1_BASE, 1000));
}
```

## 系统集成

在系统初始化阶段，所有CAN控制器都会被初始化：

```rust
/// 初始化所有CAN控制器
pub fn can_init_all(clk_freq_mhz: u32) {
    for can in [CAN0.lock(), CAN1.lock()] {
        let _ = can.init(clk_freq_mhz);
    }
}
```

## 错误处理

良好的错误处理是高质量驱动的重要特征。StarryOS的CAN驱动定义了详细的错误类型：

```rust
/// CAN错误类型
#[derive(Debug, Clone, Copy)]
pub enum CanError {
    /// 消息缓冲忙碌
    BusBusy,
    /// 发送超时
    Timeout,
    /// 帧错误
    FrameError,
    /// 总线关闭
    BusOff,
}
```

## 测试验证

为了确保驱动的正确性，StarryOS包含了单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_can_frame_new() {
        let frame = CanFrame::new(0x123, 8);
        assert_eq!(frame.id, 0x123);
        assert_eq!(frame.dlc, 8);
        assert!(!frame.is_rtr);
    }
    
    #[test]
    fn test_can_frame_set_data() {
        let mut frame = CanFrame::new(0x456, 0);
        let data = [1, 2, 3, 4];
        frame.set_data(&data);
        assert_eq!(frame.dlc, 4);
        assert_eq!(frame.data[0], 1);
        assert_eq!(frame.data[3], 4);
    }
}
```

## 总结

本文深入分析了StarryOS RK3588系统中CAN总线驱动的实现细节。通过采用中断驱动架构、合理的优先级管理和高效的缓冲区设计，该驱动能够满足实时控制系统的需求。

CAN驱动的成功实现为系统中的执行器控制和其他实时通信任务提供了坚实的基础。在下一文中，我们将探讨MIPI-CSI摄像头驱动链的复杂实现。