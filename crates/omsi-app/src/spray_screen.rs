//! The spray of the vehicles ahead on a wet road thrown onto the player's windscreen
//! (K-OMSI): a car close in front at speed, a lorry or a bus more so, wets the glass the
//! wipers have to clear (`VehicleHost::spray_ahead`, taken by `window_wipers`).

use glam::DVec2;

/// How far ahead a vehicle's spray reaches the windscreen (m), and how far to the side.
const REACH: f64 = 40.0;
const SIDE: f64 = 2.5;
/// The speed (m/s) a tyre starts to throw spray up at, and the speed it throws all of it.
const FROM: f32 = 6.0;
const FULL: f32 = 20.0;

/// The spray reaching the windscreen of `bus` from `others` (position, speed m/s, length m)
/// on roads `wetness` wet (0..1): the heaviest of them.
pub(crate) fn spray_ahead(bus: &omsi_sim::VehicleInstance, others: impl Iterator<Item = (glam::DVec3, f32, f32)>, wetness: f32) -> f32 {
    if wetness <= 0.05 {
        return 0.0;
    }
    let h = bus.heading.to_radians();
    let (fwd, right) = (DVec2::new(h.sin(), h.cos()), DVec2::new(h.cos(), -h.sin()));
    let at = bus.position.truncate();
    others
        .map(|(p, speed, length)| {
            let d = p.truncate() - at;
            let (ahead, side) = (d.dot(fwd), d.dot(right).abs());
            if !(3.0..REACH).contains(&ahead) || side > SIDE {
                return 0.0;
            }
            let fast = ((speed - FROM) / (FULL - FROM)).clamp(0.0, 1.0);
            let big = if length > 8.0 { 1.5 } else { 1.0 };
            fast * big * (1.0 - ahead / REACH) as f32
        })
        .fold(0.0f32, f32::max)
        .min(1.0)
        * wetness.clamp(0.0, 1.0)
}
