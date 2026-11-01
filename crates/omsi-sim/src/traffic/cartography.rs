//! Map-only carriageways derived from traffic lanes. Routing geometry is never modified.
use super::*;

/// Street lanes that stand for drawn roads, including editor-only paths corroborated by
/// road surfaces. Uncorroborated helpers must not make a phantom road on the GPS.
pub fn visible_road_lanes(net: &Network) -> Vec<(usize, &Lane)> {
    let mut seen = hashbrown::HashSet::<(LaneKey, u32)>::new();
    net.lanes
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LaneKind::Street && !l.invisible && l.points.len() >= 2)
        .filter(|(_, l)| {
            if let Some(key) = l.key {
                // A two-way [path] has the same geometry in both directions. Keep one
                // casing and surface; retain separate [path]s, which may be real lanes.
                return seen.insert((key, l.source));
            }
            true
        })
        .collect()
}

/// A road of the city map: the centre line of a carriageway, as wide as the map makes it.
#[derive(Clone, Debug)]
pub struct MapRoad {
    pub points: Vec<DVec3>,
    pub width: f32,
    /// A road cars drive far along (the map's own speed limits say so): drawn brighter.
    pub main: bool,
}

/// Traffic paths are lane centres, not separate streets. Adjacent paths on one spline
/// describe one carriageway; combine their footprints without filling a median. Align
/// opposing directions before averaging so bends, mirrors and ramps retain their shape.
pub fn road_geometry(net: &Network) -> Vec<MapRoad> {
    let mut roads = Vec::new();
    let mut splines = std::collections::BTreeMap::<((i32, i32), i64), Vec<&Lane>>::new();
    for (_, lane) in visible_road_lanes(net) {
        if let Some(key) = lane.key.filter(|_| lane.source == 1) {
            splines.entry((key.tile, key.id)).or_default().push(lane);
        } else {
            roads.push(MapRoad {
                points: lane.points.clone(),
                width: lane.width.max(2.6),
                main: lane.speed_limit_kmh >= 55.0,
            });
        }
    }
    for mut lanes in splines.into_values() {
        lanes.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        let mut start = 0;
        while start < lanes.len() {
            let a = lanes[start];
            let lo = a.offset - a.width.max(2.6) * 0.5;
            let mut hi = a.offset + a.width.max(2.6) * 0.5;
            let mut end = start + 1;
            while end < lanes.len() {
                let b = lanes[end];
                if b.points.len() != a.points.len() || b.offset - b.width.max(2.6) * 0.5 > hi + 0.65
                {
                    break;
                }
                let mid = a.points.len() / 2;
                let za = a.points[if a.reversed {
                    a.points.len() - 1 - mid
                } else {
                    mid
                }]
                .z;
                let zb = b.points[if b.reversed {
                    b.points.len() - 1 - mid
                } else {
                    mid
                }]
                .z;
                if (za - zb).abs() > 1.5 {
                    break;
                }
                hi = hi.max(b.offset + b.width.max(2.6) * 0.5);
                end += 1;
            }
            let b = lanes[end - 1];
            let span = b.offset - a.offset;
            let t = if span.abs() > 0.01 {
                (((lo + hi) * 0.5 - a.offset) / span) as f64
            } else {
                0.0
            };
            let point = |l: &Lane, i: usize| {
                l.points[if l.reversed {
                    l.points.len() - 1 - i
                } else {
                    i
                }]
            };
            let points = (0..a.points.len())
                .map(|i| point(a, i).lerp(point(b, i), t))
                .collect();
            roads.push(MapRoad {
                points,
                width: hi - lo,
                main: lanes[start..end].iter().any(|l| l.speed_limit_kmh >= 55.0),
            });
            start = end;
        }
    }
    // Bridge only explicit graph connections within the map's placement tolerance.
    for lane in net
        .lanes
        .iter()
        .filter(|l| l.kind == LaneKind::Street && !l.invisible && l.points.len() >= 2)
    {
        for &j in &lane.next {
            let Some(next) = net
                .lanes
                .get(j)
                .filter(|l| l.kind == LaneKind::Street && !l.invisible && l.points.len() >= 2)
            else {
                continue;
            };
            let distance = lane.end().distance(next.start());
            if distance > 0.05 && distance <= 2.0 {
                roads.push(MapRoad {
                    points: vec![lane.end(), next.start()],
                    width: lane.width.min(next.width).max(2.6),
                    main: lane.speed_limit_kmh >= 55.0 && next.speed_limit_kmh >= 55.0,
                });
            }
        }
    }
    roads
}

