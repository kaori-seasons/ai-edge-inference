//! 离线逆地理编码系统
//!
//! 基于OpenStreetMap行政边界数据的高性能本地地理编码解决方案
//! 支持Point-in-Polygon (PiP)查询，用于将GPS坐标转换为城市/地区标签

use alloc::vec::Vec;
use alloc::string::String;
use core::fmt;

/// GPS坐标（WGS84）
#[derive(Debug, Clone, Copy)]
pub struct GPSCoordinate {
    pub latitude: f64,   // 纬度
    pub longitude: f64,  // 经度
}

impl GPSCoordinate {
    pub fn new(lat: f64, lon: f64) -> Self {
        GPSCoordinate {
            latitude: lat,
            longitude: lon,
        }
    }

    /// 验证坐标有效性
    pub fn is_valid(&self) -> bool {
        self.latitude >= -90.0
            && self.latitude <= 90.0
            && self.longitude >= -180.0
            && self.longitude <= 180.0
    }

    /// 检查是否为无效坐标
    pub fn is_invalid_sentinel(&self) -> bool {
        (self.latitude == 0.0 && self.longitude == 0.0)
            || (self.latitude == 90.0 && self.longitude == 0.0)
    }
}

/// 地理位置标签
#[derive(Debug, Clone)]
pub struct LocationTag {
    pub country: String,
    pub province: String,
    pub city: String,
    pub district: String,
}

impl fmt::Display for LocationTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}, {}", self.city, self.province, self.country)
    }
}

/// 多边形顶点
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
}

/// 行政区划多边形
#[derive(Debug, Clone)]
pub struct AdminBoundary {
    pub level: u8,  // 0=国家, 1=省州, 2=城市, 3=区县
    pub name: String,
    pub vertices: Vec<Point>,
    pub parent_name: String,
    pub tags: LocationTag,
}

/// 离线地理编码系统
pub struct OfflineGeocoding {
    boundaries: Vec<AdminBoundary>,
    /// 按城市名称索引（快速查询）
    city_index: BTreeMap<String, usize>,
}

use alloc::collections::BTreeMap;

impl OfflineGeocoding {
    /// 创建新的地理编码系统
    pub fn new() -> Self {
        OfflineGeocoding {
            boundaries: Vec::new(),
            city_index: BTreeMap::new(),
        }
    }

    /// 加载OSM边界数据
    pub fn load_boundaries(&mut self, boundaries: Vec<AdminBoundary>) -> Result<(), &'static str> {
        self.boundaries = boundaries;
        
        // 构建索引
        for (idx, boundary) in self.boundaries.iter().enumerate() {
            if boundary.level == 2 {  // 城市级别
                self.city_index.insert(boundary.name.clone(), idx);
            }
        }

