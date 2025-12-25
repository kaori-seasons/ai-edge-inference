//! HNSW (Hierarchical Navigable Small Worlds) 向量索引
//!
//! 高性能的近似最近邻(ANN)算法实现，用于支持快速的k-NN搜索
//! 支持人脸特征向量相似度搜索

use alloc::vec::Vec;
use alloc::collections::{BTreeMap, BinaryHeap};
use core::cmp::Ordering;

/// 向量索引条目
#[derive(Debug, Clone)]
pub struct IndexEntry {
    pub id: u32,
    pub vector: Vec<f32>,
}

/// 搜索结果（距离、ID对）
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: u32,
    pub distance: f32,
}

impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        (self.distance - other.distance).abs() < 1e-6
    }
}

impl Eq for SearchResult {}

impl PartialOrd for SearchResult {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        other.distance.partial_cmp(&self.distance) // 反序：最小距离优先
    }
}

impl Ord for SearchResult {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

/// HNSW层中的节点
#[derive(Debug, Clone)]
struct HNSWNode {
    id: u32,
    vector: Vec<f32>,
    neighbors: Vec<Vec<u32>>,  // 多层邻接表
}

/// HNSW索引器
pub struct HNSWIndex {
    /// 所有节点
    nodes: BTreeMap<u32, HNSWNode>,
    /// 入口点
    entry_point: Option<u32>,
    /// 最大层数
    max_layer: usize,
    /// M: 每层最多邻接数
    m: usize,
    /// ef_construction: 构造时的搜索宽度
    ef_construction: usize,
    /// 向量维度
    dimension: usize,
}

impl HNSWIndex {
    /// 创建新的HNSW索引
    pub fn new(m: usize, ef_construction: usize, dimension: usize) -> Self {
        HNSWIndex {
            nodes: BTreeMap::new(),
            entry_point: None,
            max_layer: 0,
            m,
            ef_construction,
            dimension,
        }
    }

    /// 添加单个向量
    pub fn add(&mut self, id: u32, vector: &[f32]) -> Result<(), &'static str> {
        if vector.len() != self.dimension {
            return Err("Vector dimension mismatch");
        }

        if self.nodes.contains_key(&id) {
            return Err("Duplicate vector ID");
        }

        let vector = vector.to_vec();
        let mut node = HNSWNode {
            id,
            vector: vector.clone(),
            neighbors: Vec::new(),
        };

        // 初始化多层邻接表
        let layer = if self.nodes.is_empty() {
            0
        } else {
            self._select_layer()
        };

        for _ in 0..=layer {
            node.neighbors.push(Vec::new());
        }

        if self.nodes.is_empty() {
            self.entry_point = Some(id);
            self.max_layer = layer;
        } else {
            // 搜索并连接到邻近节点
            self._insert_node(&node, layer)?;
        }

        self.nodes.insert(id, node);
        Ok(())
    }