/// Conservative rectangle intersection keeps a long road crossing the view even when
/// neither endpoint is inside it. Selection is stable and nearest-first before capping.
pub fn nearby_roads(roads: &[MapRoad], center: DVec2, radius: f64, limit: usize) -> Vec<&MapRoad> {
    let mut nearby: Vec<_> = roads
        .iter()
        .filter(|r| {
            let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
            for p in &r.points {
                lo = lo.min(p.truncate());
                hi = hi.max(p.truncate());
            }
            let margin = radius + r.width as f64 * 0.5;
            lo.x <= center.x + margin
                && hi.x >= center.x - margin
                && lo.y <= center.y + margin
                && hi.y >= center.y - margin
        })
        .collect();
    nearby.sort_by(|a, b| {
        let distance = |r: &MapRoad| {
            r.points
                .iter()
                .map(|p| p.truncate().distance_squared(center))
                .fold(f64::INFINITY, f64::min)
        };
        distance(a).total_cmp(&distance(b))
    });
    nearby.truncate(limit);
    nearby
}

/// Douglas–Peucker in XY, with explicit endpoints and metre error. Rendering only;
/// never feed simplified points back into routing or vehicle physics.
pub fn simplify_road(points: &[DVec3], tolerance: f64) -> Vec<[f64; 2]> {
    if points.len() < 3 {
        return points.iter().map(|p| [p.x, p.y]).collect();
    }
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let start = points[a].truncate();
        let delta = points[b].truncate() - start;
        let mut far = None;
        let mut max = tolerance * tolerance;
        for i in a + 1..b {
            let p = points[i].truncate();
            let t = ((p - start).dot(delta) / delta.length_squared().max(1e-12)).clamp(0., 1.);
            let d = p.distance_squared(start + delta * t);
            if d > max {
                max = d;
                far = Some(i);
            }
        }
        if let Some(i) = far {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| [p.x, p.y])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cartography_groups_lanes_keeps_medians_and_levels() {
        let mut lanes = Vec::new();
        for (i, x) in [-10., -7., 7., 10.].into_iter().enumerate() {
            let mut lane = LaneBuilder::polyline(
                vec![DVec3::new(x, 0., 0.), DVec3::new(x, 100., 0.)],
                LaneKind::Street,
                3.,
            );
            lane.source = 1;
            lane.offset = x as f32;
            lane.key = Some(LaneKey {
                tile: (0, 0),
                id: 42,
                path: i as u16,
            });
            if x < 0. {
                lane.points.reverse();
                lane.reversed = true;
            }
            lanes.push(lane);
        }
        let net = Network {
            lanes,
            ..Default::default()
        };
        let roads = road_geometry(&net);
        assert_eq!(roads.len(), 2);
        assert_eq!(roads[0].width, 6.);
        assert_eq!(roads[0].points[0].x, -8.5);
        assert_eq!(net.lanes.len(), 4);
        let mut upper = net.lanes[2].clone();
        upper.points.iter_mut().for_each(|p| p.z = 8.);
        upper.key.as_mut().unwrap().path = 5;
        let net = Network {
            lanes: vec![net.lanes[2].clone(), upper],
            ..Default::default()
        };
        assert_eq!(road_geometry(&net).len(), 2);
    }
    #[test]
    fn cartography_excludes_helpers_and_duplicate_directions() {
        let mut a = LaneBuilder::polyline(vec![DVec3::ZERO, DVec3::Y * 100.], LaneKind::Street, 3.);
        a.key = Some(LaneKey {
            tile: (0, 0),
            id: 1,
            path: 0,
        });
        let mut b = a.clone();
        b.points.reverse();
        b.reversed = true;
        let mut helper = a.clone();
        helper.invisible = true;
        helper.key.as_mut().unwrap().id = 2;
        let net = Network {
            lanes: vec![b, a, helper],
            ..Default::default()
        };
        assert_eq!(road_geometry(&net).len(), 1);
    }
    #[test]
    fn cartography_selection_and_simplification_preserve_crossing_and_bend() {
        let road = MapRoad {
            points: vec![DVec3::new(-2000., 0., 0.), DVec3::new(2000., 0., 0.)],
            width: 3.,
            main: false,
        };
        assert_eq!(nearby_roads(&[road], DVec2::ZERO, 100., 10).len(), 1);
        let p = vec![
            DVec3::ZERO,
            DVec3::X,
            DVec3::X * 2.,
            DVec3::new(2., 2., 0.),
            DVec3::new(2., 4., 0.),
        ];
        assert_eq!(simplify_road(&p, 0.1), vec![[0., 0.], [2., 0.], [2., 4.]]);
    }
}
/// Some maps separate the asphalt mesh from their editor-only traffic splines. Use the
/// actual road footprint to distinguish those paths from invisible scenery helpers. This
/// changes only the navigator's copy of the network; driving still uses the original data.
pub fn confirm_road_surfaces(net: &mut Network, surfaces: &[(Vec<DVec3>, f32)]) {
    let mut segments = Vec::new();
    let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    for (pts, width) in surfaces {
        for ab in pts.windows(2) {
            let (a, b) = (ab[0], ab[1]);
            let margin = *width as f64 * 0.5 + 0.5;
            let lo = a.truncate().min(b.truncate()) - DVec2::splat(margin);
            let hi = a.truncate().max(b.truncate()) + DVec2::splat(margin);
            let (x0, y0) = Network::grid_cell(lo.extend(0.0));
            let (x1, y1) = Network::grid_cell(hi.extend(0.0));
            let i = segments.len();
            segments.push((a, b, margin));
            for x in x0..=x1 {
                for y in y0..=y1 {
                    grid.entry((x, y)).or_default().push(i);
                }
            }
        }
    }
    for lane in net
        .lanes
        .iter_mut()
        .filter(|l| l.invisible && l.kind == LaneKind::Street)
    {
        let n = (lane.length() / 8.0).ceil().max(1.0) as usize;
        let mut covered = 0;
        for k in 0..n {
            let (p, _) = lane.at(lane.length() * (k as f32 + 0.5) / n as f32);
            let on_surface = grid
                .get(&Network::grid_cell(p))
                .map(|ids| {
                    ids.iter().any(|&i| {
                        let (a, b, margin) = segments[i];
                        let ab = (b - a).truncate();
                        let t = ((p - a).truncate().dot(ab) / ab.length_squared().max(1e-6))
                            .clamp(0.0, 1.0);
                        let q = a.lerp(b, t);
                        (p - q).truncate().length() <= margin && (p.z - q.z).abs() <= 2.0
                    })
                })
                .unwrap_or(false);
            if on_surface {
                covered += 1;
            }
        }
        if covered * 4 >= n * 3 {
            lane.invisible = false;
        }
    }
    // Crossings made of objects can have no asphalt spline. Keep helpers joining two
    // corroborated roads; an isolated invisible helper still does not become a street.
    let mut prev = vec![Vec::new(); net.lanes.len()];
    for (i, lane) in net
        .lanes
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == LaneKind::Street)
    {
        for &j in &lane.next {
            if net
                .lanes
                .get(j)
                .map(|l| l.kind == LaneKind::Street)
                .unwrap_or(false)
            {
                prev[j].push(i);
            }
        }
    }
    let mut seen = vec![false; net.lanes.len()];
    let mut keep = Vec::new();
    for seed in 0..net.lanes.len() {
        if seen[seed] || !net.lanes[seed].invisible || net.lanes[seed].kind != LaneKind::Street {
            continue;
        }
        let mut queue = vec![seed];
        let mut component = Vec::new();
        let mut anchors = hashbrown::HashSet::new();
        while let Some(i) = queue.pop() {
            if seen[i] {
                continue;
            }
            seen[i] = true;
            component.push(i);
            for &j in net.lanes[i].next.iter().chain(prev[i].iter()) {
                let Some(l) = net.lanes.get(j).filter(|l| l.kind == LaneKind::Street) else {
                    continue;
                };
                if l.invisible {
                    if !seen[j] {
                        queue.push(j);
                    }
                } else {
                    anchors.insert((l.key, l.source, if l.key.is_none() { j } else { 0 }));
                }
            }
        }
        if anchors.len() >= 2 {
            keep.extend(component);
        }
    }
    for i in keep {
        net.lanes[i].invisible = false;
    }
}
