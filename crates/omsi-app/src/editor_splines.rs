//! The object editor's splines in the game (see `omsi_scene::editor`'s splines): their
//! keys, the open ends of the lane network a new spline starts at or one is bent onto (a
//! junction's arms), the spline types' list (Shift+P) and a spline's values (T).

use crate::game_lists::ListKind;
use crate::App;
use glam::{DVec2, DVec3};
use omsi_scene::spline_edit::{Field, FIELDS};
use winit::keyboard::KeyCode;

/// How far from the place looked for an open end of the network may be (m).
const OPEN_END_REACH: f64 = 20.0;

/// An open end of the street network: where a lane begins that nothing leads into (`into`:
/// a road going on from there would lead into it) or ends leading nowhere. Its place and
/// the heading a spline going on from it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OpenEnd {
    pub at: DVec3,
    pub heading: f64,
    pub into: bool,
}

/// The open ends of the street lanes within `reach` of `p`.
pub(crate) fn open_ends(net: &omsi_sim::traffic::Network, p: DVec2, reach: f64) -> Vec<OpenEnd> {
    let cell = |q: DVec2| ((q.x / 50.0).floor() as i32, (q.y / 50.0).floor() as i32);
    let (cx, cy) = cell(p);
    let mut lanes: Vec<usize> = Vec::new();
    for dx in -1..=1 {
        for dy in -1..=1 {
            if let Some(v) = net.grid.get(&(cx + dx, cy + dy)) {
                lanes.extend(v.iter().copied());
            }
        }
    }
    lanes.sort_unstable();
    lanes.dedup();
    let mut out = Vec::new();
    for l in lanes {
        let lane = &net.lanes[l];
        if lane.kind != omsi_sim::traffic::LaneKind::Street || lane.points.is_empty() {
            continue;
        }
        if net.prev.get(l).is_none_or(|p| p.is_empty()) && (lane.start().truncate() - p).length() < reach {
            out.push(OpenEnd { at: lane.start(), heading: lane.start_heading() as f64, into: true });
        }
        if lane.next.is_empty() && (lane.end().truncate() - p).length() < reach {
            out.push(OpenEnd { at: lane.end(), heading: lane.end_heading() as f64, into: false });
        }
    }
    out
}

/// Where a road's centre line meets an arm of the network near `p`: the open end nearest
/// it of the kind wanted (`into`: lanes beginning there, a road arriving; else lanes
/// ending there, a road leaving), and - a two-way road - halfway to the nearest open end of
/// the other kind running the other way beside it. Its place and heading (the way the road
/// goes on: into the arm for an arriving road, away from it for a leaving one).
pub(crate) fn arm_at(ends: &[OpenEnd], p: DVec2, into: bool) -> Option<(DVec3, f64)> {
    let near = ends.iter().filter(|e| e.into == into).min_by(|a, b| (a.at.truncate() - p).length().total_cmp(&(b.at.truncate() - p).length()))?;
    let opposite = |h: f64| ((h - near.heading + 540.0).rem_euclid(360.0) - 180.0).abs() > 150.0;
    let partner = ends
        .iter()
        .filter(|e| e.into != into && opposite(e.heading) && (e.at - near.at).length() < 12.0)
        .min_by(|a, b| (a.at - near.at).length().total_cmp(&(b.at - near.at).length()));
    let at = match partner {
        Some(o) => (near.at + o.at) * 0.5,
        None => near.at,
    };
    Some((at, near.heading))
}

/// The value list's line for each field (`label k` types it), the type, the ground
/// alignment and the actions.
pub(crate) fn value_items(app: &App) -> Vec<(String, String)> {
    let tr = |t: &str| omsi_ui::tr(t).into_owned();
    let Some((ed, w)) = app.menus.editor.as_ref().zip(app.world.as_ref()) else { return Vec::new() };
    let Some((_, file, values, ground)) = ed.spline_values(w) else {
        return vec![(tr("Pick a spline first (Y)"), "noop".into())];
    };
    let typing = app.menus.menu_edit_label;
    let typed = app.menus.menu_edit.as_deref().unwrap_or("");
    let mut out = vec![(format!("{}: {file}", tr("Type")), "type".to_string())];
    for (k, f) in FIELDS.iter().enumerate() {
        let v = values.iter().find(|(g, _)| g == f).map(|(_, v)| *v);
        let shown = if typing == Some(Some(k)) { format!("{typed}_") } else { v.map(|v| format!("{v}")).unwrap_or_else(|| "-".into()) };
        out.push((format!("{}: {shown}", tr(f.label())), format!("label {k}")));
    }
    out.push((format!("{}: {}", tr("The ground follows it"), tr(if ground { "On" } else { "Off" })), "ground".into()));
    out.push((tr("Add a spline at its end (E)"), "extend".into()));
    out.push((tr("Delete this spline"), "delete".into()));
    out
}

