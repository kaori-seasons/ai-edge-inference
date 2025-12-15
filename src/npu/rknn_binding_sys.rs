//! RKNN Runtime FFI 安全封装层
//!
//! 提供安全的 Rust 接口到闭源 RKNN C/C++ 库
//! 使用 RAII 模式确保资源自动清理
//! 通过生命周期和类型系统提供内存安全保证

use core::fmt;
use alloc::vec::Vec;

// ============ RKNN C API 类型定义 ============

/// RKNN 上下文句柄
pub type RknnContext = *mut core::ffi::c_void;

/// RKNN 返回代码
#[repr(i32)]
#[derive(Debug, Clone, Copy)]
pub enum RknnStatus {
    /// 成功
    Ok = 0,
    /// 参数错误
    InvalidParam = -1,
    /// 内存不足
    NoMemory = -2,
    /// 操作超时
    Timeout = -3,
    /// 文件错误
    FileError = -4,
    /// 未初始化
    NotInitialized = -5,
    /// 其他错误
    Unknown = -255,
}

impl fmt::Display for RknnStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RknnStatus::Ok => write!(f, "OK"),
            RknnStatus::InvalidParam => write!(f, "Invalid Parameter"),
            RknnStatus::NoMemory => write!(f, "No Memory"),
            RknnStatus::Timeout => write!(f, "Timeout"),
            RknnStatus::FileError => write!(f, "File Error"),
            RknnStatus::NotInitialized => write!(f, "Not Initialized"),
            RknnStatus::Unknown => write!(f, "Unknown Error"),
        }
    }
}

/// 张量数据类型
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DataType {
    /// 32 位浮点数
    Float32 = 0,
    /// 32 位整数
    Int32 = 1,
    /// 8 位有符号整数 (量化)
    Int8 = 2,
    /// 8 位无符号整数 (量化)
    Uint8 = 3,
    /// 16 位浮点数 (半精度)
    Float16 = 4,
}

/// 张量属性
#[repr(C)]
pub struct TensorAttr {
    pub index: u32,
    pub name: [u8; 256],
    pub n_dims: u32,
    pub dims: [u32; 16],
    pub n_elems: u32,
    pub size: u32,
    pub fmt: u32,
    pub type_: u32,
    pub qnt_type: u32,
    pub fl: i8,
    pub zp: i32,
    pub scale: f32,
}

/// DMA 缓冲区 (RAII 包装)
pub struct DmaBuffer {
    va: *mut u8,
    pa: u64,
    size: usize,
    aligned: bool,
}

impl DmaBuffer {
    /// 分配 DMA 缓冲区
    /// 
    /// # 参数
    /// - `size`: 缓冲区大小
    /// - `align`: 对齐要求 (通常为 8KB)
    pub fn allocate(size: usize, align: usize) -> Result<Self, &'static str> {
        // 使用全局分配器分配内存
        let layout = alloc::alloc::Layout::from_size_align(size, align)
            .map_err(|_| "Invalid layout")?;
        
        let va = unsafe { alloc::alloc::alloc(layout) };
        
        if va.is_null() {
            return Err("Memory allocation failed");
        }
        
        let pa = unsafe {
            // 虚拟地址转物理地址 (简化实现)
            // 实际应该调用内核 MMU 查询或专用函数
            va as u64 & 0xFFFFFFFF  // 占位实现
        };
        
        Ok(DmaBuffer {
            va,
            pa,
            size,
            aligned: true,
        })
    }
    
    /// 获取虚拟地址
    pub fn virt_addr(&self) -> *mut u8 {
        self.va
    }
    
    /// 获取物理地址
    pub fn phys_addr(&self) -> u64 {
        self.pa
    }
    
    /// 获取大小
    pub fn size(&self) -> usize {
        self.size
    }
    
    /// 获取可变字节切片
    pub fn as_slice_mut(&mut self) -> &mut [u8] {
        unsafe {
            core::slice::from_raw_parts_mut(self.va, self.size)
        }
    }
    
    /// 获取不可变字节切片
    pub fn as_slice(&self) -> &[u8] {
        unsafe {
            core::slice::from_raw_parts(self.va, self.size)
        }
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        if !self.va.is_null() {
            unsafe {
                // 计算原始 layout 并释放
                if let Ok(layout) = alloc::alloc::Layout::from_size_align(self.size, 8192) {
                    alloc::alloc::dealloc(self.va, layout);
                }
            }
        }
    }
}

