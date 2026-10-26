//! Low ground flooding in heavy rain (K-OMSI, `OMSI_FLOOD`): the basins of the loaded ground
//! worked out on a thread of their own whenever the ground has changed (`omsi_sim::flood`),
//! and their water stepped every frame with the rain; the player's bus in it: its depth for the
//! body's drag (`VehicleHost::water_depth`), the tyres' spray, and an engine that drowns when
//! the water reaches its air intake - it does not start again in this run.

use omsi_sim::flood::{self, Drainage, Flood};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;

/// The grid's cell (m) and the most cells it may have (coarser beyond).
const CELL: f64 = 1.0;
const MAX_CELLS: usize = 6_000_000;
/// Basins shallower or smaller than this are puddles (m, cells).
const MIN_DEPTH: f32 = 0.05;
const MIN_CELLS: usize = 4;
/// The ground must have stood still this long before it is worked out again (s): tiles
/// come in one after another while the map loads and the bus drives on.
const SETTLE: f32 = 3.0;
/// How often the deepest basins are written to the log while water stands (s).
const LOG_EVERY: f32 = 30.0;
/// The height of a bus's engine air intake over the road (m): water above it drowns the engine.
const INTAKE: f32 = 1.0;
/// The water a tyre throws all its spray from (m deep).
const FULL_SPRAY: f32 = 0.1;
/// The scripts' flags of a running engine and its revolutions (`startup::engine_running`'s).
const ENGINE_FLAGS: [&str; 4] = ["engine_on", "engine_running", "motor_on", "motor_running"];
const ENGINE_REVS: [&str; 4] = ["engine_n", "engine_rpm", "motor_n", "motor_rpm"];

#[derive(Default)]
pub(crate) struct FloodState {
    pub(crate) flood: Flood,
    drainage: Option<Drainage>,
    /// The ground the basins were worked out from (and the one now being worked out).
    ground: Option<Arc<omsi_sim::world_data::GroundTiles>>,
    pending: Option<Receiver<(Flood, String)>>,
    /// Seconds since the ground was last seen to change, and since the last log.
    still: f32,
    log_t: f32,
    /// The bus whose engine the water drowned (its type, by address): it stays dead for
    /// this bus; another bus (a new run) starts clean.
    drowned: Option<usize>,
    /// The depth the bus stood in when it was last written to the log.
    logged_depth: f32,
    /// A message for the driver, once (`frame_ui` shows it).
    pub(crate) notice: Option<String>,
}

/// How much of its spray a tyre throws in water `depth` deep (0..1).
pub(crate) fn spray_of(depth: f32) -> f32 {
    (depth / FULL_SPRAY).clamp(0.0, 1.0)
}

/// The road's water at a point with the flood's added: standing flood water is a full, deep
/// puddle for the spray.
pub(crate) fn with_flood(mut w: omsi_scene::puddles::Water, flood: Option<&flood::WaterAt>, x: f64, y: f64) -> omsi_scene::puddles::Water {
    if let Some(f) = flood {
        let s = spray_of(f.depth(x, y));
        if s > 0.0 {
            w.puddle = w.puddle.max(s);
            w.depth = w.depth.max(s);
            w.wet = 1.0;
        }
    }
    w
}

/// `OMSI_FLOOD=scale[,drain mm/h]`: on, at this time scale (10 when not a number) and with
/// drains of this capacity (30 mm/h).
fn drainage() -> Option<Drainage> {
    let v = omsi_cfg::flags::OMSI_FLOOD.var()?;
    let mut d = Drainage::default();
    let mut it = v.split(',').map(|x| x.trim().parse::<f32>().ok());
    if let Some(Some(s)) = it.next() {
        d.time_scale = s.max(0.0);
    }
    if let Some(Some(m)) = it.next() {
        d.drain_mm_h = m.max(0.0);
    }
    Some(d)
}

impl FloodState {
    /// A frame of `dt` seconds under `weather` over `world`.
    pub(crate) fn tick(&mut self, world: Option<&omsi_scene::World>, weather: Option<&omsi_content::weather::Weather>, dt: f32) {
        if self.drainage.is_none() {
            self.drainage = drainage();
            if let Some(d) = self.drainage {
                log::info!("flood: on - time x{}, drains {} mm/h, bare ground soaks {} mm/h", d.time_scale, d.drain_mm_h, d.soak_mm_h);
            }
        }
        let (Some(d), Some(world)) = (self.drainage, world) else { return };
        self.take_analysis();
        let data = world.data();
        if self.ground.as_ref().is_some_and(|g| Arc::ptr_eq(g, &data.ground)) {
            self.still += dt;
        } else if self.pending.is_none() {
            self.still = 0.0;
            self.ground = Some(data.ground.clone());
        }
        if self.pending.is_none() && self.still >= SETTLE && self.still - dt < SETTLE {
            self.analyse(data);
        }
        let (kind, rate) = weather.map(omsi_weather::conditions::precip_of).unwrap_or((0, 0.0));
        let rain_mm_h = if kind == 1 { omsi_scene::rain::rain_rate_mm_h(rate) } else { 0.0 };
        self.flood.step(dt, rain_mm_h, &d);
        self.log_t += dt;
        if self.log_t >= LOG_EVERY {
            self.log_t = 0.0;
            self.log_levels(rain_mm_h);
        }
    }

