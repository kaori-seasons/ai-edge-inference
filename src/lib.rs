#![no_std]

extern crate alloc;

pub mod drivers;
pub mod mm;
pub mod hal;
pub mod kernel;
pub mod npu;
pub mod system;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    loop {}
}
