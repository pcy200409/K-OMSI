//! C-ITS style warnings by the speed: what Korea's in-car C-ITS units and camera alerts tell
//! a driver, from what the game knows itself.
//!
//! * The traffic light ahead (within 200 m): its aspect and the seconds to its next change,
//!   the speed that meets it green (GLOSA) - or that the bus cannot stop before its red.
//! * The vehicle ahead: how far, and a collision warning when the gap closes too fast.
//! * A sharp curve or a lower limit ahead, while the bus is too fast for it.
//! * The map's enforcement cameras ahead (`omsi_scene::SpeedCamera`): from 1 km, again at
//!   500 m and 300 m, flashing red while the bus is faster than the camera allows; between
//!   the two ends of an average speed check, the average so far.
//!
//! Worked out a few times a second; drawn as small badges beside the speed and its limit.

use super::*;
use omsi_scene::{CameraKind, SpeedCamera};
use omsi_sim::traffic::{Aspect, TrafficLightController};

/// How often the warnings are worked out (s).
const EVERY: f32 = 0.25;
/// How far ahead the traffic light, the camera and the curves are looked for (m).
const LIGHT_REACH: f32 = 200.0;
const CAMERA_REACH: f64 = 1000.0;
const CURVE_REACH: f64 = 400.0;
/// The distances a camera is announced again at (m).
const CAMERA_STAGES: [f64; 3] = [1000.0, 500.0, 300.0];
/// A comfortable braking (m/s²) and the lateral acceleration a bus takes a curve at.
const BRAKE: f64 = 2.5;
const LATERAL: f64 = 1.8;
/// From the bus's centre to its front and a car's centre to its back (m), for the gap.
const LENGTHS: f64 = 8.0;

/// How much a warning matters: shown in that order, a danger flashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    Caution,
    Danger,
}

/// One badge.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Alert {
    pub level: Level,
    pub icon: &'static str,
    pub icon_color: Color,
    pub text: String,
    /// Flashing red (over the camera's speed, a red light the bus cannot stop for).
    pub flash: bool,
}

/// What the traffic light ahead asks of the bus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LightAdvice {
    /// It will pass on green (or is stopped at the line).
    Pass,
    /// This speed (km/h) meets it green.
    Speed(f32),
    /// Stop for it.
    Stop,
    /// Too fast to stop before its red.
    Violation,
}

/// The light at `dist` m showing `aspect`, changing in `change_in` s and letting traffic go
/// in `until_go` s (0 while it does), for a bus at `v` m/s under `limit` km/h.
pub(crate) fn light_advice(aspect: Aspect, change_in: f32, until_go: f32, dist: f64, v: f64, limit: f32) -> LightAdvice {
    let stop_dist = v * v / (2.0 * BRAKE) + v * 0.8;
    let can_stop = stop_dist < dist + 2.0;
    let arrive = if v > 0.5 { dist / v } else { f64::INFINITY };
    let limit = (limit.max(20.0) as f64) / 3.6;
    match aspect {
        Aspect::Green | Aspect::GreenYellow => {
            let left = change_in.max(0.0) as f64;
            if arrive <= left - 1.0 || v < 0.5 {
                LightAdvice::Pass
            } else if dist / (left - 1.0).max(0.1) <= limit {
                LightAdvice::Speed(((dist / (left - 1.0).max(0.1)) * 3.6) as f32)
            } else if can_stop {
                LightAdvice::Stop
            } else {
                LightAdvice::Pass
            }
        }
        Aspect::Yellow => {
            if can_stop && v > 0.5 {
                LightAdvice::Stop
            } else {
                LightAdvice::Pass
            }
        }
        Aspect::Red | Aspect::RedYellow => {
            let wait = until_go.max(0.0) as f64;
            if v < 0.5 || arrive >= wait {
                return LightAdvice::Pass;
            }
            if !can_stop && v > 15.0 / 3.6 {
                return LightAdvice::Violation;
            }
            let meet = dist / wait.max(0.1);
            if meet >= 15.0 / 3.6 && meet <= limit {
                LightAdvice::Speed((meet * 3.6) as f32)
            } else {
                LightAdvice::Stop
            }
        }
        Aspect::Dark => LightAdvice::Pass,
    }
}

