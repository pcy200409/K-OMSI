//! The object editor's two lists in the game menu's chooser: the objects to place (the
//! folders of `Sceneryobjects` and their `.sco` files, P) and the labels of the object
//! picked, each typed anew (T). See `omsi_scene::editor`.

use crate::game_lists::ListKind;
use crate::App;
use winit::keyboard::KeyCode;

/// The folder the object list opens in: the picked object's, else `Sceneryobjects`.
pub(crate) fn start_dir(app: &App) -> String {
    let picked = app.menus.editor.as_ref().and_then(|e| e.selected);
    let sco = picked.and_then(|id| app.world.as_ref().and_then(|w| w.edit_objects.lock().get(&id).map(|o| o.sco.clone())));
    sco.and_then(|p| relative(app, &p))
        .and_then(|r| r.rsplit_once('\\').map(|(d, _)| d.to_string()))
        .unwrap_or_else(|| "Sceneryobjects".into())
}

/// A file of the game's folder or of a content folder as a map names it
/// (`Sceneryobjects\...`).
fn relative(app: &App, p: &std::path::Path) -> Option<String> {
    omsi_cfg::content_roots()
        .iter()
        .chain(std::iter::once(&app.args.root))
        .find_map(|r| p.strip_prefix(r).ok())
        .map(|r| r.to_string_lossy().replace('/', "\\"))
}

/// The lines of the object list in folder `dir`: up, its folders, its objects.
pub(crate) fn object_items(app: &App, dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some((up, _)) = dir.rsplit_once('\\') {
        out.push((format!(".. ({up})"), format!("dir {up}")));
    }
    for d in omsi_scene::editor::sub_dirs(&app.args.root, dir) {
        out.push((format!("[{d}]"), format!("dir {dir}\\{d}")));
    }
    let names = omsi_scene::editor::sco_names(&app.args.root, dir);
    if names.is_empty() && out.len() <= 1 {
        out.push((omsi_ui::tr("No objects in this folder").into_owned(), "noop".into()));
    }
    for n in names {
        out.push((n.trim_end_matches(".sco").trim_end_matches(".SCO").to_string(), format!("sco {dir}\\{n}")));
    }
    out
}

/// The lines of the label list: each label (the one being typed with its cursor), then a
/// new line.
pub(crate) fn label_items(app: &App) -> Vec<(String, String)> {
    let tr = |t: &str| omsi_ui::tr(t).into_owned();
    let Some((ed, w)) = app.menus.editor.as_ref().zip(app.world.as_ref()) else { return Vec::new() };
    let Some((_, _, labels)) = ed.labels(w) else {
        return vec![(tr("Pick an object first (Enter)"), "noop".into())];
    };
    let typing = app.menus.menu_edit_label;
    let typed = app.menus.menu_edit.as_deref().unwrap_or("");
    let mut out: Vec<(String, String)> = labels
        .iter()
        .enumerate()
        .map(|(k, l)| {
            let shown = if typing == Some(Some(k)) { format!("{typed}_") } else { l.clone() };
            (format!("{}: {shown}", k + 1), format!("label {k}"))
        })
        .collect();
    let new = if typing == Some(None) { format!("{}: {typed}_", labels.len() + 1) } else { tr("+ Add a label line") };
    out.push((new, "label new".into()));
    out
}

/// Do a line of one of the lists; the list to show next (None: the menu closed).
pub(crate) fn run(app: &mut App, kind: &ListKind, verb: &str, arg: &str) -> Option<ListKind> {
    match (kind, verb) {
        (_, "noop") => Some(kind.clone()),
        (ListKind::EditorObjects(_), "dir") => Some(ListKind::EditorObjects(arg.to_string())),
        (ListKind::EditorObjects(_), "sco") => {
            app.editor_place(arg);
            None
        }
        (ListKind::EditorLabels, "label") => {
            let slot = arg.parse::<usize>().ok();
            let now = app
                .menus
                .editor
                .as_ref()
                .zip(app.world.as_ref())
                .and_then(|(e, w)| e.labels(w))
                .and_then(|(_, _, l)| slot.and_then(|k| l.get(k).cloned()))
                .unwrap_or_default();
            app.menus.menu_edit = Some(now);
            app.menus.menu_edit_label = Some(slot);
            crate::platform::text_input(app.window.as_deref(), true);
            Some(kind.clone())
        }
        _ => Some(kind.clone()),
    }
}

/// The longest label typed.
const LABEL_MAX: usize = 255;

impl App {
    /// The object list, from the editor (P).
    pub(crate) fn open_editor_objects(&mut self) {
        let dir = start_dir(self);
        self.open_game_menu();
        self.open_list(ListKind::EditorObjects(dir));
    }

    /// The picked object's labels, from the editor (T).
    pub(crate) fn open_editor_labels(&mut self) {
        let has = self.menus.editor.as_ref().zip(self.world.as_ref()).and_then(|(e, w)| e.labels(w)).is_some();
        if !has {
            self.service_msg = Some(("Pick an object first (Enter)".into(), 3.0));
            return;
        }
        self.open_game_menu();
        self.open_list(ListKind::EditorLabels);
    }

    /// Typing a label stops (nothing set).
    pub(crate) fn end_label_edit(&mut self) {
        self.menus.menu_label_preedit = 0;
        if self.menus.menu_edit_label.take().is_some() {
            self.menus.menu_edit = None;
            crate::platform::text_input(self.window.as_deref(), false);
        }
    }

