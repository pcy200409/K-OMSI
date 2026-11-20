//! Money on the cash desk, drawn: the coins a passenger pays with and the change the driver
//! hands out (which coins, and where each lies, is omsi-sim's `people::money`).

use crate::World;
use glam::{DVec3, Mat4, Vec3};
use omsi_geometry::mesh_from_o3d;
use omsi_render::{AlphaMode, MaterialId, MeshId, Renderer, Scene};
use omsi_sim::VehicleInstance;
use std::path::{Path, PathBuf};

pub use omsi_sim::people::Money;

struct DeskTicket {
    instance: usize,
    ticket_type: usize,
    local: Mat4,
    parent: Option<String>,
    layer: u32,
}

/// The mesh and materials of a coin or note, made the first time it is put down.
fn mesh(m: &mut Money, world: &World, renderer: &Renderer, scene: &mut Scene, coin: usize) -> Option<(MeshId, Vec<MaterialId>)> {
    if let Some(c) = m.meshes.get(&coin) {
        return Some(c.clone());
    }
    let c = m.currency.as_ref()?;
    let n = c.coins.len();
    let file = if coin < n { c.coins.get(coin)?.0.clone() } else { c.bills.get(coin - n)?.0.clone() };
    let o3d = omsi_o3d::load_mesh(&omsi_cfg::resolve_path(&m.dir, &file)).map_err(|e| log::warn!("{e}")).ok()?;
    let dirs = [m.dir.clone(), omsi_cfg::resolve_path(&world.root, "Texture")];
    let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
    let mats: Vec<MaterialId> = o3d
        .materials
        .iter()
        .map(|mat| {
            let tex = omsi_texture::find_texture(&mat.texture, &dirs_ref).and_then(|p| world.textures.get_gpu_fast(&p)).map(|(img, _)| renderer.add_texture_data(scene, &img));
            renderer.add_material(scene, tex, AlphaMode::Opaque, [1.0; 4], false)
        })
        .collect();
    let id = renderer.add_mesh(scene, &mesh_from_o3d(&o3d));
    m.meshes.insert(coin, (id, mats.clone()));
    Some((id, mats))
}

/// Put coins on a point of the cabin as Omsi.exe does (sub_7e7e08): each one flat at the
/// point's height, somewhere in the variation rectangle and turned at random about the
/// vertical. Nothing is stacked: the coins had been raised 3 mm per coin already lying
/// there, so a driver clicking change built an endless tower on the tray.
/// A point of `[ticket_sale_money_point_2]` / `[ticket_sale_change_point_2]` names the
/// mesh (`parent`, its `[mesh_ident]`) the money lies on: it moves with that mesh - the
/// change tray in a cash desk swinging with the driver's door (#1468).
#[allow(clippy::too_many_arguments)]
pub fn place(m: &mut Money, world: &World, renderer: &Renderer, scene: &mut Scene, coins: &[usize], point: Vec3, var: [f32; 2], change: bool, parent: Option<&str>) {
    for coin in coins {
        let Some((id, mats)) = mesh(m, world, renderer, scene, *coin) else { continue };
        let local = Money::coin_place(point, var, [m.rand_f(), m.rand_f(), m.rand_f()]);
        let inst = renderer.add_instance(scene, id, DVec3::ZERO, Mat4::IDENTITY, mats);
        m.placed.push((inst, local, *coin, change, parent.map(str::to_string)));
    }
}

/// Show the coins lying in the bus where they lie now; the ones taken away hidden.
pub fn sync(m: &mut Money, renderer: &Renderer, scene: &mut Scene, bus: &VehicleInstance) {
    for inst in m.hidden.drain(..) {
        renderer.set_params(scene, inst, &[], false, &[]);
    }
    let rot = bus.body_rotation();
    // lit like the bus's own meshes (`MeshProps::interior`, half the lamp) and its
    // passengers: the full lamp made them glow brighter than the desk they lie on
    let interior = bus.interior_light() * 0.5;
    for (inst, local, _, _, parent) in &m.placed {
        let on = parent.as_deref().map(|n| mesh_of(bus, n)).unwrap_or(Mat4::IDENTITY);
        renderer.set_transform(scene, *inst, bus.position, rot * on * *local);
        renderer.set_interior(scene, *inst, interior);
    }
}

