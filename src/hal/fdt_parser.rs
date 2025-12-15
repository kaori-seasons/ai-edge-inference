//! FDT (Flattened Device Tree) 解析器
//!
//! 用于动态解析bootloader传入的设备树,
//! 提取外设基地址和中断配置信息

use fdt_parser::Fdt;
use core::fmt;

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
    pub fn parse(dtb_ptr: u64) -> Result<Self, &'static str> {
        unsafe {
            let fdt_data = core::slice::from_raw_parts(
                dtb_ptr as *const u8,
                4096,  // 足够大的初始缓冲区
            );
            
            let fdt = Fdt::from_slice(fdt_data)
                .map_err(|_| "Invalid FDT format")?;
            
            let mut devices = Vec::new();
            
            // 遍历设备树所有节点
            for node in fdt.all_nodes() {
                let name = node.name.to_string();
                
                // 跳过特殊节点
                if name.starts_with("/") || name.is_empty() {
                    continue;
                }
                
                let device_type = node
                    .properties()
                    .find(|p| p.name == "device_type")
                    .and_then(|p| core::str::from_utf8(p.value).ok())
                    .unwrap_or("")
                    .to_string();
                
                // 解析寄存器地址
                let (reg_addr, reg_size) = node
                    .properties()
                    .find(|p| p.name == "reg")
                    .and_then(|p| {
                        if p.value.len() >= 16 {
                            let addr = u64::from_be_bytes(
                                p.value[0..8].try_into().unwrap_or_default()
                            );
                            let size = u64::from_be_bytes(
                                p.value[8..16].try_into().unwrap_or_default()
                            );
                            Some((addr, size))
                        } else {
                            None
                        }
                    })
                    .unwrap_or((0, 0));
                
                // 解析中断号
                let interrupts = node
                    .properties()
                    .find(|p| p.name == "interrupts")
                    .map(|p| {
                        p.value
                            .chunks_exact(4)
                            .filter_map(|chunk| {
                                let bytes: [u8; 4] = chunk.try_into().ok()?;
                                Some(u32::from_be_bytes(bytes))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                
                let status = node
                    .properties()
                    .find(|p| p.name == "status")
                    .and_then(|p| core::str::from_utf8(p.value).ok())
                    .unwrap_or("okay")
                    .to_string();
                
                if status != "disabled" {
                    devices.push(DeviceInfo {
                        name,
                        device_type,
                        reg_addr,
                        reg_size,
                        interrupts,
                        status,
                    });
                }
            }
            
            Ok(FdtParser { devices })
        }
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
    
    /// 打印所有设备信息
    pub fn print_devices(&self) {
        println!("[FDT Parser] Found {} devices:", self.devices.len());
        for device in &self.devices {
            println!(
                "  - {} (type: {}) @ 0x{:x} (size: 0x{:x})",
                device.name, device.device_type, device.reg_addr, device.reg_size
            );
            if !device.interrupts.is_empty() {
                print!("    Interrupts: ");
                for (i, irq) in device.interrupts.iter().enumerate() {
                    if i > 0 {
                        print!(", ");
                    }
                    print!("{}", irq);
                }
                println!();
            }
        }
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
    
    // 打印调试信息
    if parser.device_count() > 0 {
        parser.print_devices();
    }
    
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