    pub(crate) fn label_edit_text(&mut self, text: &str) {
        if self.menus.menu_edit_label.is_none() {
            return;
        }
        if let Some(d) = self.menus.menu_edit.as_mut() {
            for c in text.chars().filter(|c| !c.is_control()) {
                if d.chars().count() >= LABEL_MAX {
                    break;
                }
                d.push(c);
            }
        }
        self.refresh_list();
    }

    /// What an input method says while a label is typed: the syllable being composed
    /// (standing in the label, put back with each change) and the text it gives up. False
    /// when no label is typed (the IME's text is another field's).
    pub(crate) fn label_ime(&mut self, ime: &winit::event::Ime) -> bool {
        use winit::event::Ime;
        if self.menus.menu_edit_label.is_none() {
            self.menus.menu_label_preedit = 0;
            return false;
        }
        let composed = std::mem::take(&mut self.menus.menu_label_preedit);
        if let Some(d) = self.menus.menu_edit.as_mut() {
            for _ in 0..composed {
                d.pop();
            }
        }
        match ime {
            Ime::Preedit(text, _) => {
                self.label_edit_text(text);
                self.menus.menu_label_preedit = text.chars().filter(|c| !c.is_control()).count();
            }
            Ime::Commit(text) => self.label_edit_text(text),
            Ime::Enabled | Ime::Disabled => self.refresh_list(),
        }
        true
    }

    pub(crate) fn label_edit_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Escape => self.end_label_edit(),
            KeyCode::Backspace | KeyCode::Delete => {
                if let Some(d) = self.menus.menu_edit.as_mut() {
                    d.pop();
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                self.menus.menu_label_preedit = 0;
                let slot = self.menus.menu_edit_label;
                let text = self.menus.menu_edit.clone().unwrap_or_default();
                self.end_label_edit();
                if let Some(slot) = slot {
                    self.apply_label(slot, &text);
                }
            }
            _ => {}
        }
        self.refresh_list();
    }

    /// Label `slot` of the picked object set to `text`, its tile read again.
    fn apply_label(&mut self, slot: Option<usize>, text: &str) {
        let (Some(world), Some(ed)) = (self.world.clone(), self.menus.editor.as_mut()) else { return };
        let Some(id) = ed.selected else { return };
        let msg = match ed.set_label(&world, id, slot, text) {
            Ok(tile) => {
                self.editor_reload(&[tile]);
                omsi_ui::tr("Object {id}: label set - Ctrl+S saves").replace("{id}", &id.to_string())
            }
            Err(e) => omsi_ui::tr("Label not set ({e})").replace("{e}", &e),
        };
        log::info!("object editor: {msg}");
        self.service_msg = Some((msg, 4.0));
    }

    /// Put `file` (a `.sco` of the object list) where the middle of the view meets the
    /// ground, facing the camera.
    pub(crate) fn editor_place(&mut self, file: &str) {
        self.close_game_menu();
        let (Some(world), Some(cam)) = (self.world.clone(), self.camera.as_ref()) else { return };
        let (eye, fwd, yaw) = (cam.position, cam.forward(), cam.yaw as f64);
        let Some(at) = omsi_scene::editor::Editor::aim(&world, eye, fwd) else {
            self.service_msg = Some(("Point the view at the ground".into(), 3.0));
            return;
        };
        let heading = (((yaw + 180.0) / 5.0).round() * 5.0).rem_euclid(360.0);
        let Some(ed) = self.menus.editor.as_mut() else { return };
        let msg = match ed.place(&world, file, at, heading) {
            Ok(tile) => {
                self.editor_reload(&[tile]);
                omsi_ui::tr("{name} placed: drag it, wheel turns, T labels, Ctrl+S saves").replace("{name}", file.rsplit('\\').next().unwrap_or(file))
            }
            Err(e) => omsi_ui::tr("Not placed ({e})").replace("{e}", &e),
        };
        log::info!("object editor: {msg}");
        self.service_msg = Some((msg, 6.0));
    }

    /// Tiles whose text the editor changed: read again (their objects drawn anew).
    pub(crate) fn editor_reload(&mut self, tiles: &[(i32, i32)]) {
        let Some(world) = self.world.as_ref() else { return };
        world.forget_staged(tiles);
        if let Some(st) = self.gfx.streamer.as_mut() {
            st.reload(Some(tiles));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_folder_lists_up_its_parent() {
        // (a headless fixture: no Steam client)
        let args = <crate::cli::Args as clap::Parser>::parse_from(["openomsi", "--server", "test-server.cfg"]);
        let app = crate::assemble_app(args, Default::default());
        let items = super::object_items(&app, "Sceneryobjects\\Nowhere");
        assert_eq!(items[0].1, "dir Sceneryobjects");
        assert!(items.iter().any(|i| i.1 == "noop"));
    }

    #[test]
    fn hangul_composed_in_a_label() {
        use winit::event::Ime;
        let args = <crate::cli::Args as clap::Parser>::parse_from(["openomsi", "--server", "test-server.cfg"]);
        let mut app = crate::assemble_app(args, Default::default());
        let pre = |s: &str| Ime::Preedit(s.into(), None);
        // no label typed: the IME's text is not ours
        assert!(!app.label_ime(&pre("ㄱ")));
        app.menus.menu_edit = Some("A ".into());
        app.menus.menu_edit_label = Some(Some(0));
        for ime in [pre("ㅎ"), pre("하"), pre("한"), Ime::Commit("한".into()), pre("ㄱ"), pre("그"), pre("글")] {
            assert!(app.label_ime(&ime));
        }
        assert_eq!(app.menus.menu_edit.as_deref(), Some("A 한글"));
        assert!(app.label_ime(&Ime::Commit("글".into())));
        assert_eq!(app.menus.menu_edit.as_deref(), Some("A 한글"));
    }
}
