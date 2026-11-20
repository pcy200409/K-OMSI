//! Black ice (K-OMSI): the water on the roads freezing when the air goes below zero - first
//! on the bridges and flyovers, which the cold reaches from below as well, and in patches
//! elsewhere (a shaded stretch, a dip where the water stood) - and thawing again. It takes
//! the tyres' grip away (`RigidBody::ice`) and slows the AI traffic down on it.

use glam::DVec2;
use std::sync::atomic::{AtomicU32, Ordering};

/// How much of its grip ice takes from a tyre (an icy road gives a fifth of a dry one).
pub const GRIP_LOSS: f32 = 0.8;
/// The air temperatures (°C) the wet road freezes below: the ground and a bridge's deck.
const FREEZE_GROUND: f32 = 0.5;
const FREEZE_BRIDGE: f32 = 2.0;
/// The size of the patches the ice lies in on the ground (m).
const PATCH: f64 = 25.0;

/// How icy the roads are: on the ground (in its patches) and on bridges, 0 .. 1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ice {
    pub ground: f32,
    pub bridge: f32,
}

impl Ice {
    /// At the start of a session: water already on the road in a frost is ice.
    pub fn initial(temp: f32, wetness: f32) -> Ice {
        Ice {
            ground: if temp <= FREEZE_GROUND { wetness.clamp(0.0, 1.0) } else { 0.0 },
            bridge: if temp <= FREEZE_BRIDGE { wetness.clamp(0.0, 1.0) } else { 0.0 },
        }
    }

    /// `secs` of air at `temp` °C over roads `wetness` wet (0..1), `raining` or not.
    pub fn step(&mut self, secs: f32, temp: f32, wetness: f32, raining: bool) {
        let one = |ice: &mut f32, freeze: f32| {
            if temp <= freeze {
                // the colder, the faster the film freezes: a wet road through at -3 °C in
                // about five minutes; it holds no more ice than there was water
                let rate = ((freeze - temp) / 3.0).clamp(0.1, 1.0) / 300.0;
                *ice = (*ice + rate * secs).min(wetness.clamp(0.0, 1.0).max(*ice));
            } else {
                // thawing: the warmer, the faster, and rain on it faster still
                let rate = ((temp - freeze) / 4.0).clamp(0.05, 1.0) / 600.0 * if raining { 3.0 } else { 1.0 };
                *ice = (*ice - rate * secs).max(0.0);
            }
        };
        one(&mut self.ground, FREEZE_GROUND);
        one(&mut self.bridge, FREEZE_BRIDGE);
    }

    /// The ice under a point (world x, y): a bridge's, or the ground's in its patches.
    pub fn at(&self, p: DVec2, on_bridge: bool) -> f32 {
        if on_bridge {
            self.bridge.max(self.ground)
        } else {
            (self.ground * patch(p)).clamp(0.0, 1.0)
        }
    }
}

/// The patchiness of ice on the ground at world point `p`: 0 (dry, kept clear) .. 1.6 (where
/// the water stood), a smooth noise of `PATCH` metres.
pub fn patch(p: DVec2) -> f32 {
    let q = p / PATCH;
    let (i, f) = (q.floor(), q - q.floor());
    let u = f * f * (DVec2::splat(3.0) - 2.0 * f);
    // (splitmix64 of the cell, on unsigned words: logical shifts)
    let h = |x: f64, y: f64| -> f64 {
        let mut z = (x as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (y as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        (z ^ (z >> 31)) as f64 / u64::MAX as f64
    };
    let a = h(i.x, i.y);
    let b = h(i.x + 1.0, i.y);
    let c = h(i.x, i.y + 1.0);
    let d = h(i.x + 1.0, i.y + 1.0);
    let n = a + (b - a) * u.x + (c - a) * u.y + (a - b - c + d) * u.x * u.y;
    (((n - 0.35) * 2.6).clamp(0.0, 1.6)) as f32
}

/// The ice the AI traffic drives on (one figure for all: they slow down on an icy day).
static AI_ICE: AtomicU32 = AtomicU32::new(0);

pub fn set_ai_ice(ice: f32) {
    AI_ICE.store(ice.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// The share of their speed the AI drivers keep on today's ice (1 on a dry day, 0.6 on ice).
pub fn ai_speed_factor() -> f32 {
    1.0 - 0.4 * f32::from_bits(AI_ICE.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wet_road_freezes_in_the_frost_and_thaws_in_the_mild() {
        let mut ice = Ice::default();
        ice.step(600.0, -3.0, 1.0, false);
        assert!(ice.ground > 0.9 && ice.bridge > 0.9, "{ice:?}");
        ice.step(600.0, 6.0, 1.0, true);
        assert!(ice.ground < 0.5, "{ice:?}");
        ice.step(3600.0, 6.0, 1.0, false);
        assert_eq!(ice, Ice::default());
    }

    #[test]
    fn a_bridge_freezes_before_the_ground() {
        let mut ice = Ice::default();
        ice.step(900.0, 1.5, 1.0, false);
        assert_eq!(ice.ground, 0.0);
        assert!(ice.bridge > 0.4);
        assert!(ice.at(DVec2::ZERO, true) > 0.4);
    }

    #[test]
    fn a_dry_road_does_not_ice() {
        let mut ice = Ice::default();
        ice.step(3600.0, -8.0, 0.0, false);
        assert_eq!(ice, Ice::default());
    }

    #[test]
    fn the_ground_ices_in_patches() {
        let ice = Ice { ground: 1.0, bridge: 0.0 };
        let samples: Vec<f32> = (0..400).map(|k| ice.at(DVec2::new(k as f64 * 7.0, (k % 13) as f64 * 11.0), false)).collect();
        assert!(samples.iter().any(|&s| s == 0.0), "some of it stays clear");
        assert!(samples.iter().any(|&s| s >= 0.9), "some of it is ice");
    }
}
