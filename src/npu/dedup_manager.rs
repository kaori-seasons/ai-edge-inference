//! 图像去重与重复检测系统
//!
//! 基于SHA256哈希、EXIF元数据、感知哈希等多维度识别和防止重复上传
//! 支持物理重复（字节级相同）和逻辑重复（内容相同）检测

use alloc::vec::Vec;
use alloc::string::String;
use alloc::collections::BTreeMap;
use core::fmt;

/// 文件哈希信息（多维度）
#[derive(Debug, Clone)]
pub struct FileHashInfo {
    /// SHA256 哈希（物理重复检测）
    pub sha256: String,
    /// MD5 快速校验
    pub md5: String,
    /// 感知哈希 (pHash - 检测修改后的重复)
    pub phash: Option<u64>,
    /// 文件大小字节数
    pub file_size_bytes: u32,
    /// EXIF GPS 坐标 (如有)
    pub gps_coordinates: Option<(f32, f32)>,
    /// 拍摄时间戳 (EXIF)
    pub timestamp: Option<u64>,
}

impl FileHashInfo {
    /// 创建新的哈希信息
    pub fn new(sha256: String, md5: String, file_size_bytes: u32) -> Self {
        FileHashInfo {
            sha256,
            md5,
            phash: None,
            file_size_bytes,
            gps_coordinates: None,
            timestamp: None,
        }
    }

    /// 设置感知哈希
    pub fn with_phash(mut self, phash: u64) -> Self {
        self.phash = Some(phash);
        self
    }

    /// 设置 GPS 坐标
    pub fn with_gps(mut self, lat: f32, lon: f32) -> Self {
        self.gps_coordinates = Some((lat, lon));
        self
    }

    /// 设置拍摄时间
    pub fn with_timestamp(mut self, timestamp: u64) -> Self {
        self.timestamp = Some(timestamp);
        self
    }
}

/// 重复类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateType {
    /// 完全重复（SHA256相同）
    Exact,
    /// 逻辑重复（感知哈希相似）
    Similar,
    /// 可能重复（多个维度接近）
    Potential,
    /// 非重复
    Unique,
}

impl fmt::Display for DuplicateType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DuplicateType::Exact => write!(f, "Exact"),
            DuplicateType::Similar => write!(f, "Similar"),
            DuplicateType::Potential => write!(f, "Potential"),
            DuplicateType::Unique => write!(f, "Unique"),
        }
    }
}

/// 重复检测结果
#[derive(Debug, Clone)]
pub struct DuplicateCheckResult {
    pub photo_id: u32,
    pub duplicate_type: DuplicateType,
    /// 重复照片 ID 列表
    pub duplicate_photo_ids: Vec<u32>,
    /// 相似度分数 (0-100)
    pub similarity_score: u8,
    /// 推荐动作
    pub recommended_action: DuplicateAction,
}

/// 对重复照片的推荐动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateAction {
    /// 上传（非重复）
    Upload,
    /// 跳过上传（完全重复）
    Skip,
    /// 需要人工确认（逻辑重复）
    NeedConfirmation,
    /// 标记为重复组（可能重复，多维度相似）
    MarkAsDuplicateGroup,
}

impl fmt::Display for DuplicateAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DuplicateAction::Upload => write!(f, "Upload"),
            DuplicateAction::Skip => write!(f, "Skip"),
            DuplicateAction::NeedConfirmation => write!(f, "NeedConfirmation"),
            DuplicateAction::MarkAsDuplicateGroup => write!(f, "MarkAsDuplicateGroup"),
        }
    }
}

/// 去重索引结构
#[derive(Debug, Clone)]
pub struct DuplicateIndexEntry {
    pub photo_id: u32,
    pub file_hash_info: FileHashInfo,
    pub upload_status: UploadStatus,
    pub duplicate_group_id: Option<u32>, // 如果属于重复组
}

/// 上传状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadStatus {
    /// 未上传
    Pending,
    /// 已上传到云
    Uploaded,
    /// 被标记为重复，使用了其他照片的云路径
    Skipped,
}

