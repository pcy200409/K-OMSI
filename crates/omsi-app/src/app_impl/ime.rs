//! Typing through an input method (Korean, Japanese, Chinese ...): the window lets the
//! system's IME work while a text field has the keyboard, and what it composes goes into
//! that field - the LAN chat line, a plugin's or the game's panel field, a MileZero song
//! request. Without it the IME was off in the game's window and only the keyboard's Latin
//! letters were typed (Hangul could not be written at all).
//!
//! The syllable being composed stands in the field as typed text and is put back with each
//! change of the IME (`ㅎ`, `하`, `한` ...), so every field shows it without a drawing of its
//! own and Enter sends what is seen, even before the IME has given the syllable up.

use super::*;

impl App {
    /// Is a text field (one the IME types into) taking the keyboard?
    pub(crate) fn ime_field_open(&self) -> bool {
        self.integrations.plugins.as_ref().is_some_and(|p| p.ui.borrow().typing().is_some())
            || self.menus.game_panels.typing()
            || self.sound.milezero.typing.is_some()
            || lan::chat_open(&self.net.remotes)
    }

    /// Once a frame: the IME on while a field has the keyboard, off again for the driving
    /// (the menus' own fields switch it themselves).
    pub(crate) fn sync_ime(&mut self) {
        let menu_field = self.menus.menu_edit_icao || self.menus.menu_edit_number || self.menus.menu_edit_ident;
        let field = self.ime_field_open();
        if !field {
            self.input.ime_preedit = 0;
        }
        let want = field || menu_field;
        if want != self.input.ime_on {
            self.input.ime_on = want;
            if let Some(w) = self.window.as_ref() {
                w.set_ime_allowed(want);
            }
        }
    }

    /// What the IME says: the syllable being composed (put in the field in place of the one
    /// before), the text it gives up (typed).
    pub(crate) fn on_ime(&mut self, ime: winit::event::Ime) {
        use winit::event::Ime;
        match ime {
            Ime::Preedit(text, _) => {
                for _ in 0..std::mem::take(&mut self.input.ime_preedit) {
                    self.ime_type(None);
                }
                if !text.is_empty() && self.ime_type(Some(&text)) {
                    self.input.ime_preedit = text.chars().filter(|c| !c.is_control()).count();
                }
            }
            Ime::Commit(text) => {
                for _ in 0..std::mem::take(&mut self.input.ime_preedit) {
                    self.ime_type(None);
                }
                self.ime_type(Some(&text));
            }
            Ime::Disabled => self.input.ime_preedit = 0,
            Ime::Enabled => {}
        }
    }

    /// `text` typed into the field that has the keyboard (`None`: one character rubbed
    /// out). False when no field took it.
    fn ime_type(&mut self, text: Option<&str>) -> bool {
        if let Some(p) = self.integrations.plugins.as_ref() {
            let mut ui = p.ui.borrow_mut();
            if ui.typing().is_some() {
                ui.type_text(text);
                return true;
            }
        }
        if self.menus.game_panels.typing() {
            self.menus.game_panels.type_text(text);
            return true;
        }
        if let Some(t) = self.sound.milezero.typing.as_mut() {
            match text {
                Some(s) => {
                    if t.chars().count() < 200 {
                        t.extend(s.chars().filter(|c| !c.is_control()));
                    }
                }
                None => {
                    t.pop();
                }
            }
            return true;
        }
        if lan::chat_open(&self.net.remotes) {
            match text {
                Some(s) => lan::chat_type(&mut self.net.remotes, s),
                None => {
                    if let Some(t) = self.net.remotes.chat.typing.as_mut() {
                        t.pop();
                    }
                }
            }
            return true;
        }
        false
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use clap::Parser;
    use winit::event::Ime;

    fn app() -> App {
        crate::assemble_app(crate::Args::parse_from(["openomsi"]), Default::default())
    }

    fn pre(s: &str) -> Ime {
        Ime::Preedit(s.into(), None)
    }

    #[test]
    fn hangul_composed_in_the_chat_line() {
        let mut app = app();
        app.net.remotes.chat.open();
        app.sync_ime();
        assert!(app.input.ime_on);
        // (as Windows' Hangul IME tells it: each change, and an empty one before the commit)
        for ime in [pre("ㅎ"), pre("하"), pre("한")] {
            app.on_ime(ime);
        }
        assert_eq!(app.net.remotes.chat.typing.as_deref(), Some("한"));
        for ime in [pre(""), Ime::Commit("한".into()), pre("ㄱ"), pre("그"), pre("글")] {
            app.on_ime(ime);
        }
        assert_eq!(app.net.remotes.chat.typing.as_deref(), Some("한글"));
        // Backspace in the IME takes the jamo back, the syllable is let go
        for ime in [pre("그"), pre(""), Ime::Commit(" ".into())] {
            app.on_ime(ime);
        }
        assert_eq!(app.net.remotes.chat.typing.as_deref(), Some("한 "));
        app.net.remotes.chat.typing = None;
        app.sync_ime();
        assert!(!app.input.ime_on);
        assert_eq!(app.input.ime_preedit, 0);
    }

    #[test]
    fn nothing_typed_without_a_field() {
        let mut app = app();
        app.on_ime(pre("한"));
        app.on_ime(Ime::Commit("한".into()));
        assert_eq!(app.input.ime_preedit, 0);
        assert!(app.net.remotes.chat.typing.is_none());
    }
}
