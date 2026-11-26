//! K-OMSI: a page's own font.
//!
//! ```html
//! <meta name="omsi-font" content="font/Page-Regular.ttf">
//! <meta name="omsi-font-bold" content="font/Page-Bold.ttf">
//! ```
//!
//! The same two `<meta>` elements the classic engine reads. The files are looked up like the
//! page's pictures (next to the page first). Every character the font has is drawn with it -
//! Latin and digits too - and the rest falls back as before (Inter, then the game's fallback
//! face). Without `omsi-font-bold` bold text uses the same file (its `wght` axis at 700 when
//! it is a variable font). A page whose font was found sees `omsi.pageFont === true`, so it
//! can pick sizes that fit that font and others for the game's own faces (an older game, a
//! missing file).
//!
//! A page lives on a thread of its own, so the font is that thread's: other pages keep the
//! faces they had. The bytes of a font file are read once per process and kept (a handful of
//! files in a session).

use super::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// Largest font file that is read (bytes).
const MAX_FONT_BYTES: usize = 48 * 1024 * 1024;
/// Face numbers 0 and 1 are Inter and the game's fallback (see `face_for`).
pub(crate) const FACE: u8 = 2;

static FILES: Mutex<Vec<(PathBuf, &'static [u8])>> = Mutex::new(Vec::new());

thread_local! {
    /// The font of the page on this thread: (regular, bold).
    static PAGE: RefCell<Option<(FontRef<'static>, FontRef<'static>)>> = const { RefCell::new(None) };
}

fn read(path: &Path) -> Option<&'static [u8]> {
    let mut files = FILES.lock().unwrap();
    if let Some((_, b)) = files.iter().find(|(p, _)| p == path) {
        return Some(b);
    }
    let bytes = omsi_cfg::vfs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_FONT_BYTES || FontRef::try_from_slice(&bytes).is_err() {
        log::warn!("htmltexture: font {} cannot be used", path.display());
        return None;
    }
    let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    files.push((path.to_path_buf(), bytes));
    Some(bytes)
}

fn find(dirs: &[PathBuf], rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() || rel.contains("://") {
        return None;
    }
    dirs.iter().map(|d| omsi_cfg::resolve_path(d, rel)).find(|p| omsi_cfg::vfs::is_file(p))
}

/// Read the font the page names (looked up in `dirs`, in order) and make it this thread's
/// page font; true when there is one now.
pub(crate) fn load(dom: &Dom, dirs: &[PathBuf]) -> bool {
    let meta = |name: &str| {
        dom.nodes.iter().find_map(|n| {
            let attr = |k: &str| n.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.trim());
            (n.tag == "meta" && attr("name").is_some_and(|v| v.eq_ignore_ascii_case(name))).then(|| attr("content").unwrap_or("").to_string())
        })
    };
    let faces = meta("omsi-font").and_then(|rel| {
        let Some(path) = find(dirs, &rel) else {
            log::warn!("htmltexture: font {rel} not found");
            return None;
        };
        let reg = FontRef::try_from_slice(read(&path)?).ok()?;
        let bold_path = meta("omsi-font-bold").and_then(|rel| find(dirs, &rel));
        let bold = match bold_path.as_deref().and_then(read).and_then(|b| FontRef::try_from_slice(b).ok()) {
            Some(b) => b,
            None => {
                let mut b = reg.clone();
                b.set_variation(b"wght", 700.0);
                b
            }
        };
        log::info!("htmltexture: page font {}{}", path.display(), if bold_path.is_some() { " (with its bold file)" } else { "" });
        Some((reg, bold))
    });
    let found = faces.is_some();
    PAGE.with(|p| *p.borrow_mut() = faces);
    found
}

/// Run `f` with the page font's face for `ch` (its glyph and face number) when the page has a
/// font and the font has the character; else hand `f` back.
pub(crate) fn with_face<R, F: FnOnce(&FontRef<'static>, ab_glyph::GlyphId, u8) -> R>(bold: bool, ch: char, f: F) -> Result<R, F> {
    PAGE.with(|p| {
        let p = p.borrow();
        let Some((reg, b)) = p.as_ref() else { return Err(f) };
        let face = if bold { b } else { reg };
        let gid = face.glyph_id(ch);
        if gid.0 == 0 {
            return Err(f);
        }
        Ok(f(face, gid, FACE))
    })
}