/// 图像去重管理器
pub struct DeduplicationManager {
    /// SHA256 -> PhotoID 映射（快速精确重复检测）
    sha256_index: BTreeMap<String, Vec<u32>>,
    /// MD5 -> PhotoID 映射
    md5_index: BTreeMap<String, Vec<u32>>,
    /// pHash -> PhotoID 映射（感知重复检测）
    phash_index: BTreeMap<u64, Vec<u32>>,
    /// 完整的去重索引
    all_records: BTreeMap<u32, DuplicateIndexEntry>,
    /// 重复组管理
    duplicate_groups: BTreeMap<u32, Vec<u32>>, // GroupID -> PhotoIDs
    /// 下一个重复组 ID
    next_group_id: u32,
    /// 感知哈希相似度阈值 (汉明距离)
    phash_similarity_threshold: u32,
}

impl DeduplicationManager {
    /// 创建新的去重管理器
    pub fn new() -> Self {
        DeduplicationManager {
            sha256_index: BTreeMap::new(),
            md5_index: BTreeMap::new(),
            phash_index: BTreeMap::new(),
            all_records: BTreeMap::new(),
            duplicate_groups: BTreeMap::new(),
            next_group_id: 0,
            phash_similarity_threshold: 8, // 汉明距离 <= 8 认为相似
        }
    }

    /// 检测重复（多维度）
    pub fn check_duplicate(&mut self, photo_id: u32, hash_info: FileHashInfo) -> DuplicateCheckResult {
        // 步骤1: 精确哈希检测（O(1)）
        let exact_duplicates = self._find_exact_duplicates(&hash_info.sha256);

        if !exact_duplicates.is_empty() {
            return DuplicateCheckResult {
                photo_id,
                duplicate_type: DuplicateType::Exact,
                duplicate_photo_ids: exact_duplicates.clone(),
                similarity_score: 100,
                recommended_action: DuplicateAction::Skip,
            };
        }

        // 步骤2: 感知哈希检测（相似但可能被修改）
        let similar_duplicates = if let Some(phash) = hash_info.phash {
            self._find_similar_duplicates(phash)
        } else {
            Vec::new()
        };

        if !similar_duplicates.is_empty() {
            return DuplicateCheckResult {
                photo_id,
                duplicate_type: DuplicateType::Similar,
                duplicate_photo_ids: similar_duplicates.clone(),
                similarity_score: 85,
                recommended_action: DuplicateAction::NeedConfirmation,
            };
        }

        // 步骤3: 多维度检测（GPS + 时间 + 文件大小）
        let potential_duplicates = self._find_potential_duplicates(&hash_info);

        if !potential_duplicates.is_empty() {
            return DuplicateCheckResult {
                photo_id,
                duplicate_type: DuplicateType::Potential,
                duplicate_photo_ids: potential_duplicates.clone(),
                similarity_score: 65,
                recommended_action: DuplicateAction::MarkAsDuplicateGroup,
            };
        }

        // 非重复
        DuplicateCheckResult {
            photo_id,
            duplicate_type: DuplicateType::Unique,
            duplicate_photo_ids: Vec::new(),
            similarity_score: 0,
            recommended_action: DuplicateAction::Upload,
        }
    }

    /// 注册照片（在去重索引中）
    pub fn register_photo(&mut self, photo_id: u32, hash_info: FileHashInfo) {
        // 建立索引
        self.sha256_index
            .entry(hash_info.sha256.clone())
            .or_insert_with(Vec::new)
            .push(photo_id);

        self.md5_index
            .entry(hash_info.md5.clone())
            .or_insert_with(Vec::new)
            .push(photo_id);

        if let Some(phash) = hash_info.phash {
            self.phash_index
                .entry(phash)
                .or_insert_with(Vec::new)
                .push(photo_id);
        }

        let entry = DuplicateIndexEntry {
            photo_id,
            file_hash_info: hash_info,
            upload_status: UploadStatus::Pending,
            duplicate_group_id: None,
        };

        self.all_records.insert(photo_id, entry);
    }

