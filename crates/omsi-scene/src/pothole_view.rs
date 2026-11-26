//! The potholes near the camera drawn on the road (K-OMSI, `omsi_sim::potholes`): a dark,
//! ragged patch over each, made anew when the camera has moved on or the rain has broken
//! more of them out.

use crate::World;
use glam::{DVec2, DVec3, Mat4, Vec2, Vec3};
use omsi_geometry::MeshData;
use omsi_render::{AlphaMode, MaterialExtra, MaterialId, MeshId, Renderer, Scene};
use omsi_sim::potholes::Potholes;

/// The holes within this distance of the camera are drawn (m), made anew when it has moved
/// this far.
const REACH: f64 = 150.0;
const MOVE: f64 = 40.0;
/// Over the road by this much, so the road does not show through (m).
const LIFT: f64 = 0.012;

#[derive(Default)]
pub struct PotholeView {
    /// Where and at what density the patches were made, and the made ones.
    at: Option<(DVec2, f32)>,
    drawn: Option<(MeshId, usize)>,
    material: Option<MaterialId>,
}

impl PotholeView {
    pub fn sync(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, eye: DVec3, holes: Option<&Potholes>) {
        let Some(holes) = holes else {
            self.clear(renderer, scene);
            return;
        };
        let here = eye.truncate();
        if self.at.is_some_and(|(at, d)| (at - here).length() < MOVE && (d - holes.density).abs() < 0.0005) {
            return;
        }
        self.clear(renderer, scene);
        self.at = Some((here, holes.density));
        let material = *self.material.get_or_insert_with(|| {
            let img = omsi_texture::Image { width: 1, height: 1, rgba: vec![22, 21, 20, 215], has_alpha: true };
            let tex = renderer.add_texture(scene, &img, false);
            renderer.add_material_extra(scene, Some(tex), AlphaMode::Blend, [1.0; 4], false, None, None, None, None, [0.0; 3], MaterialExtra::default())
        });
        let mut m = MeshData::default();
        for h in holes.near(here, REACH) {
            let Some(z) = world.ground_height(h.centre.x, h.centre.y) else { continue };
            let c = h.centre - here;
            let base = m.positions.len() as u32;
            m.positions.push(Vec3::new(c.x as f32, c.y as f32, (z + LIFT) as f32));
            m.normals.push(Vec3::Z);
            m.uvs.push(Vec2::ZERO);
            // a ragged ring of ten points: no hole is round
            let rag = |k: usize| 0.8 + 0.4 * (((h.centre.x * 13.1 + h.centre.y * 7.3 + k as f64 * 3.7).sin() * 0.5 + 0.5) as f32);
            for k in 0..10 {
                let a = k as f32 / 10.0 * std::f32::consts::TAU;
                let r = h.radius * rag(k);
                m.positions.push(Vec3::new(c.x as f32 + r * a.cos(), c.y as f32 + r * a.sin(), (z + LIFT) as f32));
                m.normals.push(Vec3::Z);
                m.uvs.push(Vec2::ZERO);
            }
            for k in 0..10u32 {
                m.indices.extend_from_slice(&[base, base + 1 + k, base + 1 + (k + 1) % 10]);
            }
        }
        if m.indices.is_empty() {
            return;
        }
        let n = m.indices.len() as u32;
        m.ranges.push((0, n, 0));
        let mesh = renderer.add_mesh(scene, &m);
        let instance = renderer.add_instance(scene, mesh, DVec3::new(here.x, here.y, 0.0), Mat4::IDENTITY, vec![material]);
        self.drawn = Some((mesh, instance));
    }

    pub fn clear(&mut self, renderer: &Renderer, scene: &mut Scene) {
        self.at = None;
        if let Some((mesh, instance)) = self.drawn.take() {
            renderer.remove_instance(scene, instance);
            renderer.free_mesh(scene, mesh);
        }
    }
}
