//! 云-端分层存储与同步系统
//!
//! 支持历史数据云端存储，端侧仅保留热数据和索引
//! 集成 MinIO/SeaweedFS 对象存储，提供智能缓存和带宽优化

use alloc::vec::Vec;
use alloc::vec;
use alloc::string::String;
use alloc::string::ToString;
use alloc::format;
use alloc::collections::{BTreeMap, VecDeque};
use core::fmt;

/// 数据存储位置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageLocation {
    /// 设备本地存储
    Local,
    /// 云端存储（MinIO/SeaweedFS）
    Cloud,
    /// 设备缓存（临时）
    Cache,
}

impl fmt::Display for StorageLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageLocation::Local => write!(f, "Local"),
            StorageLocation::Cloud => write!(f, "Cloud"),
            StorageLocation::Cache => write!(f, "Cache"),
        }
    }
}

/// 云存储配置
#[derive(Debug, Clone)]
pub struct CloudStorageConfig {
    /// 存储服务类型 (0=MinIO, 1=SeaweedFS)
    pub service_type: u8,
    /// 服务器地址
    pub endpoint: String,
    /// Access Key
    pub access_key: String,
    /// Secret Key
    pub secret_key: String,
    /// Bucket 名称
    pub bucket: String,
    /// 上传重试次数
    pub max_retries: u32,
    /// 上传超时（秒）
    pub upload_timeout: u32,
}

impl Default for CloudStorageConfig {
    fn default() -> Self {
        CloudStorageConfig {
            service_type: 0, // MinIO
            endpoint: "http://192.168.1.100:9000".to_string(),
            access_key: String::new(),
            secret_key: String::new(),
            bucket: "photo-archive".to_string(),
            max_retries: 3,
            upload_timeout: 300,
        }
    }
}

/// 分层存储策略
#[derive(Debug, Clone, Copy)]
pub struct StoragePolicy {
    /// 本地热数据保留天数
    pub hot_data_days: u32,
    /// 本地最大存储容量（MB）
    pub local_max_capacity_mb: u32,
    /// 照片最大文件大小（MB）
    pub max_photo_size_mb: u32,
    /// 自动上传阈值（天数，超过此天数自动上传）
    pub auto_upload_threshold_days: u32,
    /// 启用压缩（减小云存储空间）
    pub enable_compression: bool,
    /// 启用增量备份
    pub enable_incremental_backup: bool,
}

impl Default for StoragePolicy {
    fn default() -> Self {
        StoragePolicy {
            hot_data_days: 30,
            local_max_capacity_mb: 1024, // 1GB 本地缓存
            max_photo_size_mb: 50,
            auto_upload_threshold_days: 90, // 3个月后自动上传
            enable_compression: true,
            enable_incremental_backup: true,
        }
    }
}

/// 照片元数据与存储信息
#[derive(Debug, Clone)]
pub struct PhotoStorageRecord {
    pub photo_id: u32,
    pub file_hash: String, // SHA256
    pub file_size_bytes: u32,
    pub location: StorageLocation,
    pub cloud_path: Option<String>, // 云端路径
    pub local_path: Option<String>, // 本地路径
    pub created_timestamp: u64,
    pub uploaded_timestamp: Option<u64>,
    pub is_compressed: bool,
    pub compression_ratio: f32,
    pub sync_status: SyncStatus,
}

/// 同步状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncStatus {
    /// 本地新增，未同步
    Local,
    /// 同步中
    Syncing,
    /// 已同步到云
    Synced,
    /// 同步失败
    Failed,
    /// 云端可用，本地已删除
    CloudOnly,
}

impl fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncStatus::Local => write!(f, "Local"),
            SyncStatus::Syncing => write!(f, "Syncing"),
            SyncStatus::Synced => write!(f, "Synced"),
            SyncStatus::Failed => write!(f, "Failed"),
            SyncStatus::CloudOnly => write!(f, "CloudOnly"),
        }
    }
}