/// The transform of the bus's mesh named `name` (its `[mesh_ident]`) in the bus frame.
fn mesh_of(bus: &VehicleInstance, name: &str) -> Mat4 {
    let defs = &bus.ty.model.meshes;
    bus.ty
        .meshes
        .iter()
        .rposition(|m| defs.get(m.def_index).and_then(|d| d.mesh_ident.as_deref()).is_some_and(|n| n.trim().eq_ignore_ascii_case(name.trim())))
        .and_then(|i| bus.mesh_transforms.get(i).copied())
        .unwrap_or(Mat4::IDENTITY)
}

/// The coin of the change lying on the tray OR coin/bill on the desk a ray from `origin` along `dir` points at
/// (its place in `Money::placed`), the nearest: within 2 cm of a coin's middle, as near
/// as a coin is wide. A click on it picks it up again, as in Omsi.exe (#1354, #2399: the
/// coins could not be clicked).
pub fn money_hit(
    m: &Money,
    origin: DVec3,
    dir: Vec3,
    bus: &VehicleInstance,
) -> Option<usize> {
    let rot = bus.body_rotation();
    let o = (origin - bus.position).as_vec3();
    let dir = dir.normalize_or_zero();

    m.placed
        .iter()
        .enumerate()
        .filter_map(|(k, (_, local, _, _, parent))| {
            let on = parent
                .as_deref()
                .map(|n| mesh_of(bus, n))
                .unwrap_or(Mat4::IDENTITY);

            let c = (rot * on * *local).transform_point3(Vec3::ZERO);
            let t = (c - o).dot(dir);

            (t > 0.0 && t < 3.0 && (o + dir * t).distance(c) < 0.02)
                .then_some((k, t))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|h| h.0)
}

/// The tear-off ticket blocks of a bus without a ticket printer: the ticket pack's
/// `Ticket_<n>_block.o3d` hung on the bus's `[new_attachment]` point n (the stock SD's
/// "ticket block attach points"), moving with the bus. A click on block n tears off a
/// ticket of type n - `GivenTicket` n, as a printer's script hands one over (#1413).
pub struct TicketBlocks {
    /// The bus type and the pack they were made for.
    pub made_for: (PathBuf, PathBuf),
    /// (instance, place in the bus frame, ticket type, the mesh for clicks)
    blocks: Vec<(usize, Mat4, usize, omsi_geometry::MeshData)>,
    /// Individual ticket meshes, by ticket type.
    single_meshes: Vec<(usize, MeshId, Vec<MaterialId>, omsi_geometry::MeshData)>,
    /// Tickets waiting to be placed on the desk.
    pending_tickets: Vec<usize>,
    /// Individual tickets currently on the desk.
    desk_tickets: Vec<DeskTicket>,
    /// Ticket instances waiting to be removed by a mouse click.
    pending_ticket_removals: Vec<usize>,
}

impl TicketBlocks {
    /// Allow photo shutter samples to move the blocks with the bus.
    pub fn instances(&self) -> impl Iterator<Item = usize> + '_ {
        self.blocks.iter().map(|b| b.0).chain(self.desk_tickets.iter().map(|t| t.instance))
    }

    pub fn new(world: &World, renderer: &Renderer, scene: &mut Scene, bus: &VehicleInstance, pack: &Path) -> TicketBlocks {
        let dir = pack.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut blocks = Vec::new();
        let mut single_meshes = Vec::new();
        for (n, a) in bus.ty.def.attachments.iter().enumerate() {
            let file = dir.join(format!("Ticket_{n}_block.o3d"));
            if !omsi_cfg::vfs::is_file(&file) {
                continue;
            }
            let Ok(m) = omsi_o3d::load_mesh(&file).map_err(|e| log::warn!("{e}")) else { continue };
            let dirs = [dir.clone(), omsi_cfg::resolve_path(&world.root, "Texture")];
            let dirs_ref: Vec<&Path> = dirs.iter().map(|p| p.as_path()).collect();
            let mats: Vec<MaterialId> = m
                .materials
                .iter()
                .map(|mat| {
                    let tex = omsi_texture::find_texture(&mat.texture, &dirs_ref).and_then(|p| world.textures.get_gpu_fast(&p)).map(|(img, _)| renderer.add_texture_data(scene, &img));
                    renderer.add_material(scene, tex, AlphaMode::Test, [1.0; 4], false)
                })
                .collect();
            let data = mesh_from_o3d(&m);
            let id = renderer.add_mesh(scene, &data);
            let inst = renderer.add_instance(scene, id, DVec3::ZERO, Mat4::IDENTITY, mats);
            let local = omsi_world::map_index::attachment_matrix(&omsi_scenery::sco::Attachment { ops: a.ops.clone() });
            blocks.push((inst, local, n, data));

            let single_file = [
                dir.join(format!("ticket_{n}_single.o3d")),
                dir.join(format!("Ticket_{n}_single.o3d")),
            ]
            .into_iter()
            .find(|file| omsi_cfg::vfs::is_file(file));

            if let Some(file) = single_file {
                if let Ok(single) = omsi_o3d::load_mesh(&file).map_err(|e| log::warn!("{e}"))
                {
                    let mats: Vec<MaterialId> = single
                        .materials
                        .iter()
                        .map(|mat| {
                            let tex = omsi_texture::find_texture(
                                &mat.texture,
                                &dirs_ref,
                            )
                            .and_then(|p| world.textures.get_gpu_fast(&p))
                            .map(|(img, _)| renderer.add_texture_data(scene, &img));

                            renderer.add_material(scene, tex, AlphaMode::Test, [1.0; 4], false)
                        })
                        .collect();

                    let data = mesh_from_o3d(&single);
                    let mesh = renderer.add_mesh(scene, &data);
                    single_meshes.push((n, mesh, mats, data));
                    log::info!("Loaded individual ticket model: {}", file.display());
                }
            }
        }
        if !blocks.is_empty() {
            log::info!("ticket blocks: {} of {} on the bus", blocks.len(), dir.display());
        }
        log::info!("Ticket blocks: {}, individual ticket models: {}", blocks.len(), single_meshes.len());
        TicketBlocks { made_for: (bus.ty.def.path.clone(), pack.to_path_buf()), blocks, single_meshes, pending_tickets: Vec::new(), desk_tickets: Vec::new(), pending_ticket_removals: Vec::new(), }
    }

    pub fn sync(&self, renderer: &Renderer, scene: &mut Scene, bus: &VehicleInstance) {
        let rot = bus.body_rotation();
        // (lit like the bus's own meshes: half the lamp, see `money::sync`)
        let interior = bus.interior_light() * 0.5;
        // Existing ticket blocks
        for (inst, local, _, _) in &self.blocks {
            renderer.set_transform(scene, *inst, bus.position, rot * *local);
            renderer.set_interior(scene, *inst, interior);
        }
        // Individual tickets on the desk
        for ticket in &self.desk_tickets {
            let on = ticket.parent.as_deref().map(|n| mesh_of(bus, n)).unwrap_or(Mat4::IDENTITY);

            renderer.set_transform(
                scene,
                ticket.instance,
                bus.position,
                rot * on * ticket.local,
            );
            renderer.set_interior(scene, ticket.instance, interior);
        }
    }

    /// Take the blocks out of the picture for good: the bus they hung in is gone, or another
    /// set replaces them. Left in the scene they stood where that bus last was - in the air
    /// over the road, with no bus round them (#2340, #2399).
    pub fn remove(&self, renderer: &Renderer, scene: &mut Scene) {
        for (inst, ..) in &self.blocks {
            renderer.remove_instance(scene, *inst);
        }
        for ticket in &self.desk_tickets {
            renderer.remove_instance(scene, ticket.instance);
        }
    }

    /// Where each block is drawn (its ticket type and the middle of its mesh, from the bus's
    /// position in world axes): for tests that aim the mouse at them.
    pub fn centres(&self, bus: &VehicleInstance) -> Vec<(usize, Vec3)> {
        let rot = bus.body_rotation();
        self.blocks
            .iter()
            .filter(|b| !b.3.positions.is_empty())
            .map(|(_, local, n, data)| {
                let c = data.positions.iter().copied().sum::<Vec3>() / data.positions.len() as f32;
                (*n, (rot * *local).transform_point3(c))
            })
            .collect()
    }

    /// The ticket type of the block a ray from `origin` along `dir` hits first.
    pub fn hit(&self, origin: DVec3, dir: Vec3, bus: &VehicleInstance) -> Option<usize> {
        let o = (origin - bus.position).as_vec3();
        let rot = bus.body_rotation();
        self.blocks.iter().filter_map(|(_, local, n, data)| omsi_geometry::ray_mesh(o, dir, data, &(rot * *local)).map(|t| (t, *n))).min_by(|a, b| a.0.total_cmp(&b.0)).map(|x| x.1)
    }

    /// Queue a ticket to be placed on the desk.
    pub fn request_ticket(&mut self, n: usize) {
        self.pending_tickets.push(n);
    }

    /// Take the next pending ticket request.
    pub fn take_ticket_request(&mut self) -> Option<usize> {
        if self.pending_tickets.is_empty() {
            return None;
        }

        Some(self.pending_tickets.remove(0))
    }

    /// Place an individual ticket on the desk.
    /// Existing tickets are kept.
    pub fn place_ticket(&mut self, n: usize, renderer: &Renderer, scene: &mut Scene, point: Vec3, var: [f32; 2], parent: Option<&str>, r: [f32; 3]) -> bool {
        let Some((_, mesh, mats, _)) = self.single_meshes.iter().find(|(ticket, _, _, _)| *ticket == n)
        else {
            log::warn!("No individual ticket model found for ticket type {n}");
            return false;
        };

        // Give each ticket a slightly different height to prevent z-fighting.
        let layer = self.desk_tickets.iter().map(|ticket| ticket.layer).max().map_or(0, |max| max.saturating_add(1));
        let layered_point = point + Vec3::new(0.0, 0.0, layer as f32 * 0.001);
        let local = Money::coin_place(layered_point, var, r);
        let instance = renderer.add_instance(scene, *mesh, DVec3::ZERO, Mat4::IDENTITY, mats.clone());
        self.desk_tickets.push(DeskTicket { instance, ticket_type: n, local, parent: parent.map(str::to_owned), layer });

        true
    }

    /// Remove one ticket of the requested type when the passenger takes it.
    pub fn take_ticket( &mut self, ticket_type: usize, renderer: &Renderer, scene: &mut Scene) -> bool {
        take_of_type(&mut self.desk_tickets, ticket_type).map(|instance| renderer.remove_instance(scene, instance)).is_some()
    }

    /// The individual ticket hit by a ray from the mouse.
    pub fn hit_desk_ticket(&self, origin: DVec3, dir: Vec3, bus: &VehicleInstance) -> Option<usize> {
        let o = (origin - bus.position).as_vec3();
        let rot = bus.body_rotation();

        self.desk_tickets
            .iter()
            .filter_map(|ticket| {
                let data = self.single_meshes.iter().find(|(n, _, _, _)| *n == ticket.ticket_type).map(|(_, _, _, data)| data)?;
                let on = ticket.parent.as_deref().map(|n| mesh_of(bus, n)).unwrap_or(Mat4::IDENTITY);
                let transform = rot * on * ticket.local;

                omsi_geometry::ray_mesh(o, dir, data, &transform).map(|distance| (distance, ticket.instance))
            }).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, instance)| instance)
    }

    pub fn request_remove_ticket(&mut self, instance: usize) {
        self.pending_ticket_removals.push(instance);
    }

    pub fn take_ticket_removal_request(&mut self) -> Option<usize> {
        if self.pending_ticket_removals.is_empty() {
            return None;
        }

        Some(self.pending_ticket_removals.remove(0))
    }

    pub fn remove_ticket_instance(&mut self, instance: usize, renderer: &Renderer, scene: &mut Scene) -> bool {
        take_instance(&mut self.desk_tickets, instance).map(|instance| renderer.remove_instance(scene, instance)).is_some()
    }
}

