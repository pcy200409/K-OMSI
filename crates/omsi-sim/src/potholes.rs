//! Potholes (K-OMSI): holes in the paved roads, more of them after heavy rain - water gets
//! under the asphalt, freezes and the traffic breaks it out. Where they are follows from the
//! place alone (a hash of a 4 m cell), so a hole stays where it was; how many there are
//! follows from the rain. A wheel over one drops in (`RigidBody` takes `depth_at` off the
//! ground under each tyre), and the scene draws the ones near the camera.

use crate::world_data::WorldData;
use glam::DVec2;
use std::sync::{Arc, RwLock};

/// The cell a hole may lie in (m).
pub const CELL: f64 = 4.0;
/// Holes per cell of paved road: on any road, and after the heaviest rain.
pub const BASE: f32 = 0.006;
pub const AFTER_RAIN: f32 = 0.03;

/// One hole: its middle, its radius and its depth (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hole {
    pub centre: DVec2,
    pub radius: f32,
    pub depth: f32,
}

/// The holes of the loaded roads at a density.
#[derive(Clone)]
pub struct Potholes {
    pub density: f32,
    pub world: Arc<WorldData>,
}

fn hash(x: i64, y: i64, k: u64) -> f64 {
    let mut z = (x as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (y as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f) ^ k.wrapping_mul(0x1656_67b1_9e37_79f9);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) as f64 / u64::MAX as f64
}

/// The hole of cell (cx, cy) at `density`, wherever there is road or not.
pub fn hole_of_cell(cx: i64, cy: i64, density: f32) -> Option<Hole> {
    if hash(cx, cy, 1) >= density as f64 {
        return None;
    }
    let o = DVec2::new(cx as f64, cy as f64) * CELL;
    Some(Hole {
        centre: o + DVec2::new(0.5 + hash(cx, cy, 2) * 3.0, 0.5 + hash(cx, cy, 3) * 3.0),
        radius: (0.35 + hash(cx, cy, 4) * 0.35) as f32,
        depth: (0.04 + hash(cx, cy, 5) * 0.05) as f32,
    })
}

impl Potholes {
    /// The hole of a cell, if its middle lies on a paved road.
    pub fn hole(&self, cx: i64, cy: i64) -> Option<Hole> {
        hole_of_cell(cx, cy, self.density).filter(|h| self.world.wet_road_at(h.centre.x, h.centre.y, 1.0) > 0.0)
    }

    /// How far the road drops at world (x, y) (m, 0 off the holes): a bowl in each.
    pub fn depth_at(&self, x: f64, y: f64) -> f32 {
        let (cx, cy) = ((x / CELL).floor() as i64, (y / CELL).floor() as i64);
        let mut deepest = 0.0f32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(h) = hole_of_cell(cx + dx, cy + dy, self.density) {
                    let d = (DVec2::new(x, y) - h.centre).length() as f32;
                    if d < h.radius {
                        let s = d / h.radius;
                        let drop = h.depth * (1.0 - s * s);
                        if drop > deepest && self.world.wet_road_at(h.centre.x, h.centre.y, 1.0) > 0.0 {
                            deepest = drop;
                        }
                    }
                }
            }
        }
        deepest
    }

    /// The holes within `reach` of `at`.
    pub fn near(&self, at: DVec2, reach: f64) -> Vec<Hole> {
        let (lo, hi) = (((at - DVec2::splat(reach)) / CELL).floor(), ((at + DVec2::splat(reach)) / CELL).floor());
        let mut out = Vec::new();
        for cy in lo.y as i64..=hi.y as i64 {
            for cx in lo.x as i64..=hi.x as i64 {
                if let Some(h) = self.hole(cx, cy) {
                    if (h.centre - at).length() <= reach {
                        out.push(h);
                    }
                }
            }
        }
        out
    }
}

/// The holes the vehicles drive over (the game sets them; None: no potholes).
static FIELD: RwLock<Option<Arc<Potholes>>> = RwLock::new(None);

pub fn set(p: Option<Arc<Potholes>>) {
    *FIELD.write().unwrap_or_else(|e| e.into_inner()) = p;
}

pub fn current() -> Option<Arc<Potholes>> {
    FIELD.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The density after `rain_mm` of rain fallen (its effect saturates at 60 mm).
pub fn density_after(rain_mm: f32) -> f32 {
    BASE + (AFTER_RAIN - BASE) * (rain_mm / 60.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holes_stay_where_they_are_and_rain_makes_more() {
        let count = |d: f32| (0..200).flat_map(|y| (0..200).map(move |x| (x, y))).filter(|&(x, y)| hole_of_cell(x, y, d).is_some()).count();
        let (dry, wet) = (count(BASE), count(AFTER_RAIN));
        assert!(dry > 100 && wet > dry * 3, "{dry} -> {wet}");
        // the holes of the lower density are among those of the higher one, in the same place
        for y in 0..50 {
            for x in 0..50 {
                if let Some(h) = hole_of_cell(x, y, BASE) {
                    assert_eq!(hole_of_cell(x, y, AFTER_RAIN), Some(h));
                }
            }
        }
    }

    #[test]
    fn a_hole_lies_in_its_cell_and_is_a_few_centimetres_deep() {
        for (x, y) in (0..1000).map(|k| (k, k * 7)) {
            if let Some(h) = hole_of_cell(x, y, 1.0) {
                let o = DVec2::new(x as f64, y as f64) * CELL;
                assert!(h.centre.x - o.x >= 0.5 && h.centre.x - o.x <= 3.5);
                assert!((0.04..=0.09).contains(&h.depth) && (0.35..=0.7).contains(&h.radius));
            }
        }
    }

    #[test]
    fn rain_raises_the_density_to_its_limit() {
        assert_eq!(density_after(0.0), BASE);
        assert!((density_after(600.0) - AFTER_RAIN).abs() < 1e-6);
    }
}
