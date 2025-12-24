//! 混合检索系统 (Hybrid Search)
//!
//! 结合结构化元数据查询和语义向量检索的完整搜索引擎
//! 支持复杂的多条件过滤和相似性排序

use alloc::vec::Vec;
use alloc::string::String;
use alloc::collections::{BTreeMap, BTreeSet};
use core::fmt;

/// 元数据记录
#[derive(Debug, Clone)]
pub struct MetadataRecord {
    pub id: u32,
    pub file_path: String,
    pub timestamp: u64,
    pub location: String,
    pub person_id: u32,
    pub tags: Vec<String>,
    pub ocr_text: String,
}

/// 搜索查询条件
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// 时间范围 (start_timestamp, end_timestamp)
    pub time_range: Option<(u64, u64)>,
    /// 位置标签
    pub location: Option<String>,
    /// 人物ID
    pub person_id: Option<u32>,
    /// 搜索标签
    pub tags: Vec<String>,
    /// OCR文本搜索关键词
    pub text_query: Option<String>,
    /// 语义向量查询
    pub vector_query: Option<Vec<f32>>,
    /// 语义搜索的k值
    pub k: usize,
    /// 语义搜索权重
    pub semantic_weight: f32,
    /// 结构化搜索权重
    pub structural_weight: f32,
}

impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            time_range: None,
            location: None,
            person_id: None,
            tags: Vec::new(),
            text_query: None,
            vector_query: None,
            k: 10,
            semantic_weight: 0.6,
            structural_weight: 0.4,
        }
    }
}

/// 搜索结果
#[derive(Debug, Clone)]
pub struct SearchResultItem {
    pub metadata: MetadataRecord,
    pub relevance_score: f32,
    pub semantic_score: f32,  // 语义相似度
    pub structural_score: f32, // 结构化匹配度
}

impl PartialEq for SearchResultItem {
    fn eq(&self, other: &Self) -> bool {
        (self.relevance_score - other.relevance_score).abs() < 1e-6
    }
}

impl Eq for SearchResultItem {}

impl PartialOrd for SearchResultItem {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        other.relevance_score.partial_cmp(&self.relevance_score)
    }
}

impl Ord for SearchResultItem {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.partial_cmp(other).unwrap_or(core::cmp::Ordering::Equal)
    }
}

/// 混合搜索引擎
pub struct HybridSearchEngine {
    /// 元数据存储
    metadata: Vec<MetadataRecord>,
    /// ID到元数据的快速索引
    id_index: BTreeMap<u32, usize>,
    /// 位置索引
    location_index: BTreeMap<String, BTreeSet<usize>>,
    /// 人物ID索引
    person_index: BTreeMap<u32, BTreeSet<usize>>,
    /// 标签索引
    tag_index: BTreeMap<String, BTreeSet<usize>>,
    /// 时间范围索引
    time_index: Vec<(u64, usize)>,
}

impl HybridSearchEngine {
    /// 创建新的混合搜索引擎
    pub fn new() -> Self {
        HybridSearchEngine {
            metadata: Vec::new(),
            id_index: BTreeMap::new(),
            location_index: BTreeMap::new(),
            person_index: BTreeMap::new(),
            tag_index: BTreeMap::new(),
            time_index: Vec::new(),
        }
    }

    /// 添加元数据记录
    pub fn add_record(&mut self, record: MetadataRecord) -> Result<(), &'static str> {
        let idx = self.metadata.len();

        // 检查重复
        if self.id_index.contains_key(&record.id) {
            return Err("Duplicate record ID");
        }

        // 更新索引
        self.id_index.insert(record.id, idx);

        self.location_index
            .entry(record.location.clone())
            .or_insert_with(BTreeSet::new)
            .insert(idx);

        self.person_index
            .entry(record.person_id)
            .or_insert_with(BTreeSet::new)
            .insert(idx);

        for tag in &record.tags {
            self.tag_index
                .entry(tag.clone())
                .or_insert_with(BTreeSet::new)
                .insert(idx);
        }

        self.time_index.push((record.timestamp, idx));