    /// 标记照片为已上传
    pub fn mark_as_uploaded(&mut self, photo_id: u32) -> Result<(), &'static str> {
        if let Some(entry) = self.all_records.get_mut(&photo_id) {
            entry.upload_status = UploadStatus::Uploaded;
            Ok(())
        } else {
            Err("Photo not found in dedup index")
        }
    }

    /// 标记照片为被跳过（因为重复）
    pub fn mark_as_skipped(&mut self, photo_id: u32, reuse_from: u32) -> Result<(), &'static str> {
        if let Some(entry) = self.all_records.get_mut(&photo_id) {
            entry.upload_status = UploadStatus::Skipped;
            Ok(())
        } else {
            Err("Photo not found in dedup index")
        }
    }

    /// 创建重复组
    pub fn create_duplicate_group(&mut self, photo_ids: Vec<u32>) -> u32 {
        let group_id = self.next_group_id;
        self.next_group_id += 1;

        for photo_id in &photo_ids {
            if let Some(entry) = self.all_records.get_mut(photo_id) {
                entry.duplicate_group_id = Some(group_id);
            }
        }

        self.duplicate_groups.insert(group_id, photo_ids);
        group_id
    }

    /// 查询重复组信息
    pub fn get_duplicate_group(&self, group_id: u32) -> Option<Vec<u32>> {
        self.duplicate_groups.get(&group_id).cloned()
    }

    /// 获取照片的重复组
    pub fn get_photo_duplicate_group(&self, photo_id: u32) -> Option<Vec<u32>> {
        if let Some(entry) = self.all_records.get(&photo_id) {
            if let Some(group_id) = entry.duplicate_group_id {
                return self.duplicate_groups.get(&group_id).cloned();
            }
        }
        None
    }

    /// 找精确重复（SHA256相同）
    fn _find_exact_duplicates(&self, sha256: &str) -> Vec<u32> {
        self.sha256_index
            .get(sha256)
            .cloned()
            .unwrap_or_default()
    }

    /// 找相似重复（pHash汉明距离小）
    fn _find_similar_duplicates(&self, phash: u64) -> Vec<u32> {
        let mut similar = Vec::new();

        for (other_hash, photo_ids) in &self.phash_index {
            let hamming_distance = self._hamming_distance(phash, *other_hash);

            if hamming_distance <= self.phash_similarity_threshold && hamming_distance > 0 {
                similar.extend(photo_ids.clone());
            }
        }

        similar
    }

    /// 找可能重复（多维度）
    fn _find_potential_duplicates(&self, hash_info: &FileHashInfo) -> Vec<u32> {
        let mut potential = Vec::new();

        // 文件大小相同 + GPS接近（误差<100米）+ 时间接近（误差<1小时）
        for (_, entry) in &self.all_records {
            if entry.upload_status == UploadStatus::Skipped {
                continue; // 已跳过的重复不再参与匹配
            }

            let size_match = (entry.file_hash_info.file_size_bytes as i32
                - hash_info.file_size_bytes as i32)
                .abs() < 100 * 1024; // 允许100KB误差

            let gps_match = match (hash_info.gps_coordinates, entry.file_hash_info.gps_coordinates) {
                (Some((lat1, lon1)), Some((lat2, lon2))) => {
                    // 简化计算：0.001度 ≈ 100米
                    ((lat1 - lat2).abs() < 0.001) && ((lon1 - lon2).abs() < 0.001)
                }
                _ => false,
            };

            let time_match = match (hash_info.timestamp, entry.file_hash_info.timestamp) {
                (Some(t1), Some(t2)) => (t1.saturating_sub(t2).max(t2.saturating_sub(t1))) < 3600, // 1小时内
                _ => false,
            };

            // 至少两个维度匹配
            let match_count = [size_match, gps_match, time_match].iter().filter(|&&x| x).count();
            if match_count >= 2 {
                potential.push(entry.photo_id);
            }
        }

        potential
    }

    /// 计算汉明距离（用于pHash相似度）
    fn _hamming_distance(&self, hash1: u64, hash2: u64) -> u32 {
        (hash1 ^ hash2).count_ones()
    }

    /// 生成去重统计报告
    pub fn generate_report(&self) -> String {
        let total_photos = self.all_records.len();
        let uploaded_count = self
            .all_records
            .values()
            .filter(|e| e.upload_status == UploadStatus::Uploaded)
            .count();
        let skipped_count = self
            .all_records
            .values()
            .filter(|e| e.upload_status == UploadStatus::Skipped)
            .count();

        let mut saved_size = 0u32;
        for (_, entry) in &self.all_records {
            if entry.upload_status == UploadStatus::Skipped {
                saved_size += entry.file_hash_info.file_size_bytes;
            }
        }

        alloc::format!(
            "[Deduplication Report]\n\
            Total Photos: {}\n\
            Uploaded: {}\n\
            Skipped (Duplicates): {}\n\
            Duplicate Groups: {}\n\
            Storage Saved: {}MB",
            total_photos,
            uploaded_count,
            skipped_count,
            self.duplicate_groups.len(),
            saved_size / (1024 * 1024)
        )
    }

    /// 获取上传优化建议
    pub fn get_optimization_suggestions(&self) -> Vec<String> {
        let mut suggestions = Vec::new();

        // 检查重复组大小
        for (group_id, photo_ids) in &self.duplicate_groups {
            if photo_ids.len() > 5 {
                suggestions.push(alloc::format!(
                    "重复组 {} 包含 {} 张照片，考虑自动清理最旧的副本",
                    group_id,
                    photo_ids.len()
                ));
            }
        }

        // 检查跳过率
        let skipped_count = self
            .all_records
            .values()
            .filter(|e| e.upload_status == UploadStatus::Skipped)
            .count();
        let skip_rate = (skipped_count as f32 / self.all_records.len() as f32) * 100.0;

        if skip_rate > 30.0 {
            suggestions.push(alloc::format!(
                "重复率过高 ({:.1}%)，建议清理本地重复照片",
                skip_rate
            ));
        }

        suggestions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dedup_manager_creation() {
        let manager = DeduplicationManager::new();
        assert_eq!(manager.all_records.len(), 0);
    }

    #[test]
    fn test_register_photo() {
        let mut manager = DeduplicationManager::new();
        let hash_info = FileHashInfo::new("abc123".to_string(), "def456".to_string(), 5000000);

        manager.register_photo(1, hash_info);

        assert_eq!(manager.all_records.len(), 1);
    }

    #[test]
    fn test_exact_duplicate_detection() {
        let mut manager = DeduplicationManager::new();

        let hash1 = FileHashInfo::new("same_hash".to_string(), "md5_1".to_string(), 5000000);
        let hash2 = FileHashInfo::new("same_hash".to_string(), "md5_2".to_string(), 5000000);

        manager.register_photo(1, hash1);
        manager.register_photo(2, hash2);

        let result = manager.check_duplicate(3, hash2);

        assert_eq!(result.duplicate_type, DuplicateType::Exact);
        assert_eq!(result.similarity_score, 100);
        assert_eq!(result.recommended_action, DuplicateAction::Skip);
    }

    #[test]
    fn test_similar_duplicate_detection() {
        let mut manager = DeduplicationManager::new();

        let mut hash1 = FileHashInfo::new("hash1".to_string(), "md5_1".to_string(), 5000000);
        hash1 = hash1.with_phash(0x0F0F0F0F0F0F0F0F);

        let mut hash2 = FileHashInfo::new("hash2".to_string(), "md5_2".to_string(), 5000000);
        hash2 = hash2.with_phash(0x0F0F0F0F0F0F0F00); // 仅差8位

        manager.register_photo(1, hash1);
        manager.register_photo(2, hash2);

        let result = manager.check_duplicate(3, hash2.clone());

        assert_eq!(result.duplicate_type, DuplicateType::Similar);
        assert_eq!(result.recommended_action, DuplicateAction::NeedConfirmation);
    }

    #[test]
    fn test_potential_duplicate_detection() {
        let mut manager = DeduplicationManager::new();

        let mut hash1 = FileHashInfo::new("hash1".to_string(), "md5_1".to_string(), 5000000);
        hash1 = hash1.with_gps(24.4798, 118.0894); // 厦门
        hash1 = hash1.with_timestamp(1000000);

        let mut hash2 = FileHashInfo::new("hash2".to_string(), "md5_2".to_string(), 5100000);
        hash2 = hash2.with_gps(24.4799, 118.0895); // 接近厦门
        hash2 = hash2.with_timestamp(1001000); // 相差1000秒

        manager.register_photo(1, hash1);
        manager.register_photo(2, hash2);

        let result = manager.check_duplicate(3, hash2);

        assert_eq!(result.duplicate_type, DuplicateType::Potential);
    }

    #[test]
    fn test_duplicate_group_creation() {
        let mut manager = DeduplicationManager::new();

        let group_id = manager.create_duplicate_group(alloc::vec![1, 2, 3]);

        assert_eq!(group_id, 0);
        let group = manager.get_duplicate_group(0).unwrap();
        assert_eq!(group.len(), 3);
    }

    #[test]
    fn test_mark_as_uploaded() {
        let mut manager = DeduplicationManager::new();
        let hash_info = FileHashInfo::new("hash".to_string(), "md5".to_string(), 5000000);

        manager.register_photo(1, hash_info);
        assert!(manager.mark_as_uploaded(1).is_ok());

        let entry = manager.all_records.get(&1).unwrap();
        assert_eq!(entry.upload_status, UploadStatus::Uploaded);
    }
}