        Ok(())
    }

    /// 逆地理编码：将GPS坐标转换为位置标签
    pub fn reverse_geocode(&self, coord: &GPSCoordinate) -> Result<LocationTag, &'static str> {
        // 验证坐标
        if !coord.is_valid() {
            return Err("Invalid coordinate");
        }

        if coord.is_invalid_sentinel() {
            return Err("Sentinel coordinate (0,0)");
        }

        // 执行Point-in-Polygon查询
        self._find_containing_boundary(coord)
    }

    /// 根据城市名称快速查询
    pub fn find_by_city_name(&self, city_name: &str) -> Option<LocationTag> {
        self.city_index
            .get(city_name)
            .and_then(|&idx| Some(self.boundaries[idx].tags.clone()))
    }

    /// 支持模糊城市名称查询
    pub fn find_by_city_prefix(&self, prefix: &str) -> Vec<LocationTag> {
        let mut results = Vec::new();
        for (city_name, &idx) in self.city_index.iter() {
            if city_name.starts_with(prefix) {
                results.push(self.boundaries[idx].tags.clone());
            }
        }
        results
    }

    /// 点在多边形内判断（Ray Casting算法）
    fn _is_point_in_polygon(&self, point: &GPSCoordinate, vertices: &[Point]) -> bool {
        if vertices.len() < 3 {
            return false;
        }

        let mut inside = false;
        let mut j = vertices.len() - 1;

        for i in 0..vertices.len() {
            let xi = vertices[i].lon;
            let yi = vertices[i].lat;
            let xj = vertices[j].lon;
            let yj = vertices[j].lat;

            let intersect = ((yi > point.latitude) != (yj > point.latitude))
                && (point.longitude < (xj - xi) * (point.latitude - yi) / (yj - yi) + xi);

            if intersect {
                inside = !inside;
            }

            j = i;
        }

        inside
    }

    /// 执行Point-in-Polygon查询
    fn _find_containing_boundary(
        &self,
        coord: &GPSCoordinate,
    ) -> Result<LocationTag, &'static str> {
        // 按level从高到低搜索（国家→省→城市→区县）
        let mut result: Option<LocationTag> = None;

        for boundary in self.boundaries.iter().rev() {
            if self._is_point_in_polygon(coord, &boundary.vertices) {
                if result.is_none() {
                    result = Some(boundary.tags.clone());
                } else if boundary.level > result.as_ref().unwrap().country.len() as u8 {
                    // 更新为更高级别的边界
                    result = Some(boundary.tags.clone());
                }
            }
        }

        result.ok_or("No matching location found")
    }

    /// 批量逆地理编码
    pub fn batch_reverse_geocode(
        &self,
        coords: &[GPSCoordinate],
    ) -> Vec<Result<LocationTag, &'static str>> {
        coords.iter().map(|c| self.reverse_geocode(c)).collect()
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> (usize, usize, usize, usize) {
        let mut countries = 0;
        let mut provinces = 0;
        let mut cities = 0;
        let mut districts = 0;

        for boundary in &self.boundaries {
            match boundary.level {
                0 => countries += 1,
                1 => provinces += 1,
                2 => cities += 1,
                3 => districts += 1,
                _ => {}
            }
        }

        (countries, provinces, cities, districts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_validation() {
        let valid = GPSCoordinate::new(24.4798, 118.0894);
        assert!(valid.is_valid());
        assert!(!valid.is_invalid_sentinel());

        let invalid = GPSCoordinate::new(0.0, 0.0);
        assert!(invalid.is_valid());
        assert!(invalid.is_invalid_sentinel());

        let out_of_range = GPSCoordinate::new(91.0, 180.5);
        assert!(!out_of_range.is_valid());
    }

    #[test]
    fn test_point_in_polygon() {
        let geocoding = OfflineGeocoding::new();

        // 简单矩形测试
        let rect = vec![
            Point { lat: 0.0, lon: 0.0 },
            Point { lat: 1.0, lon: 0.0 },
            Point { lat: 1.0, lon: 1.0 },
            Point { lat: 0.0, lon: 1.0 },
        ];

        let inside = GPSCoordinate::new(0.5, 0.5);
        assert!(geocoding._is_point_in_polygon(&inside, &rect));

        let outside = GPSCoordinate::new(2.0, 2.0);
        assert!(!geocoding._is_point_in_polygon(&outside, &rect));
    }

    #[test]
    fn test_location_tag_display() {
        let tag = LocationTag {
            country: "China".to_string(),
            province: "Fujian".to_string(),
            city: "Xiamen".to_string(),
            district: "Siming".to_string(),
        };

        let display = format!("{}", tag);
        assert!(display.contains("Xiamen"));
        assert!(display.contains("Fujian"));
    }

    #[test]
    fn test_batch_geocode() {
        let mut geocoding = OfflineGeocoding::new();
        
        // 创建测试数据
        let boundary = AdminBoundary {
            level: 2,
            name: "Xiamen".to_string(),
            vertices: vec![
                Point { lat: 24.0, lon: 118.0 },
                Point { lat: 25.0, lon: 118.0 },
                Point { lat: 25.0, lon: 119.0 },
                Point { lat: 24.0, lon: 119.0 },
            ],
            parent_name: "Fujian".to_string(),
            tags: LocationTag {
                country: "China".to_string(),
                province: "Fujian".to_string(),
                city: "Xiamen".to_string(),
                district: "".to_string(),
            },
        };

        geocoding.load_boundaries(vec![boundary]).unwrap();

        let coords = vec![
            GPSCoordinate::new(24.5, 118.5),
            GPSCoordinate::new(0.0, 0.0),
        ];

        let results = geocoding.batch_reverse_geocode(&coords);
        assert_eq!(results.len(), 2);
        assert!(results[0].is_ok());
    }
}
