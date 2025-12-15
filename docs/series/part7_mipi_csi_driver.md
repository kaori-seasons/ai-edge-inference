# StarryOS RK3588 边缘AI系统架构深度解析（七）：MIPI-CSI摄像头驱动链

## 引言

在前面的文章中，我们探讨了CAN总线驱动的实现实时通信机制。本文将深入分析MIPI-CSI摄像头驱动链的复杂实现，这是StarryOS系统中最为复杂和关键的驱动之一。

MIPI-CSI（Camera Serial Interface）是移动行业处理器接口联盟制定的摄像头串行接口标准，广泛应用于智能手机、平板电脑和嵌入式视觉系统中。在边缘AI应用中，高质量的图像采集是实现准确目标识别的前提。

## MIPI-CSI基础概念

### MIPI-CSI协议特点

MIPI-CSI协议具有以下关键特性：

1. **高速串行传输**：支持高达6Gbps的传输速率
2. **低功耗设计**：支持多种省电模式
3. **灵活性**：支持多种数据格式和像素类型
4. **可扩展性**：支持多Lane配置（1-4 Lane）

### RK3588 MIPI-CSI架构

RK3588芯片集成了4个MIPI CSI-2接收器，每个接收器支持：
- 最高4个Lane配置
- 最大数据速率1.5Gbps/Lane
- 支持RAW8/RAW10/RAW12等多种像素格式
- 集成ISP/CIF硬件处理器

## StarryOS MIPI-CSI驱动实现

### 驱动架构设计

StarryOS的MIPI-CSI驱动采用了分层架构设计：

```rust
/// MIPI-CSI 驱动
pub struct MipiCsi {
    dphy_base: u64,
    csi2_base: u64,
    isp_base: u64,
    
    /// 视频队列 (V4L2 模型)
    queue: VideoQueue,
}
```

驱动分为三个主要组件：
1. **MIPI D-PHY**：物理层接口
2. **CSI-2接收器**：协议层处理
3. **ISP/CIF处理器**：图像处理和DMA传输

### 核心数据结构

#### 帧缓冲管理

帧缓冲是图像采集的核心数据结构：

```rust
/// 帧缓冲状态
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameState {
    /// 空闲，等待采集
    Empty = 0,
    /// 已排队，等待DMA
    Queued = 1,
    /// 正在采集
    Capturing = 2,
    /// 采集完成
    Done = 3,
    /// 错误状态
    Error = 4,
}

/// 单个帧缓冲
pub struct FrameBuffer {
    /// 缓冲区物理地址 (DMA 地址)
    pub phys_addr: u64,
    
    /// 虚拟地址 (内核映射)
    pub virt_addr: u64,
    
    /// 缓冲区大小 (字节)
    pub size: usize,
    
    /// 当前状态
    pub state: FrameState,
    
    /// 时间戳 (纳秒)
    pub timestamp_ns: u64,
    
    /// 实际数据大小
    pub data_size: usize,
}
```

#### V4L2风格视频队列

为了高效管理帧缓冲，驱动实现了V4L2风格的视频队列：

```rust
/// V4L2 风格的视频队列
pub struct VideoQueue {
    /// 帧缓冲池
    frames: Vec<FrameBuffer>,
    
    /// 待采集队列 (生产者端)
    ready_queue: Vec<usize>,
    
    /// 已完成队列 (消费者端)
    done_queue: Vec<usize>,
    
    /// 当前正在采集的帧索引
    current_frame_index: Option<usize>,
}
```

### 初始化流程

#### MIPI D-PHY初始化

D-PHY是MIPI-CSI的物理层接口，负责高速数据传输：

