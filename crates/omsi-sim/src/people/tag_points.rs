//! K-OMSI: the card readers of a Korean bus, where its passengers tag on.
//!
//! Omsi.exe has a passenger stamp a ticket at the `[stamper]` of the passenger cabin - in
//! the German buses a validator on a pole in the saloon. The Korean buses carry their card
//! readers (T-money B600/B700/B800, Samwon FA, Cashbee ...) as meshes of the model, often in
//! several mounting variants of which the scripts show one, and their `[stamper]` was set
//! somewhere near (on the NSAC 1.5 m behind the reader, where the driver's terminal is):
//! reaching from the stamper's path point to it, the passengers bent deep at the hips.
//!
//! So on the player's bus the passengers who tag go to the reader they see: the shown mesh
//! of a reader nearest the door they came in by, a step in front of it, and reach for it
//! with the arm alone. A bus without such meshes keeps its `[stamper]`.

use super::*;

/// How far a reader may be from where the passenger stands in the doorway (m, across the
/// floor) to be theirs: the one at the door, not the one at the rear door.
pub const TAG_FROM_DOOR: f32 = 2.5;
/// How far a passenger steps up to a reader at the most (m), from the path point next to it.
pub const TAG_WALK: f32 = 0.8;

/// A mesh file of a card reader for the passengers (its stem, any case): the T-money,
/// Samwon FA and Cashbee readers - not the driver's console (`B800D`, `EB_dtag`) whose
/// screens and buttons share the names.
pub fn is_reader_mesh(stem: &str) -> bool {
    let s = stem.to_ascii_lowercase();
    const READER: [&str; 8] = ["tmoney", "samwon", "cashbee", "b600", "b700", "b800", "card", "reader"];
    const DRIVER: [&str; 4] = ["b800d", "dtag", "driver", "drv"];
    READER.iter().any(|k| s.contains(k)) && !DRIVER.iter().any(|k| s.contains(k))
}

/// The meshes of `ty` that are card readers: (mesh index, centre of its box in the mesh's
/// own frame). Looked up once for the player's bus (`PeopleSim::set_cabin`).
pub fn reader_meshes(ty: &crate::vehicle::VehicleType) -> Vec<(usize, Vec3)> {
    ty.meshes
        .iter()
        .enumerate()
        .filter(|(_, m)| m.file.file_stem().and_then(|s| s.to_str()).is_some_and(is_reader_mesh))
        .filter_map(|(i, _)| {
            let &(lo, hi) = ty.mesh_boxes.get(i)?;
            (hi.x > lo.x || hi.y > lo.y || hi.z > lo.z).then_some((i, (lo + hi) * 0.5))
        })
        .collect()
}

/// Where the readers of `meshes` shown now are, in the bus's frame.
pub fn shown_readers(v: &VehicleInstance, meshes: &[(usize, Vec3)]) -> Vec<Vec3> {
    meshes
        .iter()
        .filter(|(i, _)| v.mesh_props.get(*i).is_none_or(|p| p.visible))
        .filter_map(|&(i, c)| v.mesh_transforms.get(i).map(|m| m.transform_point3(c)))
        .filter(|p| p.is_finite())
        .collect()
}

/// The reader for a passenger who came in at `from` (bus frame): the nearest across the
/// floor, within [`TAG_FROM_DOOR`].
pub fn pick_reader(readers: &[Vec3], from: Vec3) -> Option<Vec3> {
    let flat = |p: Vec3| (p - from).truncate().length();
    readers
        .iter()
        .copied()
        .filter(|p| flat(*p) <= TAG_FROM_DOOR)
        .min_by(|a, b| flat(*a).total_cmp(&flat(*b)))
}

/// The path point a passenger walks to for the reader at `reader`: of the points of the
/// sections they are in (`from`'s), the nearest to it across the floor, one below it (a
/// floor, not a point up some stairs past it).
pub fn path_point_for(cabin: &Cabin, reader: Vec3, from: Vec3) -> Option<usize> {
    let all = cabin.all_points();
    let group = cabin.group_at(cabin.omsi_nearest(from, &all, false, false, None, None));
    let pts = cabin.in_group(all, group);
    let flat = |p: Vec3| (p - reader).truncate().length();
    pts.into_iter()
        .flatten()
        .filter_map(|k| cabin.graph.points.get(k).map(|p| (k, *p)))
        .filter(|(_, p)| p.z < reader.z && reader.z - p.z < 2.2)
        .min_by(|a, b| flat(a.1).total_cmp(&flat(b.1)))
        .map(|(k, _)| k)
}

/// How far in front of a reader `dz` above the feet a passenger stands (m, across the
/// floor): the hand reaches it with the arm a little bent, the body upright - closer to a
/// low reader, which the arm reaches hanging down.
pub fn stand_off(dz: f32) -> f32 {
    // (the shoulder some 1.40 m up, an arm of 0.55 m to the wrist reaching 85 % of it -
    // the card in the hand goes the rest)
    let below = 1.40 - dz;
    let reach = 0.55 * 0.85;
    (reach * reach - below * below).max(0.0).sqrt().clamp(0.22, 0.45)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readers_and_not_the_drivers_console() {
        for s in ["int_Tmoney_B800_F", "int_Tmoney_B700_Khoto", "int_samwonfa_HY", "Tmoney_B600_basic", "card"] {
            assert!(is_reader_mesh(s), "{s}");
        }
        for s in ["int_Tmoney_B800D", "int_Tmoney_B800D_A_RD1", "EB_dtag", "seat_front", "door_l"] {
            assert!(!is_reader_mesh(s), "{s}");
        }
    }

    #[test]
    fn the_reader_at_the_door_not_the_rear_one() {
        let front = Vec3::new(0.07, 4.8, 1.67);
        let rear = Vec3::new(0.39, 0.67, 1.48);
        assert_eq!(pick_reader(&[rear, front], Vec3::new(0.8, 4.3, 1.0)), Some(front));
        assert_eq!(pick_reader(&[front], Vec3::new(0.8, 0.5, 0.4)), None);
    }

    #[test]
    fn closer_to_a_low_reader() {
        // at the shoulder: half a metre off; at the hip: close in
        assert!((stand_off(1.4) - 0.45).abs() < 1e-6);
        assert!(stand_off(0.7) < 0.25);
        assert!(stand_off(1.1) > stand_off(0.9));
    }
}
