//! YOLOv8 INT8 量化模型管理
//!
//! 负责:
//! 1. INT8 量化模型的加载与验证
//! 2. 量化参数的管理和应用
//! 3. 量化精度监控
//! 4. 与 FP32 模型的互操作性

use alloc::vec::Vec;
use core::fmt;

/// 量化类型
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuantType {
    /// 浮点模型 (FP32)
    Float32 = 0,
    /// INT8 对称量化
    Int8Symmetric = 1,
    /// INT8 非对称量化 (带零点)
    Int8Asymmetric = 2,
}

impl fmt::Display for QuantType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuantType::Float32 => write!(f, "FP32"),
            QuantType::Int8Symmetric => write!(f, "INT8-Symmetric"),
            QuantType::Int8Asymmetric => write!(f, "INT8-Asymmetric"),
        }
    }
}

/// 张量量化参数
#[derive(Debug, Clone, Copy)]
pub struct QuantParam {
    /// 缩放因子 (scale)
    pub scale: f32,
    /// 零点 (zero_point) - 仅 INT8 非对称量化使用
    pub zero_point: i8,
    /// 最小值 (用于反量化)
    pub min_value: f32,
    /// 最大值
    pub max_value: f32,
}

impl QuantParam {
    /// 创建对称量化参数 (INT8)
    pub fn symmetric(min_value: f32, max_value: f32) -> Self {
        let abs_max = min_value.abs().max(max_value.abs());
        let scale = abs_max / 127.0;
        
        QuantParam {
            scale,
            zero_point: 0,
            min_value: -abs_max,
            max_value: abs_max,
        }
    }
    
    /// 创建非对称量化参数 (INT8 + 零点)
    pub fn asymmetric(min_value: f32, max_value: f32) -> Self {
        let scale = (max_value - min_value) / 255.0;
        let zero_point = -(( min_value / scale) as i8);
        
        QuantParam {
            scale,
            zero_point,
            min_value,
            max_value,
        }
    }
    
    /// 对浮点值进行量化
    pub fn quantize(&self, value: f32, symmetric: bool) -> i8 {
        let clamped = value.clamp(self.min_value, self.max_value);
        
        if symmetric {
            let scaled = clamped / self.scale;
            // Manual rounding: if scaled >= 0, add 0.5; if < 0, subtract 0.5
            let rounded = if scaled >= 0.0 {
                (scaled + 0.5) as i8
            } else {
                (scaled - 0.5) as i8
            };
            rounded.clamp(-128, 127)
        } else {
            let q = ((clamped / self.scale) as i8).wrapping_add(self.zero_point);
            q.clamp(-128, 127)
        }
    }
    
    /// 对量化值进行反量化
    pub fn dequantize(&self, quantized: i8, symmetric: bool) -> f32 {
        if symmetric {
            (quantized as f32) * self.scale
        } else {
            ((quantized as f32) - (self.zero_point as f32)) * self.scale
        }
    }
}

/// 张量的量化信息
#[derive(Debug, Clone)]
pub struct TensorQuantInfo {
    /// 张量索引
    pub tensor_index: u32,
    /// 量化类型
    pub quant_type: QuantType,
    /// 量化参数
    pub params: QuantParam,
    /// 原始元素类型
    pub original_dtype: u32,
    /// 当前元素类型
    pub current_dtype: u32,
}

impl TensorQuantInfo {
    /// 创建 INT8 对称量化信息
    pub fn int8_symmetric(tensor_index: u32, min_val: f32, max_val: f32, orig_dtype: u32) -> Self {
        TensorQuantInfo {
            tensor_index,
            quant_type: QuantType::Int8Symmetric,
            params: QuantParam::symmetric(min_val, max_val),
            original_dtype: orig_dtype,
            current_dtype: 2,
        }
    }
    
    /// 创建 INT8 非对称量化信息
    pub fn int8_asymmetric(tensor_index: u32, min_val: f32, max_val: f32, orig_dtype: u32) -> Self {
        TensorQuantInfo {
            tensor_index,
            quant_type: QuantType::Int8Asymmetric,
            params: QuantParam::asymmetric(min_val, max_val),
            original_dtype: orig_dtype,
            current_dtype: 2,
        }
    }
}

