use super::*;
use omsi_sim::vehicle_api::ApiValue as AutoValue;

fn map(items: Vec<(&str, AutoValue)>) -> AutoValue {
    AutoValue::Map(items.into_iter().map(|(k, v)| (k.into(), v)).collect())
}
fn click(r: &mut EngineRenderer, x: f32, y: f32) {
    r.pointer(x, y, PointerKind::Down);
    r.pointer(x, y, PointerKind::Up);
}

fn auto_fixture() -> AutoValue {
    use AutoValue as A;
    let stop = |name: &str, served: bool| map(vec![("name", A::Str(name.into())), ("served", A::Bool(served))]);
    map(vec![
        ("motion", map(vec![("speedKmh", A::Num(42.0)), ("heading", A::Num(85.0)), ("x", A::Num(120.5)), ("y", A::Num(-64.0))])),
        ("doors", map(vec![("anyOpen", A::Bool(false))])),
        ("route", map(vec![("line", A::Str("1611".into())), ("destination", A::Str("Central Terminal".into())), ("active", A::Bool(true)), ("source", A::Str("timetable".into())), ("nextIndex", A::Num(0.0)), ("delaySec", A::Num(18.0)),
            ("current", map(vec![("name", A::Str("City Hall".into())), ("arrival", A::Str("14:32".into()))])),
            ("stops", A::List(vec![stop("City Hall", false), stop("Riverside", false), stop("Central Terminal", false)]))])),
        ("media", map(vec![("text", A::Str("TRACK 03  2:14".into())), ("powerTrigger", A::Str("audio_mode".into())), ("powered", A::Bool(true)), ("volumeVar", A::Str("audio_vol".into())), ("volume", A::Num(6.0)),
            ("playlists", A::List((1..=3).map(|i| A::Str(format!("audio_playlist{i}"))).collect()))])),
    ])
}

fn auto_frame(r: &mut EngineRenderer, name: &str) {
    let frame = r.poll_frame().expect("page must redraw");
    assert_eq!(frame.len(), 800 * 480 * 4);
    // Export the actual built-in renderer's pixels when visual review is requested.
    if let Ok(dir) = std::env::var("K_OMSI_AA_PREVIEW_DIR") {
        let mut bmp = vec![0u8; 54];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&(54u32 + frame.len() as u32).to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&800i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&(-480i32).to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&32u16.to_le_bytes());
        for p in frame.chunks_exact(4) { bmp.extend_from_slice(&[p[2], p[1], p[0], 255]); }
        std::fs::write(std::path::Path::new(&dir).join(format!("android-auto-{name}.bmp")), bmp).unwrap();
    }
}

#[test]
fn android_auto_pages_telemetry_controls_and_cctv() {
    let html = include_str!("../../../docs/examples/android-auto/index.html");
    let mut r = EngineRenderer::new(800, 480, html);
    r.set_vehicle(&auto_fixture());
    r.set_env(&map(vec![("time", map(vec![("asString", AutoValue::Str("14:30:12".into()))]))]));
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("speed").as_deref(), Some("42"));
    assert_eq!(r.text_of("stop").as_deref(), Some("City Hall"));
    assert_eq!(r.text_of("clock").as_deref(), Some("14:30"));
    auto_frame(&mut r, "home");
    click(&mut r, 235.0, 420.0);
    assert_eq!(r.js.dom.nodes[r.js.dom.by_id("nav").unwrap()].inline.iter().find(|(k, _)| k == "display").map(|(_, v)| v.as_str()), Some("block"));
    assert_eq!(r.text_of("following1").as_deref(), Some("01   Riverside"));
    assert_eq!(r.text_of("following2").as_deref(), Some("02   Central Terminal"));
    assert_eq!(r.text_of("coordinates").as_deref(), Some("X  120.5 m   Y  -64.0 m"));
    auto_frame(&mut r, "nav");
    let mut ibis = auto_fixture();
    if let AutoValue::Map(items) = &mut ibis {
        if let AutoValue::Map(route) = &mut items.iter_mut().find(|(k, _)| k == "route").unwrap().1 {
            for (key, value) in route {
                match key.as_str() {
                    "source" => *value = AutoValue::Str("ibis".into()),
                    "nextIndex" => *value = AutoValue::Null,
                    "current" => *value = map(vec![("name", AutoValue::Str("Riverside".into()))]),
                    _ => {}
                }
            }
        }
    }
    r.set_vehicle(&ibis);
    r.set_vars(&[], &[]);
    assert_eq!(r.text_of("following1").as_deref(), Some("01   Central Terminal"));
    r.set_vehicle(&auto_fixture());
    r.set_vars(&[], &[]);
    click(&mut r, 400.0, 420.0);
    auto_frame(&mut r, "music");
    click(&mut r, 85.0, 260.0);
    assert_eq!(r.take_triggers(), vec!["audio_mode".to_string()]);
    click(&mut r, 550.0, 140.0);
    assert_eq!(r.take_triggers(), vec!["audio_playlist1".to_string()]);
    click(&mut r, 290.0, 260.0);
    assert_eq!(r.take_events(), vec![("audio_vol".into(), 7.0)]);
    click(&mut r, 580.0, 420.0);
    assert_eq!(r.take_events(), vec![("k_omsi_android_auto".into(), 0.0)]);
}

#[test]
fn android_auto_empty_data_and_incremental_vars_are_honest() {
    let mut r = EngineRenderer::new(800, 480, include_str!("../../../docs/examples/android-auto/index.html"));
    r.set_vars(&[("Velocity".into(), -12.0)], &[("act_route".into(), "1611".into())]);
    r.set_vars(&[("Velocity".into(), 15.0)], &[]);
    assert_eq!(r.text_of("speed").as_deref(), Some("15"));
    assert_eq!(r.text_of("route").as_deref(), Some("1611"));
    assert_eq!(r.text_of("stop").as_deref(), Some("No active route"));
    assert_eq!(r.text_of("mediaStatus").as_deref(), Some("Controls unavailable on this vehicle"));
    auto_frame(&mut r, "empty");
    click(&mut r, 580.0, 420.0);
    assert_eq!(r.take_events(), vec![("k_omsi_android_auto".into(), 0.0)]);
}
