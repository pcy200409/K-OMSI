//! Black ice (K-OMSI, the "Black ice" setting): the roads' water freezing in the frost
//! (`omsi_sim::road_ice`), stepped every frame with the weather; the player's bus gets the
//! ice under it (`VehicleHost::ice`, a bridge's when it is on one) and the AI traffic a
//! figure for the day.

use omsi_sim::road_ice::{self, Ice};

/// A road this far over the terrain is a bridge or a flyover (m): it freezes first.
const BRIDGE: f64 = 3.0;
/// The ice the driver is told about, once a run.
const NOTICE: f32 = 0.3;

#[derive(Default)]
pub(crate) struct IceState {
    ice: Option<Ice>,
    told: bool,
    /// A message for the driver, once (`frame_ui` shows it).
    pub(crate) notice: Option<String>,
}

impl IceState {
    /// A frame of `dt` seconds: `on` the setting, the roads `wetness` wet.
    pub(crate) fn tick(
        &mut self,
        on: bool,
        world: Option<&omsi_scene::World>,
        weather: Option<&omsi_content::weather::Weather>,
        wetness: f32,
        vehicle: Option<&mut omsi_sim::VehicleInstance>,
        dt: f32,
    ) {
        let Some(w) = weather.filter(|_| on) else {
            if self.ice.take().is_some() {
                road_ice::set_ai_ice(0.0);
            }
            if let Some(v) = vehicle {
                v.host.ice = 0.0;
            }
            return;
        };
        let temp = w.temp.0;
        let (kind, rate) = omsi_weather::conditions::precip_of(w);
        let ice = self.ice.get_or_insert_with(|| {
            let ice = Ice::initial(temp, wetness);
            log::info!("black ice: on at {temp:.1} °C, roads {:.0} % wet: ground {:.2}, bridges {:.2}", wetness * 100.0, ice.ground, ice.bridge);
            ice
        });
        ice.step(dt, temp, wetness, kind == 1 && rate > 0.0);
        road_ice::set_ai_ice((ice.ground * 0.7).max(ice.bridge * 0.4));
        let Some(v) = vehicle else { return };
        let (x, y) = (v.position.x, v.position.y);
        let bridge = world
            .and_then(|wd| Some(wd.ground_height(x, y)? - wd.ground_terrain(x, y)?))
            .is_some_and(|h| h > BRIDGE);
        let here = ice.at(glam::DVec2::new(x, y), bridge);
        v.host.ice = here;
        if here >= NOTICE && !self.told {
            self.told = true;
            log::info!("black ice: the bus is on ice ({here:.2}) at ({x:.0}, {y:.0}){}", if bridge { " on a bridge" } else { "" });
            self.notice = Some("Black ice: the road is frozen here - the tyres hardly grip".into());
        }
    }
}