```rust
/// 初始化 MIPI D-PHY
pub fn init_dphy(&self) {
    unsafe {
        // 1. 启用 D-PHY 时钟
        write_volatile((self.dphy_base + DPHY_CTRL) as *mut u32, 0x00000001);
        
        // 2. 配置 Lane 时序
        // Lane Escape Clock 频率配置
        write_volatile((self.dphy_base + DPHY_LANE_TIMING) as *mut u32, 0x00000014);
        
        // 3. 等待 PHY 就绪
        let mut timeout = 10000;
        loop {
            let status = read_volatile((self.dphy_base + DPHY_STATUS) as *const u32);
            if (status & 1) != 0 {  // PHY ready
                break;
            }
            timeout -= 1;
            if timeout == 0 {
                break;
            }
        }
    }
}
```

#### CSI-2接收器初始化

CSI-2接收器负责协议层处理：

```rust
/// 初始化 CSI-2 接收器
pub fn init_csi2(&self) {
    unsafe {
        // 1. 启用 CSI-2 接收器
        write_volatile((self.csi2_base + CSI2_CTRL) as *mut u32, 0x00000001);
        
        // 2. 配置数据类型 (RAW8, RAW10, RAW12 等)
        // 假设使用 RAW8 格式
        write_volatile((self.csi2_base + CSI2_DATA_TYPE) as *mut u32, 0x00000008);
        
        // 3. 清除中断标志
        write_volatile((self.csi2_base + CSI2_INT_STATUS) as *mut u32, 0xFFFFFFFF);
    }
}
```

### 三缓冲V4L2模型

为了实现高效的图像采集，StarryOS采用了三缓冲V4L2模型：

```rust
// 三缓冲方案管理DMA帧采集：
// - 缓冲区A：正在采集（MIPI-CSI DMA目标）
// - 缓冲区B：应用处理中（预处理）
// - 缓冲区C：待重用（等待下一帧）
```

这种设计的优势包括：
1. **零拷贝**：直接DMA到应用缓冲区
2. **流水线处理**：采集、处理、重用并行进行
3. **低延迟**：队列操作开销小于1.2µs per frame

### 帧采集流程

#### 队列操作

帧采集流程从队列操作开始：

```rust
/// 将帧加入采集队列 (排队)
pub fn queue_buffer(&mut self, frame_index: usize) -> Result<(), &'static str> {
    if frame_index >= self.frames.len() {
        return Err("Frame index out of bounds");
    }
    
    self.frames[frame_index].state = FrameState::Queued;
    self.ready_queue.push(frame_index);
    Ok(())
}

/// 从采集队列取出帧 (准备DMA)
pub fn dequeue_ready(&mut self) -> Option<usize> {
    if let Some(index) = self.ready_queue.pop() {
        self.frames[index].state = FrameState::Capturing;
        self.current_frame_index = Some(index);
        Some(index)
    } else {
        None
    }
}
```

#### DMA配置

DMA配置是高性能图像采集的关键：

```rust
/// 配置 DMA 描述符
pub fn config_dma(&self, frame_index: usize) -> Result<(), &'static str> {
    let frame = self.queue.get_frame(frame_index)
        .ok_or("Frame not found")?;
    
    unsafe {
        // ISP 的 DMA 地址配置 (具体寄存器因芯片而异)
        // 这里是示意性的实现
        write_volatile((self.isp_base + 0x100) as *mut u64, frame.phys_addr);
        write_volatile((self.isp_base + 0x108) as *mut u32, frame.size as u32);
    }
    
    Ok(())
}
```

#### 启动采集

启动采集过程：

```rust
/// 启动帧采集
pub fn start_capture(&mut self) -> Result<(), &'static str> {
    // 1. 从准备队列取出帧
    if let Some(frame_index) = self.queue.dequeue_ready() {
        // 2. 配置 DMA
        self.config_dma(frame_index)?;
        
        // 3. 启动采集 (设置 ISP/CIF 控制寄存器)
        unsafe {
            write_volatile((self.isp_base + 0x0) as *mut u32, 0x00000001);
        }
        
        Ok(())
    } else {
        Err("No frame ready for capture")
    }
}
```

#### 中断处理

帧采集完成后的中断处理：

