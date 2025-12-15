//! 多核启动支持
//!
//! 实现A76和A55核心的并行启动和同步机制

use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};
use lazy_static::lazy_static;

/// CPU核心状态
#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum CpuState {
    /// 核心未启动
    Offline = 0,
    /// 核心启动中
    Starting = 1,
    /// 核心已启动
    Online = 2,
}

/// CPU核心信息
pub struct CpuInfo {
    /// CPU ID (0-7)
    pub id: u32,
    /// CPU类型 (0=A76, 1=A55)
    pub cpu_type: u32,
    /// 当前状态
    pub state: AtomicU32,
}

impl CpuInfo {
    /// 创建新的CPU信息
    pub const fn new(id: u32, cpu_type: u32) -> Self {
        CpuInfo {
            id,
            cpu_type,
            state: AtomicU32::new(CpuState::Offline as u32),
        }
    }
    
    /// 获取CPU状态
    pub fn get_state(&self) -> CpuState {
        match self.state.load(Ordering::SeqCst) {
            1 => CpuState::Starting,
            2 => CpuState::Online,
            _ => CpuState::Offline,
        }
    }
    
    /// 设置CPU状态
    pub fn set_state(&self, state: CpuState) {
        self.state.store(state as u32, Ordering::SeqCst);
    }
}

/// 全局CPU信息表
lazy_static! {
    pub static ref CPU_INFO: [CpuInfo; 8] = [
        // A76核心 (0-3)
        CpuInfo::new(0, 0),
        CpuInfo::new(1, 0),
        CpuInfo::new(2, 0),
        CpuInfo::new(3, 0),
        // A55核心 (4-7)
        CpuInfo::new(4, 1),
        CpuInfo::new(5, 1),
        CpuInfo::new(6, 1),
        CpuInfo::new(7, 1),
    ];
}

/// 启动CPU核心
/// 
/// RK3588的CPU启动通过SCP (System Control Processor) 固件完成
/// 我们通过发送SGI (Software Generated Interrupt)来唤醒其他核心
pub fn start_cpu(cpu_id: u32) {
    if cpu_id >= 8 {
        println!("[MultiCore] Invalid CPU ID: {}", cpu_id);
        return;
    }
    
    // 获取CPU信息
    let cpu_info = &CPU_INFO[cpu_id as usize];
    cpu_info.set_state(CpuState::Starting);
    
    // 通过发送SGI中断来唤醒目标CPU
    // SGI15 用作CPU启动信号
    unsafe {
        use crate::hal::gic500::GIC;
        let gic = GIC.lock();
        gic.send_sgi(15, 1 << cpu_id);  // 只给指定CPU发送
    }
    
    println!("[MultiCore] Starting CPU {}", cpu_id);
}

/// 启动所有A76核心 (保留ID 0作为主CPU)
pub fn start_all_a76_cores() {
    for id in 1..4 {
        start_cpu(id);
    }
}

/// 启动所有A55核心
pub fn start_all_a55_cores() {
    for id in 4..8 {
        start_cpu(id);
    }
}

/// 启动所有CPU核心
pub fn start_all_cpus() {
    start_all_a76_cores();
    start_all_a55_cores();
}

/// 等待CPU核心启动完成
/// 
/// # 参数
/// - `cpu_id`: CPU编号
/// - `timeout_ms`: 超时时间 (毫秒)
pub fn wait_cpu_online(cpu_id: u32, timeout_ms: u32) -> bool {
    if cpu_id >= 8 {
        return false;
    }
    
    let cpu_info = &CPU_INFO[cpu_id as usize];
    let mut count = timeout_ms * 100; // 粗略估计
    
    loop {
        match cpu_info.get_state() {
            CpuState::Online => return true,
            CpuState::Offline => return false,
            _ => {}
        }
        
        if count == 0 {
            println!("[MultiCore] CPU {} startup timeout", cpu_id);
            return false;
        }
        
        count -= 1;
        unsafe { asm!("nop"); }
    }
}

