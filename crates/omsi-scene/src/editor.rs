//! The object editor: a small part of what OMSI's map editor does, inside the game. The
//! scenery objects a tile places itself (its `[object]` records) can be picked, moved,
//! turned and deleted where they stand, and the tiles changed are written as copies into
//! the content folder - which the game reads before the installation - never into the
//! original map. New objects are new `[object]` records: any `.sco` chosen from the
//! object list (P), or a copy of the object picked (C), whose type can then be stepped
//! through the other objects of its folder (V). A record added (or an object's labels
//! changed) goes into the tile's text kept in memory ([`World::tile_texts`]) and the tile
//! is read again: the new object is then a map object as any other, drawn whole - its
//! text textures, `[htmltexture]` pages and scripts included - and picked, moved and
//! turned as the others. The ground is shaped with a brush where the view points
//! (raised, lowered, flattened); a tile's ground is written as its `.map.terrain` copy.
//! Splines are not part of it (the timetable is the launcher's Timetable page).
//!
//! Keys while it is on (Ctrl+Shift+E, or the game menu):
//! Enter picks the object nearest the middle of the view, Tab the next nearest;
//! I / K / J / L move it forward, back, left and right as the camera faces, U / O down and
//! up, N / M turn it (half a metre and five degrees a press, a tenth with Shift);
//! Delete takes it away (again: back), Backspace undoes all its edits, C copies it (the
//! copy is then the one edited), V gives an object placed in this session the next
//! object of its folder, P opens the object list, T the picked object's labels, Ctrl+S
//! saves the changed tiles and Escape leaves the editor. Page Up / Page Down raise and
//! lower the ground under the middle of the view (a quarter metre, a twentieth with
//! Shift), F flattens it to the height at the middle, [ and ] make the brush smaller and
//! larger.

use crate::{ObjectEdit, World};
use glam::{DVec3, Vec3};
use hashbrown::HashMap;
use std::path::{Path, PathBuf};

/// An object put on the map in this session: a record of its own in its tile's text.
#[derive(Clone, Debug)]
pub struct Placed {
    pub id: i64,
    pub tile: (i32, i32),
    /// Its `.sco` as the record names it (relative to the game's folder).
    pub file: String,
}

#[derive(Default)]
pub struct Editor {
    pub selected: Option<i64>,
    /// The objects placed in this session (new records), oldest first.
    pub placed: Vec<Placed>,
    /// The tile of every object edited, by map id (its tile may be unloaded when saving).
    tiles: HashMap<i64, (i32, i32)>,
    /// The candidates of the last pick, nearest first (Tab walks them).
    candidates: Vec<i64>,
    next: usize,
    /// The ground brush's radius (m; 0: the default).
    brush: f64,
}

/// The ground brush's radius when none is set, and its limits (m).
const BRUSH: (f64, f64, f64) = (6.0, 1.0, 60.0);

/// What a key does in the editor.
pub enum Action {
    Copy,
    Variant,
    Pick,
    NextPick,
    Move(DVec3),
    Turn(f64),
    Delete,
    Undo,
    Save,
    Leave,
    /// Raise (or lower) the ground under the view by metres.
    Ground(f64),
    Flatten,
    /// Make the brush larger (or smaller) by this factor.
    Brush(f64),
    /// The object list (a `.sco` to place).
    Objects,
    /// The picked object's labels.
    Labels,
}

