//! GIC-500 (ARM Generic Interrupt Controller v3) 驱动
//!
//! 支持:
//! - GICD (Distributor): 中断管理和路由
//! - GICR (Redistributer): 各核心PPI/SGI处理
//! - SPI (Shared Peripheral Interrupt): 外设中断
//! - PPI (Private Peripheral Interrupt): 核心私有中断
//! - SGI (Software Generated Interrupt): 核心间通信

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use volatile::Volatile;

/// GIC-500基地址 (RK3588特定)
pub const GIC_BASE: u64 = 0xfe600000;

/// GICD (Distributor)基地址
pub const GICD_BASE: u64 = GIC_BASE + 0x00000;

/// GICR (Redistributer)基地址
pub const GICR_BASE: u64 = GIC_BASE + 0x100000;

// ============ GICD寄存器偏移 ============

/// GICD_CTRL: 分发器控制
const GICD_CTLR: u64 = 0x0;

/// GICD_TYPER: 分发器类型
const GICD_TYPER: u64 = 0x4;

/// GICD_IGROUPRn: 中断组 (0=组0, 1=组1)
const GICD_IGROUPR: u64 = 0x80;

/// GICD_ISENABLERn: 中断使能集
const GICD_ISENABLER: u64 = 0x100;

/// GICD_ICENABLERn: 中断使能清
const GICD_ICENABLER: u64 = 0x180;

/// GICD_ISPENDRn: 中断待处理集
const GICD_ISPENDR: u64 = 0x200;

/// GICD_ICPENDRn: 中断待处理清
const GICD_ICPENDR: u64 = 0x280;

/// GICD_ISACTIVERn: 中断激活集
const GICD_ISACTIVER: u64 = 0x300;

/// GICD_ICACTIVERn: 中断激活清
const GICD_ICACTIVER: u64 = 0x380;

/// GICD_IPRIORITYRn: 中断优先级
const GICD_IPRIORITYR: u64 = 0x400;

/// GICD_ICFGRn: 中断配置 (边缘/电平触发)
const GICD_ICFGR: u64 = 0xC00;

/// GICD_IROUTERn: 中断路由 (SPI only)
const GICD_IROUTER: u64 = 0x6000;

// ============ GICR寄存器偏移 ============

/// GICR_CTRL: 重定向器控制
const GICR_CTLR: u64 = 0x0;

/// GICR_IIDR: 实现标识符
const GICR_IIDR: u64 = 0x4;

/// GICR_TYPER: 重定向器类型
const GICR_TYPER: u64 = 0x8;

/// GICR_SGI_ISENABLER0: SGI/PPI使能集
const GICR_SGI_ISENABLER0: u64 = 0x100080;

/// GICR_SGI_ICENABLER0: SGI/PPI使能清
const GICR_SGI_ICENABLER0: u64 = 0x100100;

/// GICR_SGI_IPRIORITYR0-7: SGI/PPI优先级
const GICR_SGI_IPRIORITYR0: u64 = 0x100400;

/// GICR_SGI_ICFGR0: SGI/PPI配置
const GICR_SGI_ICFGR0: u64 = 0x100C00;

/// GICR_SGI_ICFGR1: SGI/PPI配置
const GICR_SGI_ICFGR1: u64 = 0x100C04;

/// GICR_SGI_ISPENDR0: SGI/PPI待处理  
const GICR_SGI_ISPENDR0: u64 = 0x100200;

/// GICR_SGI_ICPENDR0: SGI/PPI待处理
const GICR_SGI_ICPENDR0: u64 = 0x100280;

/// GICR_SGI_ISACTIVER0: SGI/PPI激活
const GICR_SGI_ISACTIVER0: u64 = 0x100300;

/// GICR_SGI_ICACTIVER0: SGI/PPI激活
const GICR_SGI_ICACTIVER0: u64 = 0x100380;

// ============ 中断号定义 ============

/// SGI中断号范围: 0-15
pub const SGI_RANGE_START: u32 = 0;
pub const SGI_RANGE_END: u32 = 15;

/// PPI中断号范围: 16-31
pub const PPI_RANGE_START: u32 = 16;
pub const PPI_RANGE_END: u32 = 31;

/// SPI中断号范围: 32-1019
pub const SPI_RANGE_START: u32 = 32;
pub const SPI_RANGE_END: u32 = 1019;

pub struct Gic500 {
    gicd_base: u64,
    gicr_base: u64,
}

impl Gic500 {
    /// 创建GIC驱动实例
    pub fn new() -> Self {
        Gic500 {
            gicd_base: GICD_BASE,
            gicr_base: GICR_BASE,
        }
    }
    