/// 等待所有CPU核心启动完成
pub fn wait_all_online(timeout_ms: u32) -> u32 {
    let mut online_count = 1; // CPU 0已在线
    
    for id in 1..8 {
        if wait_cpu_online(id, timeout_ms) {
            online_count += 1;
            println!("[MultiCore] CPU {} is now online", id);
        } else {
            println!("[MultiCore] CPU {} failed to start", id);
        }
    }
    
    online_count
}

/// 获取在线CPU数量
pub fn get_online_cpu_count() -> u32 {
    let mut count = 1; // CPU 0
    for id in 1..8 {
        if matches!(CPU_INFO[id as usize].get_state(), CpuState::Online) {
            count += 1;
        }
    }
    count
}

/// 获取A76核心数量
pub fn get_a76_online_count() -> u32 {
    (0..4)
        .filter(|&id| matches!(CPU_INFO[id].get_state(), CpuState::Online))
        .count() as u32
}

/// 获取A55核心数量
pub fn get_a55_online_count() -> u32 {
    (4..8)
        .filter(|&id| matches!(CPU_INFO[id].get_state(), CpuState::Online))
        .count() as u32
}

/// 获取当前CPU ID
#[inline]
pub fn current_cpu_id() -> u32 {
    let cpu_id: u64;
    unsafe {
        // MPIDR_EL1的AFF0字段包含CPU ID
        asm!("mrs {}, mpidr_el1", out(reg) cpu_id);
    }
    (cpu_id & 0xFF) as u32
}

/// 获取当前CPU类型 (0=A76, 1=A55)
pub fn current_cpu_type() -> u32 {
    let cpu_id = current_cpu_id();
    if cpu_id < 4 {
        0 // A76
    } else {
        1 // A55
    }
}

/// CPU间通信函数指针
type IpiHandler = fn(u32, u32);

/// 全局IPI处理函数表
lazy_static! {
    static ref IPI_HANDLERS: spin::Mutex<[Option<IpiHandler>; 16]> = 
        spin::Mutex::new([None; 16]);
}

/// 注册IPI处理函数
pub fn register_ipi_handler(vector: u32, handler: IpiHandler) {
    if vector < 16 {
        let mut handlers = IPI_HANDLERS.lock();
        handlers[vector as usize] = Some(handler);
    }
}

/// 发送核心间中断 (IPI)
pub fn send_ipi(vector: u32, cpu_mask: u32) {
    unsafe {
        use crate::hal::gic500::GIC;
        let gic = GIC.lock();
        if vector < 16 {
            gic.send_sgi(vector, cpu_mask);
        }
    }
}

/// 处理IPI中断
pub fn handle_ipi(vector: u32, cpu_id: u32) {
    let handlers = IPI_HANDLERS.lock();
    if let Some(handler) = handlers[vector as usize] {
        handler(vector, cpu_id);
    }
}

/// 多核初始化
pub fn multicore_init() {
    println!("[MultiCore] Initializing multi-core system...");
    println!("[MultiCore] CPU 0 (main) is online");
    
    // 标记CPU 0为在线
    CPU_INFO[0].set_state(CpuState::Online);
    
    // 启动其他CPU核心
    start_all_cpus();
    
    // 等待所有CPU启动
    let online_count = wait_all_online(1000);
    
    println!(
        "[MultiCore] System online: {} CPUs ({} A76, {} A55)",
        online_count,
        get_a76_online_count(),
        get_a55_online_count()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_cpu_info() {
        let cpu = CpuInfo::new(0, 0);
        assert_eq!(cpu.id, 0);
        assert_eq!(cpu.cpu_type, 0);
        assert!(matches!(cpu.get_state(), CpuState::Offline));
        
        cpu.set_state(CpuState::Online);
        assert!(matches!(cpu.get_state(), CpuState::Online));
    }
    
    #[test]
    fn test_cpu_type_detection() {
        // A76核心: ID 0-3, type 0
        for id in 0..4 {
            let cpu = &CPU_INFO[id];
            assert_eq!(cpu.cpu_type, 0);
        }
        
        // A55核心: ID 4-7, type 1
        for id in 4..8 {
            let cpu = &CPU_INFO[id];
            assert_eq!(cpu.cpu_type, 1);
        }
    }
}
