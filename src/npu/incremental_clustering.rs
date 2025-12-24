//! 增量聚类与索引管理系统
//!
//! 支持首次全量聚类和日常增量处理的生产级实现
//! 管理人脸特征聚类的动态更新和索引维护

use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use core::fmt;

/// 增量处理模式
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessingMode {
    /// 首次全量处理
    FullScan,
    /// 增量处理（新照片）
    Incremental,
    /// 重新聚类
    Rebuild,
}

/// 聚类任务状态
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClusteringTaskStatus {
    /// 等待中
    Pending,
    /// 处理中
    Processing,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
}

impl fmt::Display for ClusteringTaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusteringTaskStatus::Pending => write!(f, "Pending"),
            ClusteringTaskStatus::Processing => write!(f, "Processing"),
            ClusteringTaskStatus::Completed => write!(f, "Completed"),
            ClusteringTaskStatus::Failed => write!(f, "Failed"),
            ClusteringTaskStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// 聚类任务
#[derive(Debug, Clone)]
pub struct ClusteringTask {
    pub task_id: u32,
    pub mode: ProcessingMode,
    pub status: ClusteringTaskStatus,
    pub photo_ids: Vec<u32>,
    pub photo_count: u32,
    pub processed_count: u32,
    pub created_timestamp: u64,
    pub started_timestamp: Option<u64>,
    pub completed_timestamp: Option<u64>,
    pub error_message: Option<&'static str>,
}

impl ClusteringTask {
    pub fn new(task_id: u32, mode: ProcessingMode, photo_ids: Vec<u32>) -> Self {
        let photo_count = photo_ids.len() as u32;
        ClusteringTask {
            task_id,
            mode,
            status: ClusteringTaskStatus::Pending,
            photo_ids,
            photo_count,
            processed_count: 0,
            created_timestamp: 0,
            started_timestamp: None,
            completed_timestamp: None,
            error_message: None,
        }
    }

    pub fn get_progress_percentage(&self) -> u32 {
        if self.photo_count == 0 {
            return 0;
        }
        ((self.processed_count as u64 * 100) / (self.photo_count as u64)) as u32
    }

    pub fn get_estimated_remaining_time(&self, avg_time_per_photo: u64) -> Option<u64> {
        match self.status {
            ClusteringTaskStatus::Processing => {
                let remaining = self.photo_count.saturating_sub(self.processed_count);
                Some(remaining as u64 * avg_time_per_photo)
            }
            _ => None,
        }
    }
}

/// 聚类索引版本
#[derive(Debug, Clone)]
pub struct ClusteringIndexVersion {
    pub version: u32,
    pub created_timestamp: u64,
    pub total_clusters: u32,
    pub total_vectors: u32,
    pub task_id: u32,
}

/// 增量聚类管理器
pub struct IncrementalClusteringManager {
    /// 当前聚类任务
    current_task: Option<ClusteringTask>,
    /// 任务历史
    task_history: Vec<ClusteringTask>,
    /// 聚类索引版本
    index_versions: Vec<ClusteringIndexVersion>,
    /// 最新的聚类结果
    cluster_assignments: BTreeMap<u32, u32>, // photo_id -> cluster_id
    /// 每个簇的成员
    clusters: BTreeMap<u32, Vec<u32>>, // cluster_id -> [photo_ids]
    /// 最大簇ID
    max_cluster_id: u32,
}

impl IncrementalClusteringManager {
    /// 创建新的管理器
    pub fn new() -> Self {
        IncrementalClusteringManager {
            current_task: None,
            task_history: Vec::new(),
            index_versions: Vec::new(),
            cluster_assignments: BTreeMap::new(),
            clusters: BTreeMap::new(),
            max_cluster_id: 0,
        }
    }

    /// 提交首次全量聚类任务
    pub fn submit_full_scan_task(&mut self, photo_ids: Vec<u32>) -> Result<u32, &'static str> {
        if self.current_task.is_some() {
            return Err("Another task is in progress");
        }

        let task_id = self.task_history.len() as u32;
        let task = ClusteringTask::new(task_id, ProcessingMode::FullScan, photo_ids);

