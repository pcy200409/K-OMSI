//! K-OMSI: a paper ticket's page drawn the way the game draws it, for a look before a ride.
//! `ticket_shot <page.html> <out.rgba> <width> <height> name=text ...` writes the page's
//! picture (RGBA, the ticket lying, `width` x `height`) after its texts are given.

use omsi_sim::htmltex::{HtmlRenderer, PageApi};
use omsi_sim::vehicle_api::ApiValue;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let html = String::from_utf8_lossy(&std::fs::read(&a[1]).expect("page")).into_owned();
    let (w, h): (u32, u32) = (a[3].parse().unwrap(), a[4].parse().unwrap());
    let texts: Vec<(String, String)> = a[5..].iter().filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect();
    let mut r: Box<dyn HtmlRenderer> = if omsi_htmlpage::wanted(&html) {
        Box::new(omsi_htmlpage::EngineRenderer::with_api(w, h, &html, PageApi::Scenery))
    } else {
        Box::new(omsi_htmlengine::EngineRenderer::with_api(w, h, &html, PageApi::Scenery))
    };
    let dir = std::path::Path::new(&a[1]).parent().unwrap().to_path_buf();
    r.set_asset_dirs(vec![dir]);
    r.set_env(&ApiValue::Null);
    r.set_vars(&[], &texts);
    for _ in 0..200 {
        if let Some(px) = r.poll_frame() {
            std::fs::write(&a[2], px).unwrap();
            println!("ok {w}x{h}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    eprintln!("no frame");
    std::process::exit(1);
}
