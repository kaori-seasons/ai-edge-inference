//! 元数据索引云-端同步系统
//!
//! 支持元数据轻量化存储，本地保留倒排索引，历史数据指向云端
//! 实现高效的增量同步和版本管理

use alloc::vec::Vec;
use alloc::string::String;
use alloc::collections::BTreeMap;
use core::fmt;

/// 元数据版本信息
#[derive(Debug, Clone)]
pub struct MetadataVersion {
    pub version: u32,
    pub timestamp: u64,
    pub total_records: u32,
    pub cloud_backup_path: Option<String>,
    pub checksum: String,
    pub is_synced: bool,
}

/// 增量元数据变更
#[derive(Debug, Clone)]
pub struct MetadataDelta {
    pub from_version: u32,
    pub to_version: u32,
    pub added_records: u32,
    pub modified_records: u32,
    pub deleted_records: u32,
    pub changes: Vec<MetadataChange>,
}

/// 单条元数据变更
#[derive(Debug, Clone)]
pub struct MetadataChange {
    pub photo_id: u32,
    pub change_type: ChangeType,
    pub timestamp: u64,
    pub delta_size_bytes: u32,
}

/// 变更类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    /// 新增记录
    Add,
    /// 修改记录（如标签、聚类更新）
    Modify,
    /// 删除记录
    Delete,
}

impl fmt::Display for ChangeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeType::Add => write!(f, "Add"),
            ChangeType::Modify => write!(f, "Modify"),
            ChangeType::Delete => write!(f, "Delete"),
        }
    }
}

/// 云端元数据索引视图
/// 本地仅保留最近热数据的完整元数据，历史数据指向云端
#[derive(Debug, Clone)]
pub struct CloudMetadataIndex {
    pub photo_id: u32,
    pub cloud_url: String, // 云端元数据位置
    pub sync_timestamp: u64,
    pub etag: String, // 用于验证一致性
    pub local_cached: bool,
}

/// 元数据同步管理器
pub struct MetadataIndexSyncManager {
    /// 当前版本
    current_version: u32,
    /// 版本历史
    version_history: Vec<MetadataVersion>,
    /// 增量变更历史
    delta_history: Vec<MetadataDelta>,
    /// 本地元数据（仅热数据）
    local_metadata: BTreeMap<u32, CloudMetadataIndex>,
    /// 待同步的变更
    pending_changes: Vec<MetadataChange>,
    /// 同步政策
    sync_policy: SyncPolicy,
}

/// 同步政策
#[derive(Debug, Clone)]
pub struct SyncPolicy {
    /// 本地保留的热数据天数
    pub hot_data_days: u32,
    /// 增量同步间隔（秒）
    pub incremental_sync_interval_secs: u32,
    /// 全量同步间隔（秒）
    pub full_sync_interval_secs: u32,
    /// 是否启用压缩
    pub enable_compression: bool,
    /// 是否启用去重
    pub enable_deduplication: bool,
}

impl Default for SyncPolicy {
    fn default() -> Self {
        SyncPolicy {
            hot_data_days: 30,
            incremental_sync_interval_secs: 3600, // 每小时同步一次增量
            full_sync_interval_secs: 86400, // 每天全量备份
            enable_compression: true,
            enable_deduplication: true,
        }
    }
}

impl MetadataIndexSyncManager {
    /// 创建新的元数据同步管理器
    pub fn new(sync_policy: SyncPolicy) -> Self {
        MetadataIndexSyncManager {
            current_version: 0,
            version_history: Vec::new(),
            delta_history: Vec::new(),
            local_metadata: BTreeMap::new(),
            pending_changes: Vec::new(),
            sync_policy,
        }
    }

    /// 记录元数据变更（内部缓冲）
    pub fn track_change(
        &mut self,
        photo_id: u32,
        change_type: ChangeType,
        delta_size: u32,
    ) -> Result<(), &'static str> {
        let change = MetadataChange {
            photo_id,
            change_type,
            timestamp: 0,
            delta_size_bytes: delta_size,
        };

        self.pending_changes.push(change);

        // 当变更达到一定数量时，自动触发增量同步
        if self.pending_changes.len() > 1000 {
            self._flush_incremental_sync()?;
        }

