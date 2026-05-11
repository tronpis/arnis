//! Copernicus DEM GLO-30 — global 30m elevation via AWS S3.
//!
//! URL pattern: https://copernicus-dem-30m.s3.amazonaws.com/
//!   Copernicus_DSM_COG_10_{lat_str}_00_{lng_str}_00_DEM/
//!   Copernicus_DSM_COG_10_{lat_str}_00_{lng_str}_00_DEM.tif
//!
//! Tiles are 1° × 1° GeoTIFF (COG) at 1 arc-second resolution.
//! No API key required. Coverage: -90 to +90 lat, -180 to +180 lng.
//! Missing tiles (ocean areas) → 404, treat as NaN.

use crate::coordinate_system::geographic::LLBBox;
use crate::elevation::cache::get_cache_dir;
use crate::elevation::provider::{ElevationProvider, RawElevationGrid};

/// Lowest valid terrestrial elevation with margin; common DEM nodata sentinels are far lower.
const MIN_VALID_ELEVATION_M: f64 = -9999.0;
/// Upper sanity bound for DEM values; real Earth elevations are well below this.
const MAX_VALID_ELEVATION_M: f64 = 100000.0;

/// Copernicus DEM GLO-30 base URL on AWS S3
const COPERNICUS_BASE_URL: &str = "https://copernicus-dem-30m.s3.amazonaws.com";

fn normalize_elevation_sample(value: f64) -> f64 {
    if value.is_finite() && value > MIN_VALID_ELEVATION_M && value < MAX_VALID_ELEVATION_M {
        value
    } else {
        f64::NAN
    }
}

fn overlapping_tile_range(min: f64, max: f64, min_tile: i32, max_tile: i32) -> std::ops::RangeInclusive<i32> {
    let start = (min.floor() as i32).max(min_tile);
    let end = (max.ceil() as i32 - 1).min(max_tile);
    start..=end
}

fn overlapping_tiles(bbox: &LLBBox) -> Vec<(i32, i32)> {
    let lats = overlapping_tile_range(bbox.min().lat(), bbox.max().lat(), -90, 89);
    let lngs = overlapping_tile_range(bbox.min().lng(), bbox.max().lng(), -180, 179);

    lats.into_iter()
        .flat_map(|lat| lngs.iter().copied().map(move |lng| (lat, lng)))
        .collect()
}

/// Copernicus DEM GLO-30 global coverage provider (~30m resolution).
pub struct CopernicusDem30;