    /// 批量添加向量
    pub fn add_batch(&mut self, entries: &[IndexEntry]) -> Result<(), &'static str> {
        for entry in entries {
            self.add(entry.id, &entry.vector)?;
        }
        Ok(())
    }

    /// k-NN搜索
    pub fn search_knn(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>, &'static str> {
        if self.nodes.is_empty() {
            return Ok(Vec::new());
        }

        if query.len() != self.dimension {
            return Err("Query dimension mismatch");
        }

        let entry = self.entry_point.ok_or("No entry point")?;

        // 从顶层开始搜索到第1层
        let mut nearest = vec![entry];

        for layer in (1..=self.max_layer).rev() {
            nearest = self._search_layer(query, &nearest, 1, layer)?;
        }

        // 在第0层进行k-NN搜索
        let mut candidates: BinaryHeap<SearchResult> = BinaryHeap::new();
        let mut visited = core::collections::BTreeSet::new();

        // 初始化候选集
        for &node_id in &nearest {
            let distance = self._distance(query, &self.nodes[&node_id].vector);
            candidates.push(SearchResult {
                id: node_id,
                distance,
            });
            visited.insert(node_id);
        }

        // 扩展搜索
        let mut results: BinaryHeap<SearchResult> = candidates.clone();

        for _ in 0..self.ef_construction {
            if candidates.is_empty() {
                break;
            }

            let curr = candidates.pop().unwrap();

            if curr.distance > results.peek().map(|r| r.distance).unwrap_or(f32::MAX) {
                break;
            }

            if let Some(node) = self.nodes.get(&curr.id) {
                // 遍历第0层邻接表
                if !node.neighbors.is_empty() {
                    for &neighbor_id in &node.neighbors[0] {
                        if !visited.contains(&neighbor_id) {
                            visited.insert(neighbor_id);
                            let distance =
                                self._distance(query, &self.nodes[&neighbor_id].vector);

                            if distance < results.peek().map(|r| r.distance).unwrap_or(f32::MAX)
                                || results.len() < k
                            {
                                candidates.push(SearchResult {
                                    id: neighbor_id,
                                    distance,
                                });
                                results.push(SearchResult {
                                    id: neighbor_id,
                                    distance,
                                });

                                if results.len() > k {
                                    results.pop();
                                }
                            }
                        }
                    }
                }
            }
        }

        // 转换为排序结果
        let mut final_results: Vec<SearchResult> = results.into_iter().collect();
        final_results.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));
        final_results.truncate(k);

        Ok(final_results)
    }

    /// 半径搜索
    pub fn search_radius(&self, query: &[f32], radius: f32) -> Result<Vec<SearchResult>, &'static str> {
        if self.nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for (id, node) in &self.nodes {
            let distance = self._distance(query, &node.vector);
            if distance <= radius {
                results.push(SearchResult {
                    id: *id,
                    distance,
                });
            }
        }

        results.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(Ordering::Equal));
        Ok(results)
    }

    /// 计算两个向量的欧氏距离
    fn _distance(&self, v1: &[f32], v2: &[f32]) -> f32 {
        let mut sum = 0.0f32;
        for i in 0..v1.len() {
            let diff = v1[i] - v2[i];
            sum += diff * diff;
        }
        sum.sqrt()
    }

    /// 随机选择层
    fn _select_layer(&self) -> usize {
        let mut layer = 0;
        let mut rng = 1u32;
        while (rng & 1) == 1 && layer < self.max_layer {
            layer += 1;
            rng = rng.wrapping_mul(1103515245).wrapping_add(12345);
        }
        layer.min(self.max_layer + 1)
    }

    /// 在图中插入新节点
    fn _insert_node(&mut self, node: &HNSWNode, target_layer: usize) -> Result<(), &'static str> {
        // 简化实现：连接到邻近节点
        let mut nearest = vec![self.entry_point.unwrap()];

        for layer in (1..=target_layer).rev() {
            nearest = self._search_layer(&node.vector, &nearest, 1, layer)?;
        }

        // 在第0层连接
        if target_layer == 0 && !nearest.is_empty() {
            // 建立双向连接
            let nearest_id = nearest[0];
            if let Some(nearest_node) = self.nodes.get_mut(&nearest_id) {
                if !nearest_node.neighbors[0].contains(&node.id) {
                    nearest_node.neighbors[0].push(node.id);
                }
            }
        }

        Ok(())
    }

    /// 在特定层搜索
    fn _search_layer(
        &self,
        query: &[f32],
        entry_points: &[u32],
        num_closest: usize,
        layer: usize,
    ) -> Result<Vec<u32>, &'static str> {
        let mut nearest = Vec::new();
        let mut visited = core::collections::BTreeSet::new();

        for &entry in entry_points {
            if let Some(node) = self.nodes.get(&entry) {
                let distance = self._distance(query, &node.vector);
                nearest.push((entry, distance));
                visited.insert(entry);
            }
        }

        // 简单的贪心搜索
        loop {
            let (curr_id, _) = nearest.iter().min_by(|a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal)
            }).ok_or("Empty nearest")?;

            let curr_id = *curr_id;

            if let Some(node) = self.nodes.get(&curr_id) {
                if layer < node.neighbors.len() {
                    let mut improved = false;
                    for &neighbor_id in &node.neighbors[layer] {
                        if !visited.contains(&neighbor_id) {
                            visited.insert(neighbor_id);
                            if let Some(neighbor_node) = self.nodes.get(&neighbor_id) {
                                let distance = self._distance(query, &neighbor_node.vector);
                                if distance < nearest.iter().map(|(_, d)| *d).fold(f32::MAX, f32::min) {
                                    nearest.push((neighbor_id, distance));
                                    improved = true;
                                }
                            }
                        }
                    }
                    if !improved {
                        break;
                    }
                }
            }

            if nearest.len() > num_closest {
                nearest.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
                nearest.truncate(num_closest);
            }
        }

        Ok(nearest.iter().map(|(id, _)| *id).collect())
    }

    /// 获取索引统计信息
    pub fn get_stats(&self) -> (usize, usize) {
        (self.nodes.len(), self.max_layer + 1)
    }

    /// 删除向量
    pub fn remove(&mut self, id: u32) -> Result<(), &'static str> {
        self.nodes.remove(&id).ok_or("Vector not found")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hnsw_creation() {
        let index = HNSWIndex::new(16, 200, 512);
        let (size, layers) = index.get_stats();
        assert_eq!(size, 0);
        assert_eq!(layers, 1);
    }

    #[test]
    fn test_hnsw_add_vector() {
        let mut index = HNSWIndex::new(16, 200, 3);
        
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![0.0, 1.0, 0.0];
        
        assert!(index.add(1, &v1).is_ok());
        assert!(index.add(2, &v2).is_ok());
        
        let (size, _) = index.get_stats();
        assert_eq!(size, 2);
    }

    #[test]
    fn test_hnsw_knn_search() {
        let mut index = HNSWIndex::new(16, 200, 3);
        
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![1.1, 0.0, 0.0];
        let v3 = vec![0.0, 1.0, 0.0];
        
        index.add(1, &v1).unwrap();
        index.add(2, &v2).unwrap();
        index.add(3, &v3).unwrap();
        
        let query = vec![1.0, 0.0, 0.0];
        let results = index.search_knn(&query, 2).unwrap();
        
        assert!(results.len() > 0);
        assert_eq!(results[0].id, 1);
    }

    #[test]
    fn test_hnsw_radius_search() {
        let mut index = HNSWIndex::new(16, 200, 2);
        
        let v1 = vec![0.0, 0.0];
        let v2 = vec![0.5, 0.0];
        let v3 = vec![2.0, 0.0];
        
        index.add(1, &v1).unwrap();
        index.add(2, &v2).unwrap();
        index.add(3, &v3).unwrap();
        
        let query = vec![0.0, 0.0];
        let results = index.search_radius(&query, 1.0).unwrap();
        
        assert!(results.len() >= 2);
    }
}
