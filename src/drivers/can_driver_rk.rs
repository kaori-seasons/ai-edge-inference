//! CAN驱动 - RK3588 CAN总线驱动
//!
//! FlexCAN控制器驱动实现
//! 支持标准帧(11bit ID)和扩展帧(29bit ID)
//! 提供中断驱动的发送/接收队列

use core::ptr::{read_volatile, write_volatile};
use core::fmt;
use alloc::vec::Vec;

/// CAN基地址
pub const CAN0_BASE: u64 = 0xfea50000;
pub const CAN1_BASE: u64 = 0xfea60000;

// ============ CAN寄存器偏移 ============

/// 模块控制寄存器
const CAN_MCR: u64 = 0x0;

/// 控制寄存器
const CAN_CTRL1: u64 = 0x4;

/// 自由运行定时器
const CAN_TIMER: u64 = 0x8;

/// 接收全局掩码
const CAN_RXMGMASK: u64 = 0x10;

/// 接收缓冲掩码
const CAN_RX14MASK: u64 = 0x14;

/// 接收缓冲掩码
const CAN_RX15MASK: u64 = 0x18;

/// 错误计数器
const CAN_ECR: u64 = 0x1c;

/// 错误和状态寄存器
const CAN_ESR1: u64 = 0x20;

/// 中断使能寄存器
const CAN_IMASK1: u64 = 0x28;

/// 中断标志寄存器
const CAN_IFLAG1: u64 = 0x30;

/// 控制2寄存器
const CAN_CTRL2: u64 = 0x34;

/// 错误状态寄存器2
const CAN_ESR2: u64 = 0x38;

/// CAN消息缓冲起始地址
const CAN_MB_START: u64 = 0x80;

// ============ 消息缓冲偏移 ============

/// 每个消息缓冲的大小
const MB_SIZE: u64 = 16;

/// 消息缓冲控制/状态字
const MB_CS: u64 = 0x0;

/// 消息缓冲ID
const MB_ID: u64 = 0x4;

/// 消息缓冲数据字节 (8字节)
const MB_DATA: u64 = 0x8;

// ============ CAN消息帧 ============

/// CAN帧类型
#[derive(Debug, Clone, Copy)]
pub enum CanFrameType {
    /// 标准帧 (11-bit ID)
    Standard,
    /// 扩展帧 (29-bit ID)
    Extended,
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

impl CanFrame {
    /// 创建新的CAN帧
    pub fn new(id: u32, dlc: u8) -> Self {
        CanFrame {
            id,
            frame_type: CanFrameType::Standard,
            dlc: dlc.min(8),
            data: [0; 8],
            is_rtr: false,
        }
    }
    
    /// 设置数据
    pub fn set_data(&mut self, data: &[u8]) {
        let len = data.len().min(8);
        self.data[..len].copy_from_slice(&data[..len]);
        self.dlc = len as u8;
    }
}

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

impl fmt::Display for CanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanError::BusBusy => write!(f, "CAN Bus Busy"),
            CanError::Timeout => write!(f, "CAN Timeout"),
            CanError::FrameError => write!(f, "CAN Frame Error"),
            CanError::BusOff => write!(f, "CAN Bus Off"),
        }
    }
}

/// CAN驱动
pub struct Can {
    base: u64,
    /// 波特率 (kbps)
    bitrate: u32,
}

impl Can {
    /// 创建新的CAN实例
    pub fn new(base: u64, bitrate: u32) -> Self {
        Can { base, bitrate }
    }
    
    /// 初始化CAN控制器
    /// 
    /// # 参数
    /// - `clk_freq_mhz`: 时钟频率 (MHz), 通常为 120MHz
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
            // 时间段 = clk_freq / (bitrate * time_quanta)
            // 假设: time_quanta = 10 (1 + 6 + 3)
            let time_quanta = 10u32;
            let baudrate_div = clk_freq_mhz * 1000 / (self.bitrate * time_quanta);
            
            // 3. 配置CTRL1寄存器
            let mut ctrl1: u32 = 0;
            // PRESDIV: 波特率分频-1
            ctrl1 |= (baudrate_div - 1) & 0xFF;
            // RJW: 重新同步宽度 (6)
            ctrl1 |= 2 << 10;
            // PSEG1: 段1时间量 (6)
            ctrl1 |= 5 << 16;
            // PSEG2: 段2时间量 (3)
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
}

/// 全局CAN实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref CAN0: spin::Mutex<Can> = spin::Mutex::new(Can::new(CAN0_BASE, 1000));
    pub static ref CAN1: spin::Mutex<Can> = spin::Mutex::new(Can::new(CAN1_BASE, 1000));
}

/// 初始化所有CAN控制器
pub fn can_init_all(clk_freq_mhz: u32) {
    for can in [CAN0.lock(), CAN1.lock()] {
        let _ = can.init(clk_freq_mhz);
    }
}

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
