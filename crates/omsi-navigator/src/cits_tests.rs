use super::cits::*;
use super::*;
use omsi_scene::{CameraKind, SpeedCamera};
use omsi_sim::traffic::Aspect;

/// A straight road north, 3 km at 50 km/h.
fn road() -> Network {
    let mut l = omsi_sim::traffic::LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::new(0.0, 3000.0, 0.0)], LaneKind::Street, 3.0);
    l.speed_limit_kmh = 50.0;
    let mut net = Network { lanes: vec![l], ..Default::default() };
    net.link(1.5);
    net
}

fn nav(cameras: Vec<SpeedCamera>) -> Navigator {
    let mut n = Navigator::new(true, 0.85, "bottom-left");
    n.global = Some(std::sync::Arc::new(road()));
    n.cits.cameras = std::sync::Arc::new(cameras);
    n.speed_limit = Some(50.0);
    n
}

fn cam(x: f64, y: f64, kind: CameraKind) -> SpeedCamera {
    SpeedCamera { position: DVec3::new(x, y, 0.0), heading: 180.0, kind, limit: None }
}

fn at(y: f64, kmh: f32) -> NavFrame<'static> {
    NavFrame {
        traffic: None,
        touch: false,
        players: vec![],
        bus: DVec3::new(0.0, y, 0.0),
        heading: 0.0,
        speed_kmh: kmh,
        outside_temp: 20.0,
        inside_temp: 20.0,
        line: None,
        terminus: None,
        stops: vec![],
        delay: None,
        passengers: None,
        stop_requested: false,
        time: 0.0,
        weekday: 0,
        date: 20261020,
        language: "ENG",
        screen: (1280.0, 720.0),
        ui_scale: 1.0,
        follow_window: false,
        dt: 0.3,
        info_rect: None,
        touch_gap: None,
    }
}

#[test]
fn a_camera_is_announced_from_a_kilometre_and_flashes_while_too_fast() {
    let mut n = nav(vec![cam(6.0, 1500.0, CameraKind::Speed)]);
    n.update_cits(&at(0.0, 45.0));
    assert_eq!(n.cits.camera, None, "1.5 km away: not yet");
    n.update_cits(&at(700.0, 45.0));
    let c = n.cits.camera.clone().expect("800 m ahead");
    assert!((c.dist - 800.0).abs() < 1.0 && c.limit == Some(50.0) && c.tag == Some("Speeding"), "{c:?}");
    assert!(c.flash, "a new stage flashes a moment");
    // (the moment over: steady while within the limit)
    for _ in 0..8 {
        n.update_cits(&at(710.0, 45.0));
    }
    let c = n.cits.camera.clone().unwrap();
    assert!(!c.flash && !c.over);
    // 500 m: announced again
    n.update_cits(&at(1050.0, 45.0));
    assert!(n.cits.camera.as_ref().unwrap().flash);
    // too fast: flashing for as long as it is
    for _ in 0..8 {
        n.update_cits(&at(1250.0, 62.0));
    }
    let c = n.cits.camera.clone().unwrap();
    assert!(c.flash && c.over, "{c:?}");
    // past it: gone
    n.update_cits(&at(1520.0, 45.0));
    assert_eq!(n.cits.camera, None);
    // (none of it is a warning of the band)
    assert!(n.cits.alerts.is_empty(), "{:?}", n.cits.alerts);
}

#[test]
fn a_signal_camera_says_so_on_its_tab() {
    let mut n = nav(vec![cam(6.0, 400.0, CameraKind::SignalSpeed)]);
    n.update_cits(&at(0.0, 45.0));
    assert_eq!(n.cits.camera.as_ref().and_then(|c| c.tag), Some("Signal and speed"));
}

#[test]
fn a_camera_for_the_other_carriageway_is_left_alone() {
    let mut n = nav(vec![cam(-9.0, 600.0, CameraKind::Speed)]);
    n.update_cits(&at(0.0, 45.0));
    assert_eq!(n.cits.camera, None);
}

#[test]
fn an_average_speed_check_shows_the_average_between_its_ends() {
    let mut n = nav(vec![cam(6.0, 300.0, CameraKind::SectionStart), cam(6.0, 2400.0, CameraKind::SectionEnd)]);
    // 72 km/h = 20 m/s, 6 m a step
    let mut y = 0.0;
    while y < 1500.0 {
        n.update_cits(&at(y, 72.0));
        y += 6.0;
    }
    let a = n.cits.average.expect("inside the check");
    assert!((a.kmh - 72.0).abs() < 1.0 && a.limit == Some(50.0) && a.over, "{a:?}");
    while y < 2600.0 {
        n.update_cits(&at(y, 40.0));
        y += 6.0;
    }
    assert_eq!(n.cits.average, None, "past its end");
}

