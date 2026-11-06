//! Extract actual OMSI navigation data without opening a window or graphics adapter.
use glam::DVec2;
use omsi_sim::traffic::{
    LaneKind, Network,
    cartography::{confirm_road_surfaces, road_geometry, simplify_road},
};
use serde_json::json;
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 4,
        "usage: cartography_inspect OMSI_ROOT MAP_NAME OUTPUT.json"
    );
    let root = std::path::Path::new(&args[1]);
    let dir = root.join("maps").join(&args[2]);
    let global = omsi_map::GlobalCfg::load(&dir.join("global.cfg"))?;
    omsi_map::configure_grid(&global);
    let tiles: Vec<_> = global
        .tiles
        .iter()
        .map(|t| (t.x, t.y, omsi_cfg::resolve_path(&dir, &t.file)))
        .collect();
    let data = omsi_scene::navigation_map_of(root, &tiles, &[]);
    let mut net = Network {
        lanes: data.lanes,
        ..Default::default()
    };
    net.link(1.5);
    confirm_road_surfaces(&mut net, &data.road_surfaces);
    let roads = road_geometry(&net);
    let center = roads
        .iter()
        .flat_map(|r| r.points.iter())
        .map(|p| p.truncate())
        .fold(DVec2::ZERO, |a, p| a + p)
        / roads.iter().map(|r| r.points.len()).sum::<usize>().max(1) as f64;
    let json_roads:Vec<_>=roads.iter().map(|r|json!({"points":simplify_road(&r.points,0.35),"width":r.width,"main":r.main,"elevation":r.points.first().map(|p|p.z).unwrap_or(0.)})).collect();
    let labels: Vec<_> = data
        .signs
        .iter()
        .map(|(p, _, name)| json!({"position":[p.x,p.y],"text":name}))
        .collect();
    let surface = omsi_map::package::Package::open(&dir);
    let raw:Vec<_>=net.lanes.iter().filter(|l|l.kind==LaneKind::Street).map(|l|json!({"width":l.width,"points":l.points.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>()})).collect();
    let mut stop_types = std::collections::HashMap::new();
    let mut stops = Vec::new();
    for (_, _, path) in &tiles {
        if let Ok(tile) = omsi_map::Tile::load(path) {
            for o in tile.objects {
                let is_stop = *stop_types.entry(o.file.clone()).or_insert_with(|| {
                    omsi_scenery::SceneryObject::load(&omsi_cfg::resolve_path(root, &o.file))
                        .is_ok_and(|s| s.is_bus_stop)
                });
                if is_stop {
                    if let (Some(pos), Some(name)) = (
                        data.places.get(&o.id),
                        o.extra.first().filter(|s| !s.trim().is_empty()),
                    ) {
                        stops.push(json!({"position":[pos.x,pos.y],"name":name}));
                    }
                }
            }
        }
    }
    let stats = json!({"map":args[2],"tiles":tiles.len(),"trafficLanes":net.lanes.len(),"streetLanes":raw.len(),"mapRoads":roads.len(),"streetSigns":labels.len(),"namedStops":stops.len(),"surfacePackage":surface.is_some(),"objects":data.places.len()});
    std::fs::write(
        &args[3],
        serde_json::to_vec(
            &json!({"origin":[center.x,center.y],"roads":json_roads,"rawRoads":raw,"labels":labels,"route":[],"stops":stops,"stats":stats}),
        )?,
    )?;
    println!("{stats}");
    Ok(())
}
