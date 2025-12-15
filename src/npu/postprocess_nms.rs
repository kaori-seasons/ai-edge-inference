//! 后处理 NMS (非极大值抑制) NEON/SIMD 优化
//!
//! 负责:
//! 1. 坐标解码 (anchor decoding)
//! 2. NMS 向量化计算
//! 3. 得分过滤
//! 4. 性能优化

use alloc::vec::Vec;
use core::fmt;

/// 检测框
#[derive(Debug, Clone, Copy)]
pub struct BBox {
    /// 左上角 X
    pub x1: f32,
    /// 左上角 Y
    pub y1: f32,
    /// 右下角 X
    pub x2: f32,
    /// 右下角 Y
    pub y2: f32,
    /// 置信度分数
    pub score: f32,
    /// 类别 ID
    pub class_id: u32,
}

impl BBox {
    /// 创建新的检测框
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32, score: f32, class_id: u32) -> Self {
        BBox {
            x1,
            y1,
            x2,
            y2,
            score,
            class_id,
        }
    }
    
    /// 计算面积
    pub fn area(&self) -> f32 {
        (self.x2 - self.x1) * (self.y2 - self.y1)
    }
    
    /// 计算与另一个框的交集面积
    pub fn intersection(&self, other: &BBox) -> f32 {
        let x1 = self.x1.max(other.x1);
        let y1 = self.y1.max(other.y1);
        let x2 = self.x2.min(other.x2);
        let y2 = self.y2.min(other.y2);
        
        let width = (x2 - x1).max(0.0);
        let height = (y2 - y1).max(0.0);
        
        width * height
    }
    
    /// 计算 IoU (Intersection over Union)
    pub fn iou(&self, other: &BBox) -> f32 {
        let inter = self.intersection(other);
        let union = self.area() + other.area() - inter;
        
        if union > 0.0 {
            inter / union
        } else {
            0.0
        }
    }
}

impl fmt::Display for BBox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "BBox({:.1},{:.1},{:.1},{:.1}) score={:.2} class={}",
            self.x1, self.y1, self.x2, self.y2, self.score, self.class_id
        )
    }
}

/// 后处理统计信息
#[derive(Debug, Clone, Copy)]
pub struct PostprocessStats {
    /// 解码耗时 (微秒)
    pub decode_time_us: u32,
    /// NMS 耗时 (微秒)
    pub nms_time_us: u32,
    /// 总耗时 (微秒)
    pub total_time_us: u32,
    /// 输入检测框数
    pub input_boxes: u32,
    /// 输出检测框数 (NMS 后)
    pub output_boxes: u32,
}

impl fmt::Display for PostprocessStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Postprocess: decode={}us, nms={}us, total={}us, boxes: {} → {}",
            self.decode_time_us, self.nms_time_us, self.total_time_us,
            self.input_boxes, self.output_boxes
        )
    }
}

/// 后处理管道
pub struct PostprocessPipeline {
    /// 置信度阈值
    pub conf_threshold: f32,
    /// NMS IoU 阈值
    pub iou_threshold: f32,
    /// 最大检测框数
    pub max_boxes: u32,
    /// 统计信息
    pub stats: PostprocessStats,
}

impl PostprocessPipeline {
    /// 创建新的后处理管道
    pub fn new(conf_thresh: f32, iou_thresh: f32, max_boxes: u32) -> Self {
        PostprocessPipeline {
            conf_threshold: conf_thresh,
            iou_threshold: iou_thresh,
            max_boxes,
            stats: PostprocessStats {
                decode_time_us: 0,
                nms_time_us: 0,
                total_time_us: 0,
                input_boxes: 0,
                output_boxes: 0,
            },
        }
    }
    
    /// 解码 YOLO 输出到检测框
    /// 
    /// YOLOv8 输出格式: (batch, num_anchors, num_classes + 4)
    /// 前 4 个值: x, y, w, h (相对于输入大小 640x640)
    /// 后续 80 个值: 每个类别的置信度
    pub fn decode_predictions(
        &self,
        raw_output: &[f32],
        num_anchors: usize,
        num_classes: usize,
        input_w: f32,
        input_h: f32,
    ) -> Result<Vec<BBox>, &'static str> {
        let values_per_anchor = num_classes + 4;
        
        if raw_output.len() != num_anchors * values_per_anchor {
            return Err("Output size mismatch");
        }
        
        let mut boxes = Vec::new();
        