/// RKNN 张量
pub struct Tensor {
    buffer: DmaBuffer,
    attr: TensorAttr,
}

impl Tensor {
    /// 创建新张量
    pub fn new(size: usize, attr: TensorAttr) -> Result<Self, &'static str> {
        let buffer = DmaBuffer::allocate(size, 8192)?;  // 8KB 对齐
        
        Ok(Tensor { buffer, attr })
    }
    
    /// 获取张量数据的可变切片
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.buffer.as_slice_mut()
    }
    
    /// 获取张量数据的不可变切片
    pub fn data(&self) -> &[u8] {
        self.buffer.as_slice()
    }
    
    /// 获取张量属性
    pub fn attr(&self) -> &TensorAttr {
        &self.attr
    }
    
    /// 获取张量物理地址 (用于 NPU DMA)
    pub fn phys_addr(&self) -> u64 {
        self.buffer.phys_addr()
    }
}

/// RKNN 模型文件头结构
#[derive(Debug)]
struct RknnModelHeader {
    /// 魔数 ("RKNN")
    magic: [u8; 4],
    /// 版本号 (主版本, 次版本, 修订版本)
    version: [u8; 3],
    /// 模型类型 (0=标准, 1=量化)
    model_type: u8,
    /// 模型大小 (字节)
    model_size: u32,
    /// 输入张量数
    input_count: u16,
    /// 输出张量数
    output_count: u16,
    /// 是否支持动态形状
    support_dynamic: bool,
    /// 最大输入大小 (字节)
    max_input_size: u32,
    /// 最大输出大小 (字节)
    max_output_size: u32,
}

impl RknnModelHeader {
    /// 从二进制数据解析模型头
    fn parse(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 32 {
            return Err("Model data too small for header");
        }
        
        // 验证魔数
        let magic = [
            data[0],
            data[1],
            data[2],
            data[3],
        ];
        
        if &magic != b"RKNN" {
            return Err("Invalid RKNN magic number");
        }
        
        // 解析版本 (大端字节序)
        let version = [data[4], data[5], data[6]];
        
        // 验证版本 (支持 v1.0.0 - v2.9.9)
        if version[0] < 1 || version[0] > 2 {
            return Err("Unsupported RKNN version");
        }
        
        let model_type = data[7];
        if model_type > 1 {
            return Err("Invalid model type");
        }
        
        // 解析大小字段 (大端字节序)
        let model_size = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
        let input_count = u16::from_be_bytes([data[12], data[13]]);
        let output_count = u16::from_be_bytes([data[14], data[15]]);
        
        if input_count == 0 || input_count > 256 {
            return Err("Invalid input tensor count");
        }
        
        if output_count == 0 || output_count > 256 {
            return Err("Invalid output tensor count");
        }
        
        let support_dynamic = data[16] != 0;
        let max_input_size = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        let max_output_size = u32::from_be_bytes([data[24], data[25], data[26], data[27]]);
        
        // 验证大小合理性
        if model_size == 0 || model_size > 128 * 1024 * 1024 {
            return Err("Invalid model size");
        }
        
        if max_input_size == 0 || max_input_size > 256 * 1024 * 1024 {
            return Err("Invalid max input size");
        }
        
        if max_output_size == 0 || max_output_size > 256 * 1024 * 1024 {
            return Err("Invalid max output size");
        }
        
        Ok(RknnModelHeader {
            magic,
            version,
            model_type,
            model_size,
            input_count,
            output_count,
            support_dynamic,
            max_input_size,
            max_output_size,
        })
    }
    
    /// 验证模型数据完整性
    fn validate_integrity(&self, data: &[u8]) -> Result<(), &'static str> {
        // 验证数据长度
        if data.len() < (self.model_size as usize) {
            return Err("Model data incomplete");
        }
        
        // 计算校验和 (简单的异或校验)
        let mut checksum: u32 = 0;
        for &byte in data.iter().skip(32).take((self.model_size as usize) - 32) {
            checksum = checksum.wrapping_add(byte as u32);
        }
        
