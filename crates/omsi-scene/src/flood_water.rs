//! The water standing in the flooded low ground (K-OMSI, `omsi_sim::flood`): a flat sheet
//! over each basin's cells - the whole hollow up to its rim - drawn with the map's own water
//! and moved up and down with the basin's level. Where the ground stands above the water
//! the ground hides the sheet, so the shore follows the hollow by itself.

use crate::World;
use glam::{DVec3, Mat4, Vec2, Vec3};
use omsi_geometry::MeshData;
use omsi_render::{MeshId, Renderer, Scene};
use omsi_sim::flood::Flood;

/// Water shallower than this is not drawn: the road's own puddles show it (m).
const SHOWN_FROM: f32 = 0.03;
/// The level moves the sheet only by this much or more (m): a few millimetres a frame
/// would upload the instance every frame.
const STEP: f32 = 0.005;

#[derive(Default)]
pub struct FloodWater {
    /// The analysis the sheets were made for (`Flood::generation`).
    generation: u64,
    /// Per basin: its sheet, the sheet's corner and the level it stands at.
    sheets: Vec<Option<Sheet>>,
}

struct Sheet {
    mesh: MeshId,
    instance: usize,
    corner: DVec3,
    level: f32,
    shown: bool,
}

impl FloodWater {
    /// The sheets brought to `flood`'s levels (made anew for a new analysis).
    pub fn sync(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, flood: &Flood) {
        if flood.generation != self.generation {
            self.clear(renderer, scene);
            self.generation = flood.generation;
            self.sheets = (0..flood.basins.len()).map(|_| None).collect();
        }
        let Some(material) = world.gpu.lock().ground.as_ref().map(|g| g.water_mat) else { return };
        for k in 0..flood.basins.len() {
            let level = flood.level(k);
            let deep = flood.basins[k].depth(level) >= SHOWN_FROM;
            match &mut self.sheets[k] {
                None if deep => {
                    let (mesh_data, corner) = sheet_mesh(flood, k);
                    let mesh = renderer.add_mesh(scene, &mesh_data);
                    let corner = DVec3::new(corner.x, corner.y, 0.0);
                    let instance = renderer.add_instance(scene, mesh, corner + DVec3::Z * level as f64, Mat4::IDENTITY, vec![material]);
                    self.sheets[k] = Some(Sheet { mesh, instance, corner, level, shown: true });
                }
                None => {}
                Some(s) => {
                    if (level - s.level).abs() >= STEP {
                        s.level = level;
                        renderer.set_transform(scene, s.instance, s.corner + DVec3::Z * level as f64, Mat4::IDENTITY);
                    }
                    if s.shown != deep {
                        s.shown = deep;
                        scene.instances[s.instance].visible = deep;
                    }
                }
            }
        }
    }

    /// Every sheet taken away.
    pub fn clear(&mut self, renderer: &Renderer, scene: &mut Scene) {
        for s in self.sheets.drain(..).flatten() {
            renderer.remove_instance(scene, s.instance);
            renderer.free_mesh(scene, s.mesh);
        }
    }
}

/// The sheet of basin `k`: its cells as quads at height 0 (a row's neighbouring cells as
/// one), relative to the corner it returns (world x, y).
fn sheet_mesh(flood: &Flood, k: usize) -> (MeshData, glam::DVec2) {
    let b = &flood.basins[k];
    let w = flood.w.max(1);
    let half = flood.cell * 0.5;
    // (grown by a cell all round: the ground stands above the water there and hides it,
    // where cut off at the basin's own cells the shore showed the grid's steps)
    let mut cells: Vec<(usize, usize)> = b
        .cells
        .iter()
        .flat_map(|&i| {
            let (y, x) = (i as usize / w, i as usize % w);
            (y.saturating_sub(1)..=y + 1).flat_map(move |ny| (x.saturating_sub(1)..=(x + 1).min(w - 1)).map(move |nx| (ny, nx)))
        })
        .collect();
    cells.sort_unstable();
    cells.dedup();
    let corner = cells
        .iter()
        .map(|&(y, x)| flood.origin + glam::DVec2::new(x as f64, y as f64) * flood.cell - glam::DVec2::splat(half))
        .fold(glam::DVec2::splat(f64::MAX), |a, p| a.min(p));
    let mut m = MeshData::default();
    let mut quad = |y: usize, x0: usize, x1: usize| {
        let lo = flood.origin + glam::DVec2::new(x0 as f64, y as f64) * flood.cell - glam::DVec2::splat(half) - corner;
        let hi = lo + glam::DVec2::new((x1 - x0 + 1) as f64, 1.0) * flood.cell;
        let base = m.positions.len() as u32;
        for (x, y) in [(lo.x, lo.y), (hi.x, lo.y), (lo.x, hi.y), (hi.x, hi.y)] {
            m.positions.push(Vec3::new(x as f32, y as f32, 0.0));
            m.normals.push(Vec3::Z);
            m.uvs.push(Vec2::new(x as f32 / 40.0, y as f32 / 40.0));
        }
        // (counter-clockwise from above, as the tiles' water quad)
        m.indices.extend_from_slice(&[base, base + 1, base + 3, base, base + 3, base + 2]);
    };
    let mut run: Option<(usize, usize, usize)> = None;
    for &(y, x) in &cells {
        run = match run {
            Some((ry, x0, x1)) if ry == y && x1 + 1 == x => Some((ry, x0, x)),
            Some((ry, x0, x1)) => {
                quad(ry, x0, x1);
                Some((y, x, x))
            }
            None => Some((y, x, x)),
        };
    }
    if let Some((ry, x0, x1)) = run {
        quad(ry, x0, x1);
    }
    let n = m.indices.len() as u32;
    m.ranges.push((0, n, 0));
    (m, corner)
}
