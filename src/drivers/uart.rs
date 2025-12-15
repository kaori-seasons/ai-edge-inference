//! UART驱动 (调试输出)
//! 
//! RK3588 UART控制器驱动
//! 支持基本的串口通信

use core::fmt;
use core::ptr::{read_volatile, write_volatile};

/// UART基地址 (默认UART0在RK3588)
pub const UART0_BASE: u64 = 0xff1a0000;

/// UART寄存器偏移
const UART_RBR: u64 = 0x0;    // 接收缓冲寄存器
const UART_THR: u64 = 0x0;    // 发送保存寄存器
const UART_DLL: u64 = 0x0;    // 除数锁存器低字节
const UART_DLM: u64 = 0x4;    // 除数锁存器高字节
const UART_FCR: u64 = 0x8;    // FIFO控制寄存器
const UART_LCR: u64 = 0xc;    // 线控制寄存器
const UART_MCR: u64 = 0x10;   // 调制解调器控制寄存器
const UART_LSR: u64 = 0x14;   // 线状态寄存器
const UART_MSR: u64 = 0x18;   // 调制解调器状态寄存器
const UART_SCR: u64 = 0x1c;   // 暂存寄存器

pub struct Uart {
    base: u64,
}

impl Uart {
    /// 创建新的UART实例
    pub fn new(base: u64) -> Self {
        Uart { base }
    }
    
    /// 初始化UART
    /// 
    /// 配置:
    /// - 波特率: 115200
    /// - 数据位: 8
    /// - 停止位: 1
    /// - 奇偶校验: 无
    pub fn init(&self) {
        unsafe {
            // 1. 设置线控制寄存器 (LCR)
            //    Bit 7: DLAB (除数锁存器访问位)
            //    Bit 6: Break
            //    Bit 5-3: 奇偶校验
            //    Bit 2: 停止位
            //    Bit 1-0: 字长
            
            // 先设置DLAB为1以访问DLL/DLM
            write_volatile((self.base + UART_LCR) as *mut u8, 0x83);  // 8N1 + DLAB
            
            // 2. 设置波特率 (波特率 = UART_CLK / (16 * DLL))
            //    假设UART_CLK = 24MHz
            //    115200 = 24000000 / (16 * DLL)
            //    DLL = 24000000 / (16 * 115200) ≈ 13
            
            write_volatile((self.base + UART_DLL) as *mut u8, 13);    // 低字节
            write_volatile((self.base + UART_DLM) as *mut u8, 0);     // 高字节
            
            // 3. 清除DLAB (返回正常模式)
            write_volatile((self.base + UART_LCR) as *mut u8, 0x03);  // 8N1, DLAB=0
            
            // 4. 启用FIFO
            write_volatile((self.base + UART_FCR) as *mut u8, 0x07);  // 启能FIFO, 清空缓冲
            
            // 5. 设置调制解调器控制 (MCR)
            write_volatile((self.base + UART_MCR) as *mut u8, 0x01);  // DTR
        }
    }
    
    /// 写入一个字节
    pub fn put_char(&self, c: u8) {
        unsafe {
            // 等待发送FIFO非满
            loop {
                let lsr = read_volatile((self.base + UART_LSR) as *const u8);
                if (lsr & 0x20) != 0 {  // Bit 5: THR空
                    break;
                }
            }
            
            // 写入字节到THR
            write_volatile((self.base + UART_THR) as *mut u8, c);
        }
    }
    
    /// 写入一个字符串
    pub fn puts(&self, s: &str) {
        for c in s.chars() {
            if c == '\n' {
                self.put_char(b'\r');
            }
            self.put_char(c as u8);
        }
    }
    
    /// 读取一个字节 (阻塞)
    pub fn get_char(&self) -> u8 {
        unsafe {
            loop {
                let lsr = read_volatile((self.base + UART_LSR) as *const u8);
                if (lsr & 0x01) != 0 {  // Bit 0: 数据可用
                    break;
                }
            }
            read_volatile((self.base + UART_RBR) as *const u8)
        }
    }
}

impl fmt::Write for Uart {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.puts(s);
        Ok(())
    }
}

/// 全局UART实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref UART: spin::Mutex<Uart> = spin::Mutex::new(Uart::new(UART0_BASE));
}

/// 初始化UART
pub fn uart_init() {
    UART.lock().init();
}

/// 打印宏 (类似println!)
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {
        {
            use core::fmt::Write;
            let mut uart = $crate::drivers::uart::UART.lock();
            let _ = write!(&mut *uart, $($arg)*);
        }
    };
}

#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n"); };
    ($($arg:tt)*) => {
        {
            $crate::print!($($arg)*);
            $crate::print!("\n");
        }
    };
}
