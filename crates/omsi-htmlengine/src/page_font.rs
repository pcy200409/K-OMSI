//! K-OMSI: a page's own font.
//!
//! ```html
//! <meta name="omsi-font" content="fonts/NotoSansKR-Regular.ttf">
//! <meta name="omsi-font-bold" content="fonts/NotoSansKR-Bold.ttf">
//! ```
//!
//! The files are looked up like the page's pictures (next to the page first). Every character
//! the font has is drawn with it - Latin and digits too - and the rest falls back as before
//! (Inter, then the game's fallback face). Without `omsi-font-bold` bold text uses the same
//! file (its `wght` axis at 700 when it is a variable font). A page whose font was found sees
//! `omsi.pageFont === true`, so it can pick sizes that fit that font and others for the game's
//! own faces (an older game, a missing file).
//!
//! The font is the page's: other pages keep the faces they had. The bytes of a font file are
//! read once per process and kept (a handful of files in a session).

use super::*;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

/// Largest font file that is read (bytes).
const MAX_FONT_BYTES: usize = 48 * 1024 * 1024;
/// Face numbers 0 and 1 are Inter and the game's fallback (see `face_for`); page fonts follow.
const FIRST_ID: usize = 2;

struct Loaded {
    reg_path: PathBuf,
    bold_path: Option<PathBuf>,
    reg: &'static [u8],
    bold: Option<&'static [u8]>,
}

static FILES: Mutex<Vec<Loaded>> = Mutex::new(Vec::new());

thread_local! {
    /// The page font of the page being laid out or painted (0 = none).
    static CURRENT: Cell<u8> = const { Cell::new(0) };
    /// The parsed faces of the page fonts this thread has used: (regular, bold).
    static FACES: RefCell<HashMap<u8, Option<(FontRef<'static>, FontRef<'static>)>>> = RefCell::new(HashMap::new());
}

fn read(path: &Path) -> Option<&'static [u8]> {
    let bytes = omsi_cfg::vfs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_FONT_BYTES || FontRef::try_from_slice(&bytes).is_err() {
        log::warn!("htmltexture: font {} cannot be used", path.display());
        return None;
    }
    Some(Box::leak(bytes.into_boxed_slice()))
}

/// The number of the page font made of these files, read when they are new; `None` when the
/// regular file cannot be used (a bold file that cannot is left out).
pub(crate) fn load(reg: &Path, bold: Option<&Path>) -> Option<u8> {
    let mut files = FILES.lock().unwrap();
    if let Some(i) = files.iter().position(|l| l.reg_path == reg && l.bold_path.as_deref() == bold) {
        return Some((FIRST_ID + i) as u8);
    }
    if FIRST_ID + files.len() >= u8::MAX as usize {
        log::warn!("htmltexture: too many page fonts, {} is not used", reg.display());
        return None;
    }
    let reg_bytes = read(reg)?;
    let bold_bytes = bold.and_then(read);
    files.push(Loaded { reg_path: reg.to_path_buf(), bold_path: bold.map(Path::to_path_buf), reg: reg_bytes, bold: bold_bytes });
    log::info!("htmltexture: page font {}{}", reg.display(), if bold_bytes.is_some() { " (with its bold file)" } else { "" });
    Some((FIRST_ID + files.len() - 1) as u8)
}

fn parse(id: u8) -> Option<(FontRef<'static>, FontRef<'static>)> {
    let files = FILES.lock().unwrap();
    let l = files.get((id as usize).checked_sub(FIRST_ID)?)?;
    let reg = FontRef::try_from_slice(l.reg).ok()?;
    let bold = match l.bold {
        Some(b) => FontRef::try_from_slice(b).ok()?,
        None => {
            let mut b = reg.clone();
            b.set_variation(b"wght", 700.0);
            b
        }
    };
    Some((reg, bold))
}

/// While it lives, text is laid out and drawn with page font `id`.
pub(crate) struct Scope(u8);

impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.0));
    }
}

pub(crate) fn scope(id: Option<u8>) -> Scope {
    let id = id.unwrap_or(0);
    if id != 0 {
        FACES.with(|f| {
            f.borrow_mut().entry(id).or_insert_with(|| parse(id));
        });
    }
    Scope(CURRENT.with(|c| c.replace(id)))
}

/// Run `f` with the current page font's face for `ch` (its glyph and face number) when the
/// page has a font and the font has the character; else hand `f` back.
pub(crate) fn with_face<R, F: FnOnce(&FontRef<'static>, ab_glyph::GlyphId, u8) -> R>(bold: bool, ch: char, f: F) -> Result<R, F> {
    let id = CURRENT.with(|c| c.get());
    if id == 0 {
        return Err(f);
    }
    FACES.with(|faces| {
        let faces = faces.borrow();
        let Some(Some((reg, b))) = faces.get(&id) else { return Err(f) };
        let face = if bold { b } else { reg };
        let gid = face.glyph_id(ch);
        if gid.0 == 0 {
            return Err(f);
        }
        Ok(f(face, gid, id))
    })
}

/// `content` of the first `<meta name="{name}">` of the page.
pub(crate) fn meta_content(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<meta") {
        let start = at + i;
        let end = start + lower[start..].find('>')?;
        let tag = &html[start..end];
        if attr(tag, "name").is_some_and(|n| n.trim().eq_ignore_ascii_case(name)) {
            return attr(tag, "content").map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        }
        at = end;
    }
    None
}

/// The value of attribute `name` in a tag (`name="v"`, `name='v'` or `name=v`).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let k = from + i;
        from = k + name.len();
        if k > 0 && !lower.as_bytes()[k - 1].is_ascii_whitespace() {
            continue;
        }
        let rest = tag[from..].trim_start();
        let Some(v) = rest.strip_prefix('=') else { continue };
        let v = v.trim_start();
        return match v.chars().next()? {
            q @ ('"' | '\'') => v[1..].find(q).map(|e| v[1..1 + e].to_string()),
            _ => Some(v.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_string()),
        };
    }
    None
}

#[cfg(test)]
#[path = "page_font_tests.rs"]
mod tests;