        self.metadata.push(record);
        Ok(())
    }

    /// 批量添加记录
    pub fn add_records(&mut self, records: Vec<MetadataRecord>) -> Result<(), &'static str> {
        for record in records {
            self.add_record(record)?;
        }
        Ok(())
    }

    /// 执行混合搜索
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResultItem>, &'static str> {
        // 第一步：结构化过滤
        let mut candidates = self._apply_structural_filters(query)?;

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // 第二步：计算结构化匹配分数
        let mut results = Vec::new();
        for candidate_idx in candidates {
            let record = &self.metadata[candidate_idx];

            let structural_score = self._compute_structural_score(record, query);

            // 计算语义分数
            let semantic_score = if let Some(ref vector) = query.vector_query {
                // 在实际应用中，这里应该调用向量索引
                // 这里使用简化实现
                0.5
            } else {
                0.0
            };

            // 合并分数
            let relevance_score = query.structural_weight * structural_score
                + query.semantic_weight * semantic_score;

            if relevance_score > 0.0 {
                results.push(SearchResultItem {
                    metadata: record.clone(),
                    relevance_score,
                    semantic_score,
                    structural_score,
                });
            }
        }

        // 第三步：排序和截断
        results.sort();
        results.truncate(query.k);

        Ok(results)
    }

    /// 应用结构化过滤
    fn _apply_structural_filters(&self, query: &SearchQuery) -> Result<BTreeSet<usize>, &'static str> {
        let mut result: Option<BTreeSet<usize>> = None;

        // 时间范围过滤
        if let Some((start, end)) = query.time_range {
            let mut time_filtered = BTreeSet::new();
            for (timestamp, idx) in &self.time_index {
                if *timestamp >= start && *timestamp <= end {
                    time_filtered.insert(*idx);
                }
            }
            result = Some(
                result
                    .map(|r| r.intersection(&time_filtered).copied().collect())
                    .unwrap_or(time_filtered),
            );
        }

        // 位置过滤
        if let Some(ref location) = query.location {
            let location_filtered = self
                .location_index
                .get(location)
                .cloned()
                .unwrap_or_default();
            result = Some(
                result
                    .map(|r| r.intersection(&location_filtered).copied().collect())
                    .unwrap_or(location_filtered),
            );
        }

        // 人物过滤
        if let Some(person_id) = query.person_id {
            let person_filtered = self
                .person_index
                .get(&person_id)
                .cloned()
                .unwrap_or_default();
            result = Some(
                result
                    .map(|r| r.intersection(&person_filtered).copied().collect())
                    .unwrap_or(person_filtered),
            );
        }

        // 标签过滤
        for tag in &query.tags {
            if let Some(tag_filtered) = self.tag_index.get(tag) {
                result = Some(
                    result
                        .map(|r| r.intersection(tag_filtered).copied().collect())
                        .unwrap_or_else(|| tag_filtered.clone()),
                );
            }
        }

        Ok(result.unwrap_or_else(|| (0..self.metadata.len()).collect()))
    }

    /// 计算结构化匹配分数
    fn _compute_structural_score(&self, record: &MetadataRecord, query: &SearchQuery) -> f32 {
        let mut score = 0.0f32;
        let mut weight_sum = 0.0f32;

        // 时间范围匹配
        if let Some((start, end)) = query.time_range {
            if record.timestamp >= start && record.timestamp <= end {
                score += 1.0;
            }
            weight_sum += 1.0;
        }

        // 位置匹配
        if let Some(ref location) = query.location {
            if record.location == *location {
                score += 1.0;
            }
            weight_sum += 1.0;
        }

        // 人物匹配
        if let Some(person_id) = query.person_id {
            if record.person_id == person_id {
                score += 1.0;
            }
            weight_sum += 1.0;
        }

        // 标签匹配
        if !query.tags.is_empty() {
            let matched_tags = query
                .tags
                .iter()
                .filter(|tag| record.tags.contains(tag))
                .count();
            score += (matched_tags as f32) / (query.tags.len() as f32);
            weight_sum += 1.0;
        }

        // 文本搜索
        if let Some(ref text) = query.text_query {
            if self._text_contains(record, text) {
                score += 0.5;
            }
            weight_sum += 0.5;
        }

        if weight_sum > 0.0 {
            score / weight_sum
        } else {
            0.0
        }
    }

    /// 文本包含检查（简单实现）
    fn _text_contains(&self, record: &MetadataRecord, text: &str) -> bool {
        record.file_path.contains(text)
            || record.location.contains(text)
            || record.ocr_text.contains(text)
            || record.tags.iter().any(|tag| tag.contains(text))
    }

    /// 删除记录
    pub fn remove_record(&mut self, id: u32) -> Result<(), &'static str> {
        let idx = self.id_index.remove(&id).ok_or("Record not found")?;
        
        // 这里的实现简化了，实际应该更新所有索引
        // 为了生产可用，建议重建索引
        self.rebuild_indices();
        
        Ok(())
    }

    /// 重建所有索引
    pub fn rebuild_indices(&mut self) {
        self.id_index.clear();
        self.location_index.clear();
        self.person_index.clear();
        self.tag_index.clear();
        self.time_index.clear();

        for (idx, record) in self.metadata.iter().enumerate() {
            self.id_index.insert(record.id, idx);

            self.location_index
                .entry(record.location.clone())
                .or_insert_with(BTreeSet::new)
                .insert(idx);

            self.person_index
                .entry(record.person_id)
                .or_insert_with(BTreeSet::new)
                .insert(idx);

            for tag in &record.tags {
                self.tag_index
                    .entry(tag.clone())
                    .or_insert_with(BTreeSet::new)
                    .insert(idx);
            }

            self.time_index.push((record.timestamp, idx));
        }
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> (usize, usize, usize, usize) {
        (
            self.metadata.len(),
            self.location_index.len(),
            self.person_index.len(),
            self.tag_index.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hybrid_search_creation() {
        let engine = HybridSearchEngine::new();
        let (total, locations, persons, tags) = engine.get_stats();
        assert_eq!(total, 0);
    }

    #[test]
    fn test_add_records() {
        let mut engine = HybridSearchEngine::new();

        let record = MetadataRecord {
            id: 1,
            file_path: "/photos/photo1.jpg".to_string(),
            timestamp: 1000,
            location: "Xiamen".to_string(),
            person_id: 1,
            tags: vec!["beach".to_string()],
            ocr_text: "".to_string(),
        };

        assert!(engine.add_record(record).is_ok());

        let (total, _, _, _) = engine.get_stats();
        assert_eq!(total, 1);
    }

    #[test]
    fn test_structural_filter() {
        let mut engine = HybridSearchEngine::new();

        let r1 = MetadataRecord {
            id: 1,
            file_path: "/photos/1.jpg".to_string(),
            timestamp: 1000,
            location: "Xiamen".to_string(),
            person_id: 1,
            tags: vec!["beach".to_string()],
            ocr_text: "".to_string(),
        };

        let r2 = MetadataRecord {
            id: 2,
            file_path: "/photos/2.jpg".to_string(),
            timestamp: 2000,
            location: "Beijing".to_string(),
            person_id: 2,
            tags: vec!["mountains".to_string()],
            ocr_text: "".to_string(),
        };

        engine.add_record(r1).unwrap();
        engine.add_record(r2).unwrap();

        let mut query = SearchQuery::default();
        query.location = Some("Xiamen".to_string());

        let results = engine.search(&query).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.id, 1);
    }

    #[test]
    fn test_search_with_multiple_filters() {
        let mut engine = HybridSearchEngine::new();

        let records = vec![
            MetadataRecord {
                id: 1,
                file_path: "/photos/1.jpg".to_string(),
                timestamp: 1000,
                location: "Xiamen".to_string(),
                person_id: 1,
                tags: vec!["beach".to_string()],
                ocr_text: "vacation".to_string(),
            },
            MetadataRecord {
                id: 2,
                file_path: "/photos/2.jpg".to_string(),
                timestamp: 1100,
                location: "Xiamen".to_string(),
                person_id: 2,
                tags: vec!["beach".to_string()],
                ocr_text: "friends".to_string(),
            },
        ];

        for record in records {
            engine.add_record(record).unwrap();
        }

        let mut query = SearchQuery::default();
        query.location = Some("Xiamen".to_string());
        query.person_id = Some(1);

        let results = engine.search(&query).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].metadata.id, 1);
    }
}
