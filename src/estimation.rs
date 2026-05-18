use crate::coordinate_system::geographic::LLBBox;
use crate::coordinate_system::transformation::geo_distance;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationEstimate {
    pub time_seconds_min: u64,
    pub time_seconds_max: u64,
    pub disk_mb_min: u64,
    pub disk_mb_max: u64,
    pub world_blocks_x: usize,
    pub world_blocks_z: usize,
}

pub fn estimate_generation(bbox: &LLBBox, scale: f64, terrain: bool) -> GenerationEstimate {
    let (height_m, width_m) = geo_distance(bbox.min(), bbox.max());
    let area_km2 = (height_m * width_m) / 1_000_000.0;

    let blocks_x = (width_m * scale).max(0.0) as usize;
    let blocks_z = (height_m * scale).max(0.0) as usize;

    let terrain_factor = if terrain { 1.5 } else { 1.0 };
    let base_time = area_km2 * 15.0 * terrain_factor;

    let regions_x = (blocks_x as f64 / 512.0).ceil() as u64;
    let regions_z = (blocks_z as f64 / 512.0).ceil() as u64;
    let num_regions = regions_x.max(1) * regions_z.max(1);
    let disk_mb_est = (num_regions * 200).div_ceil(1024);

    GenerationEstimate {
        time_seconds_min: (base_time * 0.5).max(1.0) as u64,
        time_seconds_max: (base_time * 2.0).max(1.0) as u64,
        disk_mb_min: (disk_mb_est / 2).max(1),
        disk_mb_max: (disk_mb_est * 3).max(1),
        world_blocks_x: blocks_x,
        world_blocks_z: blocks_z,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_scales_with_terrain() {
        let bbox = LLBBox::new(54.627053, 9.927928, 54.634902, 9.937563).unwrap();
        let without_terrain = estimate_generation(&bbox, 1.0, false);
        let with_terrain = estimate_generation(&bbox, 1.0, true);

        assert!(without_terrain.world_blocks_x > 0);
        assert!(without_terrain.world_blocks_z > 0);
        assert!(with_terrain.time_seconds_max >= without_terrain.time_seconds_max);
    }
}