/// 同步队列任务
#[derive(Debug, Clone)]
pub struct SyncTask {
    pub task_id: u32,
    pub photo_id: u32,
    pub action: SyncAction,
    pub priority: u8, // 0=低, 1=中, 2=高
    pub retry_count: u32,
    pub created_timestamp: u64,
}

/// 同步操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    /// 上传到云
    Upload,
    /// 从云下载
    Download,
    /// 删除云端文件
    DeleteCloud,
    /// 删除本地文件
    DeleteLocal,
}

/// 本地存储状态
#[derive(Debug, Clone, Copy)]
pub struct LocalStorageStats {
    pub total_photos: u32,
    pub local_photos: u32,
    pub cloud_photos: u32,
    pub used_capacity_mb: u32,
    pub available_capacity_mb: u32,
    pub pending_upload_count: u32,
    pub pending_download_count: u32,
}

/// 云-端分层存储管理器
pub struct CloudEdgeStorageManager {
    /// 云存储配置
    config: CloudStorageConfig,
    /// 存储策略
    policy: StoragePolicy,
    /// 照片存储记录
    records: BTreeMap<u32, PhotoStorageRecord>,
    /// 同步队列（优先级队列）
    sync_queue: VecDeque<SyncTask>,
    /// 下载缓存（LRU）
    download_cache: Vec<u32>,
    /// 任务 ID 计数器
    next_task_id: u32,
    /// 去重管理器（防止重复上传）
    dedup_manager: crate::npu::dedup_manager::DeduplicationManager,
}

impl CloudEdgeStorageManager {
    /// 创建新的存储管理器
    pub fn new(config: CloudStorageConfig, policy: StoragePolicy) -> Self {
        CloudEdgeStorageManager {
            config,
            policy,
            records: BTreeMap::new(),
            sync_queue: VecDeque::new(),
            download_cache: Vec::new(),
            next_task_id: 0,
            dedup_manager: crate::npu::dedup_manager::DeduplicationManager::new(),
        }
    }

    /// 注册新照片
    pub fn register_photo(
        &mut self,
        photo_id: u32,
        file_hash: String,
        file_size_bytes: u32,
    ) -> Result<(), &'static str> {
        if file_size_bytes > self.policy.max_photo_size_mb * 1024 * 1024 {
            return Err("File too large");
        }

        let record = PhotoStorageRecord {
            photo_id,
            file_hash,
            file_size_bytes,
            location: StorageLocation::Local,
            cloud_path: None,
            local_path: Some(format!("/data/photos/{}", photo_id)),
            created_timestamp: 0,
            uploaded_timestamp: None,
            is_compressed: false,
            compression_ratio: 0.0,
            sync_status: SyncStatus::Local,
        };

        self.records.insert(photo_id, record);

        // 自动检查是否需要上传
        self._check_auto_upload()?;

