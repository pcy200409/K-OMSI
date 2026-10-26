//! Low ground filling in heavy rain (K-OMSI): the hollows of the loaded ground - an
//! underpass, a dip in a road, a field below its banks - found once from a height grid, and
//! the water in each from the rain over what drains into it, less what the drains and the
//! soil take away.
//!
//! The hollows are worked out with Priority-Flood (Barnes, Lehman, Mulla 2014): filled from
//! the grid's edge upwards, a cell that comes out higher than it lies is in a hollow, and the
//! cells filled to one level make one basin. A basin keeps its cells' heights in order, which
//! is its level-volume curve, and its catchment: every cell whose water runs into it, down the
//! steepest slope (and over flats the way the filling reached them). The water balance is a
//! few sums per basin a frame; a basin that is full spills over its rim (the overflow is let
//! go, not passed on to the next basin).

use glam::DVec2;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A height grid of the ground (row-major, `w` x `h` cells of `cell` metres; cell (0, 0) is
/// centred on `origin`). A cell of no ground (`NaN`: a tile not loaded) is open: water there
/// runs away, as over the grid's edge.
#[derive(Clone, Debug, Default)]
pub struct HeightGrid {
    pub origin: DVec2,
    pub cell: f64,
    pub w: usize,
    pub h: usize,
    pub z: Vec<f32>,
    /// A road (paved, drained) covers the cell.
    pub paved: Vec<bool>,
}

impl HeightGrid {
    pub fn new(origin: DVec2, cell: f64, w: usize, h: usize) -> HeightGrid {
        HeightGrid { origin, cell, w, h, z: vec![f32::NAN; w * h], paved: vec![false; w * h] }
    }

    /// The world (x, y) of cell `i`'s centre.
    pub fn centre(&self, i: usize) -> DVec2 {
        self.origin + DVec2::new((i % self.w) as f64, (i / self.w) as f64) * self.cell
    }

    fn open(&self, i: usize) -> bool {
        !self.z[i].is_finite()
    }

    /// The (up to eight) neighbours of cell `i`.
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
        let (w, h) = (self.w as i64, self.h as i64);
        (-1..=1i64)
            .flat_map(move |dy| (-1..=1i64).map(move |dx| (x + dx, y + dy)))
            .filter(move |&(nx, ny)| (nx, ny) != (x, y) && nx >= 0 && ny >= 0 && nx < w && ny < h)
            .map(move |(nx, ny)| (ny * w + nx) as usize)
    }

    fn on_edge(&self, i: usize) -> bool {
        let (x, y) = (i % self.w, i / self.w);
        x == 0 || y == 0 || x + 1 == self.w || y + 1 == self.h
    }
}

/// One hollow of the ground and what drains into it.
#[derive(Clone, Debug)]
pub struct Basin {
    /// The lowest ground in it and the level it spills over at (m).
    pub bottom: f32,
    pub spill: f32,
    /// Its cells' heights, lowest first (the level-volume curve).
    pub floor: Vec<f32>,
    /// Its cells (grid indices), in the same order as `floor`.
    pub cells: Vec<u32>,
    /// The area draining into it, paved (roads) and bare (m², the basin's own cells included).
    pub paved_area: f64,
    pub bare_area: f64,
    /// The middle of its cells (world x, y), for the log.
    pub centre: DVec2,
    /// Its lowest road cell (world x, y) and that road's height, if a road runs through it.
    pub lowest_road: Option<(DVec2, f32)>,
    /// One cell's area (m²).
    pub cell_area: f64,
}

impl Basin {
    /// The water it holds up to `level` (m³).
    pub fn volume_at(&self, level: f32) -> f64 {
        self.floor.iter().take_while(|&&f| f < level).map(|&f| (level - f) as f64).sum::<f64>() * self.cell_area
    }

    /// The water it holds when full.
    pub fn capacity(&self) -> f64 {
        self.volume_at(self.spill)
    }

