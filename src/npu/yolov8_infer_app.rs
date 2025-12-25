//! YOLOv8 目标识别应用
//!
//! 实现完整的推理流程:
//! 1. 从 MIPI-CSI 获取图像帧
//! 2. 预处理 (缩放+归一化)
//! 3. NPU 推理
//! 4. 后处理 (NMS+坐标解码)
//! 5. 通过 CAN 输出结果

use alloc::vec::Vec;
use core::fmt;
use alloc::format;
use super::rknn_binding_sys::ModelType;

// ============ 检测结果结构 ============

/// 单个目标检测框
#[derive(Debug, Clone)]
pub struct Detection {
    /// 目标类别 ID (0-79 for COCO)
    pub class_id: u32,
    
    /// 置信度分数 (0.0-1.0)
    pub confidence: f32,
    
    /// 边界框 (x, y, w, h) - 相对于原始图像
    pub bbox: (f32, f32, f32, f32),
}

impl fmt::Display for Detection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Class: {}, Confidence: {:.2}, BBox: ({:.1}, {:.1}, {:.1}, {:.1})",
            self.class_id, self.confidence, self.bbox.0, self.bbox.1, self.bbox.2, self.bbox.3
        )
    }
}

/// 推理结果
#[derive(Debug, Clone)]
pub struct InferenceResult {
    /// 检测到的目标列表
    pub detections: Vec<Detection>,
    
    /// 推理耗时 (毫秒)
    pub inference_time_ms: u32,
    
    /// 处理耗时 (毫秒)
    pub process_time_ms: u32,
}

// ============ YOLOv8 应用 ============

pub struct Yolov8App {
    /// 模型名称
    model_name: &'static str,
    
    /// 输入分辨率
    input_size: (u32, u32),
    
    /// 类别数
    num_classes: u32,
    
    /// 置信度阈值
    conf_threshold: f32,
    
    /// IoU NMS 阈值
    iou_threshold: f32,
    
    /// 最大检测数
    max_detections: u32,
}

impl Yolov8App {
    /// 创建新的 YOLOv8 应用
    pub fn new() -> Self {
        Yolov8App {
            model_name: "YOLOv8n",
            input_size: (640, 640),
            num_classes: 80,  // COCO 数据集
            conf_threshold: 0.5,
            iou_threshold: 0.45,
            max_detections: 300,
        }
    }
    
    /// 预处理图像
    /// 
    /// 操作:
    /// 1. 缩放到 640x640
    /// 2. 转换格式 (BGR → RGB)
    /// 3. 归一化 (0-255 → 0-1)
    pub fn preprocess_image(
        &self,
        input_data: &[u8],
        input_w: u32,
        input_h: u32,
    ) -> Result<Vec<f32>, &'static str> {
        if input_data.len() != (input_w * input_h * 3) as usize {
            return Err("Invalid input size");
        }
        
        let (target_w, target_h) = self.input_size;
        let mut output = Vec::with_capacity((target_w * target_h * 3) as usize);
        
        // 简化的缩放和转换 (实际应使用 NEON 优化)
        let scale_w = input_w as f32 / target_w as f32;
        let scale_h = input_h as f32 / target_h as f32;
        
        for y in 0..target_h {
            for x in 0..target_w {
                let src_x = ((x as f32 * scale_w) as u32).min(input_w - 1);
                let src_y = ((y as f32 * scale_h) as u32).min(input_h - 1);
                
                let idx = ((src_y * input_w + src_x) * 3) as usize;
                
                // BGR 读取, RGB 输出 (交换 B 和 R)
                let b = input_data[idx] as f32 / 255.0;
                let g = input_data[idx + 1] as f32 / 255.0;
                let r = input_data[idx + 2] as f32 / 255.0;
                
                output.push(r);
                output.push(g);
                output.push(b);
            }
        }
        