/// K-OMSI: the paper tickets passengers laid on the cash desk (`TICKET_HAND`), until the
/// driver takes them. Any bus with a money point: a card of paper made here, printed with
/// the paper the plugin chose (`Texture/K-OMSI/paper_ticket/<paper>.png` of the game
/// folder) or the default one (`paper_ticket.png` there, or in `Texture/K-OMSI`), plain
/// white without any, so no bus or ticket pack has to bring a model.
///
/// A ticket with texts (`people.set_ticket{print = {...}}`) is drawn by a page: `<paper>.html`
/// there, else `ticket.html`, gets the texts as its string variables and draws them on a
/// transparent landscape page (as a ticket is read: the paper pictures are landscape tickets
/// stood upright, turned a quarter to the left); that page, turned the same way, goes over the
/// paper. Each such ticket has a texture of its own, used again once it is off the desk.
#[derive(Default)]
pub struct PaperTickets {
    made: Option<(MeshId, omsi_geometry::MeshData)>,
    /// The material of each paper ("" the default), made the first time it is laid down.
    papers: std::collections::HashMap<String, Vec<MaterialId>>,
    /// (whose, instance, place on the point, the mesh it lies on, its own printed texture)
    desk: Vec<(u32, usize, Mat4, Option<String>, Option<Printed>)>,
    /// Printed tickets whose page has not drawn yet.
    printing: Vec<Printing>,
    /// The textures of printed tickets taken off the desk, to print the next ones on.
    spare: Vec<Printed>,
}