        Ok(())
    }

    /// 检查是否需要自动上传（策略驱动）
    fn _check_auto_upload(&mut self) -> Result<(), &'static str> {
        let stats = self.get_local_stats();

        // 判断条件1：超过本地容量
        if stats.used_capacity_mb > self.policy.local_max_capacity_mb {
            self._trigger_storage_cleanup()?;
        }

        // 判断条件2：数据超过自动上传阈值
        for (_, record) in self.records.iter_mut() {
            if record.location == StorageLocation::Local
                && record.sync_status == SyncStatus::Local
            {
                let age_days = (0 - record.created_timestamp) / 86400; // 简化计算
                if age_days > self.policy.auto_upload_threshold_days {
                    self._queue_upload_task(record.photo_id, 0)?; // 低优先级
                }
            }
        }

        Ok(())
    }

    /// 强制上传照片到云
    pub fn upload_to_cloud(&mut self, photo_id: u32) -> Result<u32, &'static str> {
        let record = self.records.get_mut(&photo_id).ok_or("Photo not found")?;

        if record.sync_status == SyncStatus::Synced {
            return Err("Already uploaded");
        }

        self._queue_upload_task(photo_id, 2) // 高优先级
    }

    /// 队列上传任务（先进行去重检查）
    fn _queue_upload_task(&mut self, photo_id: u32, priority: u8) -> Result<u32, &'static str> {
        // 步骤1: 从记录中获取哈希信息
        let hash = self
            .records
            .get(&photo_id)
            .map(|r| r.file_hash.clone())
            .ok_or("Record not found")?;

        // 步骤2: 构建去重检查信息
        let hash_info = crate::npu::dedup_manager::FileHashInfo::new(
            hash,
            "".to_string(), // MD5 可以为空或实际计算
            self.records.get(&photo_id).unwrap().file_size_bytes,
        );

        // 步骤3: 执行去重检查
        let dedup_result = self.dedup_manager.check_duplicate(photo_id, hash_info.clone());

        match dedup_result.recommended_action {
            crate::npu::dedup_manager::DuplicateAction::Skip => {
                // 完全重复，不上传
                if let Some(record) = self.records.get_mut(&photo_id) {
                    record.sync_status = SyncStatus::Synced; // 标记为已同步（实际是跳过）
                    self.dedup_manager.mark_as_skipped(photo_id, dedup_result.duplicate_photo_ids[0]).ok();
                }
                return Err("Duplicate photo, skipped");
            }
            crate::npu::dedup_manager::DuplicateAction::NeedConfirmation => {
                // 逻辑重复（感知哈希相似），等待人工确认或继续上传
                // 这里选择继续上传（可选择等待人工确认）
            }
            crate::npu::dedup_manager::DuplicateAction::MarkAsDuplicateGroup => {
                // 可能重复，创建重复组
                if !dedup_result.duplicate_photo_ids.is_empty() {
                    let mut group_photos = dedup_result.duplicate_photo_ids.clone();
                    group_photos.push(photo_id);
                    self.dedup_manager.create_duplicate_group(group_photos);
                }
            }
            crate::npu::dedup_manager::DuplicateAction::Upload => {
                // 非重复，可以上传
                self.dedup_manager.register_photo(photo_id, hash_info);
            }
        }

        // 步骤4: 构建同步任务
        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let task = SyncTask {
            task_id,
            photo_id,
            action: SyncAction::Upload,
            priority,
            retry_count: 0,
            created_timestamp: 0,
        };

        // 按优先级插入队列
        self.sync_queue.push_back(task);

        if let Some(record) = self.records.get_mut(&photo_id) {
            record.sync_status = SyncStatus::Syncing;
        }

        Ok(task_id)
    }

    /// 执行同步任务
    pub fn execute_sync_task(&mut self, task_id: u32) -> Result<(), &'static str> {
        // 找到任务
        let task = self
            .sync_queue
            .iter()
            .find(|t| t.task_id == task_id)
            .cloned()
            .ok_or("Task not found")?;

        match task.action {
            SyncAction::Upload => {
                self._execute_upload(&task)?;
            }
            SyncAction::Download => {
                self._execute_download(&task)?;
            }
            SyncAction::DeleteCloud => {
                self._execute_delete_cloud(&task)?;
            }
            SyncAction::DeleteLocal => {
                self._execute_delete_local(&task)?;
            }
        }

        // 移除已完成任务
        self.sync_queue.retain(|t| t.task_id != task_id);

        Ok(())
    }

    /// 执行上传
    fn _execute_upload(&mut self, task: &SyncTask) -> Result<(), &'static str> {
        let record = self
            .records
            .get_mut(&task.photo_id)
            .ok_or("Record not found")?;

        // 这里应该调用 MinIO/SeaweedFS API
        // 简化实现：模拟上传
        record.cloud_path = Some(format!("/photos/{}/{}", 2024, task.photo_id));
        record.sync_status = SyncStatus::Synced;
        record.uploaded_timestamp = Some(0);
        record.location = StorageLocation::Cloud;

        Ok(())
    }

    /// 执行下载
    fn _execute_download(&mut self, task: &SyncTask) -> Result<(), &'static str> {
        let record = self
            .records
            .get_mut(&task.photo_id)
            .ok_or("Record not found")?;

        // 模拟下载
        record.location = StorageLocation::Cache;
        self.download_cache.push(task.photo_id);

        Ok(())
    }

    /// 执行云删除
    fn _execute_delete_cloud(&mut self, task: &SyncTask) -> Result<(), &'static str> {
        if let Some(record) = self.records.get_mut(&task.photo_id) {
            // 调用云 API 删除
            record.cloud_path = None;
        }
        Ok(())
    }

    /// 执行本地删除
    fn _execute_delete_local(&mut self, task: &SyncTask) -> Result<(), &'static str> {
        if let Some(record) = self.records.get_mut(&task.photo_id) {
            record.local_path = None;
            record.location = StorageLocation::Cloud;
            self.download_cache.retain(|&id| id != task.photo_id);
        }
        Ok(())
    }

    /// 存储清理（LRU）
    fn _trigger_storage_cleanup(&mut self) -> Result<(), &'static str> {
        // 获取最旧的照片并上传到云
        let mut old_photos: Vec<(u32, u64)> = self
            .records
            .iter()
            .filter(|(_, r)| r.location == StorageLocation::Local)
            .map(|(id, r)| (*id, r.created_timestamp))
            .collect();

        old_photos.sort_by_key(|k| k.1); // 按时间戳排序

        // 删除前 20% 最旧的照片
        let cleanup_count = (old_photos.len() / 5).max(1);
        for (photo_id, _) in old_photos.iter().take(cleanup_count) {
            self._queue_upload_task(*photo_id, 1)?; // 中等优先级
        }

        Ok(())
    }

    /// 按需下载（当用户查看时）
    pub fn on_demand_download(&mut self, photo_id: u32) -> Result<(), &'static str> {
        let record = self.records.get(&photo_id).ok_or("Photo not found")?;

        match record.location {
            StorageLocation::Local | StorageLocation::Cache => {
                // 已在本地，无需下载
                Ok(())
            }
            StorageLocation::Cloud => {
                // 队列下载任务
                let task_id = self.next_task_id;
                self.next_task_id += 1;

                let task = SyncTask {
                    task_id,
                    photo_id,
                    action: SyncAction::Download,
                    priority: 2, // 按需下载高优先级
                    retry_count: 0,
                    created_timestamp: 0,
                };

                self.sync_queue.push_back(task);
                Ok(())
            }
        }
    }

    /// 获取照片存储信息
    pub fn get_photo_location(&self, photo_id: u32) -> Option<StorageLocation> {
        self.records.get(&photo_id).map(|r| r.location)
    }

    /// 获取本地存储统计
    pub fn get_local_stats(&self) -> LocalStorageStats {
        let mut local_photos = 0u32;
        let mut cloud_photos = 0u32;
        let mut used_capacity_mb = 0u32;
        let mut pending_upload = 0u32;
        let mut pending_download = 0u32;

        for (_, record) in self.records.iter() {
            match record.location {
                StorageLocation::Local => local_photos += 1,
                StorageLocation::Cloud => cloud_photos += 1,
                StorageLocation::Cache => local_photos += 1,
            }

            used_capacity_mb += record.file_size_bytes / (1024 * 1024);

            match record.sync_status {
                SyncStatus::Local | SyncStatus::Syncing => pending_upload += 1,
                _ => {}
            }
        }

        for task in self.sync_queue.iter() {
            if task.action == SyncAction::Download {
                pending_download += 1;
            }
        }

        LocalStorageStats {
            total_photos: self.records.len() as u32,
            local_photos,
            cloud_photos,
            used_capacity_mb,
            available_capacity_mb: self.policy.local_max_capacity_mb
                .saturating_sub(used_capacity_mb),
            pending_upload_count: pending_upload,
            pending_download_count: pending_download,
        }
    }

    /// 获取同步队列状态
    pub fn get_sync_queue_status(&self) -> (usize, usize, usize) {
        let total = self.sync_queue.len();
        let uploads = self
            .sync_queue
            .iter()
            .filter(|t| t.action == SyncAction::Upload)
            .count();
        let downloads = self
            .sync_queue
            .iter()
            .filter(|t| t.action == SyncAction::Download)
            .count();

        (total, uploads, downloads)
    }

    /// 获取去重报告
    pub fn get_dedup_report(&self) -> String {
        self.dedup_manager.generate_report()
    }

    /// 获取去重优化建议
    pub fn get_dedup_suggestions(&self) -> Vec<String> {
        self.dedup_manager.get_optimization_suggestions()
    }

    /// 生成存储报告
    pub fn generate_report(&self) -> String {
        let stats = self.get_local_stats();
        let (queue_total, queue_uploads, queue_downloads) = self.get_sync_queue_queue_status();

        alloc::format!(
            "[Storage Report]\n\
            Total Photos: {}\n\
            Local: {}, Cloud: {}\n\
            Used: {}MB / {}MB\n\
            Available: {}MB\n\
            Pending Upload: {}\n\
            Pending Download: {}\n\
            Sync Queue: {} tasks ({} uploads, {} downloads)",
            stats.total_photos,
            stats.local_photos,
            stats.cloud_photos,
            stats.used_capacity_mb,
            self.policy.local_max_capacity_mb,
            stats.available_capacity_mb,
            stats.pending_upload_count,
            stats.pending_download_count,
            queue_total,
            queue_uploads,
            queue_downloads
        )
    }

    pub fn get_sync_queue_queue_status(&self) -> (usize, usize, usize) {
        let total = self.sync_queue.len();
        let uploads = self
            .sync_queue
            .iter()
            .filter(|t| t.action == SyncAction::Upload)
            .count();
        let downloads = self
            .sync_queue
            .iter()
            .filter(|t| t.action == SyncAction::Download)
            .count();

        (total, uploads, downloads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_manager_creation() {
        let config = CloudStorageConfig::default();
        let policy = StoragePolicy::default();
        let manager = CloudEdgeStorageManager::new(config, policy);

        let (total, uploads, downloads) = manager.get_sync_queue_queue_status();
        assert_eq!(total, 0);
    }

    #[test]
    fn test_register_photo() {
        let config = CloudStorageConfig::default();
        let policy = StoragePolicy::default();
        let mut manager = CloudEdgeStorageManager::new(config, policy);

        let result = manager.register_photo(1, "abc123".to_string(), 5 * 1024 * 1024);
        assert!(result.is_ok());

        let stats = manager.get_local_stats();
        assert_eq!(stats.total_photos, 1);
        assert_eq!(stats.local_photos, 1);
    }

    #[test]
    fn test_upload_to_cloud() {
        let config = CloudStorageConfig::default();
        let policy = StoragePolicy::default();
        let mut manager = CloudEdgeStorageManager::new(config, policy);

        manager
            .register_photo(1, "abc123".to_string(), 5 * 1024 * 1024)
            .unwrap();

        let task_id = manager.upload_to_cloud(1).unwrap();
        assert!(task_id > 0);

        let (queue_total, _, _) = manager.get_sync_queue_queue_status();
        assert_eq!(queue_total, 1);
    }

    #[test]
    fn test_storage_stats() {
        let config = CloudStorageConfig::default();
        let policy = StoragePolicy::default();
        let mut manager = CloudEdgeStorageManager::new(config, policy);

        for i in 1..=5 {
            manager
                .register_photo(i, format!("hash{}", i), 10 * 1024 * 1024)
                .unwrap();
        }

        let stats = manager.get_local_stats();
        assert_eq!(stats.total_photos, 5);
        assert!(stats.used_capacity_mb > 0);
    }
}