    /// Where the water stands now, for the tyres' spray (None while flooding is off or dry).
    pub(crate) fn water_at(&self) -> Option<flood::WaterAt> {
        (self.drainage.is_some() && self.flood.water.iter().any(|w| *w > 0.0)).then(|| self.flood.water_at())
    }

    /// The player's bus in the water: the depth it stands in, and its engine drowned once the
    /// water reaches the air intake (held off every frame after: the scripts' running flags
    /// and revolutions at 0, so no starter brings it back).
    pub(crate) fn affect_player(&mut self, vehicle: Option<&mut omsi_sim::VehicleInstance>) {
        let Some(v) = vehicle else {
            self.drowned = None;
            return;
        };
        if self.drainage.is_none() {
            return;
        }
        let key = Arc::as_ptr(&v.ty) as usize;
        if self.drowned.is_some_and(|k| k != key) {
            self.drowned = None;
        }
        let depth = self.flood.depth_at(v.position.x, v.position.y, v.position.z);
        if (depth - self.logged_depth).abs() >= 0.1 || (depth > 0.0) != (self.logged_depth > 0.0) {
            self.logged_depth = depth;
            log::info!(
                "flood: the bus stands in {depth:.2} m of water at ({:.0}, {:.0}), its engine {} at {:.0} rpm",
                v.position.x, v.position.y,
                if omsi_sim::startup::engine_running(v) { "running" } else { "off" },
                omsi_sim::startup::engine_rpm(v).unwrap_or(0.0)
            );
        }
        v.host.water_depth = depth;
        if depth > INTAKE && self.drowned.is_none() {
            self.drowned = Some(key);
            log::warn!("flood: the bus stands {depth:.2} m deep at ({:.0}, {:.0}): water in the engine, it is dead for this run", v.position.x, v.position.y);
            self.notice = Some("The bus is flooded: water got into the engine, it will not start again".into());
        }
        if self.drowned == Some(key) {
            for name in ENGINE_FLAGS.iter().chain(ENGINE_REVS.iter()) {
                if v.var(name).is_some() {
                    v.set_var(name, 0.0);
                }
            }
        }
    }

    fn analyse(&mut self, data: Arc<omsi_sim::world_data::WorldData>) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending = Some(rx);
        let spawned = std::thread::Builder::new().name("flood".into()).spawn(move || {
            let t = std::time::Instant::now();
            let grid = flood::ground_grid(&data, CELL, MAX_CELLS);
            let basins = flood::find_basins(&grid, MIN_DEPTH, MIN_CELLS);
            let note = format!("{} x {} cells of {} m, {} ms", grid.w, grid.h, grid.cell, t.elapsed().as_millis());
            let _ = tx.send((Flood::of_grid(&grid, basins), note));
        });
        if let Err(e) = spawned {
            log::warn!("flood: the analysis could not start: {e}");
            self.pending = None;
        }
    }

    fn take_analysis(&mut self) {
        let Some(rx) = self.pending.as_ref() else { return };
        match rx.try_recv() {
            Ok((mut new, note)) => {
                self.pending = None;
                new.carry_over(&self.flood);
                new.generation = self.flood.generation + 1;
                let deep = new.basins.iter().filter(|b| b.spill - b.bottom >= 0.5).count();
                let catch: f64 = new.basins.iter().map(|b| b.paved_area + b.bare_area).sum();
                log::info!("flood: {} basins ({} at least 0.5 m deep, {:.0} m² draining into them) - {note}", new.basins.len(), deep, catch);
                let mut order: Vec<usize> = (0..new.basins.len()).collect();
                order.sort_by(|&a, &b| {
                    let (a, b) = (&new.basins[a], &new.basins[b]);
                    (b.spill - b.bottom).total_cmp(&(a.spill - a.bottom))
                });
                for &k in order.iter().take(10) {
                    let b = &new.basins[k];
                    let road = b.lowest_road.map(|(p, z)| format!(", lowest road {z:.2} m at ({:.0}, {:.0})", p.x, p.y)).unwrap_or_default();
                    log::info!(
                        "flood:   basin at ({:.0}, {:.0}): floor {:.2} m, spills at {:.2} m ({:.2} m deep), {:.0} m², holds {:.0} m³, catchment {:.0} m² road + {:.0} m² bare{road}",
                        b.centre.x, b.centre.y, b.bottom, b.spill, b.spill - b.bottom,
                        b.cells.len() as f64 * b.cell_area, b.capacity(), b.paved_area, b.bare_area
                    );
                }
                self.flood = new;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending = None,
        }
    }

    fn log_levels(&self, rain_mm_h: f32) {
        let mut wet: Vec<(usize, f32)> = (0..self.flood.basins.len())
            .filter(|&k| self.flood.water[k] > 0.0)
            .map(|k| (k, self.flood.basins[k].depth(self.flood.level(k))))
            .collect();
        if wet.is_empty() {
            return;
        }
        wet.sort_by(|a, b| b.1.total_cmp(&a.1));
        log::info!("flood: {:.1} mm/h of rain, water stands in {} basins", rain_mm_h, wet.len());
        for &(k, depth) in wet.iter().take(5) {
            let b = &self.flood.basins[k];
            let full = self.flood.water[k] / b.capacity().max(1e-9) * 100.0;
            log::info!("flood:   ({:.0}, {:.0}): {:.2} m deep, level {:.2} m, {:.0} m³ ({:.0}% full)", b.centre.x, b.centre.y, depth, self.flood.level(k), self.flood.water[k], full);
        }
    }
}
