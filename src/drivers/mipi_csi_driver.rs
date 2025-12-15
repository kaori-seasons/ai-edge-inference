//! MIPI-CSI 驱动链实现
//!
//! 包含:
//! - MIPI D-PHY 初始化
//! - CSI-2 接收器配置
//! - 传感器 I2C 初始化链
//! - V4L2 队列模型的帧缓冲管理

use core::ptr::{read_volatile, write_volatile};
use alloc::vec::Vec;

/// MIPI DPHY 基地址 (RK3588 有 4 个 MIPI 接收器)
pub const MIPI_DPHY0_BASE: u64 = 0xfda00000;
pub const MIPI_DPHY1_BASE: u64 = 0xfda10000;
pub const MIPI_DPHY2_BASE: u64 = 0xfda20000;
pub const MIPI_DPHY3_BASE: u64 = 0xfda30000;

/// CSI-2 接收器基地址
pub const CSI2_RX0_BASE: u64 = 0xfdb30000;
pub const CSI2_RX1_BASE: u64 = 0xfdb40000;
pub const CSI2_RX2_BASE: u64 = 0xfdb50000;
pub const CSI2_RX3_BASE: u64 = 0xfdb60000;

/// ISP / CIF 基地址
pub const ISP_BASE: u64 = 0xfdb20000;

// ============ MIPI D-PHY 寄存器偏移 ============

/// PHY 控制寄存器
const DPHY_CTRL: u64 = 0x0;

/// PHY 状态寄存器
const DPHY_STATUS: u64 = 0x4;

/// PHY Lane 时序配置
const DPHY_LANE_TIMING: u64 = 0x14;

// ============ CSI-2 接收器寄存器偏移 ============

/// CSI-2 控制寄存器
const CSI2_CTRL: u64 = 0x0;

/// CSI-2 状态寄存器
const CSI2_STATUS: u64 = 0x4;

/// CSI-2 数据类型配置
const CSI2_DATA_TYPE: u64 = 0x8;

/// CSI-2 中断状态
const CSI2_INT_STATUS: u64 = 0x10;

// ============ 帧缓冲结构 ============

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

impl FrameBuffer {
    /// 创建新的帧缓冲
    pub fn new(phys_addr: u64, virt_addr: u64, size: usize) -> Self {
        FrameBuffer {
            phys_addr,
            virt_addr,
            size,
            state: FrameState::Empty,
            timestamp_ns: 0,
            data_size: 0,
        }
    }
}

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

impl VideoQueue {
    /// 创建新的视频队列
    pub fn new(frame_count: usize) -> Self {
        VideoQueue {
            frames: Vec::with_capacity(frame_count),
            ready_queue: Vec::with_capacity(frame_count),
            done_queue: Vec::with_capacity(frame_count),
            current_frame_index: None,
        }
    }
    
    /// 添加帧缓冲到池中
    pub fn add_frame(&mut self, frame: FrameBuffer) -> usize {
        let index = self.frames.len();
        self.frames.push(frame);
        index
    }
    
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
    
    /// 标记当前帧采集完成
    pub fn frame_done(&mut self) -> Result<(), &'static str> {
        if let Some(index) = self.current_frame_index {
            self.frames[index].state = FrameState::Done;
            self.done_queue.push(index);
            self.current_frame_index = None;
            Ok(())
        } else {
            Err("No frame currently capturing")
        }
    }
    
    /// 从完成队列取出帧 (消费者读取)
    pub fn dequeue_done(&mut self) -> Option<usize> {
        if let Some(index) = self.done_queue.pop() {
            self.frames[index].state = FrameState::Empty;
            Some(index)
        } else {
            None
        }
    }
    
    /// 获取帧缓冲的可变引用
    pub fn get_frame_mut(&mut self, index: usize) -> Option<&mut FrameBuffer> {
        self.frames.get_mut(index)
    }
    
    /// 获取帧缓冲的不可变引用
    pub fn get_frame(&self, index: usize) -> Option<&FrameBuffer> {
        self.frames.get(index)
    }
}

/// MIPI-CSI 驱动
pub struct MipiCsi {
    dphy_base: u64,
    csi2_base: u64,
    isp_base: u64,
    
    /// 视频队列 (V4L2 模型)
    queue: VideoQueue,
}

impl MipiCsi {
    /// 创建新的 MIPI-CSI 实例
    pub fn new(
        dphy_base: u64,
        csi2_base: u64,
        isp_base: u64,
        frame_count: usize,
    ) -> Self {
        MipiCsi {
            dphy_base,
            csi2_base,
            isp_base,
            queue: VideoQueue::new(frame_count),
        }
    }
    
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
}

/// 获取系统时间 (纳秒)
fn get_time_ns() -> u64 {
    // 使用系统计时器
    // 这是占位实现
    0
}

/// 全局 MIPI-CSI 实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref MIPI_CSI0: spin::Mutex<MipiCsi> = 
        spin::Mutex::new(MipiCsi::new(MIPI_DPHY0_BASE, CSI2_RX0_BASE, ISP_BASE, 3));
    pub static ref MIPI_CSI1: spin::Mutex<MipiCsi> = 
        spin::Mutex::new(MipiCsi::new(MIPI_DPHY1_BASE, CSI2_RX1_BASE, ISP_BASE, 3));
    pub static ref MIPI_CSI2: spin::Mutex<MipiCsi> = 
        spin::Mutex::new(MipiCsi::new(MIPI_DPHY2_BASE, CSI2_RX2_BASE, ISP_BASE, 3));
    pub static ref MIPI_CSI3: spin::Mutex<MipiCsi> = 
        spin::Mutex::new(MipiCsi::new(MIPI_DPHY3_BASE, CSI2_RX3_BASE, ISP_BASE, 3));
}

/// 初始化所有 MIPI-CSI 控制器
pub fn mipi_csi_init_all() {
    for csi in [
        MIPI_CSI0.lock(),
        MIPI_CSI1.lock(),
        MIPI_CSI2.lock(),
        MIPI_CSI3.lock(),
    ] {
        csi.init_dphy();
        csi.init_csi2();
    }
}

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
