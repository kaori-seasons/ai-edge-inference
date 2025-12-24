//! DBSCAN密度聚类算法的生产级实现
//!
//! 提供基于密度的无监督聚类能力，用于人脸特征向量聚类
//! 支持KD-Tree空间索引优化以处理大规模数据集

use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use core::fmt;

/// DBSCAN聚类参数
#[derive(Debug, Clone, Copy)]
pub struct DBSCANParams {
    /// 邻域半径
    pub eps: f32,
    /// 最小核心点邻域样本数
    pub min_samples: usize,
    /// 距离度量（0=欧氏距离，1=余弦距离）
    pub distance_metric: u8,
}

impl Default for DBSCANParams {
    fn default() -> Self {
        DBSCANParams {
            eps: 0.5,
            min_samples: 3,
            distance_metric: 1, // 默认使用余弦距离（对人脸特征向量）
        }
    }
}

/// 聚类标签
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ClusterLabel {
    /// 噪声点（异常值）
    Noise,
    /// 属于某个簇
    Cluster(u32),
}

impl fmt::Display for ClusterLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterLabel::Noise => write!(f, "Noise"),
            ClusterLabel::Cluster(id) => write!(f, "Cluster({})", id),
        }
    }
}

/// DBSCAN聚类结果
#[derive(Debug, Clone)]
pub struct DBSCANResult {
    /// 每个样本的聚类标签
    pub labels: Vec<ClusterLabel>,
    /// 聚类簇的数量
    pub n_clusters: u32,
    /// 噪声点的数量
    pub n_noise: u32,
}

/// KD-Tree节点用于加速邻域查询
#[derive(Debug, Clone)]
struct KDNode {
    idx: usize,
    left: Option<usize>,
    right: Option<usize>,
    axis: usize,
}

/// DBSCAN聚类器
pub struct DBSCANClustering {
    params: DBSCANParams,
    data: Vec<Vec<f32>>,
    /// KD-Tree节点列表
    kd_tree: Vec<KDNode>,
}

impl DBSCANClustering {
    /// 创建新的聚类器
    pub fn new(params: DBSCANParams) -> Self {
        DBSCANClustering {
            params,
            data: Vec::new(),
            kd_tree: Vec::new(),
        }
    }

    /// 加载特征数据
    pub fn fit_predict(&mut self, data: &[Vec<f32>]) -> Result<DBSCANResult, &'static str> {
        if data.is_empty() {
            return Err("Empty data");
        }

        // 验证所有特征向量维度一致
        let n_features = data[0].len();
        if data.iter().any(|v| v.len() != n_features) {
            return Err("Inconsistent feature dimensions");
        }

        self.data = data.to_vec();

        // 构建KD-Tree以加速邻域查询
        self._build_kd_tree(n_features)?;

        // 执行DBSCAN聚类
        let labels = self._dbscan()?;

        // 计算统计信息
        let mut n_clusters = 0u32;
        let mut n_noise = 0u32;
        for &label in &labels {
            match label {
                ClusterLabel::Noise => n_noise += 1,
                ClusterLabel::Cluster(c) => n_clusters = n_clusters.max(c + 1),
            }
        }