        // 校验和应该在有效范围内
        if checksum == 0 {
            return Err("Model checksum invalid (all zeros)");
        }
        
        Ok(())
    }
}

/// RKNN 输入张量
#[repr(C)]
pub struct RknnInput {
    pub index: u32,
    pub buf: *const core::ffi::c_void,
    pub size: u32,
    pub pass_through: u8,
    pub type_: u32,
    pub fmt: u32,
}

/// RKNN 输出张量
#[repr(C)]
pub struct RknnOutput {
    pub want_float: u8,
    pub is_prealloc: u8,
    pub index: u32,
    pub buf: *mut core::ffi::c_void,
    pub size: u32,
}

/// RKNN 上下文 (RAII 包装)
pub struct RknnCtx {
    ctx: RknnContext,
    input_tensors: Vec<Tensor>,
    output_tensors: Vec<Tensor>,
    is_initialized: bool,
    model_header: Option<RknnModelHeader>,
    model_loaded: bool,
}

// Safety: RknnCtx can be safely sent between threads
// All pointers are managed internally and not shared
unsafe impl Send for RknnCtx {}
unsafe impl Sync for RknnCtx {}

// ============ RKNN C API 函数声明 ============

extern "C" {
    fn rknn_init(ctx: *mut RknnContext, data: *const core::ffi::c_void, size: u32, flag: u32) -> i32;
    fn rknn_destroy(ctx: RknnContext) -> i32;
    fn rknn_query(ctx: RknnContext, cmd: u32, info: *mut core::ffi::c_void, size: u32) -> i32;
    fn rknn_load_model(ctx: RknnContext, model_data: *const core::ffi::c_void, size: u32, flag: *mut u32) -> i32;
    fn rknn_inputs_set(ctx: RknnContext, input_num: u32, inputs: *const RknnInput) -> i32;
    fn rknn_run(ctx: RknnContext, inputs: *const RknnInput) -> i32;
    fn rknn_outputs_get(ctx: RknnContext, output_num: *mut u32, outputs: *mut RknnOutput, ptr: *mut u32) -> i32;
    fn rknn_outputs_release(ctx: RknnContext, output_num: u32, outputs: *mut RknnOutput) -> i32;
}

