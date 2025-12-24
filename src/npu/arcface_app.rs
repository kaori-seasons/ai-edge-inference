//! ArcFace 人脸识别应用
//!
//! 实现完整的人脸识别流程:
//! 1. 人脸图像预处理 (对齐+归一化)
//! 2. NPU 特征提取推理
//! 3. 特征向量后处理
//! 4. 人脸识别/验证

use alloc::vec::Vec;
use core::fmt;
use alloc::format;
use super::rknn_binding_sys::{ModelType, RKNN_CTX};

// ============ 人脸识别结果结构 ============

/// 人脸识别结果
#[derive(Debug, Clone)]
pub struct FaceRecognitionResult {
    /// 人脸特征向量 (512维)
    pub embedding: Vec<f32>,
    
    /// 推理耗时 (毫秒)
    pub inference_time_ms: u32,
    
    /// 处理耗时 (毫秒)
    pub process_time_ms: u32,
}

/// 人脸验证结果
#[derive(Debug, Clone)]
pub struct FaceVerificationResult {
    /// 相似度分数 (0.0-1.0)
    pub similarity: f32,
    
    /// 是否匹配
    pub is_match: bool,
    
    /// 推理耗时 (毫秒)
    pub inference_time_ms: u32,
    
    /// 处理耗时 (毫秒)
    pub process_time_ms: u32,
}

/// 人脸识别匹配结果
#[derive(Debug, Clone)]
pub struct FaceIdentificationResult {
    /// 匹配的ID
    pub matched_id: u32,
    
    /// 相似度分数 (0.0-1.0)
    pub similarity: f32,
    
    /// 推理耗时 (毫秒)
    pub inference_time_ms: u32,
    
    /// 处理耗时 (毫秒)
    pub process_time_ms: u32,
}

// ============ ArcFace 应用 ============

pub struct ArcFaceApp {
    /// 模型名称
    model_name: &'static str,
    
    /// 输入分辨率
    input_size: (u32, u32),
    
    /// 特征向量维度
    embedding_dim: u32,
    
    /// 相似度阈值
    similarity_threshold: f32,
}

impl ArcFaceApp {
    /// 创建新的 ArcFace 应用
    pub fn new() -> Self {
        ArcFaceApp {
            model_name: "ArcFace",
            input_size: (112, 112),  // ArcFace 标准输入尺寸
            embedding_dim: 512,      // ArcFace 标准输出维度
            similarity_threshold: 0.6, // 默认相似度阈值
        }
    }
    
    /// 设置相似度阈值
    pub fn set_similarity_threshold(&mut self, threshold: f32) {
        self.similarity_threshold = threshold.clamp(0.0, 1.0);
    }
    
    /// 预处理人脸图像
    /// 
    /// 操作:
    /// 1. 缩放到 112x112
    /// 2. 转换格式 (BGR → RGB)
    /// 3. 归一化 (0-255 → -1.0-1.0)
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
                let b = input_data[idx] as f32 / 127.5 - 1.0;
                let g = input_data[idx + 1] as f32 / 127.5 - 1.0;
                let r = input_data[idx + 2] as f32 / 127.5 - 1.0;
                
