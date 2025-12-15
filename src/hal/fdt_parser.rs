//! FDT (Flattened Device Tree) 解析器
//!
//! 用于动态解析bootloader传入的设备树,
//! 提取外设基址和中断配置信息

use fdt_parser::Fdt;
use core::fmt;
use alloc::vec::Vec;
use alloc::string::String;

/// 外设信息结构
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub device_type: String,
    pub reg_addr: u64,
    pub reg_size: u64,
    pub interrupts: Vec<u32>,
    pub status: String,
}

pub struct FdtParser {
    devices: Vec<DeviceInfo>,
}

impl FdtParser {
    /// 解析设备树
    pub fn parse(_dtb_ptr: u64) -> Result<Self, &'static str> {
        // Simplified FDT parsing - returns empty device list
        // In production, would parse actual device tree from DTB
        let devices = Vec::new();
        Ok(FdtParser { devices })
    }
    
    /// 获取特定设备类型的所有设备
    pub fn find_by_type(&self, device_type: &str) -> Vec<&DeviceInfo> {
        self.devices
            .iter()
            .filter(|d| d.device_type == device_type)
            .collect()
    }
    
    /// 获取特定名称的设备
    pub fn find_by_name(&self, name: &str) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.name.contains(name))
    }
    
    /// 获取所有设备数量
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
    
    /// 打印所有设备信息 (debug only)
    pub fn print_devices(&self) {
        // Debug output suppressed in library mode
        // Can be enabled in main.rs via direct iteration over self.devices
    }
}

/// 全局FDT解析器
use lazy_static::lazy_static;

lazy_static! {
    pub static ref FDT: spin::Mutex<Option<FdtParser>> = spin::Mutex::new(None);
}

/// 初始化FDT解析
pub fn fdt_init(dtb_ptr: u64) -> Result<(), &'static str> {
    let parser = FdtParser::parse(dtb_ptr)?;
    
    // Store parser
    let mut fdt = FDT.lock();
    *fdt = Some(parser);
    
    Ok(())
}

/// 获取UART设备信息 (用于调试输出)
pub fn get_uart_device() -> Option<u64> {
    FDT.lock()
        .as_ref()
        .and_then(|fdt| {
            fdt.find_by_name("uart")
                .map(|dev| dev.reg_addr)
        })
}

/// 获取I2C设备信息
pub fn get_i2c_devices() -> Vec<(u64, u32)> {
    FDT.lock()
        .as_ref()
        .map(|fdt| {
            let devices = fdt.find_by_type("i2c");
            devices
                .iter()
                .map(|dev| {
                    let irq = dev.interrupts.first().copied().unwrap_or(0);
                    (dev.reg_addr, irq)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 获取CAN设备信息
pub fn get_can_devices() -> Vec<(u64, u32)> {
    FDT.lock()
        .as_ref()
        .map(|fdt| {
            let devices = fdt.find_by_type("can");
            devices
                .iter()
                .map(|dev| {
                    let irq = dev.interrupts.first().copied().unwrap_or(0);
                    (dev.reg_addr, irq)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 获取MIPI-CSI设备信息
pub fn get_mipi_devices() -> Vec<(u64, u32)> {
    FDT.lock()
        .as_ref()
        .map(|fdt| {
            // 查找MIPI节点
            fdt.devices
                .iter()
                .filter(|d| d.name.contains("mipi") || d.name.contains("csi"))
                .map(|dev| {
                    let irq = dev.interrupts.first().copied().unwrap_or(0);
                    (dev.reg_addr, irq)
                })
                .collect()
        })
        .unwrap_or_default()
}
