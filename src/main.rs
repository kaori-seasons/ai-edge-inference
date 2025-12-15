#![no_std]
#![no_main]

extern crate alloc;

use core::arch::asm;
use starryos_rk3588::drivers::uart::uart_init;
use starryos_rk3588::mm::paging::paging_init;
use starryos_rk3588::hal::gic500::gic_init;
use starryos_rk3588::hal::fdt_parser::fdt_init;
use starryos_rk3588::kernel::multicore::*;
use starryos_rk3588::kernel::sched::hmp_scheduler::*;
use starryos_rk3588::kernel::sched::npu_support::*;
use starryos_rk3588::drivers::mipi_csi_driver::*;
use starryos_rk3588::npu::*;

/// 内核主程序入口 (由boot.s调用)
/// 
/// 参数:
/// x0: DTB物理地址 (由bootloader传入)
#[no_mangle]
pub extern "C" fn main(dtb_ptr: u64) -> ! {
    
    // 1. 初始化UART (第一步, 用于调试输出)
    uart_init();
    println!("[StarryOS] Booting...");
    
    // 2. 初始化内存管理 (MMU页表)
    println!("[StarryOS] Initializing memory management...");
    paging_init();
    println!("[StarryOS] MMU enabled");
    
    // 3. 初始化GIC-500中断控制器 (CPU 0)
    println!("[StarryOS] Initializing GIC-500...");
    gic_init(0);
    println!("[StarryOS] GIC initialized for CPU 0");
    
    // 4. 解析设备树 (获取外设基地址和中断配置)
    println!("[StarryOS] Parsing device tree...");
    match fdt_init(dtb_ptr) {
        Ok(_) => println!("[StarryOS] Device tree parsed successfully"),
        Err(e) => {
            println!("[StarryOS] FDT parse error: {}", e);
            panic!("Failed to parse device tree");
        }
    }
        
    // 5. 初始化驱动程序
    println!("[StarryOS] Initializing drivers...");
    
    // 初始化所有I2C控制器 (APB频率: 24MHz)
    starryos_rk3588::drivers::i2c_embedded_hal::i2c_init_all(24);
    println!("[StarryOS] I2C controllers initialized");
    
    // 初始化所有CAN控制器 (时钟频率: 120MHz)
    starryos_rk3588::drivers::can_driver_rk::can_init_all(120);
    println!("[StarryOS] CAN controllers initialized");
    
    // 6. 初始化异构调度器
    println!("[StarryOS] Initializing HMP scheduler...");
    hmp_init();
    
    // 7. 启动多核系统
    println!("[StarryOS] Starting multi-core system...");
    multicore_init();
        
    // 8. 初始化MIPI-CSI摄像头管道
    println!("[StarryOS] Initializing MIPI-CSI pipeline...");
    mipi_csi_init_all();
    println!("[StarryOS] MIPI-CSI initialized");
    
    // 9. 初始化RKNN NPU系统
    println!("[StarryOS] Initializing RKNN NPU system...");
    match rknn_init() {
        Ok(_) => println!("[StarryOS] RKNN system initialized"),
        Err(e) => println!("[StarryOS] RKNN init error: {}", e),
    }
    
    // ============ 系统就绪 ============
    
    println!("[StarryOS] ================================");
    println!("[StarryOS] Hello, StarryOS on RK3588!");
    println!("[StarryOS] ================================");
    
    // 获取系统信息
    let cpu_count = get_online_cpu_count();
    let a76_count = get_a76_online_count();
    let a55_count = get_a55_online_count();
    
    println!("[StarryOS] System Summary:");
    println!("[StarryOS]   Total CPUs: {}", cpu_count);
    println!("[StarryOS]   A76 cores: {}", a76_count);
    println!("[StarryOS]   A55 cores: {}", a55_count);
    
    // 打印HMP调度器状态
    let mut scheduler = HMP_SCHEDULER.lock();
    scheduler.print_status();
    drop(scheduler);
    
    println!("[StarryOS] System startup completed");
    
    // ============ 测试I2C驱动 ============
    
    println!("[StarryOS] Testing I2C driver...");
    {
        let mut i2c = starryos_rk3588::drivers::i2c_embedded_hal::I2C0.lock();
        match i2c.write(0x68, &[0x0D, 0xA8]) {
            Ok(_) => println!("[StarryOS] I2C write test PASSED"),
            Err(e) => println!("[StarryOS] I2C write test FAILED: {}", e),
        }
    }
    
    // ============ 测试CAN驱动 ============
    
    println!("[StarryOS] Testing CAN driver...");
    {
        let can = starryos_rk3588::drivers::can_driver_rk::CAN0.lock();
        let mut frame = starryos_rk3588::drivers::can_driver_rk::CanFrame::new(0x123, 8);
        frame.set_data(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        
        match can.send(&frame) {
            Ok(_) => println!("[StarryOS] CAN send test PASSED"),
            Err(e) => println!("[StarryOS] CAN send test FAILED: {}", e),
        }
    }
    
    // ============ 测试异构调度 ============
    
    println!("[StarryOS] Testing HMP scheduler...");
    {
        let mut scheduler = HMP_SCHEDULER.lock();
        
        // 提交高性能任务
        let mut high_perf_task = Task::new(1, 50, TaskHint::HighPerf);
        match scheduler.submit_task(high_perf_task) {
            Ok(cpu) => println!("[StarryOS] HighPerf task assigned to CPU {}", cpu),
            Err(e) => println!("[StarryOS] Failed to schedule task: {}", e),
        }
        
        // 提交低功耗任务
        let mut low_power_task = Task::new(2, 50, TaskHint::LowPower);
        match scheduler.submit_task(low_power_task) {
            Ok(cpu) => println!("[StarryOS] LowPower task assigned to CPU {}", cpu),
            Err(e) => println!("[StarryOS] Failed to schedule task: {}", e),
        }
        
        scheduler.print_status();
    }
    
    println!("[StarryOS] All tests completed!");
    
    // ============ 测试MIPI-CSI上NPU ============
    
    println!("[StarryOS] Testing MIPI-CSI capture...");
    {
        let mut csi = MIPI_CSI0.lock();
        csi.init_dphy();
        csi.init_csi2();
        println!("[StarryOS] MIPI-CSI0 initialized");
        
        // 模拟帧缓冲區添加和排队
        let frame = FrameBuffer::new(0x80000000, 0x10000000, 2097152);
        let idx = 0;
        println!("[StarryOS] Frame buffer created, phys_addr: 0x{:x}", frame.phys_addr);
    }
    
    println!("[StarryOS] Testing RKNN model loading...");
    {
        let mut ctx_lock = RKNN_CTX.lock();
        if let Some(ref mut ctx) = *ctx_lock {
            // 模拟上MODEL_DATA是一个有效的RKNN模型
            let dummy_model = b"RKNN\x00\x00\x00\x00";
            match ctx.load_model(dummy_model) {
                Ok(_) => println!("[StarryOS] Model loading PASSED"),
                Err(e) => println!("[StarryOS] Model loading FAILED: {}", e),
            }
            
            // 初始化输入和输出张量
            let input_shapes = [(1, 3, 640, 640)];
            let output_sizes = [1280, 1280, 1280];  // 三个梨算头输出
            
            match ctx.init_inputs(&input_shapes) {
                Ok(_) => println!("[StarryOS] Input tensors initialized"),
                Err(e) => println!("[StarryOS] Input init FAILED: {}", e),
            }
            
            match ctx.init_outputs(&output_sizes) {
                Ok(_) => println!("[StarryOS] Output tensors initialized"),
                Err(e) => println!("[StarryOS] Output init FAILED: {}", e),
            }
        }
    }
    
    println!("[StarryOS] Testing YOLOv8 inference...");
    {
        let app = Yolov8App::new();
        
        // 模拟一个小的模攵数据
        let input_data = alloc::vec![128u8; 640 * 640 * 3];
        let output_data = alloc::vec![0.5f32; 1280];
        
        match app.infer(&input_data, 640, 640, &output_data) {
            Ok(result) => {
                println!("[StarryOS] Inference PASSED");
                println!("[StarryOS]   Inference time: {}ms", result.inference_time_ms);
                println!("[StarryOS]   Process time: {}ms", result.process_time_ms);
                println!("[StarryOS]   Detections: {}", result.detections.len());
            }
            Err(e) => println!("[StarryOS] Inference FAILED: {}", e),
        }
    }
    
    println!("[StarryOS] All Week3 tests completed!");
    
    
    println!("[StarryOS] Testing YOLOv8 INT8 quantized model...");
    {
        let model = YoloV8Quantized::new("nano", 1.5);
        println!("[StarryOS] Model: {}", model.get_stats());
        
        // 验证量化精度
        match model.is_acceptable_precision() {
            Ok(_) => println!("[StarryOS] Quantization precision: ACCEPTABLE"),
            Err(e) => println!("[StarryOS] Quantization precision: {}", e),
        }
        
        // 预氎 FPS
        let fps = model.estimate_fps(7.5);
        println!("[StarryOS] Estimated FPS with INT8: {:.1}", fps);
    }
    
    println!("[StarryOS] Testing image preprocessing with NEON...");
    {
        let mut preprocessor = ImagePreprocessor::new(1920, 1080, 640, 640, ImageFormat::BGR24);
        
        // 模拟漄洋图像
        let fake_image = alloc::vec![128u8; 1920 * 1080 * 3];
        
        match preprocessor.preprocess(&fake_image) {
            Ok(output) => {
                println!("[StarryOS] Preprocessing PASSED");
                println!("[StarryOS]   Output size: {} floats", output.len());
            }
            Err(e) => println!("[StarryOS] Preprocessing FAILED: {}", e),
        }
    }
    
    println!("[StarryOS] Testing NMS post-processing...");
    {
        let mut postproc = PostprocessPipeline::new(0.5, 0.45, 300);
        
        // 模拟原始输出
        let raw_output = alloc::vec![0.5f32; 85 * 25200];  // YOLOv8 输出格式
        
        match postproc.postprocess(&raw_output, 25200, 80) {
            Ok(boxes) => {
                let stats = postproc.get_stats();
                println!("[StarryOS] Post-processing PASSED");
                println!("[StarryOS] {}", stats);
            }
            Err(e) => println!("[StarryOS] Post-processing FAILED: {}", e),
        }
    }
    
    println!("[StarryOS] Testing NPU task scheduling...");
    {
        let mut scheduler = NpuScheduler::new(NpuSchedulePolicy::Balanced);
        
        // 注册 NPU 上下文
        let ctx = NpuContext::new(0, "yolov8-int8");
        match scheduler.register_context(ctx) {
            Ok(_) => println!("[StarryOS] NPU context registered"),
            Err(e) => println!("[StarryOS] Failed to register context: {}", e),
        }
        
        // 不同任务类的调度决策
        let preproc_decision = scheduler.get_schedule_decision(NpuTaskType::Preprocess);
        println!("[StarryOS] Preprocess decision: {}", preproc_decision);
        
        let infer_decision = scheduler.get_schedule_decision(NpuTaskType::Inference);
        println!("[StarryOS] Inference decision: {}", infer_decision);
        
        let postproc_decision = scheduler.get_schedule_decision(NpuTaskType::Postprocess);
        println!("[StarryOS] Postprocess decision: {}", postproc_decision);
    }
    
    println!("[StarryOS] Testing end-to-end INT8 inference pipeline...");
    {
        // 1. 准备模型
        let quantized_model = YOLOV8_INT8_NANO.clone();
        println!("[StarryOS] Loaded INT8 model: {}", quantized_model.model_variant);
        
        // 2. 预处理图像
        let mut preprocessor = ImagePreprocessor::new(1920, 1080, 640, 640, ImageFormat::BGR24);
        let fake_image = alloc::vec![128u8; 1920 * 1080 * 3];
        let preprocessed = preprocessor.preprocess(&fake_image);
        
        if let Ok(data) = preprocessed {
            println!("[StarryOS] Preprocessed image: {} floats", data.len());
            
            // 3. NPU 推理 (模拟)
            let fake_output = alloc::vec![0.5f32; 85 * 25200];
            
            // 4. 后处理
            let mut postproc = PostprocessPipeline::new(0.5, 0.45, 100);
            match postproc.postprocess(&fake_output, 25200, 80) {
                Ok(detections) => {
                    let stats = postproc.get_stats();
                    println!("[StarryOS] End-to-end pipeline completed");
                    println!("[StarryOS] {}  ", stats);
                    println!("[StarryOS] Final detections: {}", detections.len());
                }
                Err(e) => println!("[StarryOS] Pipeline failed: {}", e),
            }
        }
    }
    
    println!("[StarryOS] All Week4 tests completed!");
    
    // 进入空闲循环
    loop {
        unsafe {
            asm!("wfi");  // 等待中断
        }
    }
}

/// 缺少alloc全局分配器的处理
mod kernel {
    use alloc::alloc::GlobalAlloc;
    use core::alloc::Layout;
    
    /// 简单的全局分配器 (基于固定的内存池)
    pub struct SimpleAllocator;
    
    unsafe impl GlobalAlloc for SimpleAllocator {
        unsafe fn alloc(&self, _layout: Layout) -> *mut u8 {
            // 暂时返回错误 (实际应该使用内存池)
            core::ptr::null_mut()
        }
        
        unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
            // 无操作
        }
    }
    
    #[global_allocator]
    static GLOBAL: SimpleAllocator = SimpleAllocator;
}

#[alloc_error_handler]
fn handle_alloc_error(_layout: core::alloc::Layout) -> ! {
    println!("[Error] Memory allocation failed");
    panic!("Allocation error");
}