/// How dangerous the gap of `gap` m to a vehicle at `lead` m/s is for the bus at `v` m/s.
pub(crate) fn gap_level(gap: f64, v: f64, lead: f64) -> Option<Level> {
    if v < 10.0 / 3.6 || gap > 120.0 {
        return None;
    }
    let ttc = if v > lead + 0.3 { gap / (v - lead) } else { f64::INFINITY };
    let headway = gap / v;
    if ttc < 3.0 || headway < 0.6 {
        Some(Level::Danger)
    } else if ttc < 6.0 || headway < 1.2 {
        Some(Level::Caution)
    } else {
        None
    }
}

/// The speed (km/h, by fives) a curve of radius `r` m is taken at.
pub(crate) fn curve_speed(r: f64) -> f32 {
    (((LATERAL * r).sqrt() * 3.6 / 5.0).floor() * 5.0).max(10.0) as f32
}

/// Whether a bus at `v` m/s should hear of a slower stretch (`slow` km/h) `dist` m ahead.
fn slow_ahead_matters(dist: f64, v: f64, slow: f32) -> bool {
    let vs = slow as f64 / 3.6;
    v > vs + 4.0 / 3.6 && dist <= ((v * v - vs * vs) / (2.0 * 1.2) + v * 3.0).max(120.0)
}

/// Which announcement stage a camera `dist` m ahead is in (0 from 1 km, 1 from 500 m, 2 from
/// 300 m); None farther.
pub(crate) fn camera_stage(dist: f64) -> Option<usize> {
    CAMERA_STAGES.iter().rposition(|&d| dist <= d)
}

/// The lanes ahead of the bus with where each starts (m from the bus, the first negative):
/// the route's while the bus is on it, else straight on (the gentlest way at each fork).
pub(crate) fn path_ahead(net: &Network, route: Option<(&[usize], f32)>, bus: DVec3, heading: f64, reach: f64) -> Vec<(usize, f64)> {
    let mut out = Vec::new();
    if let Some((lanes, s)) = route.filter(|r| !r.0.is_empty()) {
        let mut acc = -(s as f64);
        for &l in lanes {
            let Some(lane) = net.lanes.get(l) else { break };
            out.push((l, acc));
            acc += lane.length() as f64;
            if acc > reach {
                break;
            }
        }
        return out;
    }
    let Some((mut l, s, _)) = net.lane_along(bus, heading, LaneKind::Street, 6.0, 45.0) else { return out };
    let mut acc = -(s as f64);
    while acc <= reach && out.len() < 64 {
        out.push((l, acc));
        let lane = &net.lanes[l];
        acc += lane.length() as f64;
        let end = lane.end_heading();
        let next = lane
            .next
            .iter()
            .filter(|&&n| net.lanes[n].kind == LaneKind::Street && !out.iter().any(|o| o.0 == n))
            .map(|&n| (n, omsi_sim::traffic::wrap_deg(net.lanes[n].start_heading() - end).abs()))
            .filter(|n| n.1 < 60.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match next {
            Some((n, _)) => l = n,
            None => break,
        }
    }
    out
}

/// Where camera `cam` stands on the path (m ahead, and the lane it is at), when it stands
/// beside it for the way the bus goes: on its right or over it (Korea drives on the right;
/// a camera for the other carriageway stands on the bus's left).
pub(crate) fn on_path(net: &Network, path: &[(usize, f64)], cam: DVec3) -> Option<(f64, usize)> {
    path.iter().filter_map(|&(l, start)| beside(net, l, start, cam)).min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The path's lanes and, beside each, the other lanes of its carriageway running the same
/// way and on straight from them for a little: a camera stands over or beside the right-hand
/// lane while the bus may be in another (or in a turning lane at the junction it watches).
pub(crate) fn corridor(net: &Network, path: &[(usize, f64)]) -> Vec<(usize, f64)> {
    let mut out: Vec<(usize, f64)> = path.to_vec();
    let add = |out: &mut Vec<(usize, f64)>, l: usize, start: f64| match out.iter_mut().find(|o| o.0 == l) {
        Some(o) => o.1 = o.1.min(start),
        None => out.push((l, start)),
    };
    for &(l, start) in path {
        let mut side = Vec::new();
        for go_right in [true, false] {
            let mut cur = l;
            for _ in 0..5 {
                let lane = &net.lanes[cur];
                match if go_right { lane.right } else { lane.left } {
                    Some(n) if n != l => {
                        side.push(n);
                        cur = n;
                    }
                    _ => break,
                }
            }
        }
        for n in side {
            // (and on straight from it, some 120 m)
            let (mut cur, mut at) = (n, start);
            for _ in 0..6 {
                add(&mut out, cur, at);
                let lane = &net.lanes[cur];
                at += lane.length() as f64;
                if at - start > lane.length() as f64 + 120.0 {
                    break;
                }
                let end = lane.end_heading();
                let next = lane.next.iter().filter(|&&x| net.lanes[x].kind == LaneKind::Street).map(|&x| (x, omsi_sim::traffic::wrap_deg(net.lanes[x].start_heading() - end).abs())).filter(|x| x.1 < 30.0).min_by(|a, b| a.1.total_cmp(&b.1));
                match next {
                    Some((x, _)) => cur = x,
                    None => break,
                }
            }
        }
    }
    out
}

fn beside(net: &Network, l: usize, start: f64, cam: DVec3) -> Option<(f64, usize)> {
    let lane = &net.lanes[l];
    let (s, d) = lane.nearest_point(cam)?;
    if d > 16.0 {
        return None;
    }
    let (p, h) = lane.at(s);
    let h = (h as f64).to_radians();
    let right = DVec2::new(h.cos(), -h.sin());
    let off = (cam - p).truncate().dot(right);
    (-4.0..=16.0).contains(&off).then_some((start + s as f64, l))
}

/// A camera being approached: which, how far, and the speed it holds to.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Approach {
    camera: usize,
    dist: f64,
    limit: Option<f32>,
}

/// An average speed check being driven through.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Section {
    /// The odometer and the clock where it began.
    from_m: f64,
    from_t: f32,
    limit: Option<f32>,
}