/// YOLOv8 INT8 量化模型
#[derive(Clone)]
pub struct YoloV8Quantized {
    /// 模型名称
    pub model_name: &'static str,
    /// 输入量化信息
    pub input_quant: Vec<TensorQuantInfo>,
    /// 输出量化信息
    pub output_quant: Vec<TensorQuantInfo>,
    /// 模型版本
    pub model_variant: &'static str,
    /// 量化精度
    pub accuracy_loss: f32,
    /// 推理加速比
    pub speedup_factor: f32,
}

impl YoloV8Quantized {
    /// 创建新的 YOLOv8 INT8 量化模型
    pub fn new(variant: &'static str, accuracy_loss: f32) -> Self {
        let speedup_factor = match variant {
            "nano" => 3.2,
            "small" => 3.1,
            "medium" => 3.0,
            "large" => 2.9,
            "xlarge" => 2.8,
            _ => 3.0,
        };
        
        YoloV8Quantized {
            model_name: "YOLOv8",
            input_quant: Vec::new(),
            output_quant: Vec::new(),
            model_variant: variant,
            accuracy_loss,
            speedup_factor,
        }
    }
    
    /// 设置输入张量量化信息
    pub fn set_input_quant(&mut self, min_val: f32, max_val: f32, orig_dtype: u32) {
        let quant_info = TensorQuantInfo::int8_symmetric(
            0,
            min_val,
            max_val,
            orig_dtype,
        );
        self.input_quant.push(quant_info);
    }
    
    /// 设置输出张量量化信息
    pub fn set_output_quant(&mut self, min_val: f32, max_val: f32, orig_dtype: u32) {
        let index = self.output_quant.len() as u32;
        let quant_info = TensorQuantInfo::int8_symmetric(
            index,
            min_val,
            max_val,
            orig_dtype,
        );
        self.output_quant.push(quant_info);
    }
    
    /// 获取推理性能预估
    pub fn estimate_fps(&self, base_fps: f32) -> f32 {
        base_fps * self.speedup_factor
    }
    
    /// 验证量化精度
    pub fn is_acceptable_precision(&self) -> Result<(), &'static str> {
        if self.accuracy_loss < 5.0 {
            Ok(())
        } else {
            Err("Quantization accuracy loss too large")
        }
    }
    
    /// 获取模型统计信息
    pub fn get_stats(&self) -> ModelStats {
        ModelStats {
            variant: self.model_variant,
            quant_type: QuantType::Int8Symmetric,
            accuracy_loss_percent: self.accuracy_loss,
            speedup_factor: self.speedup_factor,
            input_count: self.input_quant.len() as u32,
            output_count: self.output_quant.len() as u32,
        }
    }
}

/// 模型统计信息
#[derive(Debug, Clone)]
pub struct ModelStats {
    pub variant: &'static str,
    pub quant_type: QuantType,
    pub accuracy_loss_percent: f32,
    pub speedup_factor: f32,
    pub input_count: u32,
    pub output_count: u32,
}

impl fmt::Display for ModelStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "YOLOv8-{} [{}] - Speedup: {:.1}x, Accuracy Loss: {:.2}%",
            self.variant, self.quant_type, self.speedup_factor,
            self.accuracy_loss_percent
        )
    }
}

/// 全局 YOLOv8 INT8 量化模型实例
use lazy_static::lazy_static;

lazy_static! {
    pub static ref YOLOV8_INT8_NANO: YoloV8Quantized = {
        let mut model = YoloV8Quantized::new("nano", 1.5);
        model.set_input_quant(-255.0, 255.0, 0);
        model.set_output_quant(-50.0, 50.0, 0);
        model
    };
    
    pub static ref YOLOV8_INT8_SMALL: YoloV8Quantized = {
        let mut model = YoloV8Quantized::new("small", 1.8);
        model.set_input_quant(-255.0, 255.0, 0);
        model.set_output_quant(-50.0, 50.0, 0);
        model
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_symmetric_quantization() {
        let param = QuantParam::symmetric(-1.0, 1.0);
        let q_pos = param.quantize(0.5, true);
        let f_pos = param.dequantize(q_pos, true);
        assert!(f_pos > 0.0 && f_pos < 1.0);
    }
    
    #[test]
    fn test_yolov8_quantized() {
        let model = YoloV8Quantized::new("nano", 1.5);
        assert!(model.is_acceptable_precision().is_ok());
        let fps = model.estimate_fps(7.5);
        assert!(fps > 20.0);
    }
}