/// The spline type list in folder `dir`: up, its folders, its `.sli` files.
pub(crate) fn type_items(app: &App, dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some((up, _)) = dir.rsplit_once('\\') {
        out.push((format!(".. ({up})"), format!("dir {up}")));
    }
    for d in omsi_scene::editor::sub_dirs(&app.args.root, dir) {
        out.push((format!("[{d}]"), format!("dir {dir}\\{d}")));
    }
    let names = omsi_scene::editor::files_of(&app.args.root, dir, "sli");
    if names.is_empty() && out.len() <= 1 {
        out.push((omsi_ui::tr("No splines in this folder").into_owned(), "noop".into()));
    }
    for n in names {
        let shown = n.rsplit_once('.').map(|(a, _)| a.to_string()).unwrap_or_else(|| n.clone());
        out.push((shown, format!("sli {dir}\\{n}")));
    }
    out
}

/// Do a line of the spline lists.
pub(crate) fn run(app: &mut App, kind: &ListKind, verb: &str, arg: &str) -> Option<ListKind> {
    match (kind, verb) {
        (_, "noop") => Some(kind.clone()),
        (ListKind::EditorSplineTypes(_), "dir") => Some(ListKind::EditorSplineTypes(arg.to_string())),
        (ListKind::EditorSplineTypes(_), "sli") => {
            app.close_game_menu();
            app.editor_spline_type_chosen(arg);
            None
        }
        (ListKind::EditorSplineValues, "type") => Some(ListKind::EditorSplineTypes(type_dir(app))),
        (ListKind::EditorSplineValues, "label") => {
            let k = arg.parse::<usize>().ok();
            let now = app.menus.editor.as_ref().zip(app.world.as_ref()).and_then(|(e, w)| e.spline_values(w)).and_then(|(_, _, v, _)| k.and_then(|k| FIELDS.get(k)).and_then(|f| v.iter().find(|(g, _)| g == f).map(|(_, x)| format!("{x}")))).unwrap_or_default();
            app.menus.menu_edit = Some(now);
            app.menus.menu_edit_label = Some(k);
            crate::platform::text_input(app.window.as_deref(), true);
            Some(kind.clone())
        }
        (ListKind::EditorSplineValues, "ground") => {
            app.editor_spline_op(|ed, w, _| ed.toggle_spline_ground(w).map(|(t, _)| t));
            Some(kind.clone())
        }
        (ListKind::EditorSplineValues, "extend") => {
            app.close_game_menu();
            app.editor_spline_op(|ed, w, _| ed.extend_spline(w));
            None
        }
        (ListKind::EditorSplineValues, "delete") => {
            app.close_game_menu();
            app.editor_spline_op(|ed, w, _| ed.delete_spline(w));
            None
        }
        _ => Some(kind.clone()),
    }
}

/// The folder the type list opens in: the picked spline's, else `Splines`.
fn type_dir(app: &App) -> String {
    app.menus
        .editor
        .as_ref()
        .and_then(|e| e.spline_type.clone())
        .and_then(|f| f.rsplit_once('\\').map(|(d, _)| d.to_string()))
        .unwrap_or_else(|| "Splines".into())
}