impl Editor {
    /// The objects in front of the camera, those nearest the middle of the view first.
    pub fn pick(&mut self, world: &World, eye: DVec3, forward: Vec3) -> Option<i64> {
        let f = forward.as_dvec3().normalize_or_zero();
        let objects = world.edit_objects.lock();
        let edits = world.object_edits.lock();
        let mut scored: Vec<(f64, i64)> = objects
            .iter()
            .filter_map(|(id, o)| {
                let e = edits.get(id).copied().unwrap_or_default();
                if e.deleted {
                    return None;
                }
                let d = o.pos + e.moved - eye;
                let along = d.dot(f);
                if !(1.0..150.0).contains(&along) {
                    return None;
                }
                // off the view's axis, as an angle (a near post beside the middle before a
                // far house right in it)
                let off = (d - f * along).length() / along;
                (off < 0.35).then_some((off + along * 0.0015, *id))
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.candidates = scored.into_iter().map(|s| s.1).take(12).collect();
        self.next = 1;
        self.selected = self.candidates.first().copied();
        self.selected
    }

    pub fn next_pick(&mut self) -> Option<i64> {
        if self.candidates.is_empty() {
            return None;
        }
        let id = self.candidates[self.next % self.candidates.len()];
        self.next += 1;
        self.selected = Some(id);
        self.selected
    }

    /// The object picked put where the mouse drags it: its foot on the ground at `ground`,
    /// its turn kept.
    pub fn drag_to(&mut self, world: &World, renderer: &omsi_render::Renderer, scene: &mut omsi_render::Scene, ground: DVec3) -> Option<String> {
        let id = self.selected?;
        let (tile, pos) = world.edit_objects.lock().get(&id).map(|o| (o.tile, o.pos))?;
        let mut e = world.object_edits.lock().get(&id).copied().unwrap_or_default();
        e.moved = ground - pos;
        self.tiles.insert(id, tile);
        world.apply_object_edit(renderer, scene, id, e);
        Some(self.describe(world))
    }

    /// What the other players' games need to show the object being edited as it is now
    /// (`all`: every object edited or added this session): LAN commands, see
    /// `App::editor_broadcast`. An object placed in this session goes as `objadd` (the
    /// others' games have no record of it), where it stands now.
    pub fn sync_lines(&self, world: &World, _root: &Path, all: bool) -> Vec<String> {
        let mut out = Vec::new();
        let placed_ids: Vec<i64> = self.placed.iter().map(|p| p.id).collect();
        let edits = world.object_edits.lock();
        let line = |id: i64, e: &ObjectEdit| format!("objedit {id} {:.3} {:.3} {:.3} {:.2} {}", e.moved.x, e.moved.y, e.moved.z, e.turned, e.deleted as u8);
        if all {
            for (id, e) in edits.iter().filter(|(id, _)| !placed_ids.contains(id)) {
                out.push(line(*id, e));
            }
        } else if let Some(id) = self.selected.filter(|id| !placed_ids.contains(id)) {
            if let Some(e) = edits.get(&id) {
                out.push(line(id, e));
            }
        }
        let objects = world.edit_objects.lock();
        let added_line = |p: &Placed| {
            let o = objects.get(&p.id)?;
            let e = edits.get(&p.id).copied().unwrap_or_default();
            let f = o.xf.transform_vector3(Vec3::Y);
            let h = (f.x as f64).atan2(f.y as f64).to_degrees() + e.turned;
            let at = o.pos + e.moved;
            Some(format!("objadd {} {:.2} {:.2} {:.2} {:.1} {} {}", p.id, at.x, at.y, at.z, h, e.deleted as u8, p.file.replace('\\', "/")))
        };
        if all {
            out.extend(self.placed.iter().filter_map(added_line));
        } else if let Some(p) = self.selected.and_then(|id| self.placed.iter().find(|p| p.id == id)) {
            out.extend(added_line(p));
        }
        out
    }

    /// Change the picked object; returns what to say.
    pub fn apply(&mut self, world: &World, renderer: &omsi_render::Renderer, scene: &mut omsi_render::Scene, action: &Action) -> Option<String> {
        let id = self.selected?;
        let tile = world.edit_objects.lock().get(&id).map(|o| o.tile)?;
        let mut e = world.object_edits.lock().get(&id).copied().unwrap_or_default();
        match action {
            Action::Move(d) => e.moved += *d,
            Action::Turn(t) => e.turned += *t,
            Action::Delete => e.deleted = !e.deleted,
            Action::Undo => e = ObjectEdit::default(),
            _ => return None,
        }
        self.tiles.insert(id, tile);
        world.apply_object_edit(renderer, scene, id, e);
        Some(self.describe(world))
    }

    /// A new id: above every one the map has handed out (`[NextIDCode]` of global.cfg) and
    /// every one loaded or placed now.
    fn new_id(&self, world: &World) -> i64 {
        let loaded = world.edit_objects.lock().keys().copied().max().unwrap_or(0);
        let placed = self.placed.iter().map(|p| p.id).max().unwrap_or(0);
        let texts = world.tile_texts.lock().values().flat_map(|t| record_ids(t)).max().unwrap_or(0);
        world.global.next_id_code.max(loaded + 1).max(placed + 1).max(texts + 1)
    }

    /// Put `file` (a `.sco`, relative to the game's folder) on the map at `at` (its foot on
    /// the ground there), turned to `heading` (degrees clockwise from north). Returns the
    /// tile to read again.
    pub fn place(&mut self, world: &World, file: &str, at: DVec3, heading: f64) -> Result<(i32, i32), String> {
        let size = omsi_map::tile_size();
        let tile = ((at.x / size).floor() as i32, (at.y / size).floor() as i32);
        let mut text = tile_text(world, tile)?;
        let version = tile_version(&text);
        if version < 9 {
            return Err(format!("tile ({}, {}) is of an old version ({version}): objects are not added to it", tile.0, tile.1));
        }
        let id = self.new_id(world);
        let local = [at.x - tile.0 as f64 * size, at.y - tile.1 as f64 * size, 0.0];
        let record = new_record(version, file, id, local, heading, &[], eol(&text));
        if !text.ends_with('\n') && !text.is_empty() {
            text.push_str(eol(&text));
        }
        text.push_str(&record);
        world.tile_texts.lock().insert(tile, text);
        self.placed.push(Placed { id, tile, file: file.to_string() });
        self.tiles.insert(id, tile);
        self.selected = Some(id);
        Ok(tile)
    }

    /// A copy of the picked object two metres to its right (its labels, height over the
    /// ground and tilt with it), which is then the one picked. Returns the tile to read
    /// again.
    pub fn copy(&mut self, world: &World) -> Result<(i32, i32), String> {
        let id = self.selected.ok_or("Pick an object first (Enter)")?;
        let (tile, xf) = world.edit_objects.lock().get(&id).map(|o| (o.tile, o.xf)).ok_or("The object is not loaded")?;
        let e = world.object_edits.lock().get(&id).copied().unwrap_or_default();
        let f = xf.transform_vector3(Vec3::Y);
        let heading = (f.x as f64).atan2(f.y as f64).to_degrees() + e.turned;
        let right = DVec3::new(heading.to_radians().cos(), -heading.to_radians().sin(), 0.0);
        let text = tile_text(world, tile)?;
        let new_id = self.new_id(world);
        let (text, file) = copy_record(&text, id, new_id, e.moved + right * 2.0, e.turned).ok_or_else(|| format!("object {id} was not found in its tile"))?;
        world.tile_texts.lock().insert(tile, text);
        self.placed.push(Placed { id: new_id, tile, file });
        self.tiles.insert(new_id, tile);
        self.selected = Some(new_id);
        Ok(tile)
    }

    /// The object picked, if placed in this session, takes the next object of its folder.
    /// Returns the tile to read again.
    pub fn variant(&mut self, world: &World, root: &Path) -> Result<(i32, i32), String> {
        let id = self.selected.ok_or("Pick an object first (Enter)")?;
        let k = self.placed.iter().position(|p| p.id == id).ok_or("V changes only an object placed in this session (P or C)")?;
        let cur = self.placed[k].file.clone();
        let (dir_rel, name) = match cur.rfind(['\\', '/']) {
            Some(i) => (&cur[..i], &cur[i + 1..]),
            None => ("", cur.as_str()),
        };
        let names = sco_names(root, dir_rel);
        if names.is_empty() {
            return Err(format!("no objects found in {dir_rel}"));
        }
        let i = names.iter().position(|n| n.eq_ignore_ascii_case(name)).map(|i| (i + 1) % names.len()).unwrap_or(0);
        let file = if dir_rel.is_empty() { names[i].clone() } else { format!("{dir_rel}\\{}", names[i]) };
        let tile = self.placed[k].tile;
        let text = tile_text(world, tile)?;
        let text = set_record_file(&text, id, &file).ok_or_else(|| format!("object {id} was not found in its tile"))?;
        world.tile_texts.lock().insert(tile, text);
        self.placed[k].file = file;
        Ok(tile)
    }

    /// The labels of the object picked, as its record has them now (its id and tile too).
    pub fn labels(&self, world: &World) -> Option<(i64, (i32, i32), Vec<String>)> {
        let id = self.selected?;
        let tile = world.edit_objects.lock().get(&id).map(|o| o.tile).or_else(|| self.tiles.get(&id).copied())?;
        let text = tile_text(world, tile).ok()?;
        Some((id, tile, record_labels(&text, id)?))
    }

    /// Set label `k` of object `id` (lines up to it added empty); `None`: a new last line.
    /// Returns the tile to read again.
    pub fn set_label(&mut self, world: &World, id: i64, k: Option<usize>, value: &str) -> Result<(i32, i32), String> {
        let tile = world.edit_objects.lock().get(&id).map(|o| o.tile).or_else(|| self.tiles.get(&id).copied()).ok_or("The object is not loaded")?;
        let text = tile_text(world, tile)?;
        let mut labels = record_labels(&text, id).ok_or_else(|| format!("object {id} was not found in its tile"))?;
        let k = k.unwrap_or(labels.len());
        if labels.len() <= k {
            labels.resize(k + 1, String::new());
        }
        labels[k] = value.replace(['\r', '\n'], " ");
        let text = set_record_labels(&text, id, &labels).ok_or_else(|| format!("object {id} was not found in its tile"))?;
        world.tile_texts.lock().insert(tile, text);
        self.tiles.insert(id, tile);
        Ok(tile)
    }

    /// The object picked and what has been done to it.
    pub fn describe(&self, world: &World) -> String {
        let Some(id) = self.selected else { return "Object editor: Enter picks the object in the middle of the view".into() };
        let name = world
            .edit_objects
            .lock()
            .get(&id)
            .and_then(|o| o.sco.file_name().map(|n| n.to_string_lossy().to_string()));
        let Some(name) = name else {
            return match self.placed.iter().find(|p| p.id == id) {
                Some(p) => format!("New object {id} {}: loading...", p.file.rsplit(['\\', '/']).next().unwrap_or(&p.file)),
                None => format!("Object {id}: not loaded"),
            };
        };
        let new = self.placed.iter().any(|p| p.id == id);
        let e = world.object_edits.lock().get(&id).copied().unwrap_or_default();
        if e.deleted {
            format!("Object {id} {name}: deleted (Delete brings it back)")
        } else if new && e == ObjectEdit::default() {
            format!("New object {id} {name}: V for the next of its folder, T labels, C copies it")
        } else if e == ObjectEdit::default() {
            omsi_ui::i18n::said(omsi_ui::tr("Object {id} {name}").replace("{id}", &id.to_string()).replace("{name}", &name), format!("Object {id} {name}"))
        } else {
            format!("Object {id} {name}: moved {:+.2} / {:+.2} / {:+.2} m, turned {:+.1}°", e.moved.x, e.moved.y, e.moved.z, e.turned)
        }
    }

    /// Where the middle of the view meets the ground (within 400 m).
    pub fn aim(world: &World, eye: DVec3, forward: Vec3) -> Option<DVec3> {
        let f = forward.as_dvec3().normalize_or_zero();
        let mut t = 0.5;
        while t < 400.0 {
            let p = eye + f * t;
            if world.ground_terrain(p.x, p.y).is_some_and(|g| p.z <= g) {
                return Some(p);
            }
            t += if t < 50.0 { 0.25 } else { 1.0 };
        }
        None
    }

    /// The brush's radius now.
    pub fn brush(&self) -> f64 {
        if self.brush > 0.0 { self.brush } else { BRUSH.0 }
    }

    /// Shape the ground about `at` (raise by `Ground`'s metres, or flatten to the height at
    /// `at`); `Brush` resizes the brush. Returns what to say and the tiles to read again.
    pub fn ground(&mut self, world: &World, at: Option<DVec3>, action: &Action) -> (String, Vec<(i32, i32)>) {
        if let Action::Brush(f) = action {
            self.brush = (self.brush() * f).clamp(BRUSH.1, BRUSH.2);
            return (format!("Ground brush: {:.1} m", self.brush), Vec::new());
        }
        let Some(at) = at else { return ("Point the view at the ground".into(), Vec::new()) };
        let r = self.brush();
        let size = omsi_map::tile_size();
        let target = world.ground_terrain(at.x, at.y).unwrap_or(at.z);
        let mut changed = Vec::new();
        let mut edits = world.terrain_edits.lock();
        let (tx0, tx1) = (((at.x - r) / size).floor() as i32, ((at.x + r) / size).floor() as i32);
        let (ty0, ty1) = (((at.y - r) / size).floor() as i32, ((at.y + r) / size).floor() as i32);
        for tx in tx0..=tx1 {
            for ty in ty0..=ty1 {
                if !edits.contains_key(&(tx, ty)) {
                    let Some(src) = world.tile_source(tx, ty) else { continue };
                    let t = omsi_map::Terrain::load(&crate::tile_companion(&src, ".terrain")).unwrap_or_else(|_| omsi_map::Terrain::flat());
                    edits.insert((tx, ty), t);
                }
                let t = edits.get_mut(&(tx, ty)).unwrap();
                if shape(t, (tx as f64 * size, ty as f64 * size), at, r, action, target) {
                    changed.push((tx, ty));
                }
            }
        }
        let what = match action {
            Action::Ground(d) if *d > 0.0 => format!("Ground raised {:.2} m", d),
            Action::Ground(d) => format!("Ground lowered {:.2} m", -d),
            _ => format!("Ground flattened to {:.2} m", target),
        };
        (format!("{what} (brush {:.1} m, {} tile(s)) - Ctrl+S saves", r, changed.len()), changed)
    }

    /// Write every tile with edits as a copy under `content` (the map's own folder there),
    /// from the file the game reads it from (or its text as edited in this session).
    /// Returns the files written.
    pub fn save(&self, world: &World, map_rel: &str, content: &Path, original: &Path) -> Result<Vec<PathBuf>, String> {
        let edits = world.object_edits.lock().clone();
        let texts = world.tile_texts.lock().clone();
        let mut by_tile: HashMap<(i32, i32), HashMap<i64, ObjectEdit>> = HashMap::new();
        for (id, e) in edits {
            let Some(tile) = self.tiles.get(&id) else { continue };
            by_tile.entry(*tile).or_default().insert(id, e);
        }
        for tile in texts.keys() {
            by_tile.entry(*tile).or_default();
        }
        let map_dir = Path::new(map_rel).parent().unwrap_or(Path::new(""));
        let mut written = Vec::new();
        for ((tx, ty), edits) in by_tile {
            let src = world.tile_source(tx, ty).ok_or_else(|| omsi_ui::tr("tile ({tx}, {ty}) is not in the map").replace("{tx}", &tx.to_string()).replace("{ty}", &ty.to_string()))?;
            // (a LAN host's map is sealed: never written out in plain form, edited or not)
            if omsi_cfg::vfs::is_sealed(&src) {
                return Err(omsi_ui::tr("{file} is the LAN host's map: not written").replace("{file}", &src.display().to_string()));
            }
            let name = src.file_name().ok_or_else(|| omsi_ui::tr("tile without a name").into_owned())?.to_owned();
            let out = content.join(map_dir).join(&name);
            // never into the installation itself
            if let (Ok(o), Ok(r)) = (out.parent().map(|p| p.to_path_buf()).unwrap_or_default().canonicalize().or_else(|_| Ok::<_, std::io::Error>(out.clone())), original.canonicalize()) {
                if o.starts_with(&r) {
                    return Err(omsi_ui::tr("{file} lies in the original installation: not written").replace("{file}", &out.display().to_string()));
                }
            }
            // a tile whose text was edited here (objects added, labels changed) is written
            // as OMSI's editor writes tiles: UTF-16 (a label may hold any letter)
            let edited = texts.get(&(tx, ty));
            let (text, enc) = match edited {
                Some(t) => (t.clone(), Encoding::Utf16Le),
                None => {
                    let bytes = omsi_cfg::vfs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
                    decode(&bytes)
                }
            };
            let (new_text, n) = rewrite_tile(&text, &edits);
            if n == 0 && edited.is_none() {
                return Err(omsi_ui::tr("the objects edited were not found in {file}").replace("{file}", &src.display().to_string()));
            }
            std::fs::create_dir_all(out.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
            let data = encode(&new_text, enc);
            std::fs::write(&out, data).map_err(|e| format!("{}: {e}", out.display()))?;
            log::info!("object editor: {} objects of tile ({tx}, {ty}) changed, written to {}", n, out.display());
            written.push(out);
        }
        // the ground the brush shaped: each tile's .map.terrain
        let terrains = world.terrain_edits.lock().clone();
        for ((tx, ty), t) in terrains {
            let src = world.tile_source(tx, ty).ok_or_else(|| omsi_ui::tr("tile ({tx}, {ty}) is not in the map").replace("{tx}", &tx.to_string()).replace("{ty}", &ty.to_string()))?;
            // (a LAN host's map is sealed: never written out in plain form, edited or not)
            if omsi_cfg::vfs::is_sealed(&src) {
                return Err(omsi_ui::tr("{file} is the LAN host's map: not written").replace("{file}", &src.display().to_string()));
            }
            let name = format!("{}.terrain", src.file_name().ok_or_else(|| omsi_ui::tr("tile without a name").into_owned())?.to_string_lossy());
            let out = content.join(map_dir).join(&name);
            if let (Some(dir), Ok(r)) = (out.parent(), original.canonicalize()) {
                if dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()).starts_with(&r) {
                    return Err(omsi_ui::tr("{file} lies in the original installation: not written").replace("{file}", &out.display().to_string()));
                }
            }
            std::fs::create_dir_all(out.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
            std::fs::write(&out, t.to_bytes()).map_err(|e| format!("{}: {e}", out.display()))?;
            log::info!("map editor: the ground of tile ({tx}, {ty}) written to {}", out.display());
            written.push(out);
        }
        Ok(written)
    }
}

/// The `.sco` files of folder `dir_rel` (relative to the game's folder, `\` or `/`), from
/// the content folders and the game's folder, sorted, each name once.
pub fn sco_names(root: &Path, dir_rel: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for base in omsi_cfg::content_roots().iter().chain(std::iter::once(&root.to_path_buf())) {
        let dir = omsi_cfg::resolve_path(base, dir_rel);
        for (n, is_dir) in omsi_cfg::vfs::list_dir(&dir).unwrap_or_default() {
            let n = n.to_string_lossy().to_string();
            if !is_dir && n.to_ascii_lowercase().ends_with(".sco") && !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
    }
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// The folders in folder `dir_rel` (as [`sco_names`]).
pub fn sub_dirs(root: &Path, dir_rel: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for base in omsi_cfg::content_roots().iter().chain(std::iter::once(&root.to_path_buf())) {
        let dir = omsi_cfg::resolve_path(base, dir_rel);
        for (n, is_dir) in omsi_cfg::vfs::list_dir(&dir).unwrap_or_default() {
            let n = n.to_string_lossy().to_string();
            if is_dir && !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
    }
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// The tile's text as the game reads it now: edited in this session, or its file.
fn tile_text(world: &World, tile: (i32, i32)) -> Result<String, String> {
    if let Some(t) = world.tile_texts.lock().get(&tile) {
        return Ok(t.clone());
    }
    let src = world.tile_source(tile.0, tile.1).ok_or_else(|| format!("tile ({}, {}) is not in the map", tile.0, tile.1))?;
    if omsi_cfg::vfs::is_sealed(&src) {
        return Err(format!("{} is the LAN host's map: not edited", src.display()));
    }
    let bytes = omsi_cfg::vfs::read(&src).map_err(|e| format!("{}: {e}", src.display()))?;
    // (as the game reads it: a tile without a byte order mark in the map's code page)
    Ok(omsi_cfg::decode_text(&bytes))
}

/// The text of an edited tile as the game reads it in place of its file (UTF-16 with its
/// byte order mark, as OMSI's editor writes tiles).
pub fn tile_bytes(text: &str) -> Vec<u8> {
    encode(text, Encoding::Utf16Le)
}

/// The tile's `[version]` (0: none).
pub fn tile_version(text: &str) -> i32 {
    let mut lines = text.lines();
    while let Some(l) = lines.next() {
        if l.trim().eq_ignore_ascii_case("[version]") {
            return lines.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
        }
    }
    0
}

/// The line ending the tile uses.
fn eol(text: &str) -> &'static str {
    if text.contains("\r\n") || text.is_empty() { "\r\n" } else { "\n" }
}

/// Where the parts of an `[object]` record of a version-9-or-later tile stand, counted from
/// its keyword line: the file, the id, x (then y, z), the heading and the label count.
#[derive(Clone, Copy)]
struct Layout {
    file: usize,
    id: usize,
    x: usize,
    heading: usize,
    count: usize,
}

fn layout(version: i32) -> Layout {
    Layout { file: 2, id: 3, x: 4, heading: 7, count: if version >= 12 { 10 } else { 8 } }
}

/// The `[object]` record of `id`: its first line and one past its last (its labels
/// counted, not guessed: a label may look like a keyword).
fn find_record(lines: &[&str], id: i64, version: i32) -> Option<(usize, usize)> {
    let l = layout(version);
    let body = |s: &str| s.trim_end_matches(['\r', '\n']).trim().to_string();
    for i in 0..lines.len() {
        if !body(lines[i]).eq_ignore_ascii_case("[object]") {
            continue;
        }
        if lines.get(i + l.id).and_then(|s| body(s).parse::<i64>().ok()) != Some(id) {
            continue;
        }
        let n = lines.get(i + l.count).and_then(|s| body(s).parse::<usize>().ok()).unwrap_or(0);
        return Some((i, (i + l.count + 1 + n).min(lines.len())));
    }
    None
}

/// The ids of every `[object]` record (version 9 or later).
fn record_ids(text: &str) -> Vec<i64> {
    let lines: Vec<&str> = text.lines().collect();
    let l = layout(tile_version(text));
    (0..lines.len())
        .filter(|i| lines[*i].trim().eq_ignore_ascii_case("[object]"))
        .filter_map(|i| lines.get(i + l.id).and_then(|s| s.trim().parse().ok()))
        .collect()
}

/// A new `[object]` record (version 9 or later): no tilt, its foot `pos[2]` over the ground.
fn new_record(version: i32, file: &str, id: i64, pos: [f64; 3], heading: f64, labels: &[String], eol: &str) -> String {
    let mut lines = vec!["[object]".to_string(), "0".into(), file.to_string(), id.to_string(), num(pos[0]), num(pos[1]), num(pos[2]), num(heading)];
    if version >= 12 {
        lines.push("0".into());
        lines.push("0".into());
    }
    lines.push(labels.len().to_string());
    lines.extend(labels.iter().cloned());
    lines.push(String::new());
    let mut s = lines.join(eol);
    s.push_str(eol);
    s
}

/// The tile with a copy of `template`'s record right after it: id `new_id`, moved by
/// `moved` (x, y and the height over the ground) and turned by `turned`. Returns the text
/// and the copy's file.
pub fn copy_record(text: &str, template: i64, new_id: i64, moved: DVec3, turned: f64) -> Option<(String, String)> {
    let version = tile_version(text);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (start, end) = find_record(&lines, template, version)?;
    let l = layout(version);
    let body = |s: &str| s.trim_end_matches(['\r', '\n']).to_string();
    let e = eol(text);
    let mut copy = String::new();
    let mut file = String::new();
    for (k, line) in lines[start..end].iter().enumerate() {
        let b = body(line);
        let new = if k == l.id {
            new_id.to_string()
        } else if (l.x..l.x + 3).contains(&k) || k == l.heading {
            let d = if k == l.heading { turned } else { [moved.x, moved.y, moved.z][k - l.x] };
            match b.trim().parse::<f64>() {
                Ok(v) => num(v + d),
                Err(_) => b.clone(),
            }
        } else {
            if k == l.file {
                file = b.trim().to_string();
            }
            b.clone()
        };
        copy.push_str(&new);
        copy.push_str(e);
    }
    copy.push_str(e);
    let mut out = String::with_capacity(text.len() + copy.len());
    for line in &lines[..end] {
        out.push_str(line);
    }
    if !out.ends_with('\n') {
        out.push_str(e);
    }
    out.push_str(e);
    out.push_str(&copy);
    for line in &lines[end..] {
        out.push_str(line);
    }
    Some((out, file))
}

/// The tile with the file of `id`'s record changed.
pub fn set_record_file(text: &str, id: i64, file: &str) -> Option<String> {
    let version = tile_version(text);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (start, _) = find_record(&lines, id, version)?;
    let at = start + layout(version).file;
    let line = lines.get(at)?;
    let ending = &line[line.trim_end_matches(['\r', '\n']).len()..];
    let mut out = String::with_capacity(text.len());
    for (i, l) in lines.iter().enumerate() {
        if i == at {
            out.push_str(file);
            out.push_str(ending);
        } else {
            out.push_str(l);
        }
    }
    Some(out)
}

/// The labels of `id`'s record (all the lines its count says, empty ones too).
pub fn record_labels(text: &str, id: i64) -> Option<Vec<String>> {
    let version = tile_version(text);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (start, end) = find_record(&lines, id, version)?;
    let first = start + layout(version).count + 1;
    Some(lines[first.min(end)..end].iter().map(|l| l.trim_end_matches(['\r', '\n']).to_string()).collect())
}

/// The tile with `id`'s labels replaced by `labels` (the count with them).
pub fn set_record_labels(text: &str, id: i64, labels: &[String]) -> Option<String> {
    let version = tile_version(text);
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let (start, end) = find_record(&lines, id, version)?;
    let count = start + layout(version).count;
    if count >= lines.len() {
        return None;
    }
    let e = eol(text);
    let mut out = String::with_capacity(text.len() + 64);
    for l in &lines[..count] {
        out.push_str(l);
    }
    out.push_str(&labels.len().to_string());
    out.push_str(e);
    for l in labels {
        out.push_str(l);
        out.push_str(e);
    }
    // (a record that was the file's last line without an ending)
    for l in &lines[end..] {
        out.push_str(l);
    }
    Some(out)
}

/// Shape one tile's ground (its origin `o`) about `at` within `r`: raise by `Ground`'s
/// metres, or bring toward `target` (`Flatten`), weighted smoothly to nothing at the rim.
/// True when a point moved.
fn shape(t: &mut omsi_map::Terrain, o: (f64, f64), at: DVec3, r: f64, action: &Action, target: f64) -> bool {
    let n = t.samples();
    let step = omsi_map::tile_size() / t.cells as f64;
    let mut any = false;
    for iy in 0..n {
        for ix in 0..n {
            let (x, y) = (o.0 + ix as f64 * step, o.1 + iy as f64 * step);
            let d = ((x - at.x).powi(2) + (y - at.y).powi(2)).sqrt();
            if d >= r {
                continue;
            }
            let w = (1.0 - (d / r).powi(2)).powi(2);
            let h = &mut t.heights[iy * n + ix];
            let new = match action {
                Action::Ground(m) => *h as f64 + m * w,
                _ => *h as f64 + (target - *h as f64) * w.min(1.0),
            };
            if (new - *h as f64).abs() > 1e-5 {
                *h = new as f32;
                any = true;
            }
        }
    }
    any
}

/// How a tile file is written: OMSI's editor saves UTF-16 (little endian, with its byte
/// order mark); hand-made ones are ASCII or Latin-1.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Encoding {
    Utf8,
    Latin1,
    Utf16Le,
}

fn decode(bytes: &[u8]) -> (String, Encoding) {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return (String::from_utf16_lossy(&units), Encoding::Utf16Le);
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(t) => (t, Encoding::Utf8),
        Err(_) => (bytes.iter().map(|&b| b as char).collect(), Encoding::Latin1),
    }
}

fn encode(text: &str, enc: Encoding) -> Vec<u8> {
    match enc {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Latin1 => text.chars().map(|c| c as u32 as u8).collect(),
        Encoding::Utf16Le => [0xFF, 0xFE].into_iter().chain(text.encode_utf16().flat_map(|u| u.to_le_bytes())).collect(),
    }
}

/// A number as a tile file writes it.
fn num(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// The tile file with the edits applied to its `[object]` records (by map id): a moved or
/// turned object gets its position (x, y, height over the ground) and heading changed, a
/// deleted one loses its record. Everything else stays as it was, line endings included.
/// Returns the text and how many records changed.
pub fn rewrite_tile(text: &str, edits: &HashMap<i64, ObjectEdit>) -> (String, usize) {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let body = |l: &str| l.trim_end_matches(['\r', '\n']).to_string();
    let ending = |l: &str| l[l.trim_end_matches(['\r', '\n']).len()..].to_string();
    let version = tile_version(text);
    let mut out = String::with_capacity(text.len());
    let mut changed = 0;
    let mut i = 0;
    while i < lines.len() {
        let is_object = body(lines[i]).trim().eq_ignore_ascii_case("[object]");
        let id = lines.get(i + 3).and_then(|l| body(l).trim().parse::<i64>().ok());
        let edit = if is_object { id.and_then(|id| edits.get(&id)) } else { None };
        let Some(e) = edit else {
            out.push_str(lines[i]);
            i += 1;
            continue;
        };
        changed += 1;
        if e.deleted {
            // the record: its labels counted where the version says where the count is
            // (a label may look like a keyword), else up to the next keyword
            let counted = (version >= 9).then(|| find_record(&lines[i..], id.unwrap_or(0), version)).flatten().map(|(_, end)| i + end);
            match counted {
                Some(end) => i = end,
                None => {
                    i += 1;
                    while i < lines.len() && !body(lines[i]).trim_start().starts_with('[') {
                        i += 1;
                    }
                }
            }
            continue;
        }
        // [object], 0, file, id, x, y, z, heading, …
        for k in 0..4 {
            out.push_str(lines[i + k]);
        }
        let deltas = [e.moved.x, e.moved.y, e.moved.z, e.turned];
        for (k, d) in deltas.iter().enumerate() {
            let Some(l) = lines.get(i + 4 + k) else { break };
            match body(l).trim().parse::<f64>() {
                Ok(v) if *d != 0.0 => {
                    out.push_str(&num(v + d));
                    out.push_str(&ending(l));
                }
                _ => out.push_str(l),
            }
        }
        i += 8;
    }
    (out, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_brush_raises_the_middle_most_and_not_its_rim() {
        let mut t = omsi_map::Terrain::flat();
        let size = omsi_map::tile_size();
        let at = DVec3::new(size * 0.5, size * 0.5, 0.0);
        assert!(shape(&mut t, (0.0, 0.0), at, 12.0, &Action::Ground(1.0), 0.0));
        assert!((t.sample(at.x as f32, at.y as f32) - 1.0).abs() < 1e-4);
        assert_eq!(t.sample(at.x as f32 + 18.0, at.y as f32), 0.0);
        // flattened back to 0 where the weight is full
        shape(&mut t, (0.0, 0.0), at, 12.0, &Action::Flatten, 0.0);
        assert!(t.sample(at.x as f32, at.y as f32).abs() < 1e-4);
        assert_eq!(omsi_map::Terrain::parse(&t.to_bytes()).unwrap(), t);
    }

    const TILE: &str = "[version]\r\n4\r\n\r\n[object]\r\n0\r\nSceneryobjects\\a.sco\r\n7\r\n5\r\n6\r\n0.25\r\n90\r\n0\r\n0\r\n0\r\n\r\nObject Nr. 1\r\n[object]\r\n0\r\nSceneryobjects\\b.sco\r\n8\r\n1\r\n2\r\n0\r\n0\r\n0\r\n0\r\n2\r\nHalt\r\nx\r\n\r\n[spline]\r\n0\r\n";

    #[test]
    fn a_moved_object_changes_its_lines_and_a_deleted_one_goes() {
        let mut edits = HashMap::new();
        edits.insert(7, ObjectEdit { moved: DVec3::new(1.5, -1.0, 0.0), turned: 12.5, deleted: false });
        edits.insert(8, ObjectEdit { deleted: true, ..Default::default() });
        let (out, n) = rewrite_tile(TILE, &edits);
        assert_eq!(n, 2);
        assert!(out.contains("[object]\r\n0\r\nSceneryobjects\\a.sco\r\n7\r\n6.5\r\n5\r\n0.25\r\n102.5\r\n0\r\n"), "{out:?}");
        assert!(!out.contains("b.sco") && !out.contains("Halt"), "{out:?}");
        assert!(out.ends_with("[spline]\r\n0\r\n"));
        // nothing to do: the same text
        assert_eq!(rewrite_tile(TILE, &HashMap::new()), (TILE.to_string(), 0));
    }

    #[test]
    fn a_utf16_tile_is_written_back_as_utf16() {
        let bytes = encode(TILE, Encoding::Utf16Le);
        assert_eq!(&bytes[..4], &[0xFF, 0xFE, b'[', 0]);
        let (text, enc) = decode(&bytes);
        assert_eq!((text.as_str(), enc), (TILE, Encoding::Utf16Le));
    }

    /// A tile of version 14, as OMSI's editor writes them: a sign with labels that look
    /// like keywords, and a post.
    const TILE14: &str = "[version]\r\n14\r\n\r\n[object]\r\n0\r\nSceneryobjects\\A\\sign.sco\r\n5\r\n10\r\n20\r\n0.5\r\n90\r\n0\r\n0\r\n2\r\n[A38]\r\nTotnes\r\n\r\n[object]\r\n0\r\nSceneryobjects\\A\\post.sco\r\n6\r\n1\r\n2\r\n0\r\n0\r\n0\r\n0\r\n0\r\n\r\n";

    fn parsed(text: &str) -> omsi_map::Tile {
        omsi_map::Tile::parse(&omsi_cfg::CfgFile::from_str("tile_0_0.map", text))
    }

    #[test]
    fn a_new_record_reads_back_as_the_game_reads_it() {
        let mut text = TILE14.to_string();
        text.push_str(&new_record(14, "Sceneryobjects\\B\\bit.sco", 40, [12.5, 30.0, 0.0], 270.0, &["정류장".to_string()], "\r\n"));
        let t = parsed(&text);
        assert_eq!(t.objects.len(), 3);
        let o = &t.objects[2];
        assert_eq!((o.file.as_str(), o.id, o.pos, o.rot[0]), ("Sceneryobjects\\B\\bit.sco", 40, [12.5, 30.0, 0.0], 270.0));
        assert_eq!(o.extra, vec!["정류장".to_string()]);
        // the sign before it kept its labels
        assert_eq!(t.objects[0].extra, vec!["[A38]".to_string(), "Totnes".to_string()]);
        assert_eq!(record_ids(&text), vec![5, 6, 40]);
    }

    #[test]
    fn a_copy_follows_its_template_with_its_own_id_and_labels() {
        let (out, file) = copy_record(TILE14, 5, 99, DVec3::new(2.0, 0.0, 0.0), 10.0).unwrap();
        assert_eq!(file, "Sceneryobjects\\A\\sign.sco");
        let t = parsed(&out);
        let ids: Vec<i64> = t.objects.iter().map(|o| o.id).collect();
        assert_eq!(ids, vec![5, 99, 6]);
        assert_eq!((t.objects[1].pos, t.objects[1].rot[0]), ([12.0, 20.0, 0.5], 100.0));
        assert_eq!(t.objects[1].extra, t.objects[0].extra);
    }

    #[test]
    fn labels_and_files_change_in_their_record_only() {
        assert_eq!(record_labels(TILE14, 5).unwrap(), vec!["[A38]".to_string(), "Totnes".to_string()]);
        let out = set_record_labels(TILE14, 5, &["[A38]".into(), "논산터미널".into(), "x".into()]).unwrap();
        let t = parsed(&out);
        assert_eq!(t.objects[0].extra, vec!["[A38]".to_string(), "논산터미널".to_string(), "x".to_string()]);
        assert_eq!(t.objects[1].id, 6);
        // a post without labels gets its first
        let out = set_record_labels(&out, 6, &["a".into()]).unwrap();
        assert_eq!(parsed(&out).objects[1].extra, vec!["a".to_string()]);
        let out = set_record_file(&out, 6, "Sceneryobjects\\A\\lamp.sco").unwrap();
        assert_eq!(parsed(&out).objects[1].file, "Sceneryobjects\\A\\lamp.sco");
        assert_eq!(parsed(&out).objects[0].file, "Sceneryobjects\\A\\sign.sco");
    }

    #[test]
    fn a_deleted_sign_goes_with_labels_that_look_like_keywords() {
        let mut edits = HashMap::new();
        edits.insert(5, ObjectEdit { deleted: true, ..Default::default() });
        let (out, n) = rewrite_tile(TILE14, &edits);
        assert_eq!(n, 1);
        let t = parsed(&out);
        assert_eq!(t.objects.iter().map(|o| o.id).collect::<Vec<_>>(), vec![6]);
        assert!(!out.contains("Totnes"));
    }
}