#[test]
fn the_light_ahead_says_how_to_meet_it() {
    let v = 50.0 / 3.6;
    // green long enough: through
    assert_eq!(light_advice(Aspect::Green, 12.0, 0.0, 100.0, v, 50.0), LightAdvice::Pass);
    // green for 3 s more, 100 m off: too far to make it, room to stop
    assert_eq!(light_advice(Aspect::Green, 3.0, 0.0, 100.0, v, 50.0), LightAdvice::Stop);
    // green for 9 s, 120 m off at 50: a little faster makes it, within the limit
    assert!(matches!(light_advice(Aspect::Green, 9.0, 0.0, 120.0, 12.0, 60.0), LightAdvice::Speed(s) if s > 43.0 && s < 60.0));
    // red for 20 s more, 100 m off: 18 km/h meets the green
    assert!(matches!(light_advice(Aspect::Red, 20.0, 20.0, 100.0, v, 50.0), LightAdvice::Speed(s) if (s - 18.0).abs() < 0.5));
    // red 20 m ahead at 50: it cannot stop
    assert_eq!(light_advice(Aspect::Red, 20.0, 20.0, 20.0, v, 50.0), LightAdvice::Violation);
    // standing at the line: nothing to say
    assert_eq!(light_advice(Aspect::Red, 20.0, 20.0, 5.0, 0.0, 50.0), LightAdvice::Pass);
}

#[test]
fn the_gap_to_the_car_ahead_warns_when_it_closes() {
    let v = 50.0 / 3.6;
    assert_eq!(gap_level(60.0, v, v), None);
    assert_eq!(gap_level(14.0, v, v), Some(Level::Caution));
    assert_eq!(gap_level(20.0, v, 0.0), Some(Level::Danger));
    assert_eq!(gap_level(5.0, 2.0, 0.0), None, "crawling in a queue");
}

#[test]
fn curves_stages_and_the_way_ahead() {
    assert_eq!(curve_speed(50.0), 30.0);
    assert_eq!(curve_speed(400.0), 95.0);
    assert_eq!(camera_stage(1200.0), None);
    assert_eq!(camera_stage(800.0), Some(0));
    assert_eq!(camera_stage(400.0), Some(1));
    assert_eq!(camera_stage(250.0), Some(2));
    let net = road();
    let p = path_ahead(&net, None, DVec3::new(0.0, 100.0, 0.0), 0.0, 1000.0);
    assert_eq!(p.len(), 1);
    assert!((p[0].1 + 100.0).abs() < 0.5, "{p:?}");
    assert!(on_path(&net, &p, DVec3::new(5.0, 600.0, 0.0)).is_some_and(|(d, _)| (d - 500.0).abs() < 0.5));
}

#[test]
fn cameras_are_known_by_their_objects() {
    use omsi_scene::camera_kind;
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\speedcctv.sco"), Some(CameraKind::Speed));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\camera.sco"), Some(CameraKind::Unspecified));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\trafficlight_speedcctv.sco"), Some(CameraKind::SignalSpeed));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\areacctvstart.sco"), Some(CameraKind::SectionStart));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\areacctvend.sco"), Some(CameraKind::SectionEnd));
    assert_eq!(camera_kind("Sceneryobjects\\Korean Sceneryobject\\Speedcam_Pole\\Steel(Std Type)\\SteelGrey\\interval_spdcampole_6m.sco"), Some(CameraKind::SectionPoint));
    assert_eq!(camera_kind("Sceneryobjects\\Korean Sceneryobject\\Speedcam_Pole\\Steel(Std Type)\\SteelGrey\\spd+tflcampole_6m.sco"), Some(CameraKind::SignalSpeed));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Standard Set\\Ampel Mast(Traffic light Pole)\\Gray(New)\\KOR_AmpleMast_Camera_S.sco"), Some(CameraKind::Unspecified));
    assert_eq!(camera_kind("Sceneryobjects\\Korean Sceneryobject\\StreetFurniture\\speedcam_type3.sco"), Some(CameraKind::Speed));
    assert_eq!(camera_kind("Sceneryobjects\\Blitzer\\Blitzer_Saeule.sco"), Some(CameraKind::Speed));
    assert_eq!(camera_kind("Sceneryobjects\\Buildings_Freeman\\speedcamsign.sco"), None);
    assert_eq!(camera_kind("Sceneryobjects\\Buildings_Freeman\\speedcamerasign2.sco"), None);
    assert_eq!(camera_kind("Sceneryobjects\\Korean Sceneryobject\\Speedbump\\speedbump_14m.sco"), None);
    assert_eq!(omsi_scene::camera_limit("x\\speedcam_60kmh.sco", &[]), Some(60.0));
    assert_eq!(omsi_scene::camera_limit("x\\spd+tflcampole_6m.sco", &[]), None);
}