    /// The level `volume` m³ of water stands at (`bottom` when empty, at most `spill`).
    pub fn level_of(&self, volume: f64) -> f32 {
        if volume <= 0.0 || self.floor.is_empty() {
            return self.bottom;
        }
        // over the first k cells (lowest first) the water is level L where
        // sum (L - f_i) * a = volume, as long as L stays under cell k's floor
        let mut sum = 0.0f64;
        for (k, &f) in self.floor.iter().enumerate() {
            if k > 0 {
                let level = ((volume / self.cell_area + sum) / k as f64) as f32;
                if level <= f {
                    return level.min(self.spill);
                }
            }
            sum += f as f64;
        }
        (((volume / self.cell_area + sum) / self.floor.len() as f64) as f32).min(self.spill)
    }

    /// The deepest point's depth at `level`.
    pub fn depth(&self, level: f32) -> f32 {
        (level - self.bottom).max(0.0)
    }
}

#[derive(PartialEq)]
struct Cell(f32, usize);
impl Eq for Cell {}
impl Ord for Cell {
    fn cmp(&self, o: &Self) -> Ordering {
        // a min-heap on the height; ties by index, so the order is the same on every run
        o.0.total_cmp(&self.0).then_with(|| o.1.cmp(&self.1))
    }
}
impl PartialOrd for Cell {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The basins of `g` at least `min_depth` deep and `min_cells` cells large (the shallower
/// and smaller ones are puddles, which the road's own puddle picture shows).
pub fn find_basins(g: &HeightGrid, min_depth: f32, min_cells: usize) -> Vec<Basin> {
    let n = g.w * g.h;
    if n == 0 {
        return Vec::new();
    }
    const NONE: u32 = u32::MAX;
    // Priority-Flood: from the open cells and the edge inwards, lowest first; a cell reached
    // from a higher filled neighbour is filled to that height
    let mut fill = vec![f32::NAN; n];
    let mut parent = vec![NONE; n];
    let mut done = vec![false; n];
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        if g.open(i) {
            done[i] = true;
            fill[i] = f32::NEG_INFINITY;
        } else if g.on_edge(i) || g.neighbours(i).any(|j| g.open(j)) {
            done[i] = true;
            fill[i] = g.z[i];
            heap.push(Cell(g.z[i], i));
        }
    }
    while let Some(Cell(f, i)) = heap.pop() {
        for j in g.neighbours(i) {
            if done[j] {
                continue;
            }
            done[j] = true;
            fill[j] = g.z[j].max(f);
            parent[j] = i as u32;
            heap.push(Cell(fill[j], j));
        }
    }
    // the hollows: filled above their ground; joined into basins by their fill level
    let wet = |i: usize| !g.open(i) && fill[i] > g.z[i] + 1e-4;
    let mut basin_of = vec![NONE; n];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for s in 0..n {
        if !wet(s) || basin_of[s] != NONE {
            continue;
        }
        let id = groups.len() as u32;
        let mut cells = vec![s];
        basin_of[s] = id;
        let mut k = 0;
        while k < cells.len() {
            let i = cells[k];
            k += 1;
            for j in g.neighbours(i) {
                if basin_of[j] == NONE && wet(j) && (fill[j] - fill[i]).abs() < 1e-4 {
                    basin_of[j] = id;
                    cells.push(j);
                }
            }
        }
        groups.push(cells);
    }
    // where each cell's water goes: down the steepest slope, or over a flat the way the
    // filling came (towards the outlet); followed to the first basin on the way
    let next = |i: usize| -> u32 {
        let z = g.z[i];
        let mut best = (0.0f32, NONE);
        for j in g.neighbours(i) {
            if g.open(j) {
                return NONE;
            }
            let dist = if (j % g.w != i % g.w) && (j / g.w != i / g.w) { std::f32::consts::SQRT_2 } else { 1.0 };
            let drop = (z - g.z[j]) / dist;
            if drop > best.0 {
                best = (drop, j as u32);
            }
        }
        if best.1 != NONE { best.1 } else { parent[i] }
    };
    // 0..: the basin; NONE: away over the edge; PENDING while being followed
    const PENDING: u32 = NONE - 1;
    let mut sink = vec![NONE - 2; n];
    let mut path = Vec::new();
    for s in 0..n {
        if g.open(s) || sink[s] != NONE - 2 {
            continue;
        }
        let mut i = s;
        let to = loop {
            if basin_of[i] != NONE {
                break basin_of[i];
            }
            if sink[i] != NONE - 2 {
                break if sink[i] == PENDING { NONE } else { sink[i] };
            }
            sink[i] = PENDING;
            path.push(i);
            let j = next(i);
            if j == NONE || g.open(j as usize) {
                break NONE;
            }
            i = j as usize;
        };
        for p in path.drain(..) {
            sink[p] = to;
        }
    }
    let cell_area = g.cell * g.cell;
    let mut paved = vec![0usize; groups.len()];
    let mut bare = vec![0usize; groups.len()];
    for i in 0..n {
        let to = if basin_of[i] != NONE { basin_of[i] } else { sink[i] };
        if (to as usize) < groups.len() {
            if g.paved[i] { paved[to as usize] += 1 } else { bare[to as usize] += 1 }
        }
    }
    let mut basins = Vec::new();
    for (b, cells) in groups.into_iter().enumerate() {
        let spill = fill[cells[0]];
        let mut order: Vec<(f32, u32)> = cells.iter().map(|&i| (g.z[i], i as u32)).collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let bottom = order[0].0;
        if spill - bottom < min_depth || order.len() < min_cells {
            continue;
        }
        let centre = cells.iter().map(|&i| g.centre(i)).sum::<DVec2>() / cells.len() as f64;
        let lowest_road = order.iter().find(|o| g.paved[o.1 as usize]).map(|o| (g.centre(o.1 as usize), o.0));
        basins.push(Basin {
            bottom,
            spill,
            floor: order.iter().map(|o| o.0).collect(),
            cells: order.iter().map(|o| o.1).collect(),
            paved_area: paved[b] as f64 * cell_area,
            bare_area: bare[b] as f64 * cell_area,
            centre,
            lowest_road,
            cell_area,
        });
    }
    basins
}

/// How the water comes and goes.
#[derive(Clone, Copy, Debug)]
pub struct Drainage {
    /// What the drains of the paved ground carry away (mm/h of rain on it).
    pub drain_mm_h: f32,
    /// What bare ground soaks up (mm/h of rain on it) - only the rest runs off.
    pub soak_mm_h: f32,
    /// The share of the rain on a road and on bare ground that runs off.
    pub runoff_paved: f32,
    pub runoff_bare: f32,
    /// Game seconds per real second of the water's coming and going.
    pub time_scale: f32,
}

impl Default for Drainage {
    fn default() -> Drainage {
        Drainage { drain_mm_h: 30.0, soak_mm_h: 10.0, runoff_paved: 0.9, runoff_bare: 0.3, time_scale: 10.0 }
    }
}

/// The water's change in a basin (m³/s) under `rain_mm_h` of rain.
pub fn net_inflow(b: &Basin, rain_mm_h: f32, d: &Drainage) -> f64 {
    let mm_h = |v: f32| v.max(0.0) as f64 / 1000.0 / 3600.0;
    let paved = mm_h(rain_mm_h * d.runoff_paved - d.drain_mm_h) * b.paved_area;
    // (with no rain the drains still take what stands in the basin, at the same rate)
    let drains = if rain_mm_h * d.runoff_paved < d.drain_mm_h { mm_h(d.drain_mm_h - rain_mm_h * d.runoff_paved) * b.paved_area } else { 0.0 };
    let bare = mm_h(rain_mm_h * d.runoff_bare - d.soak_mm_h) * b.bare_area;
    let soaks = if rain_mm_h * d.runoff_bare < d.soak_mm_h { mm_h(d.soak_mm_h - rain_mm_h * d.runoff_bare) * b.bare_area } else { 0.0 };
    paved + bare - drains - soaks
}

/// The basins of the loaded ground with the water in each.
#[derive(Clone, Debug, Default)]
pub struct Flood {
    pub basins: Vec<Basin>,
    /// The water in each basin (m³).
    pub water: Vec<f64>,
    /// The grid the basins' cells count in (its cell (0, 0)'s centre, cell size, width).
    pub origin: DVec2,
    pub cell: f64,
    pub w: usize,
    /// Which analysis this is (a new one, a new set of basins: their water drawn anew).
    pub generation: u64,
    /// The basin each basin cell belongs to and the cell's floor (grid index -> both).
    pub cell_basin: std::sync::Arc<std::collections::HashMap<u32, (u32, f32)>>,
}

/// Where the water stands, as a snapshot for another thread (the tyres' spray): the basins'
/// cells and their levels now.
#[derive(Clone)]
pub struct WaterAt {
    cells: std::sync::Arc<std::collections::HashMap<u32, (u32, f32)>>,
    levels: Vec<f32>,
    origin: DVec2,
    cell: f64,
    w: usize,
}

impl WaterAt {
    /// The water over the ground at world (x, y) (m).
    pub fn depth(&self, x: f64, y: f64) -> f32 {
        if self.cell <= 0.0 {
            return 0.0;
        }
        let gx = ((x - self.origin.x) / self.cell).round();
        let gy = ((y - self.origin.y) / self.cell).round();
        if gx < 0.0 || gy < 0.0 || gx >= self.w as f64 {
            return 0.0;
        }
        match self.cells.get(&((gy as usize * self.w + gx as usize) as u32)) {
            Some(&(k, floor)) => (self.levels[k as usize] - floor).max(0.0),
            None => 0.0,
        }
    }
}

impl Flood {
    pub fn new(basins: Vec<Basin>) -> Flood {
        let water = vec![0.0; basins.len()];
        Flood { basins, water, ..Default::default() }
    }