/// A printed ticket's own texture and material.
#[derive(Clone, Copy)]
struct Printed {
    texture: omsi_render::TextureId,
    material: MaterialId,
    size: (u32, u32),
}

/// A printed ticket waiting for its page's picture.
struct Printing {
    person: u32,
    page: omsi_sim::htmltex::HtmlTexture,
    texture: omsi_render::TextureId,
    /// The paper under the print (upright, as the paper pictures are).
    base: omsi_texture::Image,
}

/// The paper ticket's size (m): a Korean bus ticket.
const PAPER_W: f32 = 0.075;
const PAPER_L: f32 = 0.13;
/// A ticket without a paper picture is printed on plain white of this size (7.5 x 13 cm).
const PLAIN: (u32, u32) = (512, 888);

impl PaperTickets {
    fn mesh(&mut self, renderer: &Renderer, scene: &mut Scene) -> MeshId {
        if let Some((id, _)) = &self.made {
            return *id;
        }
        let (w, l, z) = (PAPER_W / 2.0, PAPER_L / 2.0, 0.0015);
        let data = omsi_geometry::MeshData {
            positions: vec![Vec3::new(-w, -l, z), Vec3::new(w, -l, z), Vec3::new(w, l, z), Vec3::new(-w, l, z)],
            normals: vec![Vec3::Z; 4],
            uvs: vec![glam::Vec2::new(0.0, 1.0), glam::Vec2::new(1.0, 1.0), glam::Vec2::new(1.0, 0.0), glam::Vec2::new(0.0, 0.0)],
            ranges: vec![(0, 6, 0)],
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: false,
        };
        let id = renderer.add_mesh(scene, &data);
        self.made = Some((id, data));
        id
    }

