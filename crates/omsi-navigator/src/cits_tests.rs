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
    assert!((c.dist - 800.0).abs() < 1.0 && c.limit == Some(50.0) && c.tag.is_none(), "{c:?}");
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
    assert_eq!(n.cits.camera.as_ref().and_then(|c| c.tag), Some("Signal"));
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
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\camera.sco"), Some(CameraKind::Speed));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\areacctvstart.sco"), Some(CameraKind::SectionStart));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Photoemergyzone\\areacctvend.sco"), Some(CameraKind::SectionEnd));
    assert_eq!(camera_kind("Sceneryobjects\\Korean Sceneryobject\\Speedcam_Pole\\Steel(Std Type)\\SteelGrey\\interval_spdcampole_6m.sco"), Some(CameraKind::SectionPoint));
    assert_eq!(camera_kind("Sceneryobjects\\Korean Sceneryobject\\Speedcam_Pole\\Steel(Std Type)\\SteelGrey\\spd+tflcampole_6m.sco"), Some(CameraKind::SignalSpeed));
    assert_eq!(camera_kind("Sceneryobjects\\KOR_Standard Set\\Ampel Mast(Traffic light Pole)\\Gray(New)\\KOR_AmpleMast_Camera_S.sco"), Some(CameraKind::SignalSpeed));
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