    /// 初始化GICD (Distributor)
    pub fn init_gicd(&self) {
        unsafe {
            // 1. 禁用GICD
            write_volatile(
                (self.gicd_base + GICD_CTLR) as *mut u32,
                0,
            );
            
            // 2. 获取中断数量
            let typer = read_volatile(
                (self.gicd_base + GICD_TYPER) as *const u32
            );
            let num_interrupts = ((typer & 0x1F) + 1) * 32;
            
            // 3. 禁用所有SPI (32-1019)
            for i in (32..num_interrupts).step_by(32) {
                write_volatile(
                    (self.gicd_base + GICD_ICENABLER + (i / 32) as u64 * 4) as *mut u32,
                    0xFFFFFFFF,
                );
            }
            
            // 4. 清除所有SPI的待处理标志
            for i in (32..num_interrupts).step_by(32) {
                write_volatile(
                    (self.gicd_base + GICD_ICPENDR + (i / 32) as u64 * 4) as *mut u32,
                    0xFFFFFFFF,
                );
            }
            
            // 5. 设置所有SPI为组0 (安全分组)
            for i in (32..num_interrupts).step_by(32) {
                write_volatile(
                    (self.gicd_base + GICD_IGROUPR + (i / 32) as u64 * 4) as *mut u32,
                    0,
                );
            }
            
            // 6. 设置所有SPI的优先级为最低 (0xF0)
            for i in 32..num_interrupts {
                write_volatile(
                    (self.gicd_base + GICD_IPRIORITYR + i as u64) as *mut u8,
                    0xF0,
                );
            }
            
            // 7. 设置所有SPI为电平触发
            for i in (32..num_interrupts).step_by(16) {
                write_volatile(
                    (self.gicd_base + GICD_ICFGR + (i / 16) as u64 * 4) as *mut u32,
                    0,  // 0=电平触发, 1=边缘触发
                );
            }
            
            // 8. 启用GICD (Distributor Enable)
            write_volatile(
                (self.gicd_base + GICD_CTLR) as *mut u32,
                0x01,  // Group0使能, Group1禁用
            );
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
            
            // 2. 清除所有SGI/PPI的待处理标志
            write_volatile(
                (gicr_cpu_base + GICR_SGI_ICPENDR0) as *mut u32,
                0xFFFFFFFF,
            );
            
            // 3. 清除所有SGI/PPI的激活标志
            write_volatile(
                (gicr_cpu_base + GICR_SGI_ICACTIVER0) as *mut u32,
                0xFFFFFFFF,
            );
            
            // 4. 设置所有SGI/PPI的优先级为最低 (0xF0)
            for i in 0..8 {
                write_volatile(
                    (gicr_cpu_base + GICR_SGI_IPRIORITYR0 + i * 4) as *mut u32,
                    0xF0F0F0F0,
                );
            }
            
            // 5. 配置所有PPI为电平触发
            write_volatile(
                (gicr_cpu_base + GICR_SGI_ICFGR1) as *mut u32,
                0,  // PPI: 0=电平触发, 1=边缘触发
            );
        }
    }
    
    /// 使能中断
    pub fn enable_interrupt(&self, irq: u32) {
        unsafe {
            if irq >= SPI_RANGE_START && irq <= SPI_RANGE_END {
                // SPI中断
                let byte_offset = irq / 8;
                let bit_offset = irq % 8;
                let reg_offset = (irq - 32) / 32;
                
                write_volatile(
                    (self.gicd_base + GICD_ISENABLER + reg_offset as u64 * 4) as *mut u32,
                    1 << bit_offset,
                );
            }
        }
    }
    
    /// 禁用中断
    pub fn disable_interrupt(&self, irq: u32) {
        unsafe {
            if irq >= SPI_RANGE_START && irq <= SPI_RANGE_END {
                // SPI中断
                let reg_offset = (irq - 32) / 32;
                let bit_offset = (irq - 32) % 32;
                
                write_volatile(
                    (self.gicd_base + GICD_ICENABLER + reg_offset as u64 * 4) as *mut u32,
                    1 << bit_offset,
                );
            }
        }
    }
    
    /// 设置中断优先级
    pub fn set_priority(&self, irq: u32, priority: u8) {
        unsafe {
            if irq >= SPI_RANGE_START && irq <= SPI_RANGE_END {
                // SPI中断
                write_volatile(
                    (self.gicd_base + GICD_IPRIORITYR + irq as u64) as *mut u8,
                    priority,
                );
            }
        }
    }
    
    /// 发送核间中断 (SGI)
    pub fn send_sgi(&self, irq: u32, cpu_mask: u32) {
        unsafe {
            // SGI寄存器格式:
            // [63:40] - 保留
            // [39:16] - 目标CPU掩码
            // [15:4] - 保留
            // [3:0] - SGI号 (0-15)
            
            if irq >= SGI_RANGE_START && irq <= SGI_RANGE_END {
                let sgi_value = ((cpu_mask as u64) << 16) | (irq as u64);
                asm!("msr icc_sgi1r_el1, {}", in(reg) sgi_value);
            }
        }
    }
    
    /// 读取当前中断号
    pub fn read_iar() -> u32 {
        let iar: u32;
        unsafe {
            asm!("mrs {}, icc_iar1_el1", out(reg) iar);
        }
        iar & 0xFFFFFF  // 仅提取中断ID [23:0]
    }
    
    /// 写回中断确认
    pub fn write_eoir(iar: u32) {
        unsafe {
            asm!("msr icc_eoir1_el1, {}", in(reg) iar);
        }
    }
}

/// 全局GIC实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref GIC: spin::Mutex<Gic500> = spin::Mutex::new(Gic500::new());
}

/// 初始化GIC-500
pub fn gic_init(cpu_id: u32) {
    let gic = GIC.lock();
    
    if cpu_id == 0 {
        gic.init_gicd();
    }
    
    gic.init_gicr(cpu_id);
}
