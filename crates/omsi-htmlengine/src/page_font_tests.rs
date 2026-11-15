use super::*;

const SHOW: &str = "<body><p id=a>text</p><script>\
    window.omsi = window.omsi || {};\
    window.omsi.update = function () { document.getElementById('a').textContent = omsi.pageFont === true ? 'yes' : 'no'; };\
    </script></body>";

/// A page in a fresh folder; with `font`, `fonts/page.ttf` is there (Inter's own file: any
/// font does for what these tests look at).
fn page(name: &str, head: &str, font: bool) -> EngineRenderer {
    let dir = std::env::temp_dir().join(format!("omsi_pagefont_{}_{}", name, std::process::id()));
    std::fs::create_dir_all(dir.join("fonts")).unwrap();
    if font {
        std::fs::write(dir.join("fonts").join("page.ttf"), INTER).unwrap();
    }
    let mut r = EngineRenderer::new(120, 40, &format!("<head>{head}</head>{SHOW}"));
    r.set_asset_dirs(vec![dir]);
    r.set_vars(&[], &[]);
    r
}

#[test]
fn the_meta_tags_are_read_in_any_quoting() {
    let html = "<head><meta charset=\"utf-8\"><META NAME='omsi-font' content=\"fonts/a b.ttf\"><meta content=x.ttf name=omsi-font-bold></head>";
    assert_eq!(page_font::meta_content(html, "omsi-font").as_deref(), Some("fonts/a b.ttf"));
    assert_eq!(page_font::meta_content(html, "omsi-font-bold").as_deref(), Some("x.ttf"));
    assert_eq!(page_font::meta_content(html, "omsi-engine"), None);
    assert_eq!(page_font::meta_content("<meta name=\"omsi-font\" content=\"\">", "omsi-font"), None);
}

#[test]
fn a_page_with_its_font_is_told_so() {
    let r = page("found", "<meta name=\"omsi-font\" content=\"fonts/page.ttf\">", true);
    assert!(r.page_font.is_some());
    assert_eq!(r.text_of("a").as_deref(), Some("yes"));
}

#[test]
fn a_missing_font_file_or_no_meta_leaves_the_page_as_it_was() {
    let r = page("missing", "<meta name=\"omsi-font\" content=\"fonts/page.ttf\">", false);
    assert!(r.page_font.is_none());
    assert_eq!(r.text_of("a").as_deref(), Some("no"));
    let r = page("nometa", "", true);
    assert!(r.page_font.is_none());
    assert_eq!(r.text_of("a").as_deref(), Some("no"));
}

#[test]
fn a_file_that_is_no_font_is_not_used() {
    let dir = std::env::temp_dir().join(format!("omsi_pagefont_bad_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bad.ttf"), b"this is no font").unwrap();
    let mut r = EngineRenderer::new(40, 40, "<head><meta name=\"omsi-font\" content=\"bad.ttf\"></head><body>x</body>");
    r.set_asset_dirs(vec![dir]);
    assert!(r.page_font.is_none());
    assert!(r.poll_frame().is_some(), "the page is drawn all the same");
}

#[test]
fn the_page_font_draws_only_while_its_page_is_drawn() {
    let mut r = page("scope", "<meta name=\"omsi-font\" content=\"fonts/page.ttf\">", true);
    let id = r.page_font.unwrap();
    assert!(id >= 2, "0 and 1 are Inter and the game's fallback");
    {
        let _s = page_font::scope(Some(id));
        let got = page_font::with_face(false, 'A', |_, g, face| (g.0 != 0, face));
        assert!(matches!(got, Ok((true, f)) if f == id));
        // a character the font lacks goes on to the faces the page had before
        assert!(page_font::with_face(false, '\u{AC00}', |_, _, _| ()).is_err());
    }
    assert!(page_font::with_face(false, 'A', |_, _, _| ()).is_err(), "outside the page nothing changes");
    // drawing the page leaves no page font behind for the next page of the thread
    assert!(r.poll_frame().is_some());
    assert!(page_font::with_face(false, 'A', |_, _, _| ()).is_err());
}

#[test]
fn the_same_files_are_one_page_font() {
    let a = page("same", "<meta name=\"omsi-font\" content=\"fonts/page.ttf\">", true);
    let dir = std::env::temp_dir().join(format!("omsi_pagefont_same_{}", std::process::id()));
    let mut b = EngineRenderer::new(40, 40, "<head><meta name=\"omsi-font\" content=\"fonts/page.ttf\"></head><body>x</body>");
    b.set_asset_dirs(vec![dir]);
    assert_eq!(a.page_font, b.page_font);
}