/// The warnings' state between frames.
#[derive(Default)]
pub(crate) struct Cits {
    pub cameras: std::sync::Arc<Vec<SpeedCamera>>,
    t: f32,
    clock: f32,
    odometer: f64,
    approach: Option<Approach>,
    stage: Option<(usize, usize)>,
    /// Seconds the camera badge still flashes for a new stage.
    pulse: f32,
    section: Option<Section>,
    /// The traffic light ahead, the camera ahead and the average speed check driven through,
    /// for the signal box and the speed sign.
    pub light: Option<LightView>,
    pub camera: Option<CameraView>,
    pub average: Option<AverageView>,
    /// The other warnings, the most urgent first: the band over the stop card.
    pub alerts: Vec<Alert>,
}

/// The traffic light ahead as the signal box shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LightView {
    pub aspect: Aspect,
    /// Seconds to its next change (None: it does not change, as far as is known).
    pub secs: Option<u32>,
    pub advice: LightAdvice,
    /// The line under it: the advised speed, or to stop.
    pub note: Option<String>,
}

/// The camera ahead: the speed sign grows and gets a red tab with the distance.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CameraView {
    pub dist: f64,
    pub limit: Option<f32>,
    /// What else it watches (`Signal`, `Section`), on the tab.
    pub tag: Option<&'static str>,
    pub over: bool,
    /// Flashing: over its speed, or a moment at a new stage (1 km, 500 m, 300 m).
    pub flash: bool,
}

/// An average speed check being driven through: its average so far against its limit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct AverageView {
    pub kmh: f32,
    pub limit: Option<f32>,
    pub over: bool,
}

impl Cits {
    /// Flashing now (twice a second).
    pub fn blink(&self) -> bool {
        (self.clock * 2.5).fract() < 0.55
    }
}

const RED: Color = Color::rgba(226, 38, 38, 1.0);
const AMBER: Color = Color::rgba(240, 160, 20, 1.0);

