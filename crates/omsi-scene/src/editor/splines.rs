//! The object editor's splines (see `crate::spline_edit` for the records): one picked where
//! the view points (Y), its type stepped through its folder (V) or chosen (Shift+P), its
//! length (I / K), curve (N / M) and start height (U / O) changed, its values typed (T),
//! a new one added at its end (E) or at an open end of the network, its end bent onto an
//! open end of a junction (G), its ground alignment switched (A), taken away (Delete), and
//! every spline change undone in turn (Backspace). As with objects, the changed tiles'
//! texts are kept in memory and the tiles read again; Ctrl+S writes them.

use super::{tile_text, tile_version, Editor};
use crate::spline_edit::{self as se, Field};
use crate::World;
use glam::{DVec2, DVec3};
use omsi_geometry::SplineCurve;
use std::path::Path;

/// The spline picked: its id and the tile its record is in.
pub type Picked = (i64, (i32, i32));

/// How far the view's point on the ground may be from a spline to pick it (m).
const PICK_REACH: f64 = 8.0;

/// A new spline's length (m).
pub const NEW_LENGTH: f64 = 10.0;

impl Editor {
    /// The tiles about `p` (it and its eight neighbours) that the map has.
    fn tiles_about(world: &World, p: DVec2) -> Vec<(i32, i32)> {
        let size = omsi_map::tile_size();
        let (tx, ty) = ((p.x / size).floor() as i32, (p.y / size).floor() as i32);
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if world.tile_source(tx + dx, ty + dy).is_some() {
                    out.push((tx + dx, ty + dy));
                }
            }
        }
        out
    }

    /// The splines of tile `tile` as its text has them now, with their curves.
    fn splines_of(world: &World, tile: (i32, i32)) -> Vec<(omsi_map::MapSpline, SplineCurve)> {
        let Ok(text) = tile_text(world, tile) else { return Vec::new() };
        let t = omsi_map::Tile::parse(&omsi_cfg::CfgFile::from_str("tile.map", &text));
        let size = omsi_map::tile_size();
        let origin = DVec2::new(tile.0 as f64 * size, tile.1 as f64 * size);
        t.splines.into_iter().filter(|s| !s.deleted).map(|s| {
            let c = SplineCurve::from_map(&s, origin);
            (s, c)
        }).collect()
    }

    /// The spline picked: its record and its curve.
    pub fn spline_now(&self, world: &World) -> Option<(omsi_map::MapSpline, SplineCurve, (i32, i32))> {
        let (id, tile) = self.spline?;
        Self::splines_of(world, tile).into_iter().find(|(s, _)| s.id == id).map(|(s, c)| (s, c, tile))
    }

    /// Pick the spline nearest `at` (the view's point on the ground); the object picked is
    /// let go.
    pub fn pick_spline(&mut self, world: &World, at: DVec3) -> Option<i64> {
        let mut best: Option<(f64, Picked, String)> = None;
        for tile in Self::tiles_about(world, at.truncate()) {
            for (s, c) in Self::splines_of(world, tile) {
                let n = (c.length / 1.0).ceil().max(1.0) as usize;
                let d = (0..=n).map(|k| (c.point_at(c.length * k as f64 / n as f64).truncate() - at.truncate()).length()).fold(f64::MAX, f64::min);
                if d < PICK_REACH && best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, (s.id, tile), s.file.clone()));
                }
            }
        }
        let (_, picked, file) = best?;
        self.spline = Some(picked);
        self.spline_type = Some(file);
        self.selected = None;
        Some(picked.0)
    }

    /// What the spline picked is now.
    pub fn describe_spline(&self, world: &World) -> String {
        let Some((s, c, _)) = self.spline_now(world) else { return omsi_ui::tr("Spline: not found").into_owned() };
        let name = s.file.rsplit(['\\', '/']).next().unwrap_or(&s.file).to_string();
        let turn = if s.radius == 0.0 { 0.0 } else { (s.length / s.radius).to_degrees() };
        let h = c.end_point().z - c.start.z;
        omsi_ui::tr("Spline {id} {name}: {len} m, turn {turn}°, rises {h} m - I/K length, N/M curve, U/O height, E add, G join, T values, A ground, Delete")
            .replace("{id}", &s.id.to_string())
            .replace("{name}", &name)
            .replace("{len}", &format!("{:.1}", s.length))
            .replace("{turn}", &format!("{turn:+.1}"))
            .replace("{h}", &format!("{h:+.2}"))
    }

    /// Remember the texts of `tiles` (before a change), for Backspace.
    fn remember(&mut self, world: &World, tiles: &[(i32, i32)]) {
        let texts = world.tile_texts.lock();
        let snap: Vec<((i32, i32), Option<String>)> = tiles.iter().map(|t| (*t, texts.get(t).cloned())).collect();
        self.undo.push(snap);
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
    }

    /// Put a changed text in for `tile`.
    fn put(world: &World, tile: (i32, i32), text: String) {
        world.tile_texts.lock().insert(tile, text);
    }

    /// A spline tile's text, refused unless its records name their neighbours (version 11
    /// on).
    fn spline_text(world: &World, tile: (i32, i32)) -> Result<String, String> {
        let text = tile_text(world, tile)?;
        let v = tile_version(&text);
        if v < 11 {
            return Err(format!("tile ({}, {}) is of an old version ({v}): its splines are not edited", tile.0, tile.1));
        }
        Ok(text)
    }

    /// Undo the last spline change. Returns the tiles to read again.
    pub fn undo_spline(&mut self, world: &World) -> Option<Vec<(i32, i32)>> {
        let snap = self.undo.pop()?;
        let mut texts = world.tile_texts.lock();
        let tiles = snap.iter().map(|s| s.0).collect();
        for (tile, text) in snap {
            match text {
                Some(t) => {
                    texts.insert(tile, t);
                }
                None => {
                    texts.remove(&tile);
                }
            }
        }
        Some(tiles)
    }

    /// Set a value of the spline picked. Returns the tiles to read again.
    pub fn set_spline(&mut self, world: &World, f: Field, v: f64) -> Result<Vec<(i32, i32)>, String> {
        let (id, tile) = self.spline.ok_or("Pick a spline first (Y)")?;
        if f == Field::Length && v < 0.1 {
            return Err("A spline is at least 0.1 m long".into());
        }
        let text = Self::spline_text(world, tile)?;
        let new = se::set_field(&text, id, f, v).ok_or("The spline was not found in its tile")?;
        self.remember(world, &[tile]);
        Self::put(world, tile, new);
        Ok(vec![tile])
    }

    /// Change a value of the spline picked by `d`.
    pub fn adjust_spline(&mut self, world: &World, f: Field, d: f64) -> Result<Vec<(i32, i32)>, String> {
        let (id, tile) = self.spline.ok_or("Pick a spline first (Y)")?;
        let text = Self::spline_text(world, tile)?;
        let now = se::field(&text, id, f).ok_or("The spline was not found in its tile")?;
        // (the length: the curve's turn kept)
        if f == Field::Length {
            let r = se::field(&text, id, Field::Radius).unwrap_or(0.0);
            let len = (now + d).max(0.5);
            let tiles = self.set_spline(world, Field::Length, len)?;
            if r != 0.0 {
                let turn = now / r;
                let text = tile_text(world, tile)?;
                if let Some(t) = se::set_field(&text, id, Field::Radius, len / turn) {
                    Self::put(world, tile, t);
                }
            }
            return Ok(tiles);
        }
        self.set_spline(world, f, now + d)
    }

    /// Turn the spline picked by `deg` more (its length kept; right positive).
    pub fn turn_spline(&mut self, world: &World, deg: f64) -> Result<Vec<(i32, i32)>, String> {
        let (id, tile) = self.spline.ok_or("Pick a spline first (Y)")?;
        let text = Self::spline_text(world, tile)?;
        let len = se::field(&text, id, Field::Length).ok_or("The spline was not found in its tile")?;
        let r = se::field(&text, id, Field::Radius).unwrap_or(0.0);
        let turn = if r == 0.0 { 0.0 } else { (len / r).to_degrees() } + deg;
        let radius = if turn.abs() < 0.05 { 0.0 } else { len / turn.to_radians() };
        self.set_spline(world, Field::Radius, radius)
    }

    /// The spline picked takes the next type of its folder.
    pub fn spline_variant(&mut self, world: &World, root: &Path) -> Result<Vec<(i32, i32)>, String> {
        let (s, _, _) = self.spline_now(world).ok_or("Pick a spline first (Y)")?;
        let (dir, name) = match s.file.rfind(['\\', '/']) {
            Some(i) => (s.file[..i].to_string(), s.file[i + 1..].to_string()),
            None => (String::new(), s.file.clone()),
        };
        let names = super::files_of(root, &dir, "sli");
        if names.is_empty() {
            return Err(format!("no splines found in {dir}"));
        }
        let i = names.iter().position(|n| n.eq_ignore_ascii_case(&name)).map(|i| (i + 1) % names.len()).unwrap_or(0);
        let file = if dir.is_empty() { names[i].clone() } else { format!("{dir}\\{}", names[i]) };
        self.set_spline_type(world, &file)
    }

    /// The spline picked becomes of type `file` (a `.sli`, relative to the game's folder).
    pub fn set_spline_type(&mut self, world: &World, file: &str) -> Result<Vec<(i32, i32)>, String> {
        let (id, tile) = self.spline.ok_or("Pick a spline first (Y)")?;
        let text = Self::spline_text(world, tile)?;
        let new = se::set_file(&text, id, file).ok_or("The spline was not found in its tile")?;
        self.remember(world, &[tile]);
        Self::put(world, tile, new);
        self.spline_type = Some(file.to_string());
        Ok(vec![tile])
    }

    /// A new id for a spline (as for objects, above the splines of the texts edited).
    fn new_spline_id(&self, world: &World) -> i64 {
        let texts = world.tile_texts.lock().values().flat_map(|t| se::spline_ids(t)).max().unwrap_or(0);
        self.new_id(world).max(texts + 1)
    }

    /// A new spline of `file` from `start` (world, its height absolute) heading `heading`,
    /// after spline `prev` (0: none), written into the tile it starts in; it is then the one
    /// picked. Returns the tiles to read again.
    #[allow(clippy::too_many_arguments)]
    fn add_spline(&mut self, world: &World, file: &str, start: DVec3, heading: f64, length: f64, radius: f64, grad: f64, prev: Option<Picked>, tex_offset: f64) -> Result<Vec<(i32, i32)>, String> {
        let size = omsi_map::tile_size();
        let tile = ((start.x / size).floor() as i32, (start.y / size).floor() as i32);
        let text = Self::spline_text(world, tile)?;
        if let Some((pid, ptile)) = prev {
            let ptext = Self::spline_text(world, ptile)?;
            let (_, _, next) = se::links(&ptext, pid).ok_or("The spline was not found in its tile")?;
            if next != 0 {
                return Err(format!("spline {pid} goes on already (into {next})"));
            }
        }
        let id = self.new_spline_id(world);
        let local = [start.x - tile.0 as f64 * size, start.y - tile.1 as f64 * size, start.z];
        let rec = se::new_record(tile_version(&text), file, id, prev.map(|p| p.0).unwrap_or(0), local, heading, length, radius, grad, tex_offset, if text.contains("\r\n") || text.is_empty() { "\r\n" } else { "\n" });
        let mut tiles = vec![tile];
        if let Some((_, ptile)) = prev.filter(|p| p.1 != tile) {
            tiles.push(ptile);
        }
        self.remember(world, &tiles);
        Self::put(world, tile, se::add_record(&text, &rec));
        if let Some((pid, ptile)) = prev {
            let ptext = tile_text(world, ptile)?;
            if let Some(t) = se::set_links(&ptext, pid, None, Some(id)) {
                Self::put(world, ptile, t);
            }
        }
        self.spline = Some((id, tile));
        self.spline_type = Some(file.to_string());
        self.selected = None;
        Ok(tiles)
    }

    /// A new spline at the end of the one picked: of its type, straight on as it ends,
    /// climbing as it ends. Returns the tiles to read again.
    pub fn extend_spline(&mut self, world: &World) -> Result<Vec<(i32, i32)>, String> {
        let (s, c, tile) = self.spline_now(world).ok_or("Pick a spline first (Y)")?;
        let end = c.end_point();
        let heading = c.heading_at(c.length);
        let grad = c.slope_at(c.length) * 100.0;
        self.add_spline(world, &s.file.clone(), end, heading, NEW_LENGTH, 0.0, grad, Some((s.id, tile)), s.tex_offset + s.length)
    }

    /// A new spline of the type last used (or picked) starting at `start` heading
    /// `heading` (an open end of the network, or where the view points).
    pub fn start_spline(&mut self, world: &World, start: DVec3, heading: f64) -> Result<Vec<(i32, i32)>, String> {
        let file = self.spline_type.clone().ok_or("Choose a spline type first (Shift+P), or pick a spline (Y)")?;
        self.add_spline(world, &file, start, heading, NEW_LENGTH, 0.0, 0.0, None, 0.0)
    }

    /// Bend the spline picked so that it ends at `target` (one arc from its start, as it
    /// starts), climbing evenly to its height. Returns the tiles to read again and the
    /// heading it arrives with.
    pub fn join_spline(&mut self, world: &World, target: DVec3) -> Result<(Vec<(i32, i32)>, f64), String> {
        let (s, c, tile) = self.spline_now(world).ok_or("Pick a spline first (Y)")?;
        let (len, radius, arrives) = se::arc_to(c.start.truncate(), c.heading_deg, target.truncate()).ok_or("The end cannot be reached with one curve from the spline's start")?;
        let grad = (target.z - c.start.z) / len * 100.0;
        let text = Self::spline_text(world, tile)?;
        let mut t = text.clone();
        for (f, v) in [(Field::Length, len), (Field::Radius, radius), (Field::GradStart, grad), (Field::GradEnd, grad)] {
            t = se::set_field(&t, s.id, f, v).ok_or("The spline was not found in its tile")?;
        }
        // (a [spline_h] climbs its own height change)
        if s.is_h {
            t = se::set_field(&t, s.id, Field::DeltaH, target.z - c.start.z).unwrap_or(t);
        }
        self.remember(world, &[tile]);
        Self::put(world, tile, t);
        Ok((vec![tile], arrives))
    }

    /// Switch the spline picked's ground alignment. Returns the tiles to read again (the
    /// ground of its neighbours may follow it) and whether it is on now.
    pub fn toggle_spline_ground(&mut self, world: &World) -> Result<(Vec<(i32, i32)>, bool), String> {
        let (id, tile) = self.spline.ok_or("Pick a spline first (Y)")?;
        let text = Self::spline_text(world, tile)?;
        let new = se::toggle_terrain_align(&text, id).ok_or("The spline was not found in its tile")?;
        let on = se::terrain_aligned(&new, id).unwrap_or(false);
        self.remember(world, &[tile]);
        Self::put(world, tile, new);
        let size = omsi_map::tile_size();
        let centre = DVec2::new((tile.0 as f64 + 0.5) * size, (tile.1 as f64 + 0.5) * size);
        Ok((Self::tiles_about(world, centre), on))
    }

    /// Take the spline picked away (with the rows along it); the splines before and after
    /// it, in any tile about it, no longer name it. The one before it is picked then.
    /// Returns the tiles to read again.
    pub fn delete_spline(&mut self, world: &World) -> Result<Vec<(i32, i32)>, String> {
        let (id, tile) = self.spline.ok_or("Pick a spline first (Y)")?;
        let text = Self::spline_text(world, tile)?;
        let (new, prev, next) = se::delete(&text, id).ok_or("The spline was not found in its tile")?;
        let size = omsi_map::tile_size();
        let centre = DVec2::new((tile.0 as f64 + 0.5) * size, (tile.1 as f64 + 0.5) * size);
        let about = Self::tiles_about(world, centre);
        // the neighbours' links in the other tiles
        let mut changed: Vec<((i32, i32), String)> = vec![(tile, new)];
        for t in about.iter().filter(|t| **t != tile) {
            let Ok(other) = tile_text(world, *t) else { continue };
            let ids = se::spline_ids(&other);
            if (prev != 0 && ids.contains(&prev)) || (next != 0 && ids.contains(&next)) {
                let u = se::unlink(&other, id);
                if u != other {
                    changed.push((*t, u));
                }
            }
        }
        let tiles: Vec<(i32, i32)> = changed.iter().map(|c| c.0).collect();
        self.remember(world, &tiles);
        for (t, text) in changed {
            Self::put(world, t, text);
        }
        // the one before it, if it is about
        self.spline = (prev != 0).then(|| about.iter().find(|t| tile_text(world, **t).map(|x| se::spline_ids(&x).contains(&prev)).unwrap_or(false)).map(|t| (prev, *t))).flatten();
        Ok(tiles)
    }

    /// The values of the spline picked, for the list (T): each field and its value, and
    /// whether the ground follows it.
    pub fn spline_values(&self, world: &World) -> Option<(i64, String, Vec<(Field, f64)>, bool)> {
        let (id, tile) = self.spline?;
        let text = tile_text(world, tile).ok()?;
        let (file, _, _) = se::links(&text, id)?;
        let values = se::FIELDS.iter().filter_map(|f| se::field(&text, id, *f).map(|v| (*f, v))).collect();
        Some((id, file, values, se::terrain_aligned(&text, id).unwrap_or(false)))
    }
}