impl App {
    /// A key of the editor's splines; true when it was one (else the objects' keys go on).
    pub(crate) fn editor_spline_key(&mut self, code: KeyCode, shift: bool, ctrl: bool) -> bool {
        if ctrl {
            return false;
        }
        let picked = self.menus.editor.as_ref().is_some_and(|e| e.spline.is_some());
        let (Some(world), Some(cam)) = (self.world.clone(), self.camera.as_ref()) else { return false };
        let (eye, fwd, yaw) = (cam.position, cam.forward(), cam.yaw as f64);
        let aim = omsi_scene::editor::Editor::aim(&world, eye, fwd);
        let fine = if shift { 0.1 } else { 1.0 };
        match code {
            KeyCode::KeyY => {
                let Some(at) = aim else {
                    self.service_msg = Some((omsi_ui::tr("Point the view at the ground").into_owned(), 3.0));
                    return true;
                };
                let ed = self.menus.editor.as_mut().unwrap();
                let msg = match ed.pick_spline(&world, at) {
                    Some(_) => ed.describe_spline(&world),
                    None => omsi_ui::tr("No spline where the view points").into_owned(),
                };
                self.service_msg = Some((msg, 6.0));
                true
            }
            KeyCode::KeyP if shift => {
                let dir = type_dir(self);
                self.open_game_menu();
                self.open_list(ListKind::EditorSplineTypes(dir));
                true
            }
            KeyCode::KeyR => {
                self.rebuild_station_links();
                true
            }
            KeyCode::KeyE if !picked => {
                // a new spline from the open end of the network nearest the view's point
                // (going away from it), else from the point itself, heading as the view
                let Some(at) = aim else {
                    self.service_msg = Some((omsi_ui::tr("Point the view at the ground").into_owned(), 3.0));
                    return true;
                };
                let arm = self.session.traffic.as_ref().and_then(|t| arm_at(&open_ends(&t.net, at.truncate(), OPEN_END_REACH), at.truncate(), false));
                let (start, heading) = arm.unwrap_or((at, yaw));
                self.editor_spline_op(move |ed, w, _| ed.start_spline(w, start, heading));
                true
            }
            KeyCode::KeyE => {
                self.editor_spline_op(|ed, w, _| ed.extend_spline(w));
                true
            }
            KeyCode::KeyG if picked => {
                let end = self.menus.editor.as_ref().and_then(|e| e.spline_now(&world)).map(|(_, c, _)| c.end_point());
                let Some(end) = end else { return true };
                let arm = self.session.traffic.as_ref().and_then(|t| arm_at(&open_ends(&t.net, end.truncate(), OPEN_END_REACH), end.truncate(), true));
                let Some((target, heading)) = arm else {
                    self.service_msg = Some((omsi_ui::tr("No open end of a road near the spline's end").into_owned(), 4.0));
                    return true;
                };
                let ed = self.menus.editor.as_mut().unwrap();
                match ed.join_spline(&world, target) {
                    Ok((tiles, arrives)) => {
                        self.editor_reload(&tiles);
                        let off = ((arrives - heading + 540.0).rem_euclid(360.0) - 180.0).abs();
                        let msg = omsi_ui::tr("Spline bent onto the road's end (it arrives {off}° off its direction) - Ctrl+S saves").replace("{off}", &format!("{off:.1}"));
                        self.service_msg = Some((msg, 6.0));
                    }
                    Err(e) => self.service_msg = Some((omsi_ui::tr(&e).into_owned(), 5.0)),
                }
                true
            }
            _ if !picked => false,
            KeyCode::KeyI => self.editor_spline_op(move |ed, w, _| ed.adjust_spline(w, Field::Length, fine)),
            KeyCode::KeyK => self.editor_spline_op(move |ed, w, _| ed.adjust_spline(w, Field::Length, -fine)),
            KeyCode::KeyM => self.editor_spline_op(move |ed, w, _| ed.turn_spline(w, fine)),
            KeyCode::KeyN => self.editor_spline_op(move |ed, w, _| ed.turn_spline(w, -fine)),
            KeyCode::KeyO => self.editor_spline_op(move |ed, w, _| ed.adjust_spline(w, Field::Height, 0.1 * fine)),
            KeyCode::KeyU => self.editor_spline_op(move |ed, w, _| ed.adjust_spline(w, Field::Height, -0.1 * fine)),
            KeyCode::KeyV => self.editor_spline_op(|ed, w, root| ed.spline_variant(w, root)),
            KeyCode::KeyA => self.editor_spline_op(|ed, w, _| ed.toggle_spline_ground(w).map(|(t, _)| t)),
            KeyCode::Delete => self.editor_spline_op(|ed, w, _| ed.delete_spline(w)),
            KeyCode::Backspace => {
                let ed = self.menus.editor.as_mut().unwrap();
                match ed.undo_spline(&world) {
                    Some(tiles) => {
                        self.editor_reload(&tiles);
                        self.service_msg = Some((omsi_ui::tr("Spline change undone").into_owned(), 3.0));
                    }
                    None => self.service_msg = Some((omsi_ui::tr("Nothing to undo").into_owned(), 3.0)),
                }
                true
            }
            KeyCode::KeyT => {
                self.open_game_menu();
                self.open_list(ListKind::EditorSplineValues);
                true
            }
            // (the objects' keys do nothing to a spline)
            KeyCode::KeyJ | KeyCode::KeyL | KeyCode::KeyC => true,
            _ => false,
        }
    }