    /// The picture file of `paper` (none or unknown: the default paper; nothing: plain).
    fn paper_file(world: &World, paper: Option<&str>) -> Option<PathBuf> {
        let key = paper.map(str::trim).unwrap_or("");
        let own = omsi_cfg::resolve_path(&world.root, "Texture/K-OMSI/paper_ticket");
        let komsi = omsi_cfg::resolve_path(&world.root, "Texture/K-OMSI");
        let mine = (!key.is_empty()).then(|| omsi_texture::find_texture(&format!("{key}.png"), &[own.as_path()])).flatten();
        mine.or_else(|| omsi_texture::find_texture("paper_ticket.png", &[own.as_path(), komsi.as_path()]))
    }

    /// The material of `paper` (none or unknown: the default paper).
    fn paper(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, paper: Option<&str>) -> Vec<MaterialId> {
        let key = paper.map(str::trim).unwrap_or("").to_string();
        if let Some(m) = self.papers.get(&key) {
            return m.clone();
        }
        let own = omsi_cfg::resolve_path(&world.root, "Texture/K-OMSI/paper_ticket");
        let komsi = omsi_cfg::resolve_path(&world.root, "Texture/K-OMSI");
        let found = if key.is_empty() {
            let dirs = [own.as_path(), komsi.as_path()];
            omsi_texture::find_texture("paper_ticket.png", &dirs)
        } else {
            omsi_texture::find_texture(&format!("{key}.png"), &[own.as_path()])
        };
        let mats = match found {
            Some(path) => {
                let tex = world.textures.get_gpu_fast(&path).map(|(img, _)| renderer.add_texture_data(scene, &img));
                vec![renderer.add_material(scene, tex, AlphaMode::Opaque, [1.0; 4], false)]
            }
            None if !key.is_empty() => {
                log::warn!("paper tickets: no Texture/K-OMSI/paper_ticket/{key}.png, the default paper");
                self.paper(world, renderer, scene, None)
            }
            None => {
                log::info!("paper tickets: no Texture/K-OMSI/paper_ticket/paper_ticket.png, drawn plain");
                vec![renderer.add_material(scene, None, AlphaMode::Opaque, [1.0; 4], false)]
            }
        };
        self.papers.insert(key, mats.clone());
        mats
    }