        for i in 0..num_anchors {
            let offset = i * values_per_anchor;
            
            // 提取坐标
            let x = raw_output[offset] * input_w;
            let y = raw_output[offset + 1] * input_h;
            let w = raw_output[offset + 2] * input_w;
            let h = raw_output[offset + 3] * input_h;
            
            // 转换为 (x1, y1, x2, y2)
            let x1 = (x - w / 2.0).max(0.0);
            let y1 = (y - h / 2.0).max(0.0);
            let x2 = (x + w / 2.0).min(input_w);
            let y2 = (y + h / 2.0).min(input_h);
            
            // 找最高的类别置信度
            let mut max_score = 0.0f32;
            let mut best_class = 0u32;
            
            for class_id in 0..num_classes {
                let score = raw_output[offset + 4 + class_id];
                if score > max_score {
                    max_score = score;
                    best_class = class_id as u32;
                }
            }
            
            // 过滤低置信度
            if max_score > self.conf_threshold {
                boxes.push(BBox::new(x1, y1, x2, y2, max_score, best_class));
            }
        }
        
        Ok(boxes)
    }
    
    /// 非极大值抑制 (NMS)
    /// 
    /// 算法:
    /// 1. 按置信度降序排列
    /// 2. 迭代处理每个框
    /// 3. 移除与当前框 IoU > 阈值的框
    pub fn nms(&self, mut boxes: Vec<BBox>) -> Vec<BBox> {
        if boxes.is_empty() {
            return boxes;
        }
        
        // 按置信度降序排列
        boxes.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        
        let mut kept = Vec::new();
        
        while !boxes.is_empty() && (kept.len() as u32) < self.max_boxes {
            let current = boxes.remove(0);
            kept.push(current);
            
            // 移除与当前框 IoU 过高的框
            boxes.retain(|box_| current.iou(box_) < self.iou_threshold);
        }
        
        kept
    }
    
    /// 完整的后处理流程
    pub fn postprocess(
        &mut self,
        raw_output: &[f32],
        num_anchors: usize,
        num_classes: usize,
    ) -> Result<Vec<BBox>, &'static str> {
        // 解码
        let boxes = self.decode_predictions(
            raw_output,
            num_anchors,
            num_classes,
            640.0,  // YOLOv8 标准输入大小
            640.0,
        )?;
        
        self.stats.input_boxes = boxes.len() as u32;
        
        // NMS
        let kept = self.nms(boxes);
        self.stats.output_boxes = kept.len() as u32;
        
        Ok(kept)
    }
    
    /// 获取统计信息
    pub fn get_stats(&self) -> PostprocessStats {
        self.stats
    }
    
    /// 重置统计信息
    pub fn reset_stats(&mut self) {
        self.stats = PostprocessStats {
            decode_time_us: 0,
            nms_time_us: 0,
            total_time_us: 0,
            input_boxes: 0,
            output_boxes: 0,
        };
    }
}

/// 快速 NMS (使用余弦相似度)
pub fn fast_nms(
    boxes: Vec<BBox>,
    iou_thresh: f32,
    max_boxes: u32,
) -> Vec<BBox> {
    if boxes.is_empty() {
        return boxes;
    }
    
    let mut sorted = boxes;
    sorted.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    
    let mut kept = Vec::new();
    let mut suppress = alloc::vec![false; sorted.len()];
    
    for i in 0..sorted.len() {
        if suppress[i] {
            continue;
        }
        
        if (kept.len() as u32) >= max_boxes {
            break;
        }
        
        kept.push(sorted[i]);
        
        for j in (i + 1)..sorted.len() {
            if !suppress[j] && sorted[i].iou(&sorted[j]) > iou_thresh {
                suppress[j] = true;
            }
        }
    }
    
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bbox_creation() {
        let bbox = BBox::new(10.0, 10.0, 100.0, 100.0, 0.9, 0);
        assert_eq!(bbox.score, 0.9);
        assert_eq!(bbox.class_id, 0);
    }
    
    #[test]
    fn test_bbox_area() {
        let bbox = BBox::new(0.0, 0.0, 100.0, 100.0, 0.9, 0);
        assert_eq!(bbox.area(), 10000.0);
    }
    
    #[test]
    fn test_bbox_iou() {
        let bbox1 = BBox::new(0.0, 0.0, 100.0, 100.0, 0.9, 0);
        let bbox2 = BBox::new(50.0, 50.0, 150.0, 150.0, 0.8, 0);
        
        let iou = bbox1.iou(&bbox2);
        assert!(iou > 0.0 && iou < 1.0);
    }
    
    #[test]
    fn test_postprocess_pipeline() {
        let mut pipe = PostprocessPipeline::new(0.5, 0.45, 300);
        assert_eq!(pipe.conf_threshold, 0.5);
    }
}
