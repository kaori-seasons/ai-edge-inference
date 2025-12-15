//! AArch64 页表管理
//!
//! 实现4级页表结构:
//! - L0: PGD (Page Global Directory)
//! - L1: PUD (Page Upper Directory)
//! - L2: PMD (Page Middle Directory)
//! - L3: PTE (Page Table Entry)

use core::arch::asm;
use volatile::Volatile;

/// 页表项 (Page Table Entry)
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct Pte(u64);

impl Pte {
    /// 表项有效标志
    const VALID: u64 = 1 << 0;
    
    /// 表项类型: 0=表, 1=块/页
    const TYPE: u64 = 1 << 1;
    
    /// 访问权限 (AP)
    const AP_MASK: u64 = 0x3 << 6;
    const AP_EL0_NONE: u64 = 0 << 6;      // EL0无访问权限
    const AP_EL0_RW: u64 = 1 << 6;         // EL0可读写
    const AP_KERN_RO: u64 = 2 << 6;        // 内核只读
    const AP_KERN_RW: u64 = 3 << 6;        // 内核可读写
    
    /// 缓存策略 (AttrIdx)
    const ATTR_IDX_MASK: u64 = 0x7 << 2;
    const ATTR_DEVICE: u64 = 0 << 2;       // 设备内存 (MMIO)
    const ATTR_NORMAL: u64 = 1 << 2;       // 普通内存 (缓存)
    const ATTR_NORMAL_NC: u64 = 2 << 2;    // 普通内存 (非缓存)
    
    /// 访问标志 (Access Flag)
    const AF: u64 = 1 << 10;
    
    /// 脏标志 (Dirty bit)
    const DBM: u64 = 1 << 51;
    
    /// 执行权限禁用标志
    const UXN: u64 = 1 << 54;              // User eXecute Never
    const PXN: u64 = 1 << 53;              // Privileged eXecute Never
    
    /// 创建有效的块表项
    /// 
    /// # 参数
    /// - `phys_addr`: 物理地址
    /// - `attr`: 内存属性 (Device/Normal/Normal_NC)
    /// - `exec`: 是否允许执行
    fn block(phys_addr: u64, attr: u64, exec: bool) -> Self {
        let mut entry = phys_addr
            | Self::VALID
            | Self::TYPE
            | Self::AP_KERN_RW
            | Self::AF
            | attr;
        
        if !exec {
            entry |= Self::PXN | Self::UXN;
        }
        
        Pte(entry)
    }
    
    /// 创建指向下一级表的表项
    fn table(table_addr: u64) -> Self {
        Pte(table_addr | Self::VALID)
    }
    
    /// 获取物理地址
    pub fn address(&self) -> u64 {
        self.0 & 0xFFFF_FFFF_F000
    }
}

/// 页表 (页对齐, 512个表项)
#[repr(align(4096))]
pub struct PageTable {
    entries: [Volatile<Pte>; 512],
}

impl PageTable {
    /// 创建新的页表
    pub const fn new() -> Self {
        PageTable {
            entries: [Volatile::new(Pte(0)); 512],
        }
    }
    
    /// 设置表项
    pub fn set(&mut self, index: usize, entry: Pte) {
        if index < 512 {
            self.entries[index].write(entry);
        }
    }
    
    /// 获取表项
    pub fn get(&self, index: usize) -> Pte {
        if index < 512 {
            self.entries[index].read()
        } else {
            Pte(0)
        }
    }
}

/// 4级页表结构
pub struct PageTableManager {
    l0: &'static mut PageTable,  // PGD
}