    /// `person`'s ticket on `paper` with `print` on it: its page set going (the paper alone
    /// shows until the page has drawn). None when there is no page for it.
    fn printed(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, person: u32, paper: Option<&str>, print: &[(String, String)]) -> Option<Printed> {
        let own = omsi_cfg::resolve_path(&world.root, "Texture/K-OMSI/paper_ticket");
        let key = paper.map(str::trim).unwrap_or("");
        let names = [(!key.is_empty()).then(|| format!("{key}.html")), Some("ticket.html".to_string())];
        let Some(path) = names.into_iter().flatten().map(|n| own.join(n)).find(|p| p.is_file()) else {
            log::warn!("paper tickets: no Texture/K-OMSI/paper_ticket/ticket.html, the ticket's texts are not printed");
            return None;
        };
        let html = match std::fs::read(&path) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                log::warn!("paper tickets: {}: {e}", path.display());
                return None;
            }
        };
        let base = Self::paper_file(world, paper)
            .and_then(|p| omsi_texture::decode_file(&p).map_err(|e| log::warn!("paper tickets: {}: {e}", p.display())).ok())
            .filter(|img| img.width > 0 && img.height > 0 && img.rgba.len() >= (img.width * img.height * 4) as usize)
            .unwrap_or_else(|| omsi_texture::Image { width: PLAIN.0, height: PLAIN.1, rgba: vec![255; (PLAIN.0 * PLAIN.1 * 4) as usize], has_alpha: false });
        let size = (base.width, base.height);
        // (the page is the ticket as it is read: lying, as wide as the paper is long)
        let mut page = omsi_sim::htmltex::HtmlTexture::with_api(0, size.1 as i32, size.0 as i32, &html, omsi_sim::htmltex::PageApi::Scenery).with_asset_dirs(vec![own.clone()]);
        page.set_texts(print, &omsi_sim::vehicle_api::ApiValue::Null);
        let data = omsi_texture::TextureData::from_image(base.clone());
        let p = match self.spare.iter().position(|s| s.size == size) {
            Some(k) => {
                let s = self.spare.remove(k);
                if renderer.update_texture_mips(scene, s.texture, &base) {
                    renderer.rebind_textures(scene, &[s.texture]);
                }
                s
            }
            None => {
                let texture = renderer.add_texture_data(scene, &data);
                let material = renderer.add_material(scene, Some(texture), AlphaMode::Opaque, [1.0; 4], false);
                Printed { texture, material, size }
            }
        };
        self.printing.retain(|w| w.person != person);
        self.printing.push(Printing { person, page, texture: p.texture, base });
        Some(p)
    }

    /// The pages that drew: their print over the paper, onto the ticket.
    fn print_frames(&mut self, renderer: &Renderer, scene: &mut Scene) {
        let mut k = 0;
        while k < self.printing.len() {
            let w = &mut self.printing[k];
            let Some(rgba) = w.page.poll() else {
                k += 1;
                continue;
            };
            let img = print_over(&w.base, &rgba, w.page.width, w.page.height);
            if renderer.update_texture_mips(scene, w.texture, &img) {
                renderer.rebind_textures(scene, &[w.texture]);
            }
            self.printing.remove(k);
        }
    }

    /// `person`'s ticket, on `paper` (with the texts `print`, see [`PaperTickets`]), laid on
    /// the money point (`r`: the coins' random place on it).
    #[allow(clippy::too_many_arguments)]
    pub fn place(&mut self, world: &World, renderer: &Renderer, scene: &mut Scene, person: u32, paper: Option<&str>, print: &[(String, String)], point: Vec3, var: [f32; 2], parent: Option<&str>, r: [f32; 3]) {
        self.remove(person, renderer, scene);
        let id = self.mesh(renderer, scene);
        let printed = if print.is_empty() { None } else { self.printed(world, renderer, scene, person, paper, print) };
        let mats = match printed {
            Some(p) => vec![p.material],
            None => self.paper(world, renderer, scene, paper),
        };
        // (a little above the coins, one over the other: no flicker between them)
        let local = Money::coin_place(point + Vec3::new(0.0, 0.0, 0.002 + self.desk.len() as f32 * 0.001), var, r);
        let inst = renderer.add_instance(scene, id, DVec3::ZERO, Mat4::IDENTITY, mats);
        self.desk.push((person, inst, local, parent.map(str::to_string), printed));
    }

    /// `person`'s ticket off the desk (the driver took it).
    pub fn remove(&mut self, person: u32, renderer: &Renderer, scene: &mut Scene) {
        let spare = &mut self.spare;
        self.desk.retain(|(p, inst, _, _, printed)| {
            if *p == person {
                renderer.remove_instance(scene, *inst);
                spare.extend(*printed);
            }
            *p != person
        });
        self.printing.retain(|w| w.person != person);
    }

    /// Every ticket out of the picture (the bus is gone or another is driven).
    pub fn clear(&mut self, renderer: &Renderer, scene: &mut Scene) {
        for (_, inst, _, _, printed) in self.desk.drain(..) {
            renderer.remove_instance(scene, inst);
            self.spare.extend(printed);
        }
        self.printing.clear();
    }

    pub fn instances(&self) -> impl Iterator<Item = usize> + '_ {
        self.desk.iter().map(|t| t.1)
    }

    /// The tickets where the bus is now (and the printed ones whose page drew, printed).
    pub fn sync(&mut self, renderer: &Renderer, scene: &mut Scene, bus: &VehicleInstance) {
        if !self.printing.is_empty() {
            self.print_frames(renderer, scene);
        }
        let rot = bus.body_rotation();
        for (_, inst, local, parent, _) in &self.desk {
            let on = parent.as_deref().map(|n| mesh_of(bus, n)).unwrap_or(Mat4::IDENTITY);
            renderer.set_transform(scene, *inst, bus.position, rot * on * *local);
        }
    }

    /// Whose ticket a ray from `origin` along `dir` hits first.
    pub fn hit(&self, origin: DVec3, dir: Vec3, bus: &VehicleInstance) -> Option<u32> {
        let (_, data) = self.made.as_ref()?;
        let o = (origin - bus.position).as_vec3();
        let rot = bus.body_rotation();
        self.desk
            .iter()
            .filter_map(|(p, _, local, parent, _)| {
                let on = parent.as_deref().map(|n| mesh_of(bus, n)).unwrap_or(Mat4::IDENTITY);
                omsi_geometry::ray_mesh(o, dir, data, &(rot * on * *local)).map(|t| (t, *p))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|h| h.1)
    }
}

