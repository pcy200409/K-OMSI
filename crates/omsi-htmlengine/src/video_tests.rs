//! `<video>`: frames from a real mp4 (`tests/data/red_blue.mp4`: 64x48, 10 fps, one second
//! red, then one second blue, H.264 High profile), the clock, looping and the script API.

use crate::tests::{bmp, install_decoder, px};
use super::*;
use std::time::{Duration, Instant};

const CLIP: &[u8] = include_bytes!("../tests/data/red_blue.mp4");

/// A page in a fresh folder holding `clip.mp4` and `poster.bmp` (4x4 green).
fn video_page(name: &str, html: &str, w: u32, h: u32) -> EngineRenderer {
    install_decoder();
    let dir = std::env::temp_dir().join(format!("omsi_htmlvideo_{}_{}", name, std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("clip.mp4"), CLIP).unwrap();
    std::fs::write(dir.join("poster.bmp"), bmp(4, 4, [0, 255, 0])).unwrap();
    let mut r = EngineRenderer::new(w, h, html);
    r.set_asset_dirs(vec![dir]);
    r
}

/// Put the page's clock at `t` seconds after its start.
fn clock_at(r: &mut EngineRenderer, t: f64) {
    r.start = Instant::now() - Duration::from_secs_f64(t);
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Hue {
    Red,
    Blue,
    Green,
    Other,
}

fn hue(p: [u8; 4]) -> Hue {
    match p {
        [r, g, b, 255] if r > 180 && g < 70 && b < 70 => Hue::Red,
        [r, g, b, 255] if b > 180 && r < 70 && g < 70 => Hue::Blue,
        [r, g, b, 255] if g > 180 && r < 70 && b < 70 => Hue::Green,
        _ => Hue::Other,
    }
}

/// The script state of the page's only video.
fn media(r: &EngineRenderer) -> Media {
    r.js.media.values().next().cloned().expect("the page has a video")
}

/// Poll (keeping the clock at `t`) until pixel (x, y) of the last frame has `want`.
fn wait_for(r: &mut EngineRenderer, t: f64, x: u32, y: u32, want: Hue, last: &mut Option<Vec<u8>>) {
    let w = r.width;
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        clock_at(r, t);
        if let Some(f) = r.poll_frame() {
            *last = Some(f);
        }
        if let Some(f) = last.as_deref() {
            if hue(px(f, w, x, y)) == want {
                return;
            }
        }
        assert!(Instant::now() < until, "pixel ({x},{y}) never became {want:?} at t={t}: {:?}", last.as_deref().map(|f| px(f, w, x, y)));
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_page_lays_a_video_out_like_a_picture() {
    // (no decoding needed: the box and what is inside it)
    let r = EngineRenderer::new(400, 200, "<body style='margin:0'><video id=v width=320><source src='x.mp4'>old browser</video></body>");
    let got = with_fonts(|reg, bold| {
        let lay = Layouter { dom: &r.js.dom, reg, bold, vw: 400.0, vh: 200.0, imgs: &*r.imgs };
        let v = r.js.dom.by_id("v").unwrap();
        let st = lay.style_of(v, &Style::default());
        let kid = r.js.dom.nodes[v].kids[0];
        (st.width, st.height, lay.style_of(kid, &st).none, video_src(&r.js.dom, v))
    })
    .unwrap();
    assert_eq!(got.0, Some(Len::Px(320.0)));
    assert_eq!(got.1, Some(Len::Px(160.0)), "300x150 proportions until the frame size is known");
    assert!(got.2, "the fallback inside <video> is not drawn");
    assert_eq!(got.3, "x.mp4");
}

#[test]
fn a_video_without_a_file_shows_its_poster() {
    let html = "<body style='margin:0;background:#000'><video src='missing.mp4' poster='poster.bmp' autoplay width=16 height=8></video></body>";
    let mut r = video_page("poster", html, 20, 10);
    let f = r.poll_frame().unwrap();
    assert_eq!(hue(px(&f, 20, 8, 4)), Hue::Green);
    assert_eq!(px(&f, 20, 18, 4), [0, 0, 0, 255]);
    // nothing plays, so nothing is drawn again
    assert!(r.poll_frame().is_none());
}

#[test]
fn script_api_of_a_video_before_it_plays() {
    let mut r = EngineRenderer::new(
        8,
        8,
        "<body><video id=v src='none.mp4' loop></video><script>var v = document.getElementById('v');
         var a = [v.paused, v.loop, v.autoplay, v.muted, isNaN(v.duration), v.getAttribute('loop')];
         v.play().catch(function () {});
         a.push(v.paused);
         v.currentTime = 3; a.push(v.currentTime);
         v.loop = false; a.push(v.getAttribute('loop'));
         v.pause(); a.push(v.paused);
         var s = a.join(',');</script></body>",
    );
    let s = r.js.global.lock().unwrap().vars.get("s").map(to_str);
    assert_eq!(s.as_deref(), Some("true,true,false,false,true,,false,3,null,true"));
    assert!(r.poll_frame().is_some());
}

#[cfg(windows)]
#[test]
fn a_video_plays_its_frames_at_the_box_size_and_loops() {
    let html = "<body style='margin:0;background:#000'><video src='clip.mp4' autoplay loop muted style='width:32px;height:24px'></video></body>";
    let mut r = video_page("loop", html, 40, 30);
    let mut last = None;
    // (the clock of a video starts when it is first drawn: here at 0)
    wait_for(&mut r, 0.0, 16, 12, Hue::Red, &mut last);
    let f = last.clone().unwrap();
    assert_eq!(px(&f, 40, 36, 12), [0, 0, 0, 255], "the video stays inside its box");
    let sizes: Vec<(u32, u32)> = r.imgs.videos.players.lock().unwrap().values().filter_map(|p| p.frame.as_ref().map(|i| (i.w, i.h))).collect();
    assert_eq!(sizes, vec![(32, 24)], "frames come at the size of the box");
    wait_for(&mut r, 1.4, 16, 12, Hue::Blue, &mut last);
    // the second time round
    wait_for(&mut r, 2.4, 16, 12, Hue::Red, &mut last);
    wait_for(&mut r, 3.5, 16, 12, Hue::Blue, &mut last);
    let f = last.unwrap();
    assert_eq!(px(&f, 40, 36, 12), [0, 0, 0, 255], "only the video was drawn again");
    let m = media(&r);
    assert!(!m.paused && !m.ended);
    assert!((m.duration - 2.0).abs() < 0.15, "duration {}", m.duration);
    assert!((m.time - 1.5).abs() < 0.2, "currentTime {} on the second loop", m.time);
}

#[cfg(windows)]
#[test]
fn a_video_without_loop_ends_on_its_last_frame() {
    let mut r = video_page("end", "<body style='margin:0'><video src='clip.mp4' autoplay width=16 height=12></video></body>", 16, 12);
    let mut last = None;
    wait_for(&mut r, 0.1, 8, 6, Hue::Red, &mut last);
    wait_for(&mut r, 3.0, 8, 6, Hue::Blue, &mut last);
    let until = Instant::now() + Duration::from_secs(10);
    while !media(&r).ended {
        assert!(Instant::now() < until, "never ended");
        clock_at(&mut r, 3.0);
        let _ = r.poll_frame();
        std::thread::sleep(Duration::from_millis(10));
    }
    let m = media(&r);
    assert!(m.paused && m.ended);
    assert!((m.time - 2.0).abs() < 0.15, "currentTime {}", m.time);
    // play() starts it again from the beginning
    r.js.run("document.querySelector('video').play()").unwrap();
    wait_for(&mut r, 3.1, 8, 6, Hue::Red, &mut last);
}

#[cfg(windows)]
#[test]
fn scripts_pause_and_seek_a_video() {
    let html = "<body style='margin:0'><video id=v src='clip.mp4' width=16 height=12></video></body>";
    let mut r = video_page("seek", html, 16, 12);
    let mut last = None;
    // without autoplay the first frame is shown and the clock stands
    wait_for(&mut r, 0.0, 8, 6, Hue::Red, &mut last);
    clock_at(&mut r, 1.5);
    let _ = r.poll_frame();
    assert_eq!((media(&r).paused, media(&r).time), (true, 0.0));
    std::thread::sleep(Duration::from_millis(200));
    clock_at(&mut r, 1.6);
    assert!(r.poll_frame().is_none(), "a paused video draws nothing new");
    // a seek shows the frame of that time though paused
    r.js.run("document.getElementById('v').currentTime = 1.5").unwrap();
    wait_for(&mut r, 1.7, 8, 6, Hue::Blue, &mut last);
    assert!(media(&r).paused);
    // back to the start, then play
    r.js.run("var v = document.getElementById('v'); v.currentTime = 0; v.play()").unwrap();
    wait_for(&mut r, 1.8, 8, 6, Hue::Red, &mut last);
    wait_for(&mut r, 2.9, 8, 6, Hue::Blue, &mut last);
    let t = media(&r).time;
    assert!((t - 1.1).abs() < 0.15, "currentTime {t} after playing 1.1 s");
}



#[cfg(windows)]
#[test]
fn a_video_inside_an_archive_plays_from_memory() {
    // (a mod packed as .zip: the file is no file on disk)
    let dir = std::env::temp_dir().join(format!("omsi_htmlvideo_mem_{}", std::process::id())).join("pack.zip");
    omsi_cfg::vfs::put_memory_file(dir.join("clip.mp4"), Arc::from(CLIP));
    let mut r = EngineRenderer::new(16, 12, "<body style='margin:0'><video src='clip.mp4' autoplay loop width=16 height=12></video></body>");
    r.set_asset_dirs(vec![dir]);
    let mut last = None;
    wait_for(&mut r, 0.0, 8, 6, Hue::Red, &mut last);
    wait_for(&mut r, 1.5, 8, 6, Hue::Blue, &mut last);
}

/// Cost of a real-size video (set `KOMSI_VIDEO` to an mp4): decoding keeps up, and a new
/// frame drawn into the last one takes how long. `cargo test -p omsi-htmlengine --release -- --ignored bench_video --nocapture`
#[cfg(windows)]
#[test]
#[ignore]
fn bench_video() {
    let Some(path) = std::env::var_os("KOMSI_VIDEO") else { return };
    let path = std::path::PathBuf::from(path);
    let html = format!(
        "<body style='margin:0;background:#123'><div style='height:60px;background:#fff;font-size:30px'>next stop</div><video src='{}' autoplay loop muted style='width:857px;height:458px;margin-left:83px'></video><div style='font-size:20px;color:#fff'>Bluecity</div></body>",
        path.file_name().unwrap().to_string_lossy()
    );
    let mut r = EngineRenderer::new(1024, 578, &html);
    r.set_asset_dirs(vec![path.parent().unwrap().to_path_buf()]);
    let started = Instant::now();
    let (mut frames, mut busy) = (0u32, Duration::ZERO);
    while started.elapsed() < Duration::from_secs(10) {
        let t = Instant::now();
        if r.poll_frame().is_some() {
            frames += 1;
            busy += t.elapsed();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let m = media(&r);
    eprintln!("{frames} frames in 10 s ({:.1}/s), {:.2} ms each on the game's thread, video at {:.2}s of {:.2}s", frames as f64 / 10.0, busy.as_secs_f64() * 1000.0 / frames.max(1) as f64, m.time, m.duration);
}

const TONE: &[u8] = include_bytes!("../tests/data/red_tone.mp4");

/// A page whose `tone.mp4` is two seconds of red with a 440 Hz tone.
fn tone_page(name: &str, html: &str) -> EngineRenderer {
    let r = video_page(name, html, 16, 12);
    let dir = r.imgs.dirs[0].clone();
    std::fs::write(dir.join("tone.mp4"), TONE).unwrap();
    r
}

/// Poll until the page's sounds satisfy `ok`.
fn wait_sounds(r: &mut EngineRenderer, t: f64, what: &str, ok: impl Fn(&[omsi_sim::htmltex::PageSound]) -> bool) -> Vec<omsi_sim::htmltex::PageSound> {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        clock_at(r, t);
        let _ = r.poll_frame();
        let s = r.sounds();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < until, "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(windows)]
#[test]
fn a_video_that_is_not_muted_hands_its_sound_to_the_game() {
    let mut r = tone_page("sound", "<body style='margin:0'><video id=v src='tone.mp4' autoplay loop width=16 height=12></video></body>");
    let s = wait_sounds(&mut r, 0.0, "a sound", |s| s.len() == 1);
    let stream = s[0].stream.clone().downcast::<omsi_media::StreamBuf>().expect("a mixer stream");
    let until = Instant::now() + Duration::from_secs(10);
    while stream.buffered() < 0.5 {
        assert!(Instant::now() < until, "the tone was never decoded");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(s[0].gain, 1.0);
    // paused: no sound; playing again: a new stream from where it stands
    r.js.run("document.getElementById('v').pause()").unwrap();
    wait_sounds(&mut r, 0.5, "silence when paused", |s| s.is_empty());
    assert!(stream.is_closed(), "the old stream is closed, its decoder ends");
    r.js.run("var v = document.getElementById('v'); v.play(); v.volume = 0.5").unwrap();
    let again = wait_sounds(&mut r, 0.6, "a sound again", |s| s.len() == 1);
    assert_ne!(again[0].id, s[0].id);
    assert_eq!(again[0].gain, 0.5);
    // muted from a script
    r.js.run("document.getElementById('v').muted = true").unwrap();
    wait_sounds(&mut r, 0.7, "silence when muted", |s| s.is_empty());
}

#[cfg(windows)]
#[test]
fn a_muted_video_has_no_sound() {
    let mut r = tone_page("mute", "<body style='margin:0'><video src='tone.mp4' autoplay loop muted width=16 height=12></video></body>");
    let mut last = None;
    wait_for(&mut r, 0.0, 8, 6, Hue::Red, &mut last);
    for k in 0..20 {
        clock_at(&mut r, 0.1 + k as f64 * 0.05);
        let _ = r.poll_frame();
        assert!(r.sounds().is_empty());
        std::thread::sleep(Duration::from_millis(10));
    }
}