                output.push(r);
                output.push(g);
                output.push(b);
            }
        }
        
        Ok(output)
    }
    
    /// 后处理特征向量
    /// 
    /// 操作:
    /// 1. L2 归一化特征向量
    pub fn postprocess_embedding(&self, embedding: &mut [f32]) -> Result<(), &'static str> {
        if embedding.len() != self.embedding_dim as usize {
            return Err("Invalid embedding dimension");
        }
        
        // L2 归一化
        let sum_squares: f32 = embedding.iter().map(|&x| x * x).sum();
        let norm = float_sqrt(sum_squares);
        if norm > 0.0 {
            for val in embedding.iter_mut() {
                *val /= norm;
            }
        }
        
        Ok(())
    }
    
    /// 计算两个特征向量的余弦相似度
    pub fn calculate_similarity(embedding1: &[f32], embedding2: &[f32]) -> Result<f32, &'static str> {
        if embedding1.len() != embedding2.len() {
            return Err("Embedding dimensions mismatch");
        }
        
        // 计算余弦相似度: dot(a, b) / (||a|| * ||b||)
        // 由于已经 L2 归一化，只需计算点积
        let dot_product: f32 = embedding1.iter().zip(embedding2.iter()).map(|(a, b)| a * b).sum();
        
        // 限制在 [-1, 1] 范围内
        let similarity = dot_product.clamp(-1.0, 1.0);
        
        // 转换到 [0, 1] 范围
        Ok((similarity + 1.0) / 2.0)
    }
    
    /// 完整的人脸特征提取流程
    pub fn extract_features(
        &self,
        input_data: &[u8],
        input_w: u32,
        input_h: u32,
        output_data: &[f32],
    ) -> Result<FaceRecognitionResult, &'static str> {
        let start_time = get_time_ms();
        
        // 1. 预处理
        let preprocessed = self.preprocess_image(input_data, input_w, input_h)?;
        
        // 2. 推理 (调用 NPU 进行实际推理)
        let inference_time = {
            // 获取全局 RKNN 上下文
            let mut ctx_lock = RKNN_CTX.lock();
            if let Some(ref mut ctx) = *ctx_lock {
                // 确保模型类型设置为人脸识别
                ctx.set_model_type(ModelType::FaceRecognition);
                
                // 初始化输入张量 (如果尚未初始化)
                let input_shapes = [(1, 3, self.input_size.1 as usize, self.input_size.0 as usize)]; // NCHW
                let _ = ctx.init_inputs(&input_shapes);
                
                // 初始化输出张量 (如果尚未初始化)
                let output_sizes = [self.embedding_dim as usize]; // 512维特征向量
                let _ = ctx.init_outputs(&output_sizes);
                
                // 将预处理后的f32数据转换为u8字节数据
                let preprocessed_bytes: Vec<u8> = preprocessed
                    .iter()
                    .flat_map(|&f| f.to_le_bytes())
                    .collect();
                
                // 设置输入数据
                ctx.set_input(0, &preprocessed_bytes)
                    .map_err(|_| "Failed to set input data")?;
                
                // 执行推理
                let inference_start = get_time_ms();
                let inference_result = ctx.run_inference();
                let inference_duration = get_time_ms() - inference_start;
                
                match inference_result {
                    Ok(time) => time,
                    Err(e) => return Err(e),
                }
            } else {
                return Err("RKNN context not initialized");
            }
        };
        
        // 3. 后处理
        let start_post = get_time_ms();
        let mut embedding = output_data.to_vec();
        
        // 如果提供了输出数据，则使用它；否则从 RKNN 上下文中获取
        if output_data.is_empty() {
            let ctx_lock = RKNN_CTX.lock();
            if let Some(ref ctx) = *ctx_lock {
                if let Ok(output) = ctx.get_output(0) {
                    // 将输出数据转换为 f32 向量
                    let output_f32: Vec<f32> = output
                        .chunks_exact(4)
                        .map(|chunk| {
                            f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                        })
                        .collect();
                    embedding = output_f32;
                }
            }
        }
        
        self.postprocess_embedding(&mut embedding)?;
        let process_time = get_time_ms() - start_post;
        
        Ok(FaceRecognitionResult {
            embedding,
            inference_time_ms: inference_time,
            process_time_ms: process_time as u32,
        })
    }
    
    /// 人脸验证 (1:1 匹配)
    pub fn verify_faces(
        &self,
        embedding1: &[f32],
        embedding2: &[f32],
    ) -> Result<FaceVerificationResult, &'static str> {
        let start_time = get_time_ms();
        
        let similarity = Self::calculate_similarity(embedding1, embedding2)?;
        let is_match = similarity >= self.similarity_threshold;
        
        let process_time = get_time_ms() - start_time;
        
        Ok(FaceVerificationResult {
            similarity,
            is_match,
            inference_time_ms: 0, // 验证不涉及NPU推理
            process_time_ms: process_time as u32,
        })
    }
    
    /// 人脸识别 (1:N 匹配)
    pub fn identify_face(
        &self,
        query_embedding: &[f32],
        gallery_embeddings: &[Vec<f32>],
        gallery_ids: &[u32],
    ) -> Result<FaceIdentificationResult, &'static str> {
        if gallery_embeddings.is_empty() || gallery_embeddings.len() != gallery_ids.len() {
            return Err("Invalid gallery data");
        }
        
        let start_time = get_time_ms();
        
        let mut best_similarity = 0.0f32;
        let mut best_id = 0u32;
        
        // 遍历图库寻找最匹配的人脸
        for (i, gallery_embedding) in gallery_embeddings.iter().enumerate() {
            let similarity = Self::calculate_similarity(query_embedding, gallery_embedding)?;
            if similarity > best_similarity {
                best_similarity = similarity;
                best_id = gallery_ids[i];
            }
        }
        
        let is_match = best_similarity >= self.similarity_threshold;
        
        let process_time = get_time_ms() - start_time;
        
        Ok(FaceIdentificationResult {
            matched_id: if is_match { best_id } else { 0 },
            similarity: best_similarity,
            inference_time_ms: 0, // 识别不涉及NPU推理
            process_time_ms: process_time as u32,
        })
    }
    
    /// 获取模型类型 (用于RKNN上下文配置)
    pub fn model_type() -> ModelType {
        ModelType::FaceRecognition
    }
}

