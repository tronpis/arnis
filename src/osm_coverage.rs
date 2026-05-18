use crate::coordinate_system::geographic::LLBBox;
use crate::coordinate_system::transformation::geo_distance;
use crate::osm_parser::ProcessedElement;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageLevel {
    High,
    Medium,
    Low,
    VeryLow,
}

impl CoverageLevel {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageLevel::High => "high",
            CoverageLevel::Medium => "medium",
            CoverageLevel::Low => "low",
            CoverageLevel::VeryLow => "very_low",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OsmCoverageReport {
    pub building_count: usize,
    pub road_count: usize,
    pub area_km2: f64,
    pub buildings_per_km2: f64,
    pub roads_per_km2: f64,
    pub coverage_level: CoverageLevel,
}

pub fn assess_osm_coverage(elements: &[ProcessedElement], bbox: &LLBBox) -> OsmCoverageReport {
    let (height_m, width_m) = geo_distance(bbox.min(), bbox.max());
    let area_km2 = (height_m * width_m) / 1_000_000.0;

    let building_count = elements
        .iter()
        .filter(|element| element.tags().contains_key("building"))
        .count();
    let road_count = elements
        .iter()
        .filter(|element| element.tags().contains_key("highway"))
        .count();

    let buildings_per_km2 = building_count as f64 / area_km2.max(0.01);
    let roads_per_km2 = road_count as f64 / area_km2.max(0.01);

    let coverage_level = match buildings_per_km2 {
        x if x > 100.0 => CoverageLevel::High,
        x if x > 20.0 => CoverageLevel::Medium,
        x if x > 5.0 => CoverageLevel::Low,
        _ => CoverageLevel::VeryLow,
    };

    OsmCoverageReport {
        building_count,
        road_count,
        area_km2,
        buildings_per_km2,
        roads_per_km2,
        coverage_level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osm_parser::ProcessedWay;
    use std::collections::HashMap;

    fn tagged_way(id: u64, key: &str) -> ProcessedElement {
        ProcessedElement::Way(ProcessedWay {
            id,
            nodes: Vec::new(),
            tags: HashMap::from([(key.to_string(), "yes".to_string())]),
        })
    }

    #[test]
    fn assesses_very_low_coverage() {
        let bbox = LLBBox::new(0.0, 0.0, 0.01, 0.01).unwrap();
        let report = assess_osm_coverage(&[], &bbox);

        assert_eq!(report.coverage_level, CoverageLevel::VeryLow);
        assert_eq!(report.building_count, 0);
        assert_eq!(report.road_count, 0);
    }

    #[test]
    fn assesses_high_coverage() {
        let bbox = LLBBox::new(0.0, 0.0, 0.005, 0.005).unwrap();
        let elements: Vec<_> = (0..40).map(|id| tagged_way(id, "building")).collect();
        let report = assess_osm_coverage(&elements, &bbox);

        assert_eq!(report.coverage_level, CoverageLevel::High);
        assert_eq!(report.building_count, 40);
    }
}
