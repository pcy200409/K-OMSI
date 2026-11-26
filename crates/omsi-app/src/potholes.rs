//! Potholes (K-OMSI, the "Potholes" setting): the holes of the paved roads
//! (`omsi_sim::potholes`), more of them as the rain of this run adds up; handed to the
//! vehicles' physics and, through `gfx.potholes`, drawn near the camera.

use omsi_sim::potholes::{self, Potholes};
use std::sync::Arc;

/// The rain counts this many times over (as the flood's water does: a run is short).
const RAIN_PACE: f32 = 10.0;
/// The loaded ground is taken anew this often (s): new tiles bring their roads.
const REFRESH: f32 = 2.0;

#[derive(Default)]
pub(crate) struct PotholeState {
    /// The rain fallen in this run (mm, at `RAIN_PACE`).
    rain_mm: f32,
    pub(crate) field: Option<Arc<Potholes>>,
    since: f32,
}

impl PotholeState {
    pub(crate) fn tick(&mut self, on: bool, world: Option<&omsi_scene::World>, weather: Option<&omsi_content::weather::Weather>, dt: f32) {
        let Some(world) = world.filter(|_| on) else {
            if self.field.take().is_some() {
                potholes::set(None);
            }
            return;
        };
        if let Some((1, rate)) = weather.map(omsi_weather::conditions::precip_of) {
            self.rain_mm += omsi_scene::rain::rain_rate_mm_h(rate) * dt / 3600.0 * RAIN_PACE;
        }
        self.since += dt;
        let density = potholes::density_after(self.rain_mm);
        let stale = self.field.as_ref().is_none_or(|f| (f.density - density).abs() >= 0.0005);
        if stale || self.since >= REFRESH {
            if self.field.is_none() {
                log::info!("potholes: on, {:.1} per 1000 m² of road", density / (potholes::CELL * potholes::CELL) as f32 * 1000.0);
            }
            self.since = 0.0;
            let field = Arc::new(Potholes { density, world: world.data() });
            potholes::set(Some(field.clone()));
            self.field = Some(field);
        }
    }
}