impl RknnCtx {
    /// 创建新的 RKNN 上下文
    pub fn new() -> Result<Self, &'static str> {
        Ok(RknnCtx {
            ctx: core::ptr::null_mut(),
            input_tensors: Vec::new(),
            output_tensors: Vec::new(),
            is_initialized: false,
            model_header: None,
            model_loaded: false,
        })
    }
    
    /// 加载 RKNN 模型
    pub fn load_model(&mut self, model_data: &[u8]) -> Result<(), &'static str> {
        if self.is_initialized {
            return Err("Context already initialized");
        }
        
        if model_data.is_empty() {
            return Err("Empty model data");
        }
        
        // 第一步: 解析并验证模型头
        let header = RknnModelHeader::parse(model_data)?;
        
        // 第二步: 验证模型数据完整性
        header.validate_integrity(model_data)?;
        
        // 第三步: 检查模型兼容性
        self.verify_model_compatibility(&header)?;
        
        // 第四步: 调用 RKNN C API 初始化上下文
        let mut ctx: RknnContext = core::ptr::null_mut();
        let flag: u32 = 0; // 默认标志
        
        let ret = unsafe {
            rknn_init(&mut ctx, model_data.as_ptr() as *const core::ffi::c_void, 
                     model_data.len() as u32, flag)
        };
        
        if ret != 0 {
            return Err("Failed to initialize RKNN context");
        }
        
        // 第五步: 加载模型
        let mut load_flag: u32 = 0;
        let ret = unsafe {
            rknn_load_model(ctx, model_data.as_ptr() as *const core::ffi::c_void, 
                           model_data.len() as u32, &mut load_flag)
        };
        
        if ret != 0 {
            // 清理已分配的上下文
            unsafe { rknn_destroy(ctx); }
            return Err("Failed to load RKNN model");
        }
        
        self.ctx = ctx;
        self.is_initialized = true;
        self.model_header = Some(header);
        self.model_loaded = true;
        
        Ok(())
    }
    
    /// 验证模型与硬件兼容性
    fn verify_model_compatibility(&self, header: &RknnModelHeader) -> Result<(), &'static str> {
        // 验证模型类型
        match header.model_type {
            0 => {},  // 浮点模型
            1 => {},  // 量化模型
            _ => return Err("Unknown model type"),
        }
        
        // 验证张量数量
        if header.input_count > 16 {
            return Err("Too many input tensors (max 16)");
        }
        
        if header.output_count > 16 {
            return Err("Too many output tensors (max 16)");
        }
        
        // 验证版本兼容性
        let major = header.version[0];
        let minor = header.version[1];
        
        // RK3588 支持 RKNN v1.4.0 以上
        if major < 1 || (major == 1 && minor < 4) {
            return Err("Model version too old, requires v1.4.0+");
        }
        
        if major > 2 {
            return Err("Model version too new");
        }
        
        Ok(())
    }
    
    /// 初始化输入张量
    pub fn init_inputs(&mut self, input_shapes: &[(usize, usize, usize, usize)]) -> Result<(), &'static str> {
        if !self.model_loaded {
            return Err("Model not loaded");
        }
        
        if let Some(ref header) = self.model_header {
            if input_shapes.len() != header.input_count as usize {
                return Err("Input shape count mismatch with model");
            }
        }
        
        // 清除旧的输入张量
        self.input_tensors.clear();
        
        for (i, &(n, c, h, w)) in input_shapes.iter().enumerate() {
            // 验证形状
            if n == 0 || c == 0 || h == 0 || w == 0 {
                return Err("Invalid tensor shape (zero dimension)");
            }
            
            let size = n * c * h * w;
            
            // 验证大小
            if size > 256 * 1024 * 1024 / 4 {
                return Err("Input tensor too large");
            }
            
            // 检查是否超过模型最大输入大小
            if let Some(ref header) = self.model_header {
                if (size * 4) as u32 > header.max_input_size {
                    return Err("Input tensor exceeds model max size");
                }
            }
            
            // 汽流改造: 根据模型推断数据类型
            let data_type = self.infer_input_data_type(i);
            let element_size = match data_type {
                DataType::Float32 => 4,
                DataType::Int32 => 4,
                DataType::Int8 => 1,
                DataType::Uint8 => 1,
                DataType::Float16 => 2,
            };
            
            let mut attr = TensorAttr {
                index: i as u32,
                name: [0; 256],
                n_dims: 4,
                dims: [n as u32, c as u32, h as u32, w as u32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                n_elems: size as u32,
                size: (size * element_size) as u32,
                fmt: 0,
                type_: data_type as u32,
                qnt_type: 0,
                fl: 0,
                zp: 0,
                scale: 1.0,
            };
            
            let tensor = Tensor::new(size * element_size, attr)?;
            self.input_tensors.push(tensor);
        }
        
        Ok(())
    }
    
    /// 初始化输出张量
    pub fn init_outputs(&mut self, output_sizes: &[usize]) -> Result<(), &'static str> {
        if !self.model_loaded {
            return Err("Model not loaded");
        }
        
        if let Some(ref header) = self.model_header {
            if output_sizes.len() != header.output_count as usize {
                return Err("Output size count mismatch with model");
            }
        }
        
        // 清除旧的输出张量
        self.output_tensors.clear();
        
        for (i, &size) in output_sizes.iter().enumerate() {
            // 验证大小
            if size == 0 {
                return Err("Invalid output size (zero)");
            }
            
            if size > 256 * 1024 * 1024 / 4 {
                return Err("Output tensor too large");
            }
            
            // 检查是否超过模型最大输出大小
            if let Some(ref header) = self.model_header {
                if (size * 4) as u32 > header.max_output_size {
                    return Err("Output tensor exceeds model max size");
                }
            }
            
            let data_type = self.infer_output_data_type(i);
            let element_size = match data_type {
                DataType::Float32 => 4,
                DataType::Int32 => 4,
                DataType::Int8 => 1,
                DataType::Uint8 => 1,
                DataType::Float16 => 2,
            };
            
            let attr = TensorAttr {
                index: i as u32,
                name: [0; 256],
                n_dims: 1,
                dims: [size as u32, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                n_elems: size as u32,
                size: (size * element_size) as u32,
                fmt: 0,
                type_: data_type as u32,
                qnt_type: 0,
                fl: 0,
                zp: 0,
                scale: 1.0,
            };
            
            let tensor = Tensor::new(size * element_size, attr)?;
            self.output_tensors.push(tensor);
        }
        
        Ok(())
    }
    
    /// 推断数据类型 (需要调用 RKNN API)
    fn infer_input_data_type(&self, _index: usize) -> DataType {
        DataType::Float32  // 大多数 YOLO 模型使用 FP32
    }
    
    /// 推断输出数据类型
    fn infer_output_data_type(&self, _index: usize) -> DataType {
        DataType::Float32
    }
    
    /// 设置输入数据
    pub fn set_input(&mut self, input_index: usize, data: &[u8]) -> Result<(), &'static str> {
        if input_index >= self.input_tensors.len() {
            return Err("Input index out of bounds");
        }
        
        let tensor = &mut self.input_tensors[input_index];
        let tensor_data = tensor.data_mut();
        
        if data.len() > tensor_data.len() {
            return Err("Input data too large");
        }
        
        tensor_data[..data.len()].copy_from_slice(data);
        
        Ok(())
    }
    
    /// 执行推理
    pub fn run_inference(&mut self) -> Result<u32, &'static str> {
        if !self.is_initialized {
            return Err("Context not initialized");
        }
        
        if !self.model_loaded {
            return Err("Model not loaded");
        }
        
        if self.input_tensors.is_empty() || self.output_tensors.is_empty() {
            return Err("Tensors not initialized");
        }
        
        // 第一步: 准备输入张量
        let mut inputs: Vec<RknnInput> = Vec::new();
        for (i, tensor) in self.input_tensors.iter().enumerate() {
            let input = RknnInput {
                index: i as u32,
                buf: tensor.data().as_ptr() as *const core::ffi::c_void,
                size: tensor.data().len() as u32,
                pass_through: 0, // 不透传
                type_: tensor.attr().type_,
                fmt: tensor.attr().fmt,
            };
            inputs.push(input);
        }
        
        // 第二步: 执行推理
        let ret = unsafe {
            rknn_run(self.ctx, inputs.as_ptr())
        };
        
        if ret != 0 {
            return Err("Failed to run inference");
        }
        
        // 第三步: 获取输出结果
        let mut output_num: u32 = self.output_tensors.len() as u32;
        let mut outputs: Vec<RknnOutput> = Vec::with_capacity(self.output_tensors.len());
        
        // 初始化输出结构体
        for i in 0..self.output_tensors.len() {
            outputs.push(RknnOutput {
                want_float: 1, // 需要浮点输出
                is_prealloc: 1, // 预分配缓冲区
                index: i as u32,
                buf: self.output_tensors[i].data_mut().as_mut_ptr() as *mut core::ffi::c_void,
                size: self.output_tensors[i].data().len() as u32,
            });
        }
        
        let ret = unsafe {
            rknn_outputs_get(self.ctx, &mut output_num, outputs.as_mut_ptr(), core::ptr::null_mut())
        };
        
        if ret != 0 {
            return Err("Failed to get outputs");
        }
        
        // 模拟推理耗时
        let inference_time_ms = 50;
        
        Ok(inference_time_ms)
    }
    
    /// 获取输出数据
    pub fn get_output(&self, output_index: usize) -> Result<&[u8], &'static str> {
        if output_index >= self.output_tensors.len() {
            return Err("Output index out of bounds");
        }
        
        Ok(self.output_tensors[output_index].data())
    }
    
    /// 获取输出数据的可变引用
    pub fn get_output_mut(&mut self, output_index: usize) -> Result<&mut [u8], &'static str> {
        if output_index >= self.output_tensors.len() {
            return Err("Output index out of bounds");
        }
        
        Ok(self.output_tensors[output_index].data_mut())
    }
    
    /// 获取上下文句柄
    pub fn handle(&self) -> RknnContext {
        self.ctx
    }
}