impl Navigator {
    pub(crate) fn update_cits(&mut self, f: &NavFrame) {
        let v = (f.speed_kmh.abs() / 3.6) as f64;
        self.cits.clock += f.dt;
        self.cits.odometer += v * f.dt as f64;
        self.cits.pulse = (self.cits.pulse - f.dt).max(0.0);
        self.cits.t -= f.dt;
        if self.cits.t > 0.0 {
            return;
        }
        self.cits.t = EVERY;
        let units = omsi_ui::units::Units::now();
        let kmh = f.speed_kmh.abs();
        let mut alerts = Vec::new();
        let mut light_view = None;
        let mut camera_view = None;
        let mut average = None;
        let limit_here = self.speed_limit;

        // --- the traffic light ahead and the vehicle ahead: the traffic's own network
        if let Some(t) = f.traffic {
            let lanes = t.lanes_ahead_of(f.bus, f.heading, LIGHT_REACH);
            let light = lanes.iter().skip(1).filter(|(l, _)| t.net.lanes[*l].traffic_light.is_some()).min_by(|a, b| a.1.total_cmp(&b.1)).copied();
            if let Some((lane, dist)) = light {
                if let Some((c, li)) = t.net.lanes[lane].traffic_light {
                    if let Some((state, _)) = t.light_state(c, li) {
                        let aspect = TrafficLightController::aspect(state);
                        let change_in = t.lights.get(c).and_then(|ctl| ctl.time_to_change(li)).unwrap_or(f32::INFINITY);
                        let until_go = t.light_until_go(c, li).unwrap_or(0.0);
                        if aspect != Aspect::Dark {
                            let lv = light_view_of(aspect, change_in, until_go, dist as f64, v, limit_here.unwrap_or(50.0), units);
                            if lv.advice == LightAdvice::Violation {
                                alerts.push(Alert { level: Level::Danger, icon: "traffic", icon_color: RED, text: omsi_ui::tr("Red light ahead").into_owned(), flash: true });
                            }
                            light_view = Some(lv);
                        }
                    }
                }
            }
            if let Some((own, s, _)) = t.net.lane_along(f.bus, f.heading, LaneKind::Street, 4.0, 45.0) {
                let ahead: HashMap<usize, f32> = lanes.iter().copied().collect();
                let lead = t
                    .cars
                    .iter()
                    .filter(|c| !c.gone)
                    .filter_map(|c| {
                        let d0 = *ahead.get(&c.state.lane)?;
                        let d = if c.state.lane == own { (c.state.s - s) as f64 } else { (d0 + c.state.s) as f64 };
                        (d > 0.5).then_some((d - LENGTHS, c.state.speed.max(0.0) as f64))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((gap, lead_v)) = lead {
                    if let Some(level) = gap_level(gap.max(0.0), v, lead_v) {
                        let danger = level == Level::Danger;
                        let what = if danger { omsi_ui::tr("Collision risk") } else { omsi_ui::tr("Car ahead") };
                        alerts.push(Alert { level, icon: "directions_car", icon_color: if danger { RED } else { AMBER }, text: format!("{what} {}", units.distance(gap.max(0.0))), flash: danger });
                    }
                }
            }
        }

        // --- the road ahead: the navigator's network (the route's lanes are its own)
        let global = self.global.clone();
        let net = global.as_deref().or(f.traffic.map(|t| &t.net));
        if let Some(net) = net {
            let route = (self.route.on_route && self.destination.is_none()).then(|| (&self.route.lanes[self.route.progress.min(self.route.lanes.len())..], self.route.s));
            let path = path_ahead(net, route, f.bus, f.heading, CAMERA_REACH + 50.0);
            let lanes_by = corridor(net, &path);
            // a sharp curve or a lower limit
            let mut curve: Option<(f64, f32)> = None;
            let mut drop: Option<(f64, f32)> = None;
            for &(l, start) in &path {
                if start > CURVE_REACH {
                    break;
                }
                let lane = &net.lanes[l];
                let len = lane.length() as f64;
                if curve.is_none() && len > 40.0 && lane.curvature.len() == lane.points.len() {
                    let k = lane.curvature.iter().zip(&lane.dist).map(|(k, &d)| (k.abs(), d)).filter(|&(_, d)| start + d as f64 > 0.0).max_by(|a, b| a.0.total_cmp(&b.0));
                    if let Some((k, d)) = k.filter(|k| k.0 > 1.0 / 150.0) {
                        let adv = curve_speed(1.0 / k as f64);
                        let at = (start + d as f64).max(0.0);
                        if slow_ahead_matters(at, v, adv) && limit_here.is_none_or(|l| adv < l - 4.0) {
                            curve = Some((at, adv));
                        }
                    }
                }
                let lim = lane.speed_limit_kmh;
                if drop.is_none() && start > 0.0 && lim > 1.0 && lim < 200.0 && limit_here.is_some_and(|h| lim < h - 1.0) && slow_ahead_matters(start, v, lim) {
                    drop = Some((start, lim));
                }
            }
            if let Some((d, adv)) = curve {
                alerts.push(Alert { level: Level::Caution, icon: "turn_right", icon_color: AMBER, text: format!("{} {} · {:.0}", omsi_ui::tr("Sharp curve"), units.distance(d), units.speed(adv).0), flash: false });
            }
            if let Some((d, lim)) = drop {
                alerts.push(Alert { level: Level::Caution, icon: "speed", icon_color: RED, text: format!("{} {:.0} · {}", omsi_ui::tr("Limit ahead"), units.speed_limit(lim), units.distance(d)), flash: false });
            }
            // the cameras
            let cams = self.cits.cameras.clone();
            let near = cams
                .iter()
                .enumerate()
                .filter(|(_, c)| (c.position - f.bus).truncate().length() < CAMERA_REACH + 60.0)
                .filter_map(|(i, c)| {
                    let (d, l) = on_path(net, &lanes_by, c.position)?;
                    (d > 0.0 && d <= CAMERA_REACH).then(|| {
                        let lim = c.limit.or_else(|| Some(net.lanes[l].speed_limit_kmh).filter(|v| *v > 1.0 && *v < 200.0));
                        Approach { camera: i, dist: d, limit: lim }
                    })
                })
                .collect::<Vec<_>>();
            // (a camera, its sign and its mast stand together: the one that says most of them)
            let first = near.iter().map(|a| a.dist).fold(f64::INFINITY, f64::min);
            let near = near.into_iter().filter(|a| a.dist < first + 20.0).max_by_key(|a| (kind_rank(cams[a.camera].kind), std::cmp::Reverse((a.dist * 10.0) as i64)));
            // passed the one approached: an average speed check begins or ends there
            if let Some(prev) = self.cits.approach.filter(|p| p.dist < 40.0 && near.is_none_or(|n| n.camera != p.camera)) {
                match cams.get(prev.camera).map_or(CameraKind::Speed, |c| c.kind) {
                    CameraKind::SectionStart => self.cits.section = Some(Section { from_m: self.cits.odometer, from_t: self.cits.clock, limit: prev.limit }),
                    CameraKind::SectionPoint if self.cits.section.is_none() => self.cits.section = Some(Section { from_m: self.cits.odometer, from_t: self.cits.clock, limit: prev.limit }),
                    CameraKind::SectionEnd | CameraKind::SectionPoint => self.cits.section = None,
                    _ => {}
                }
            }
            self.cits.approach = near;
            let stage = near.and_then(|n| camera_stage(n.dist).map(|s| (n.camera, s)));
            if stage.is_some() && stage != self.cits.stage {
                self.cits.pulse = 1.6;
            }
            self.cits.stage = stage;
            if let Some(n) = near {
                let over = n.limit.is_some_and(|l| kmh > l + 0.5);
                let tag = match cams[n.camera].kind {
                    CameraKind::Speed => None,
                    CameraKind::SignalSpeed => Some("Signal"),
                    _ => Some("Section"),
                };
                camera_view = Some(CameraView { dist: n.dist, limit: n.limit, tag, over, flash: over || self.cits.pulse > 0.0 });
            }
        }
        // inside an average speed check: the average so far
        if let Some(sec) = self.cits.section {
            let (m, t) = (self.cits.odometer - sec.from_m, (self.cits.clock - sec.from_t) as f64);
            if m > 20_000.0 || t > 1800.0 {
                self.cits.section = None;
            } else if t > 2.0 {
                let kmh = (m / t * 3.6) as f32;
                average = Some(AverageView { kmh, limit: sec.limit, over: sec.limit.is_some_and(|l| kmh > l + 0.5) });
            }
        }
        alerts.sort_by(|a, b| b.level.cmp(&a.level));
        self.cits.alerts = alerts;
        self.cits.light = light_view;
        self.cits.camera = camera_view;
        self.cits.average = average;
    }
}

fn kind_rank(k: CameraKind) -> u8 {
    match k {
        CameraKind::Speed => 0,
        CameraKind::SignalSpeed => 1,
        CameraKind::SectionPoint => 2,
        CameraKind::SectionStart | CameraKind::SectionEnd => 3,
    }
}

fn light_view_of(aspect: Aspect, change_in: f32, until_go: f32, dist: f64, v: f64, limit: f32, units: omsi_ui::units::Units) -> LightView {
    let secs = (change_in.is_finite() && change_in < 600.0).then(|| change_in.ceil().max(0.0) as u32);
    let advice = light_advice(aspect, change_in, until_go, dist, v, limit);
    let note = match advice {
        LightAdvice::Speed(s) => Some(format!("{} {:.0}", omsi_ui::tr("Advised"), units.speed((s / 5.0).floor() * 5.0).0)),
        LightAdvice::Stop => Some(omsi_ui::tr("Prepare to stop").into_owned()),
        _ => None,
    };
    LightView { aspect, secs, advice, note }
}