impl ElevationProvider for CopernicusDem30 {
    fn name(&self) -> &'static str {
        "copernicus_30m"
    }

    fn coverage_bboxes(&self) -> Option<Vec<LLBBox>> {
        None // Global coverage
    }

    fn native_resolution_m(&self) -> f64 {
        30.0
    }

    fn clone_box(&self) -> Box<dyn ElevationProvider> {
        Box::new(CopernicusDem30)
    }

    fn fetch_raw(
        &self,
        bbox: &LLBBox,
        grid_width: usize,
        grid_height: usize,
    ) -> Result<RawElevationGrid, Box<dyn std::error::Error>> {
        use reqwest::blocking::Client;
        use std::io::Cursor;
        use std::time::Duration;
        use tiff::decoder::{Decoder, DecodingResult};

        let client = Client::builder()
            .timeout(Duration::from_secs(120))
            .user_agent(concat!(
                "Arnis/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/louis-e/arnis)"
            ))
            .build()?;

        let tile_cache_dir = get_cache_dir(self.name());
        if !tile_cache_dir.exists() {
            std::fs::create_dir_all(&tile_cache_dir)?;
        }

        // Determine which 1°×1° tiles overlap the bbox
        let min_lat = bbox.min().lat();
        let max_lat = bbox.max().lat();
        let min_lng = bbox.min().lng();
        let max_lng = bbox.max().lng();

        // Collect all tile coordinates that overlap our bbox. Tile coordinates
        // are 1° lower-left origins, so a bbox whose max edge lies exactly on
        // an integer degree does not overlap the next tile.
        let tiles = overlapping_tiles(bbox);

        println!(
            "Downloading {} Copernicus DEM tiles ({}m resolution)...",
            tiles.len(),
            self.native_resolution_m()
        );

        // Download and cache tiles, then merge into a single grid
        let mut tile_data: std::collections::HashMap<(i32, i32), Vec<f64>> =
            std::collections::HashMap::new();
        let mut tile_dims: std::collections::HashMap<(i32, i32), (usize, usize)> =
            std::collections::HashMap::new();

        for &(lat, lng) in &tiles {
            let lat_str = if lat >= 0 {
                format!("N{:02}", lat)
            } else {
                format!("S{:02}", lat.abs())
            };
            let lng_str = if lng >= 0 {
                format!("E{:03}", lng)
            } else {
                format!("W{:03}", lng.abs())
            };

            let tile_name = format!("Copernicus_DSM_COG_10_{}_00_{}_00_DEM", lat_str, lng_str);
            let tif_url = format!("{}/{}/{}.tif", COPERNICUS_BASE_URL, tile_name, tile_name);
            let cache_path = tile_cache_dir.join(format!("{}_{}.tiff", lat_str, lng_str));

            // Try to load from cache first
            let tif_data = if cache_path.exists() {
                std::fs::read(&cache_path)?
            } else {
                // Download from S3
                let response = client.get(&tif_url).send();
                match response {
                    Ok(resp) => {
                        if resp.status().is_success() {
                            let bytes = resp.bytes()?;
                            // Cache for future use
                            std::fs::write(&cache_path, &bytes[..])?;
                            bytes.to_vec()
                        } else if resp.status() == 404 {
                            // Ocean area or missing tile - skip
                            continue;
                        } else {
                            eprintln!(
                                "Warning: Copernicus tile {} returned status {}",
                                tif_url,
                                resp.status()
                            );
                            continue;
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to download Copernicus tile {}: {}",
                            tif_url, e
                        );
                        continue;
                    }
                }
            };

            // Parse GeoTIFF using tiff crate
            let cursor = Cursor::new(tif_data);
            let mut decoder = Decoder::new(cursor)?;

            let width = decoder.dimensions()?.0 as usize;
            let height = decoder.dimensions()?.1 as usize;
            tile_dims.insert((lat, lng), (width, height));

            // Read the image data - Copernicus DEM typically uses 32-bit float, but the decoder handles 64-bit as well.
            let heights: Vec<f64> = match decoder.read_image()? {
                DecodingResult::F32(values) => values
                    .into_iter()
                    .map(|value| normalize_elevation_sample(f64::from(value)))
                    .collect(),
                DecodingResult::F64(values) => {
                    values.into_iter().map(normalize_elevation_sample).collect()
                }
                other => {
                    return Err(
                        format!("Unsupported Copernicus DEM sample format: {:?}", other).into(),
                    );
                }
            };
            tile_data.insert((lat, lng), heights);
        }

        if tile_data.is_empty() {
            // All tiles were missing (ocean area) - return all NaN grid
            let nan_grid = vec![vec![f64::NAN; grid_width]; grid_height];
            return Ok(RawElevationGrid {
                heights_meters: nan_grid,
            });
        }

        // Now we need to sample from these tiles onto our output grid
        // For simplicity, use nearest-neighbor sampling from the tile data
        let mut heights_meters: Vec<Vec<f64>> = vec![vec![f64::NAN; grid_width]; grid_height];

        // Calculate the geographic bounds per grid cell
        let lat_step = (max_lat - min_lat) / grid_height as f64;
        let lng_step = (max_lng - min_lng) / grid_width as f64;

        for (row_idx, row) in heights_meters.iter_mut().enumerate() {
            for (col_idx, height) in row.iter_mut().enumerate() {
                // Calculate the geographic coordinate of this grid cell center
                let sample_lat = max_lat - (row_idx as f64 + 0.5) * lat_step;
                let sample_lng = min_lng + (col_idx as f64 + 0.5) * lng_step;

                // Find which tile contains this point
                let tile_lat = sample_lat.floor() as i32;
                let tile_lng = sample_lng.floor() as i32;

                if let Some(tile_heights) = tile_data.get(&(tile_lat, tile_lng)) {
                    if let Some(&(w, h)) = tile_dims.get(&(tile_lat, tile_lng)) {
                        // Calculate pixel coordinates within the tile
                        let local_lat_frac = sample_lat - tile_lat as f64;
                        let local_lng_frac = sample_lng - tile_lng as f64;

                        // Convert to pixel coordinates (GeoTIFF origin is top-left)
                        let pixel_x = (local_lng_frac * w as f64) as usize;
                        let pixel_y = ((1.0 - local_lat_frac) * h as f64) as usize;

                        if pixel_x < w && pixel_y < h {
                            let idx = pixel_y * w + pixel_x;
                            if idx < tile_heights.len() {
                                *height = tile_heights[idx];
                            }
                        }
                    }
                }
            }
        }

        Ok(RawElevationGrid { heights_meters })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copernicus_name() {
        let provider = CopernicusDem30;
        assert_eq!(provider.name(), "copernicus_30m");
    }

    #[test]
    fn test_copernicus_global_coverage() {
        let provider = CopernicusDem30;
        assert!(provider.coverage_bboxes().is_none());
    }

    #[test]
    fn test_copernicus_resolution() {
        let provider = CopernicusDem30;
        assert!((provider.native_resolution_m() - 30.0).abs() < 0.1);
    }

    #[test]
    fn test_normalize_elevation_sample_keeps_valid_values() {
        assert_eq!(normalize_elevation_sample(42.5), 42.5);
        assert_eq!(normalize_elevation_sample(-430.0), -430.0);
    }

    #[test]
    fn test_normalize_elevation_sample_filters_invalid_values() {
        assert!(normalize_elevation_sample(f64::NAN).is_nan());
        assert!(normalize_elevation_sample(f64::INFINITY).is_nan());
        assert!(normalize_elevation_sample(-32767.0).is_nan());
        assert!(normalize_elevation_sample(100000.0).is_nan());
    }

    #[test]
    fn test_overlapping_tiles_excludes_integer_max_edge() {
        let bbox = LLBBox::new(10.25, 20.25, 11.0, 21.0).unwrap();
        assert_eq!(overlapping_tiles(&bbox), vec![(10, 20)]);
    }

    #[test]
    fn test_overlapping_tiles_clamps_global_edges() {
        let bbox = LLBBox::new(-90.0, -180.0, 90.0, 180.0).unwrap();
        let tiles = overlapping_tiles(&bbox);

        assert_eq!(tiles.len(), 180 * 360);
        assert!(tiles.contains(&(-90, -180)));
        assert!(tiles.contains(&(89, 179)));
        assert!(!tiles.contains(&(90, 180)));
    }
}