        Ok(output)
    }
    
    /// 后处理推理输出
    /// 
    /// 操作:
    /// 1. 解析原始输出张量
    /// 2. 置信度过滤
    /// 3. NMS (非极大值抑制)
    /// 4. 坐标解码
    pub fn postprocess_output(
        &self,
        output_data: &[f32],
        input_w: u32,
        input_h: u32,
    ) -> Result<Vec<Detection>, &'static str> {
        let mut detections = Vec::new();
        
        // YOLOv8 输出格式: [x, y, w, h, conf, class1, class2, ...]
        // 解析输出 (简化实现)
        let stride = (4 + 1 + self.num_classes) as usize;
        let num_predictions = output_data.len() / stride;
        
        if num_predictions == 0 {
            return Ok(detections);
        }
        
        let mut candidates: Vec<_> = (0..num_predictions)
            .filter_map(|i| {
                let offset = i * stride;
                if offset + stride > output_data.len() {
                    return None;
                }
                
                let x = output_data[offset];
                let y = output_data[offset + 1];
                let w = output_data[offset + 2];
                let h = output_data[offset + 3];
                let conf = output_data[offset + 4];
                
                if conf < self.conf_threshold {
                    return None;
                }
                
                // 找最高的类别置信度
                let mut max_class_conf = 0.0f32;
                let mut max_class_id = 0u32;
                
                for class_id in 0..self.num_classes {
                    let class_conf = output_data[offset + 5 + class_id as usize];
                    if class_conf > max_class_conf {
                        max_class_conf = class_conf;
                        max_class_id = class_id;
                    }
                }
                
                let final_conf = conf * max_class_conf;
                
                if final_conf < self.conf_threshold {
                    return None;
                }
                
                Some((
                    Detection {
                        class_id: max_class_id,
                        confidence: final_conf,
                        bbox: (x, y, w, h),
                    },
                    final_conf,
                ))
            })
            .collect();
        
        // 按置信度排序 (用于 NMS)
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        
        // NMS 过滤 (简化实现)
        while !candidates.is_empty() && detections.len() < self.max_detections as usize {
            let (detection, _) = candidates.remove(0);
            
            // 检查与已有检测的 IoU
            let (x1, y1, w1, h1) = detection.bbox;
            let area1 = w1 * h1;
            
            candidates.retain(|(other, _)| {
                let (x2, y2, w2, h2) = other.bbox;
                
                // 计算 IoU
                let x_inter_min = x1.max(x2);
                let y_inter_min = y1.max(y2);
                let x_inter_max = (x1 + w1).min(x2 + w2);
                let y_inter_max = (y1 + h1).min(y2 + h2);
                
                if x_inter_max <= x_inter_min || y_inter_max <= y_inter_min {
                    return true;  // 保留 (不重叠)
                }
                
                let inter_area = (x_inter_max - x_inter_min) * (y_inter_max - y_inter_min);
                let area2 = w2 * h2;
                let union_area = area1 + area2 - inter_area;
                let iou = inter_area / union_area;
                
                iou <= self.iou_threshold  // 如果 IoU 低于阈值则保留
            });
            
            detections.push(detection);
        }
        
        Ok(detections)
    }
    
    /// 完整的推理流程
    pub fn infer(
        &self,
        input_data: &[u8],
        input_w: u32,
        input_h: u32,
        output_data: &[f32],
    ) -> Result<InferenceResult, &'static str> {
        let start_time = get_time_ms();
        
        // 1. 预处理
        let _preprocessed = self.preprocess_image(input_data, input_w, input_h)?;
        
        // 2. 推理 (在实际应用中, 这里会调用 NPU)
        // let inference_output = rknn_ctx.run_inference(&preprocessed)?;
        
        let inference_time = get_time_ms() - start_time;
        
        // 3. 后处理
        let start_post = get_time_ms();
        let detections = self.postprocess_output(output_data, input_w, input_h)?;
        let process_time = get_time_ms() - start_post;
        
        Ok(InferenceResult {
            detections,
            inference_time_ms: inference_time as u32,
            process_time_ms: process_time as u32,
        })
    }
    
    /// 转换检测结果为 CAN 消息
    pub fn detection_to_can_message(&self, detection: &Detection) -> CanMessage {
        CanMessage {
            can_id: 0x123,  // 目标识别消息 ID
            dlc: 8,
            data: [
                detection.class_id as u8,
                (detection.confidence * 255.0) as u8,
                (detection.bbox.0 as u8),
                (detection.bbox.1 as u8),
                (detection.bbox.2 as u8),
                (detection.bbox.3 as u8),
                0,
                0,
            ],
        }
    }
    
    /// 获取模型类型 (用于RKNN上下文配置)
    pub fn model_type() -> ModelType {
        ModelType::ObjectDetection
    }
}

/// CAN 消息用于传输检测结果
#[derive(Debug, Clone)]
pub struct CanMessage {
    pub can_id: u32,
    pub dlc: u8,
    pub data: [u8; 8],
}

/// 获取当前时间 (毫秒)
fn get_time_ms() -> u64 {
    0  // 占位实现
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_yolov8_app_creation() {
        let app = Yolov8App::new();
        assert_eq!(app.input_size, (640, 640));
        assert_eq!(app.num_classes, 80);
    }
    
    #[test]
    fn test_detection_display() {
        let detection = Detection {
            class_id: 0,
            confidence: 0.95,
            bbox: (100.0, 100.0, 50.0, 75.0),
        };
        
        let display_str = format!("{}", detection);
        assert!(display_str.contains("Class: 0"));
        assert!(display_str.contains("0.95"));
    }
    
    #[test]
    fn test_model_type() {
        assert_eq!(Yolov8App::model_type(), ModelType::ObjectDetection);
    }
}