```rust
/// 处理帧采集完成中断
pub fn on_frame_done(&mut self) -> Result<(), &'static str> {
    unsafe {
        // 1. 获取实际采集的数据大小
        let data_size = read_volatile((self.isp_base + 0x10C) as *const u32) as usize;
        
        // 2. 更新帧信息
        if let Some(index) = self.queue.current_frame_index {
            if let Some(frame) = self.queue.get_frame_mut(index) {
                frame.data_size = data_size;
                frame.timestamp_ns = get_time_ns();
            }
        }
    }
    
    // 3. 标记帧完成
    self.queue.frame_done()?;
    
    // 4. 清除 ISP 中断标志
    unsafe {
        write_volatile((self.isp_base + 0x18) as *mut u32, 0x00000001);
    }
    
    Ok(())
}
```

### 数据访问接口

为应用程序提供便捷的数据访问接口：

```rust
/// 获取采集完成的帧
pub fn get_captured_frame(&mut self) -> Option<usize> {
    self.queue.dequeue_done()
}

/// 获取帧缓冲数据
pub fn get_frame_data(&self, frame_index: usize) -> Option<&[u8]> {
    if let Some(frame) = self.queue.get_frame(frame_index) {
        unsafe {
            Some(core::slice::from_raw_parts(
                frame.virt_addr as *const u8,
                frame.data_size,
            ))
        }
    } else {
        None
    }
}
```

## 全局实例管理

为了方便在系统中使用，StarryOS为每个MIPI-CSI控制器创建了全局实例：

```rust
/// 全局 MIPI-CSI 实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref MIPI_CSI0: spin::Mutex<MipiCsi> = 
        spin::Mutex::new(MipiCsi::new(MIPI_DPHY0_BASE, CSI2_RX0_BASE, ISP_BASE, 3));
    pub static ref MIPI_CSI1: spin::Mutex<MipiCsi> = 
        spin::Mutex::new(MipiCsi::new(MIPI_DPHY1_BASE, CSI2_RX1_BASE, ISP_BASE, 3));
    // ... 其他MIPI-CSI控制器
}
```

## 系统集成

在系统初始化阶段，所有MIPI-CSI控制器都会被初始化：

```rust
/// 初始化所有 MIPI-CSI 控制器
pub fn mipi_csi_init_all() {
    for csi in [
        MIPI_CSI0.lock(),
        MIPI_CSI1.lock(),
        // ... 其他MIPI-CSI控制器
    ] {
        csi.init_dphy();
        csi.init_csi2();
    }
}
```

## 性能优化

### 高效队列操作

队列操作经过精心优化，确保低延迟：

```rust
// 队列操作开销<1.2µs per frame（原子操作+DMA配置）
```

### 内存管理

通过物理地址直接DMA，避免不必要的内存拷贝：

```rust
/// 缓冲区物理地址 (DMA 地址)
pub phys_addr: u64,

/// 虚拟地址 (内核映射)
pub virt_addr: u64,
```

## 测试验证

为了确保驱动的正确性，StarryOS包含了单元测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_frame_buffer() {
        let frame = FrameBuffer::new(0x80000000, 0x10000000, 2097152);
        assert_eq!(frame.state, FrameState::Empty);
        assert_eq!(frame.size, 2097152);
    }
    
    #[test]
    fn test_video_queue() {
        let mut queue = VideoQueue::new(3);
        let frame = FrameBuffer::new(0x80000000, 0x10000000, 2097152);
        let idx = queue.add_frame(frame);
        
        queue.queue_buffer(idx).unwrap();
        let ready = queue.dequeue_ready();
        assert_eq!(ready, Some(idx));
    }
}
```

## 总结

本文深入分析了StarryOS RK3588系统中MIPI-CSI摄像头驱动链的复杂实现。通过采用V4L2风格的三缓冲模型、高效的队列管理和直接DMA传输，该驱动能够实现高性能的图像采集。

MIPI-CSI驱动的成功实现为系统的AI推理能力提供了高质量的图像输入源。在下一文中，我们将探讨RKNN NPU安全FFI封装的实现细节。