/// The page's picture (`pw` x `ph`, the ticket lying) turned a quarter to the left - as the
/// paper pictures are stood upright - over `base` (`ph` x `pw`).
pub fn print_over(base: &omsi_texture::Image, page: &[u8], pw: u32, ph: u32) -> omsi_texture::Image {
    let (w, h) = (base.width, base.height);
    let mut out = base.rgba.clone();
    if (w, h) != (ph, pw) || page.len() < (pw * ph * 4) as usize {
        log::warn!("paper tickets: a page of {pw}x{ph} for a paper of {w}x{h}, not printed");
        return base.clone();
    }
    for y in 0..h {
        for x in 0..w {
            // (turned left: the page's top right corner comes to the top left)
            let (px, py) = (pw - 1 - y, x);
            let s = ((py * pw + px) * 4) as usize;
            let a = page[s + 3] as u32;
            if a == 0 {
                continue;
            }
            let d = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                out[d + c] = ((page[s + c] as u32 * a + out[d + c] as u32 * (255 - a)) / 255) as u8;
            }
            out[d + 3] = 255;
        }
    }
    omsi_texture::Image { width: w, height: h, rgba: out, has_alpha: false }
}

fn take_of_type(desk: &mut Vec<DeskTicket>, ticket_type: usize) -> Option<usize> {
    let index = desk.iter().position(|ticket| ticket.ticket_type == ticket_type)?;
    Some(desk.remove(index).instance)
}