/// A camera beside the right-hand lane is the bus's in the left-hand lane too, and a camera
/// for the other carriageway is not, even on a wide road.
#[test]
fn a_camera_beside_the_right_hand_lane_counts_from_the_other_lanes() {
    let lane = |x: f64| {
        let mut l = omsi_sim::traffic::LaneBuilder::polyline(vec![DVec3::new(x, 0.0, 0.0), DVec3::new(x, 1000.0, 0.0)], LaneKind::Street, 3.0);
        l.speed_limit_kmh = 50.0;
        l
    };
    let mut net = Network { lanes: vec![lane(0.0), lane(3.5), lane(7.0), lane(-7.0)], ..Default::default() };
    net.link(1.5);
    // (one spline's paths beside each other: the left one 0, then 1, then 2 on the right)
    // (the camera stands 19 m right of lane 0: too far for it, within reach of the lanes beside it)
    net.lanes[0].right = Some(1);
    net.lanes[1].left = Some(0);
    net.lanes[1].right = Some(2);
    net.lanes[2].left = Some(1);
    let path = path_ahead(&net, None, DVec3::new(0.0, 100.0, 0.0), 0.0, 1000.0);
    assert_eq!(path.first().map(|p| p.0), Some(0));
    let lanes = corridor(&net, &path);
    let right = on_path(&net, &lanes, DVec3::new(19.0, 600.0, 0.0));
    assert!(right.is_some_and(|(d, l)| (d - 500.0).abs() < 0.5 && l != 0), "{right:?}");
    assert_eq!(on_path(&net, &lanes, DVec3::new(-11.0, 600.0, 0.0)), None);
}

/// At a junction every path starts the way the road ends: the right turn's light is not the
/// bus's while it goes straight on, and is while its route turns right.
#[test]
fn the_way_at_a_junction_is_the_one_the_bus_takes() {
    let road = omsi_sim::traffic::LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::new(0.0, 100.0, 0.0)], LaneKind::Street, 3.0);
    let straight = omsi_sim::traffic::LaneBuilder::polyline(vec![DVec3::new(0.0, 100.0, 0.0), DVec3::new(0.0, 130.0, 0.0)], LaneKind::Street, 3.0);
    // (a right turn: on north a few metres, then east)
    let right = omsi_sim::traffic::LaneBuilder::polyline(vec![DVec3::new(0.0, 100.0, 0.0), DVec3::new(0.0, 103.0, 0.0), DVec3::new(6.0, 112.0, 0.0), DVec3::new(30.0, 115.0, 0.0)], LaneKind::Street, 3.0);
    let mut net = Network { lanes: vec![road, right, straight], ..Default::default() };
    net.link(1.5);
    net.lanes[0].next = vec![1, 2];
    let way = path_along(&net, None, DVec3::new(0.0, 20.0, 0.0), 0.0, 200.0, &[]);
    assert_eq!(way.iter().map(|w| w.0).collect::<Vec<_>>(), vec![0, 2]);
    let guide = [DVec3::new(10.0, 113.0, 0.0), DVec3::new(25.0, 115.0, 0.0)];
    let way = path_along(&net, None, DVec3::new(0.0, 20.0, 0.0), 0.0, 200.0, &guide);
    assert_eq!(way.iter().map(|w| w.0).collect::<Vec<_>>(), vec![0, 1]);
}

/// A camera alone, with no plate by it, watches the red light where it stands by the way's
/// stop line, else the speed; a plate says for itself.
#[test]
fn a_camera_alone_watches_the_light_it_stands_by() {
    assert_eq!(camera_watches(CameraKind::Unspecified, 300.0, &[260.0]), CameraKind::Signal);
    assert_eq!(camera_watches(CameraKind::Unspecified, 300.0, &[320.0]), CameraKind::Signal);
    assert_eq!(camera_watches(CameraKind::Unspecified, 300.0, &[120.0, 600.0]), CameraKind::Speed);
    assert_eq!(camera_watches(CameraKind::Unspecified, 300.0, &[]), CameraKind::Speed);
    assert_eq!(camera_watches(CameraKind::SignalSpeed, 300.0, &[]), CameraKind::SignalSpeed);
    assert_eq!(camera_watches(CameraKind::Speed, 300.0, &[300.0]), CameraKind::Speed);
    // the plate by a camera box says what it watches
    let mut n = nav(vec![cam(6.0, 400.0, CameraKind::Unspecified), cam(6.5, 401.0, CameraKind::SignalSpeed)]);
    n.update_cits(&at(0.0, 45.0));
    assert_eq!(n.cits.camera.as_ref().and_then(|c| c.tag), Some("Signal and speed"));
    // alone (no traffic: no stop line known), the speed
    let mut n = nav(vec![cam(6.0, 400.0, CameraKind::Unspecified)]);
    n.update_cits(&at(0.0, 45.0));
    assert_eq!(n.cits.camera.as_ref().and_then(|c| c.tag), Some("Speeding"));
}