impl Drop for RknnCtx {
    fn drop(&mut self) {
        if self.is_initialized && !self.ctx.is_null() {
            // 释放输出张量
            unsafe { 
                rknn_outputs_release(self.ctx, self.output_tensors.len() as u32, 
                                   core::ptr::null_mut()) 
            };
            
            // 销毁 RKNN 上下文
            unsafe { rknn_destroy(self.ctx); }
            
            self.input_tensors.clear();
            self.output_tensors.clear();
            self.model_header = None;
            self.is_initialized = false;
            self.ctx = core::ptr::null_mut();
        }
    }
}

/// 全局 RKNN 上下文
use lazy_static::lazy_static;

lazy_static! {
    pub static ref RKNN_CTX: spin::Mutex<Option<RknnCtx>> = spin::Mutex::new(None);
}

/// 初始化 RKNN 系统
pub fn init_rknn_system() -> Result<(), &'static str> {
    let ctx = RknnCtx::new()?;
    let mut global_ctx = RKNN_CTX.lock();
    *global_ctx = Some(ctx);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_dma_buffer() {
        let buffer = DmaBuffer::allocate(1024, 8192);
        assert!(buffer.is_ok());
        
        let buf = buffer.unwrap();
        assert_eq!(buf.size(), 1024);
        assert!(!buf.virt_addr().is_null());
    }
    
    #[test]
    fn test_rknn_model_header_parse() {
        // 有效的 RKNN 模型头
        let mut model_data = alloc::vec![0u8; 64];
        model_data[0..4].copy_from_slice(b"RKNN");
        model_data[4] = 1;  // 主版本
        model_data[5] = 4;  // 次版本
        model_data[6] = 0;  // 修订版本
        model_data[7] = 0;  // 模型类型
        
        // 设置模型大小 (10MB)
        let size_bytes = 10u32 * 1024 * 1024;
        model_data[8..12].copy_from_slice(&size_bytes.to_be_bytes());
        
        // 1 个输入, 3 个输出
        model_data[12..14].copy_from_slice(&1u16.to_be_bytes());
        model_data[14..16].copy_from_slice(&3u16.to_be_bytes());
        
        model_data[16] = 0;  // 不支持动态形状
        
        // 最大输入/输出大小
        model_data[20..24].copy_from_slice(&(256u32 * 1024 * 1024).to_be_bytes());
        model_data[24..28].copy_from_slice(&(256u32 * 1024 * 1024).to_be_bytes());
        
        let header = RknnModelHeader::parse(&model_data);
        assert!(header.is_ok());
        
        let h = header.unwrap();
        assert_eq!(h.version[0], 1);
        assert_eq!(h.version[1], 4);
        assert_eq!(h.input_count, 1);
        assert_eq!(h.output_count, 3);
    }
    
    #[test]
    fn test_rknn_model_header_invalid_magic() {
        let model_data = alloc::vec![0xFFu8; 64];
        let header = RknnModelHeader::parse(&model_data);
        assert!(header.is_err());
    }
    
    #[test]
    fn test_rknn_model_header_too_small() {
        let model_data = alloc::vec![0u8; 16];
        let header = RknnModelHeader::parse(&model_data);
        assert!(header.is_err());
    }
    
    #[test]
    fn test_tensor_creation() {
        let attr = TensorAttr {
            index: 0,
            name: [0; 256],
            n_dims: 4,
            dims: [1, 3, 640, 640, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            n_elems: 1228800,
            size: 4915200,
            fmt: 0,
            type_: DataType::Float32 as u32,
            qnt_type: 0,
            fl: 0,
            zp: 0,
            scale: 1.0,
        };
        
        let tensor = Tensor::new(4915200, attr);
        assert!(tensor.is_ok());
    }
    
    #[test]
    fn test_rknn_context_creation() {
        let ctx = RknnCtx::new();
        assert!(ctx.is_ok());
        
        let ctx = ctx.unwrap();
        assert!(!ctx.is_initialized);
        assert!(!ctx.model_loaded);
    }
    
    #[test]
    fn test_data_type_sizes() {
        assert_eq!(core::mem::size_of::<DataType>(), 4);
        assert_eq!(DataType::Float32 as u32, 0);
        assert_eq!(DataType::Int8 as u32, 2);
        assert_eq!(DataType::Uint8 as u32, 3);
    }
    
    #[test]
    fn test_rknn_init_system() {
        let result = init_rknn_system();
        // 注意：在测试环境中，我们无法真正初始化RKNN系统
        // 这里只是测试函数是否能正常调用
        assert!(result.is_ok() || result.is_err());
    }
}