        Ok(())
    }

    /// 执行增量同步
    pub fn flush_incremental_sync(&mut self) -> Result<MetadataDelta, &'static str> {
        self._flush_incremental_sync()
    }

    /// 内部增量同步
    fn _flush_incremental_sync(&mut self) -> Result<MetadataDelta, &'static str> {
        if self.pending_changes.is_empty() {
            return Err("No pending changes");
        }

        let from_version = self.current_version;
        self.current_version += 1;

        let mut added = 0;
        let mut modified = 0;
        let mut deleted = 0;

        for change in &self.pending_changes {
            match change.change_type {
                ChangeType::Add => added += 1,
                ChangeType::Modify => modified += 1,
                ChangeType::Delete => deleted += 1,
            }
        }

        let delta = MetadataDelta {
            from_version,
            to_version: self.current_version,
            added_records: added,
            modified_records: modified,
            deleted_records: deleted,
            changes: self.pending_changes.clone(),
        };

        // 记录版本
        let version = MetadataVersion {
            version: self.current_version,
            timestamp: 0,
            total_records: self.local_metadata.len() as u32,
            cloud_backup_path: Some(format!("/metadata/v{}", self.current_version)),
            checksum: self._compute_checksum(),
            is_synced: false, // 待上传到云
        };

        self.version_history.push(version);
        self.delta_history.push(delta.clone());

        // 清空待同步队列
        self.pending_changes.clear();

        Ok(delta)
    }

    /// 执行全量同步
    pub fn full_sync(&mut self) -> Result<MetadataVersion, &'static str> {
        self.current_version += 1;

        let version = MetadataVersion {
            version: self.current_version,
            timestamp: 0,
            total_records: self.local_metadata.len() as u32,
            cloud_backup_path: Some(format!("/metadata/full/{}", self.current_version)),
            checksum: self._compute_checksum(),
            is_synced: false,
        };

        self.version_history.push(version.clone());
        self.pending_changes.clear();

        Ok(version)
    }

    /// 计算校验和
    fn _compute_checksum(&self) -> String {
        // 简化实现：使用本地数据量作为校验和
        alloc::format!("checksum_{:x}", self.local_metadata.len())
    }

    /// 从云端恢复历史元数据
    pub fn restore_from_cloud(
        &mut self,
        version: u32,
        cloud_data: &[u8],
    ) -> Result<(), &'static str> {
        // 验证版本存在
        if !self.version_history.iter().any(|v| v.version == version) {
            return Err("Version not found");
        }

        // 这里应该反序列化云端数据并加载到本地
        // 简化实现
        Ok(())
    }

    /// 获取版本历史
    pub fn get_version_history(&self) -> &[MetadataVersion] {
        &self.version_history
    }

    /// 获取增量历史
    pub fn get_delta_history(&self) -> &[MetadataDelta] {
        &self.delta_history
    }

    /// 获取待同步数量
    pub fn get_pending_changes_count(&self) -> usize {
        self.pending_changes.len()
    }

    /// 计算存储节省（相比不压缩情况）
    pub fn estimate_storage_savings(&self) -> (u32, f32) {
        let total_size: u32 = self.pending_changes.iter().map(|c| c.delta_size_bytes).sum();

        let compressed_size = if self.sync_policy.enable_compression {
            (total_size as f32 * 0.6) as u32 // 假设60%压缩率
        } else {
            total_size
        };

        let savings = total_size.saturating_sub(compressed_size);
        let ratio = if total_size > 0 {
            1.0 - (compressed_size as f32 / total_size as f32)
        } else {
            0.0
        };

        (savings, ratio)
    }

    /// 生成同步报告
    pub fn generate_sync_report(&self) -> String {
        let pending = self.pending_changes.len();
        let (savings_bytes, savings_ratio) = self.estimate_storage_savings();

        alloc::format!(
            "[Metadata Sync Report]\n\
            Current Version: {}\n\
            Local Records: {}\n\
            Pending Changes: {}\n\
            Version History: {}\n\
            Estimated Savings: {}MB ({:.1}%)\n\
            Last Sync: {}\n\
            Policy: Hot data {} days, Incremental sync every {} hours",
            self.current_version,
            self.local_metadata.len(),
            pending,
            self.version_history.len(),
            savings_bytes / (1024 * 1024),
            savings_ratio * 100.0,
            self.version_history
                .last()
                .map(|v| v.timestamp)
                .unwrap_or(0),
            self.sync_policy.hot_data_days,
            self.sync_policy.incremental_sync_interval_secs / 3600
        )
    }

    /// 计算增量大小
    pub fn estimate_delta_size(&self) -> u32 {
        self.pending_changes.iter().map(|c| c.delta_size_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_sync_manager_creation() {
        let policy = SyncPolicy::default();
        let manager = MetadataIndexSyncManager::new(policy);

        assert_eq!(manager.current_version, 0);
        assert_eq!(manager.get_pending_changes_count(), 0);
    }

    #[test]
    fn test_track_changes() {
        let policy = SyncPolicy::default();
        let mut manager = MetadataIndexSyncManager::new(policy);

        manager
            .track_change(1, ChangeType::Add, 100)
            .unwrap();
        manager
            .track_change(2, ChangeType::Modify, 50)
            .unwrap();

        assert_eq!(manager.get_pending_changes_count(), 2);
    }

    #[test]
    fn test_incremental_sync() {
        let policy = SyncPolicy::default();
        let mut manager = MetadataIndexSyncManager::new(policy);

        manager
            .track_change(1, ChangeType::Add, 100)
            .unwrap();
        manager
            .track_change(2, ChangeType::Modify, 50)
            .unwrap();

        let delta = manager.flush_incremental_sync().unwrap();
        assert_eq!(delta.added_records, 1);
        assert_eq!(delta.modified_records, 1);
        assert_eq!(manager.get_pending_changes_count(), 0);
    }

    #[test]
    fn test_storage_savings_estimate() {
        let policy = SyncPolicy::default();
        let mut manager = MetadataIndexSyncManager::new(policy);

        for i in 0..10 {
            manager
                .track_change(i, ChangeType::Add, 1000)
                .unwrap();
        }

        let (savings, ratio) = manager.estimate_storage_savings();
        assert!(savings > 0);
        assert!(ratio > 0.0 && ratio < 1.0);
    }
}