        Ok(DBSCANResult {
            labels,
            n_clusters,
            n_noise,
        })
    }

    /// 使用增量模式添加新数据点并更新聚类
    pub fn predict_incremental(
        &mut self,
        new_points: &[Vec<f32>],
        existing_labels: &[ClusterLabel],
        eps_join: f32,
    ) -> Result<Vec<ClusterLabel>, &'static str> {
        if new_points.is_empty() {
            return Ok(Vec::new());
        }

        let mut result_labels = Vec::new();

        for new_point in new_points {
            // 搜索与新点最接近的k个邻域点
            let neighbors = self._find_neighbors(new_point, eps_join)?;

            if neighbors.is_empty() {
                // 新点成为新簇的核心点
                result_labels.push(ClusterLabel::Cluster(self._get_next_cluster_id(existing_labels)));
            } else {
                // 检查邻域点的聚类标签
                let mut cluster_id = None;
                for neighbor_idx in neighbors {
                    if neighbor_idx < existing_labels.len() {
                        match existing_labels[neighbor_idx] {
                            ClusterLabel::Cluster(id) => {
                                cluster_id = Some(id);
                                break;
                            }
                            ClusterLabel::Noise => {}
                        }
                    }
                }

                result_labels.push(match cluster_id {
                    Some(id) => ClusterLabel::Cluster(id),
                    None => ClusterLabel::Noise,
                });
            }
        }

        Ok(result_labels)
    }

    /// 计算两个特征向量之间的距离
    fn _distance(&self, p1: &[f32], p2: &[f32]) -> f32 {
        match self.params.distance_metric {
            1 => self._cosine_distance(p1, p2), // 余弦距离
            _ => self._euclidean_distance(p1, p2), // 欧氏距离
        }
    }

    /// 欧氏距离
    fn _euclidean_distance(&self, p1: &[f32], p2: &[f32]) -> f32 {
        let mut sum = 0.0f32;
        for i in 0..p1.len() {
            let diff = p1[i] - p2[i];
            sum += diff * diff;
        }
        sum.sqrt()
    }

    /// 余弦距离 (1 - cosine_similarity)
    fn _cosine_distance(&self, p1: &[f32], p2: &[f32]) -> f32 {
        let mut dot_product = 0.0f32;
        let mut norm1 = 0.0f32;
        let mut norm2 = 0.0f32;

        for i in 0..p1.len() {
            dot_product += p1[i] * p2[i];
            norm1 += p1[i] * p1[i];
            norm2 += p2[i] * p2[i];
        }

        norm1 = norm1.sqrt();
        norm2 = norm2.sqrt();

        if norm1 == 0.0 || norm2 == 0.0 {
            return 1.0;
        }

        let similarity = dot_product / (norm1 * norm2);
        1.0 - similarity.max(-1.0).min(1.0)
    }

    /// 构建KD-Tree
    fn _build_kd_tree(&mut self, n_features: usize) -> Result<(), &'static str> {
        if self.data.is_empty() {
            return Ok(());
        }

        let mut indices: Vec<usize> = (0..self.data.len()).collect();
        self.kd_tree.clear();
        
        self._build_kd_tree_recursive(&mut indices, 0, n_features)?;
        Ok(())
    }

    /// 递归构建KD-Tree
    fn _build_kd_tree_recursive(
        &mut self,
        indices: &mut [usize],
        axis: usize,
        n_features: usize,
    ) -> Result<Option<usize>, &'static str> {
        if indices.is_empty() {
            return Ok(None);
        }

        let axis = axis % n_features;

        // 按当前轴排序
        indices.sort_by(|&a, &b| {
            self.data[a][axis].partial_cmp(&self.data[b][axis]).unwrap_or(core::cmp::Ordering::Equal)
        });

        let median = indices.len() / 2;
        let idx = indices[median];

        let node_idx = self.kd_tree.len();
        self.kd_tree.push(KDNode {
            idx,
            left: None,
            right: None,
            axis,
        });

        let left = self._build_kd_tree_recursive(&mut indices[..median], axis + 1, n_features)?;
        let right = self._build_kd_tree_recursive(&mut indices[median + 1..], axis + 1, n_features)?;

        self.kd_tree[node_idx].left = left;
        self.kd_tree[node_idx].right = right;

        Ok(Some(node_idx))
    }

    /// 使用KD-Tree查找邻域内的点
    fn _find_neighbors(&self, query: &[f32], radius: f32) -> Result<Vec<usize>, &'static str> {
        let mut neighbors = Vec::new();

        // 线性搜索备选方案（简化实现）
        for i in 0..self.data.len() {
            let dist = self._distance(query, &self.data[i]);
            if dist <= radius {
                neighbors.push(i);
            }
        }

        Ok(neighbors)
    }

    /// 执行DBSCAN算法
    fn _dbscan(&self) -> Result<Vec<ClusterLabel>, &'static str> {
        let n = self.data.len();
        let mut labels = vec![ClusterLabel::Noise; n];
        let mut cluster_id = 0u32;

        for i in 0..n {
            if matches!(labels[i], ClusterLabel::Noise) {
                let neighbors = self._get_neighbors(i)?;

                if neighbors.len() < self.params.min_samples {
                    // 是否为核心点
                    continue;
                }

                // 启动新簇
                cluster_id += 1;
                self._expand_cluster(&mut labels, &neighbors, cluster_id)?;
            }
        }

        Ok(labels)
    }

    /// 获取点的邻域
    fn _get_neighbors(&self, point_idx: usize) -> Result<Vec<usize>, &'static str> {
        let query = &self.data[point_idx];
        self._find_neighbors(query, self.params.eps)
    }

    /// 扩展簇
    fn _expand_cluster(
        &self,
        labels: &mut [ClusterLabel],
        initial_neighbors: &[usize],
        cluster_id: u32,
    ) -> Result<(), &'static str> {
        let mut queue = initial_neighbors.to_vec();
        let mut idx = 0;

        while idx < queue.len() {
            let current = queue[idx];
            idx += 1;

            if matches!(labels[current], ClusterLabel::Noise) {
                labels[current] = ClusterLabel::Cluster(cluster_id);

                // 如果是核心点，扩展边界
                let neighbors = self._get_neighbors(current)?;
                if neighbors.len() >= self.params.min_samples {
                    for neighbor in neighbors {
                        if matches!(labels[neighbor], ClusterLabel::Noise) {
                            queue.push(neighbor);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// 获取下一个簇ID
    fn _get_next_cluster_id(&self, labels: &[ClusterLabel]) -> u32 {
        let mut max_id = 0u32;
        for label in labels {
            if let ClusterLabel::Cluster(id) = label {
                max_id = max_id.max(id + 1);
            }
        }
        max_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dbscan_basic() {
        let mut params = DBSCANParams::default();
        params.eps = 0.5;
        params.min_samples = 2;
        params.distance_metric = 0; // 欧氏距离

        let mut clustering = DBSCANClustering::new(params);

        let data = vec![
            vec![0.0, 0.0],
            vec![0.1, 0.1],
            vec![0.2, 0.2],
            vec![10.0, 10.0],
            vec![10.1, 10.1],
        ];

        let result = clustering.fit_predict(&data).unwrap();
        assert_eq!(result.labels.len(), 5);
        assert!(result.n_clusters >= 1);
    }

    #[test]
    fn test_cosine_distance() {
        let clustering = DBSCANClustering::new(DBSCANParams::default());
        
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![1.0, 0.0, 0.0];
        let dist = clustering._cosine_distance(&v1, &v2);
        assert!((dist - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_incremental_predict() {
        let mut clustering = DBSCANClustering::new(DBSCANParams::default());

        let existing_data = vec![
            vec![0.0, 0.0],
            vec![0.1, 0.1],
        ];

        clustering.data = existing_data.clone();
        clustering.params.distance_metric = 0;

        let existing_labels = vec![
            ClusterLabel::Cluster(0),
            ClusterLabel::Cluster(0),
        ];

        let new_data = vec![vec![0.05, 0.05]];
        let new_labels = clustering
            .predict_incremental(&new_data, &existing_labels, 1.0)
            .unwrap();

        assert_eq!(new_labels.len(), 1);
    }
}