        self.current_task = Some(task.clone());
        Ok(task_id)
    }

    /// 提交增量处理任务
    pub fn submit_incremental_task(&mut self, new_photo_ids: Vec<u32>) -> Result<u32, &'static str> {
        if self.current_task.is_some() {
            return Err("Another task is in progress");
        }

        if self.index_versions.is_empty() {
            return Err("No baseline clustering found, use full scan instead");
        }

        let task_id = self.task_history.len() as u32;
        let task = ClusteringTask::new(task_id, ProcessingMode::Incremental, new_photo_ids);

        self.current_task = Some(task.clone());
        Ok(task_id)
    }

    /// 更新任务进度
    pub fn update_progress(
        &mut self,
        photo_id: u32,
        cluster_id: u32,
    ) -> Result<(), &'static str> {
        if let Some(ref mut task) = self.current_task {
            // 分配聚类标签
            self.cluster_assignments.insert(photo_id, cluster_id);

            // 更新簇成员表
            self.clusters
                .entry(cluster_id)
                .or_insert_with(Vec::new)
                .push(photo_id);

            self.max_cluster_id = self.max_cluster_id.max(cluster_id);

            task.processed_count += 1;

            if task.processed_count == task.photo_count {
                self._mark_task_completed()?;
            }

            Ok(())
        } else {
            Err("No active task")
        }
    }

    /// 完成任务
    fn _mark_task_completed(&mut self) -> Result<(), &'static str> {
        if let Some(mut task) = self.current_task.take() {
            task.status = ClusteringTaskStatus::Completed;

            // 创建索引版本快照
            let version = ClusteringIndexVersion {
                version: self.index_versions.len() as u32 + 1,
                created_timestamp: 0, // 应该设置实际时间戳
                total_clusters: self.clusters.len() as u32,
                total_vectors: self.cluster_assignments.len() as u32,
                task_id: task.task_id,
            };

            self.index_versions.push(version);
            self.task_history.push(task);

            Ok(())
        } else {
            Err("No active task to complete")
        }
    }

    /// 获取当前任务状态
    pub fn get_current_task_status(&self) -> Option<(u32, ClusteringTaskStatus, u32)> {
        self.current_task.as_ref().map(|task| {
            (
                task.task_id,
                task.status,
                task.get_progress_percentage(),
            )
        })
    }

    /// 取消当前任务
    pub fn cancel_current_task(&mut self) -> Result<(), &'static str> {
        if let Some(mut task) = self.current_task.take() {
            task.status = ClusteringTaskStatus::Cancelled;
            self.task_history.push(task);
            Ok(())
        } else {
            Err("No active task to cancel")
        }
    }

    /// 获取photo的聚类ID
    pub fn get_cluster_id(&self, photo_id: u32) -> Option<u32> {
        self.cluster_assignments.get(&photo_id).copied()
    }

    /// 获取簇的所有照片
    pub fn get_cluster_photos(&self, cluster_id: u32) -> Option<Vec<u32>> {
        self.clusters.get(&cluster_id).cloned()
    }

    /// 合并两个簇
    pub fn merge_clusters(&mut self, source_id: u32, target_id: u32) -> Result<(), &'static str> {
        if !self.clusters.contains_key(&source_id) {
            return Err("Source cluster not found");
        }

        if !self.clusters.contains_key(&target_id) {
            return Err("Target cluster not found");
        }

        if let Some(source_photos) = self.clusters.remove(&source_id) {
            for photo_id in source_photos {
                self.cluster_assignments.insert(photo_id, target_id);
                self.clusters
                    .entry(target_id)
                    .or_insert_with(Vec::new)
                    .push(photo_id);
            }
        }

        Ok(())
    }

    /// 分割簇
    pub fn split_cluster(
        &mut self,
        cluster_id: u32,
        split_indices: &[usize],
    ) -> Result<u32, &'static str> {
        if !self.clusters.contains_key(&cluster_id) {
            return Err("Cluster not found");
        }

        let original_photos = self.clusters.remove(&cluster_id).ok_or("Cluster not found")?;

        let new_cluster_id = self.max_cluster_id + 1;
        self.max_cluster_id = new_cluster_id;

        let mut kept_photos = Vec::new();
        let mut moved_photos = Vec::new();

        for (i, photo_id) in original_photos.iter().enumerate() {
            if split_indices.contains(&i) {
                moved_photos.push(*photo_id);
                self.cluster_assignments.insert(*photo_id, new_cluster_id);
            } else {
                kept_photos.push(*photo_id);
                self.cluster_assignments.insert(*photo_id, cluster_id);
            }
        }

        if !kept_photos.is_empty() {
            self.clusters.insert(cluster_id, kept_photos);
        }

        if !moved_photos.is_empty() {
            self.clusters.insert(new_cluster_id, moved_photos);
        }

        Ok(new_cluster_id)
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> (u32, u32, u32, u32) {
        let total_clusters = self.clusters.len() as u32;
        let total_vectors = self.cluster_assignments.len() as u32;
        let completed_tasks = self.task_history.len() as u32;
        let avg_cluster_size = if total_clusters > 0 {
            total_vectors / total_clusters
        } else {
            0
        };

        (total_clusters, total_vectors, completed_tasks, avg_cluster_size)
    }

    /// 获取索引版本历史
    pub fn get_index_versions(&self) -> &[ClusteringIndexVersion] {
        &self.index_versions
    }

    /// 获取任务历史
    pub fn get_task_history(&self) -> &[ClusteringTask] {
        &self.task_history
    }

    /// 从特定版本恢复
    pub fn restore_from_version(&mut self, version: u32) -> Result<(), &'static str> {
        // 这里应该从存储中加载该版本的数据
        // 简化实现
        Err("Not implemented")
    }

    /// 导出当前聚类结果
    pub fn export_clustering_result(&self) -> BTreeMap<u32, u32> {
        self.cluster_assignments.clone()
    }

    /// 导入聚类结果
    pub fn import_clustering_result(&mut self, assignments: BTreeMap<u32, u32>) {
        self.cluster_assignments = assignments.clone();

        // 重建簇表
        self.clusters.clear();
        for (photo_id, cluster_id) in assignments {
            self.clusters
                .entry(cluster_id)
                .or_insert_with(Vec::new)
                .push(photo_id);
            self.max_cluster_id = self.max_cluster_id.max(cluster_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let photo_ids = vec![1, 2, 3, 4, 5];
        let task = ClusteringTask::new(1, ProcessingMode::FullScan, photo_ids);

        assert_eq!(task.task_id, 1);
        assert_eq!(task.photo_count, 5);
        assert_eq!(task.status, ClusteringTaskStatus::Pending);
        assert_eq!(task.get_progress_percentage(), 0);
    }

    #[test]
    fn test_progress_calculation() {
        let photo_ids = vec![1, 2, 3, 4, 5];
        let mut task = ClusteringTask::new(1, ProcessingMode::FullScan, photo_ids);

        task.processed_count = 2;
        assert_eq!(task.get_progress_percentage(), 40);

        task.processed_count = 5;
        assert_eq!(task.get_progress_percentage(), 100);
    }

    #[test]
    fn test_incremental_clustering() {
        let mut manager = IncrementalClusteringManager::new();

        let photo_ids = vec![1, 2, 3];
        let task_id = manager.submit_full_scan_task(photo_ids).unwrap();
        assert_eq!(task_id, 0);

        manager.update_progress(1, 0).unwrap();
        manager.update_progress(2, 0).unwrap();
        manager.update_progress(3, 1).unwrap();

        assert_eq!(manager.get_cluster_id(1), Some(0));
        assert_eq!(manager.get_cluster_id(3), Some(1));
    }

    #[test]
    fn test_merge_clusters() {
        let mut manager = IncrementalClusteringManager::new();

        manager.cluster_assignments.insert(1, 0);
        manager.cluster_assignments.insert(2, 0);
        manager.cluster_assignments.insert(3, 1);

        manager.clusters.insert(0, vec![1, 2]);
        manager.clusters.insert(1, vec![3]);

        assert!(manager.merge_clusters(0, 1).is_ok());
        assert_eq!(manager.get_cluster_id(1), Some(1));
        assert_eq!(manager.get_cluster_id(2), Some(1));
    }

    #[test]
    fn test_stats() {
        let mut manager = IncrementalClusteringManager::new();

        manager.cluster_assignments.insert(1, 0);
        manager.cluster_assignments.insert(2, 0);
        manager.cluster_assignments.insert(3, 1);

        manager.clusters.insert(0, vec![1, 2]);
        manager.clusters.insert(1, vec![3]);

        let (total_clusters, total_vectors, _, avg_size) = manager.get_stats();
        assert_eq!(total_clusters, 2);
        assert_eq!(total_vectors, 3);
        assert_eq!(avg_size, 1);
    }
}