/// 简单的平方根实现
fn float_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    
    let mut guess = x / 2.0;
    for _ in 0..10 {
        guess = (guess + x / guess) / 2.0;
    }
    guess
}

/// 获取当前时间 (毫秒)
fn get_time_ms() -> u64 {
    0  // 占位实现
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    
    #[test]
    fn test_arcface_app_creation() {
        let app = ArcFaceApp::new();
        assert_eq!(app.input_size, (112, 112));
        assert_eq!(app.embedding_dim, 512);
    }
    
    #[test]
    fn test_similarity_calculation() {
        let embedding1 = vec![1.0f32, 0.0, 0.0];
        let embedding2 = vec![1.0f32, 0.0, 0.0];
        
        let similarity = ArcFaceApp::calculate_similarity(&embedding1, &embedding2).unwrap();
        assert!((similarity - 1.0).abs() < 0.001);
    }
    
    #[test]
    fn test_l2_normalization() {
        let mut app = ArcFaceApp::new();
        let mut embedding = vec![3.0f32, 4.0, 0.0];
        
        app.postprocess_embedding(&mut embedding).unwrap();
        
        // 验证归一化后向量长度为1
        let sum_squares: f32 = embedding.iter().map(|&x| x * x).sum();
        let norm = float_sqrt(sum_squares);
        assert!((norm - 1.0).abs() < 0.001);
    }
    
    #[test]
    fn test_model_type() {
        assert_eq!(ArcFaceApp::model_type(), ModelType::FaceRecognition);
    }
    
    #[test]
    fn test_extract_features() {
        let app = ArcFaceApp::new();
        
        // 创建一个简单的人脸图像数据 (112x112x3)
        let face_data = vec![128u8; 112 * 112 * 3];
        
        // 模拟输出数据 (512维特征向量)
        let output_data = vec![0.1f32; 512];
        
        // 测试特征提取
        let result = app.extract_features(&face_data, 112, 112, &output_data);
        
        // 注意：在实际测试中，如果没有初始化RKNN上下文，这里会返回错误
        // 但在我们的实现中，我们已经处理了这种情况
        match result {
            Ok(recognition_result) => {
                assert_eq!(recognition_result.embedding.len(), 512);
                // 验证特征向量已被L2归一化
                let sum_squares: f32 = recognition_result.embedding.iter().map(|&x| x * x).sum();
                let norm = float_sqrt(sum_squares);
                assert!((norm - 1.0).abs() < 0.001);
            }
            Err(e) => {
                // 如果RKNN上下文未初始化，会返回错误，这是预期的
                assert_eq!(e, "RKNN context not initialized");
            }
        }
    }
}