    /// Do a spline change; its tiles read again and what it is now said. Always true (the
    /// key was the editor's).
    pub(crate) fn editor_spline_op(&mut self, op: impl FnOnce(&mut omsi_scene::editor::Editor, &omsi_scene::World, &std::path::Path) -> Result<Vec<(i32, i32)>, String>) -> bool {
        let Some(world) = self.world.clone() else { return true };
        let root = self.args.root.clone();
        let Some(ed) = self.menus.editor.as_mut() else { return true };
        let msg = match op(ed, &world, &root) {
            Ok(tiles) => {
                let ed = self.menus.editor.as_ref().unwrap();
                let msg = if ed.spline.is_some() { ed.describe_spline(&world) } else { omsi_ui::tr("Spline deleted - Backspace brings it back").into_owned() };
                self.editor_reload(&tiles);
                msg
            }
            Err(e) => omsi_ui::tr(&e).into_owned(),
        };
        log::info!("object editor: {msg}");
        self.service_msg = Some((msg, 6.0));
        true
    }

    /// A spline type chosen in the list (Shift+P): the picked spline takes it, else it is
    /// the type of the next new spline (E).
    pub(crate) fn editor_spline_type_chosen(&mut self, file: &str) {
        let picked = self.menus.editor.as_ref().is_some_and(|e| e.spline.is_some());
        if picked {
            let f = file.to_string();
            self.editor_spline_op(move |ed, w, _| ed.set_spline_type(w, &f));
        } else if let Some(ed) = self.menus.editor.as_mut() {
            ed.spline_type = Some(file.to_string());
            self.service_msg = Some((omsi_ui::tr("{name}: E starts a spline of it at the open road end nearest the view").replace("{name}", file.rsplit('\\').next().unwrap_or(file)), 6.0));
        }
    }

    /// A value typed in the spline's list (T).
    pub(crate) fn apply_spline_value(&mut self, k: usize, text: &str) {
        let Some(f) = FIELDS.get(k).copied() else { return };
        let Ok(v) = text.trim().replace(',', ".").parse::<f64>() else {
            self.service_msg = Some((omsi_ui::tr("Not a number: {text}").replace("{text}", text), 3.0));
            return;
        };
        self.editor_spline_op(move |ed, w, _| ed.set_spline(w, f, v));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn end(x: f64, y: f64, heading: f64, into: bool) -> OpenEnd {
        OpenEnd { at: DVec3::new(x, y, 0.0), heading, into }
    }

    #[test]
    fn a_road_meets_a_two_way_arm_in_its_middle() {
        // an arm open to the south: its lane in (heading north) at x 1.75, out at -1.75
        let ends = [end(1.75, 0.0, 0.0, true), end(-1.75, 0.0, 180.0, false), end(40.0, 0.0, 90.0, true)];
        let (at, h) = arm_at(&ends, DVec2::new(0.0, -5.0), true).unwrap();
        assert!((at - DVec3::ZERO).length() < 1e-9 && h == 0.0);
        // leaving it: the heading of the lane that ends there
        let (at, h) = arm_at(&ends, DVec2::new(0.0, -5.0), false).unwrap();
        assert!((at - DVec3::ZERO).length() < 1e-9 && h == 180.0);
        // a one-way end alone
        let (at, _) = arm_at(&ends[2..], DVec2::new(38.0, 0.0), true).unwrap();
        assert_eq!(at, DVec3::new(40.0, 0.0, 0.0));
        assert!(arm_at(&ends[2..], DVec2::ZERO, false).is_none());
    }
}