    /// The basins of `grid`.
    pub fn of_grid(grid: &HeightGrid, basins: Vec<Basin>) -> Flood {
        let cell_basin = basins
            .iter()
            .enumerate()
            .flat_map(|(k, b)| b.cells.iter().zip(&b.floor).map(move |(&c, &f)| (c, (k as u32, f))))
            .collect();
        Flood { origin: grid.origin, cell: grid.cell, w: grid.w, cell_basin: std::sync::Arc::new(cell_basin), ..Flood::new(basins) }
    }

    /// Where the water stands now, for another thread.
    pub fn water_at(&self) -> WaterAt {
        WaterAt {
            cells: self.cell_basin.clone(),
            levels: (0..self.basins.len()).map(|k| if self.water[k] > 0.0 { self.level(k) } else { f32::MIN }).collect(),
            origin: self.origin,
            cell: self.cell,
            w: self.w,
        }
    }

    /// The water standing over `ground` (m) at world (x, y): the deepest of the basins of the
    /// cell and its neighbours there (the water's sheet reaches a cell beyond its basin).
    pub fn depth_at(&self, x: f64, y: f64, ground: f64) -> f32 {
        if self.cell_basin.is_empty() || self.cell <= 0.0 {
            return 0.0;
        }
        let gx = ((x - self.origin.x) / self.cell).round() as i64;
        let gy = ((y - self.origin.y) / self.cell).round() as i64;
        let mut level = f32::MIN;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (cx, cy) = (gx + dx, gy + dy);
                if cx < 0 || cy < 0 || cx >= self.w as i64 {
                    continue;
                }
                if let Some(&(k, _)) = self.cell_basin.get(&((cy * self.w as i64 + cx) as u32)) {
                    if self.water[k as usize] > 0.0 {
                        level = level.max(self.level(k as usize));
                    }
                }
            }
        }
        (level as f64 - ground).max(0.0) as f32
    }

    /// `dt` real seconds of `rain_mm_h` of rain (0 for none, and for snow).
    pub fn step(&mut self, dt: f32, rain_mm_h: f32, d: &Drainage) {
        let secs = (dt * d.time_scale) as f64;
        for (b, w) in self.basins.iter().zip(self.water.iter_mut()) {
            *w = (*w + net_inflow(b, rain_mm_h, d) * secs).clamp(0.0, b.capacity());
        }
    }

    /// The level of basin `k` now.
    pub fn level(&self, k: usize) -> f32 {
        self.basins[k].level_of(self.water[k])
    }

    /// Water from a new analysis: a basin of the new set keeps the level of the old basin
    /// over its deepest cell (tiles coming in or going redo the analysis while it rains).
    pub fn carry_over(&mut self, old: &Flood) {
        for (k, b) in self.basins.iter().enumerate() {
            let c = b.centre;
            let near = old.basins.iter().enumerate().find(|(_, o)| (o.centre - c).length() < 1.0 && (o.bottom - b.bottom).abs() < 0.01);
            if let Some((j, _)) = near {
                self.water[k] = b.volume_at(old.level(j)).min(b.capacity());
            }
        }
    }
}