/// Korean crossing sets put a stop line's light on a path of its own, a metre long, beside
/// the junction's path that goes on the same way. The way may take either: the light is the
/// bus's all the same, and the left turn's light beside them is not while it goes straight.
#[test]
fn a_light_on_a_path_beside_the_way_is_the_ways() {
    let p = |pts: &[(f64, f64)]| omsi_sim::traffic::LaneBuilder::polyline(pts.iter().map(|&(x, y)| DVec3::new(x, y, 0.0)).collect(), LaneKind::Street, 3.0);
    let mut stop = p(&[(0.0, 100.0), (0.0, 101.0)]);
    stop.traffic_light = Some((0, 0));
    let mut left = p(&[(0.0, 100.0), (0.0, 104.0), (-6.0, 112.0), (-30.0, 115.0)]);
    left.traffic_light = Some((0, 1));
    let lanes = vec![p(&[(0.0, 0.0), (0.0, 100.0)]), p(&[(0.0, 100.0), (0.0, 130.0)]), stop, left, p(&[(0.0, 130.0), (0.0, 400.0)])];
    let mut net = Network { lanes, ..Default::default() };
    net.link(1.5);
    // (the junction's path first: of two that turn as little, the way takes the first)
    net.lanes[0].next = vec![1, 3, 2];
    let way = path_along(&net, None, DVec3::new(0.0, 20.0, 0.0), 0.0, 300.0, &[]);
    assert_eq!(way.iter().map(|w| w.0).take(2).collect::<Vec<_>>(), vec![0, 1]);
    let lit = lights_on_way(&net, &way);
    assert_eq!(lit.len(), 1, "{lit:?}");
    assert_eq!(lit[0].0, 2);
    assert!((lit[0].1 - 80.0).abs() < 0.01, "{lit:?}");
    // the light on the way itself
    net.lanes[0].next = vec![2, 1, 3];
    let way = path_along(&net, None, DVec3::new(0.0, 20.0, 0.0), 0.0, 300.0, &[]);
    assert_eq!(lights_on_way(&net, &way).first().map(|l| l.0), Some(2));
}

/// The head drawn for a light is the one by the bus's stop line, not another approach's
/// showing the light too beside other arrows; of the heads there, one whose discs are the
/// light rather than an arrow.
#[test]
fn the_head_drawn_is_the_one_by_the_stop_line() {
    use omsi_scene::{Lens, LensColor, LensColor::*, LensShape, LensShape::*, SignalHead};
    let head = |at: (f64, f64), lenses: &[(LensColor, LensShape, i64, usize)]| SignalHead {
        crossing: 7,
        lenses: lenses.iter().enumerate().map(|(k, &(color, shape, crossing, light))| Lens { slot: k as u8 + 1, color, shape, crossing, light }).collect(),
        place: Some(DVec3::new(at.0, at.1, 0.0)),
    };
    // the stop line at (0, 100), the lane going north
    let stop = (DVec3::new(0.0, 100.0, 0.0), 0.0);
    let ours = [7, 8];
    let mine = head((4.0, 125.0), &[(Red, Disc, 7, 0), (Yellow, Disc, 7, 0), (Green, Disc, 7, 0)]);
    let theirs = head((-4.0, 95.0), &[(Red, Disc, 7, 0), (Yellow, Disc, 7, 0), (Green, Left, 7, 2), (Green, Disc, 7, 0)]);
    let heads = [theirs.clone(), mine.clone()];
    assert_eq!(pick_head(heads.iter(), &ours, 0, stop), Some(&mine));
    // (seen from the other approach, the other head)
    assert_eq!(pick_head(heads.iter(), &ours, 0, (DVec3::new(-4.0, 120.0, 0.0), 180.0)), Some(&theirs));
    // a left turn's light: the head by the line whose discs show it, over the one where it is
    // only the arrow beside another light's discs
    let arrow = head((3.0, 120.0), &[(Red, Disc, 8, 0), (Yellow, Disc, 8, 0), (Green, Left, 7, 1), (Green, Disc, 8, 0)]);
    let own = head((-3.0, 122.0), &[(Red, Disc, 7, 1), (Yellow, Disc, 7, 1), (Green, Left, 7, 1)]);
    let heads = [arrow.clone(), own.clone()];
    assert_eq!(pick_head(heads.iter(), &ours, 1, stop), Some(&own));
    assert_eq!(pick_head([arrow.clone()].iter(), &ours, 1, stop), Some(&arrow));
    // a head with no place known is drawn when no other shows the light
    let unplaced = SignalHead { place: None, ..mine.clone() };
    assert_eq!(pick_head([unplaced.clone()].iter(), &ours, 0, stop), Some(&unplaced));
}
