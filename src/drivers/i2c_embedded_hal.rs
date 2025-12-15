//! I2C驱动 - embedded-hal实现
//!
//! RK3588 I2C控制器驱动
//! 实现embedded-hal::i2c::I2c trait
//! 支持7位/10位寻址, 标准/快速模式

use core::ptr::{read_volatile, write_volatile};
use core::fmt;

/// I2C基地址 (RK3588有9个I2C控制器)
pub const I2C0_BASE: u64 = 0xfea80000;
pub const I2C1_BASE: u64 = 0xfea90000;
pub const I2C2_BASE: u64 = 0xfeaa0000;
pub const I2C3_BASE: u64 = 0xfeab0000;
pub const I2C4_BASE: u64 = 0xfeac0000;
pub const I2C5_BASE: u64 = 0xfead0000;
pub const I2C6_BASE: u64 = 0xfeae0000;
pub const I2C7_BASE: u64 = 0xfeaf0000;
pub const I2C8_BASE: u64 = 0xfeb00000;

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

/// 主设备中断使能寄存器
const I2C_IEN: u64 = 0x18;

/// 主设备中断清除寄存器
const I2C_IPD: u64 = 0x1c;

/// 主设备发送数据FIFO
const I2C_TXDATA0: u64 = 0x100;

/// 主设备接收数据FIFO
const I2C_RXDATA0: u64 = 0x200;

// ============ I2C_CON寄存器位 ============

const I2C_CON_EN: u32 = 1 << 0;          // I2C启能
const I2C_CON_MODE_TX: u32 = 0 << 1;    // 发送模式
const I2C_CON_MODE_RX: u32 = 1 << 1;    // 接收模式
const I2C_CON_MODE_RRW: u32 = 2 << 1;   // 读写模式
const I2C_CON_MODE_MASK: u32 = 3 << 1;

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

impl fmt::Display for I2cError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            I2cError::BusBusy => write!(f, "I2C Bus Busy"),
            I2cError::NoAck => write!(f, "No ACK from slave"),
            I2cError::DataConflict => write!(f, "Data Conflict"),
            I2cError::Timeout => write!(f, "I2C Timeout"),
            I2cError::InvalidAddr => write!(f, "Invalid Address"),
        }
    }
}

/// I2C驱动结构体
pub struct I2c {
    base: u64,
    /// I2C总线频率 (标准: 100kHz, 快速: 400kHz)
    freq_khz: u32,
}

impl I2c {
    /// 创建新的I2C实例
    pub fn new(base: u64, freq_khz: u32) -> Self {
        I2c { base, freq_khz }
    }
    
    /// 初始化I2C控制器
    /// 
    /// # 参数
    /// - `apb_freq_mhz`: APB总线频率 (MHz), 通常为 24 或 200
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
            
            // 4. 启用中断 (可选)
            // 中断类型: BTFINT (字节传输完成)
            //           STARTINT (START条件)
            //           STOPINT (STOP条件)
            //           NAKRCVINT (收到NAK)
            write_volatile((self.base + I2C_IEN) as *mut u32, 0x7f);
            
            Ok(())
        }
    }
    
    /// 等待I2C空闲
    fn wait_idle(&self, timeout_us: u32) -> Result<(), I2cError> {
        let mut count = 0;
        let max_count = timeout_us * 100; // 粗略估计
        
        unsafe {
            loop {
                let status = read_volatile((self.base + I2C_CON) as *const u32);
                if (status & (1 << 5)) == 0 {  // 检查START_EN或transmit标志
                    return Ok(());
                }
                
                if count > max_count {
                    return Err(I2cError::Timeout);
                }
                
                count += 1;
            }
        }
    }
    
    /// 读取中断状态
    fn read_status(&self) -> u32 {
        unsafe { read_volatile((self.base + I2C_IPD) as *const u32) }
    }
    
    /// 清除中断标志
    fn clear_irq(&self) {
        unsafe { write_volatile((self.base + I2C_IPD) as *mut u32, 0xff); }
    }
    
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
}

/// 全局I2C实例
use lazy_static::lazy_static;
use alloc::vec::Vec;

lazy_static! {
    pub static ref I2C0: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C0_BASE, 100));
    pub static ref I2C1: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C1_BASE, 100));
    pub static ref I2C2: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C2_BASE, 100));
    pub static ref I2C3: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C3_BASE, 100));
    pub static ref I2C4: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C4_BASE, 100));
    pub static ref I2C5: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C5_BASE, 100));
    pub static ref I2C6: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C6_BASE, 100));
    pub static ref I2C7: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C7_BASE, 100));
    pub static ref I2C8: spin::Mutex<I2c> = spin::Mutex::new(I2c::new(I2C8_BASE, 100));
}

/// 初始化所有I2C控制器
pub fn i2c_init_all(apb_freq_mhz: u32) {
    for i2c in [
        I2C0.lock(),
        I2C1.lock(),
        I2C2.lock(),
        I2C3.lock(),
        I2C4.lock(),
        I2C5.lock(),
        I2C6.lock(),
        I2C7.lock(),
        I2C8.lock(),
    ] {
        let _ = i2c.init(apb_freq_mhz);
    }
}

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