impl PageTableManager {
    /// 初始化页表管理器
    /// 
    /// 设置:
    /// - DDR内存: 1GB @ 0x0, 缓存可用 (NORMAL)
    /// - MMIO区域: 256MB @ 0xfe000000, 非缓存 (DEVICE)
    pub fn init() -> Self {
        unsafe {
            // L0表 (PGD)
            let l0_addr = 0x1000 as *mut PageTable;
            let l0 = &mut *l0_addr;
            *l0 = PageTable::new();
            
            // L1表
            let l1_addr = 0x2000 as *mut PageTable;
            let l1 = &mut *l1_addr;
            *l1 = PageTable::new();
            
            // L2表 (DDR: 1GB)
            let l2_addr = 0x3000 as *mut PageTable;
            let l2 = &mut *l2_addr;
            *l2 = PageTable::new();
            
            // L3表 (MMIO: 256MB)
            let l3_addr = 0x4000 as *mut PageTable;
            let l3 = &mut *l3_addr;
            *l3 = PageTable::new();
            
            // 配置L0 -> L1
            l0.set(0, Pte::table(l1_addr as u64));
            
            // 配置L1 -> L2 (DDR)
            l1.set(0, Pte::table(l2_addr as u64));
            
            // 配置L2 -> 1GB块 @ 0x0 (DDR内存, 缓存)
            for i in 0..512 {
                let phys_addr = (i as u64) * 0x200000; // 2MB块
                l2.set(i, Pte::block(phys_addr, Pte::ATTR_NORMAL, true));
            }
            
            // 配置L1 -> L3 (MMIO)
            l1.set(511, Pte::table(l3_addr as u64)); // 顶部地址空间
            
            // 配置L3 -> 256MB块 @ 0xfe000000 (设备寄存器, 非缓存)
            for i in 0..512 {
                let phys_addr = 0xfe000000 + ((i as u64) * 0x200000);
                l3.set(i, Pte::block(phys_addr, Pte::ATTR_DEVICE, false));
            }
            
            PageTableManager {
                l0,
            }
        }
    }
    
    /// 启用MMU
    /// 
    /// 配置:
    /// - TTBR0_EL1: 页表基址
    /// - TCR_EL1: 转换控制
    /// - SCTLR_EL1: 启能MMU
    pub fn enable(&self) {
        unsafe {
            // 1. 设置TTBR0_EL1 (转换表基地址寄存器)
            let l0_addr = self.l0 as *const _ as u64;
            asm!("msr ttbr0_el1, {}", in(reg) l0_addr);
            
            // 2. 配置TCR_EL1 (转换控制寄存器)
            // IPS: 40-bit物理地址空间 (1TB)
            // TG0: 4KB粒度
            // SH0: Inner shareable (L1 cache)
            // ORGN0: Write-Back Read-Allocate Write-Allocate (WBWA)
            // IRGN0: WBWA
            // T0SZ: 32位虚拟地址空间 (4GB)
            let tcr: u64 = (1 << 32)     // IPS=01 (40-bit)
                | (0 << 14)               // TG0=00 (4KB)
                | (3 << 12)               // SH0=11 (Inner shareable)
                | (1 << 10)               // ORGN0=01 (Write-Back)
                | (1 << 8)                // IRGN0=01 (Write-Back)
                | (32);                   // T0SZ=32
            
            asm!("msr tcr_el1, {}", in(reg) tcr);
            
            // 3. 设置MAIR_EL1 (Memory Attribute Indirection Register)
            // Attr0: Device (0x00)
            // Attr1: Normal (0xFF)
            // Attr2: Normal non-cacheable (0x44)
            let mair: u64 = 0x00FF4400u64;
            asm!("msr mair_el1, {}", in(reg) mair);
            
            // 4. 启能MMU (SCTLR_EL1.M=1)
            let mut sctlr: u64;
            asm!("mrs {}, sctlr_el1", out(reg) sctlr);
            sctlr |= 1;  // M位
            sctlr |= 1 << 2;  // C位 (数据缓存)
            sctlr |= 1 << 12; // I位 (指令缓存)
            asm!("msr sctlr_el1, {}", in(reg) sctlr);
            
            // 指令同步屏障
            asm!("isb");
        }
    }
}

/// 全局页表管理器
pub fn paging_init() {
    let mgr = PageTableManager::init();
    mgr.enable();
}