/// A road this far over the terrain is a bridge or a flyover (m).
const ELEVATED: f32 = 3.0;

/// The height grid of the loaded ground (`cell` metres; coarser when the loaded tiles would
/// make more than `max_cells`): the road the wheels stand on where there is one - an
/// underpass, not the bridge over it - else the terrain. The cells of tiles not loaded are open.
pub fn ground_grid(world: &crate::world_data::WorldData, cell: f64, max_cells: usize) -> HeightGrid {
    let keys: Vec<(i32, i32)> = world.ground.terrains.keys().copied().collect();
    let Some(&first) = keys.first() else { return HeightGrid::default() };
    let (mut lo, mut hi) = (first, first);
    for &(x, y) in &keys {
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    let ts = omsi_map::tile_size();
    let (span_x, span_y) = ((hi.0 - lo.0 + 1) as f64 * ts, (hi.1 - lo.1 + 1) as f64 * ts);
    let mut cell = cell;
    while (span_x / cell).ceil() * (span_y / cell).ceil() > max_cells as f64 {
        cell *= 2.0;
    }
    let (w, h) = ((span_x / cell).ceil() as usize, (span_y / cell).ceil() as usize);
    let origin = DVec2::new(lo.0 as f64 * ts, lo.1 as f64 * ts) + DVec2::splat(cell * 0.5);
    let mut g = HeightGrid::new(origin, cell, w, h);
    let tsf = ts as f32;
    for i in 0..w * h {
        let p = g.centre(i);
        let key = crate::world_data::tile_key(p.x, p.y);
        let Some(terrain) = world.ground.terrains.get(&key) else { continue };
        let (lx, ly) = ((p.x - key.0 as f64 * ts) as f32, (p.y - key.1 as f64 * ts) as f32);
        // the road the wheels stand on, else the ground; not a bridge deck or an elevated
        // road over it (a cell under a flyover stood as a pit 20 m deep between its decks)
        let ground = terrain.sample(lx, ly);
        // (an elevated road, more than ELEVATED over the ground, is a deck too: the water
        // runs on under it - between the ramps of a flyover the ground stood as a pit)
        let road = world.ground.surfaces.get(&key).and_then(|s| s.sample_road(lx, ly)).filter(|r| *r < ground + ELEVATED);
        // the map's own water (a river, the sea): the water runs off into it
        if road.is_none() {
            if let Some(c) = world.ground.waters.get(&key) {
                let (u, v) = (lx / tsf, ly / tsf);
                let level = (c[0] * (1.0 - u) + c[1] * u) * (1.0 - v) + (c[2] * (1.0 - u) + c[3] * u) * v;
                if ground < level - 0.2 {
                    continue;
                }
            }
        }
        g.z[i] = road.unwrap_or(ground);
        g.paved[i] = road.is_some();
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat 20 x 20 grid at height 1 with a bowl of `depth` at its middle (5 x 5 cells).
    fn bowl(depth: f32, paved: bool) -> HeightGrid {
        let mut g = HeightGrid::new(DVec2::ZERO, 1.0, 20, 20);
        for y in 0..20 {
            for x in 0..20 {
                let i = y * 20 + x;
                g.z[i] = 1.0;
                g.paved[i] = paved;
                if (8..13).contains(&x) && (8..13).contains(&y) {
                    g.z[i] = 1.0 - depth;
                }
            }
        }
        g
    }

    #[test]
    fn a_bowl_is_one_basin_up_to_its_rim() {
        let b = find_basins(&bowl(0.5, true), 0.05, 1);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].cells.len(), 25);
        assert!((b[0].spill - 1.0).abs() < 1e-6 && (b[0].bottom - 0.5).abs() < 1e-6);
        assert!((b[0].capacity() - 25.0 * 0.5).abs() < 1e-3);
        // the whole flat drains into it: nothing runs off it to the edge but the edge itself
        assert!(b[0].paved_area > 25.0 && b[0].paved_area <= 400.0);
    }

    #[test]
    fn a_slope_has_no_basin_and_a_shallow_dip_is_a_puddle() {
        let mut g = HeightGrid::new(DVec2::ZERO, 1.0, 10, 10);
        for i in 0..100 {
            g.z[i] = (i % 10) as f32 * 0.1;
        }
        assert!(find_basins(&g, 0.05, 1).is_empty());
        assert!(find_basins(&bowl(0.02, true), 0.05, 1).is_empty());
    }

    #[test]
    fn an_unloaded_neighbour_lets_the_water_out() {
        let mut g = bowl(0.5, true);
        // a tile not loaded next to the bowl: an open cell beside it
        g.z[10 * 20 + 13] = f32::NAN;
        assert!(find_basins(&g, 0.05, 1).is_empty());
    }

    #[test]
    fn level_and_volume_agree() {
        let b = &find_basins(&bowl(0.5, true), 0.05, 1)[0];
        for v in [0.0, 1.0, 5.0, 12.0] {
            let l = b.level_of(v);
            assert!((b.volume_at(l) - v).abs() < 1e-3, "{v} -> {l}");
        }
        assert_eq!(b.level_of(1e9), b.spill);
    }

    #[test]
    fn rain_beyond_the_drains_fills_and_the_drains_empty_it() {
        let d = Drainage::default();
        let mut f = Flood::new(find_basins(&bowl(0.5, true), 0.05, 1));
        // a shower the drains can take: nothing stands
        f.step(600.0, 20.0, &d);
        assert_eq!(f.water[0], 0.0);
        // a cloudburst: water rises
        f.step(60.0, 60.0, &d);
        let after = f.water[0];
        assert!(after > 0.0);
        assert!(f.level(0) > f.basins[0].bottom);
        // the rain stops: it drains away again
        f.step(60.0, 0.0, &d);
        assert!(f.water[0] < after);
        f.step(36000.0, 0.0, &d);
        assert_eq!(f.water[0], 0.0);
    }

    #[test]
    fn the_depth_at_a_point_is_the_level_over_its_ground() {
        let g = bowl(0.5, true);
        let mut f = Flood::of_grid(&g, find_basins(&g, 0.05, 1));
        assert_eq!(f.depth_at(10.0, 10.0, 0.5), 0.0);
        f.water[0] = f.basins[0].volume_at(0.8);
        assert!((f.depth_at(10.0, 10.0, 0.5) - 0.3).abs() < 1e-4);
        // far from the bowl: dry
        assert_eq!(f.depth_at(1.0, 1.0, 1.0), 0.0);
        // and for the spray: over the bowl's floor, not over its rim
        let at = f.water_at();
        assert!((at.depth(10.0, 10.0) - 0.3).abs() < 1e-4);
        assert_eq!(at.depth(1.0, 1.0), 0.0);
    }

    #[test]
    fn a_full_basin_spills() {
        let d = Drainage::default();
        let mut f = Flood::new(find_basins(&bowl(0.5, true), 0.05, 1));
        f.step(1e6, 60.0, &d);
        assert!((f.level(0) - f.basins[0].spill).abs() < 1e-4);
    }

    #[test]
    fn bare_ground_soaks_up_more_than_a_road_drains() {
        let d = Drainage::default();
        let road = find_basins(&bowl(0.5, true), 0.05, 1);
        let field = find_basins(&bowl(0.5, false), 0.05, 1);
        assert!(net_inflow(&road[0], 60.0, &d) > net_inflow(&field[0], 60.0, &d));
    }
}