fn take_instance(desk: &mut Vec<DeskTicket>, instance: usize) -> Option<usize> {
    let index = desk.iter().position(|ticket| ticket.instance == instance)?;
    Some(desk.remove(index).instance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desk(tickets: &[(usize, usize)]) -> Vec<DeskTicket> {
        tickets.iter().enumerate().map(|(layer, &(instance, ticket_type))| DeskTicket { instance, ticket_type, local: Mat4::IDENTITY, parent: None, layer: layer as u32 }).collect()
    }

    fn left(desk: &[DeskTicket]) -> Vec<(usize, usize)> {
        desk.iter().map(|t| (t.instance, t.ticket_type)).collect()
    }

    #[test]
    fn the_passenger_takes_the_accepted_type_and_leaves_a_later_one() {
        let mut d = desk(&[(10, 0), (11, 1)]);
        assert_eq!(take_of_type(&mut d, 0), Some(10));
        assert_eq!(left(&d), vec![(11, 1)]);
    }

    #[test]
    fn a_ticket_the_driver_took_back_is_not_taken_again_nor_swapped_for_another_type() {
        let mut d = desk(&[(10, 0), (11, 1)]);
        assert_eq!(take_instance(&mut d, 10), Some(10));
        assert_eq!(take_instance(&mut d, 10), None);
        assert_eq!(take_of_type(&mut d, 0), None);
        assert_eq!(left(&d), vec![(11, 1)]);

        let mut d = desk(&[(10, 0), (11, 1), (12, 0)]);
        assert_eq!(take_instance(&mut d, 10), Some(10));
        assert_eq!(take_of_type(&mut d, 0), Some(12));
        assert_eq!(left(&d), vec![(11, 1)]);
    }
}
