use super::*;
use crate::people::cabin::BusNow;
use crate::people::model::BusId;
use crate::people::pax::Task;
use std::sync::Arc;

/// The synthetic bus of the BRT tests as the player's, with a seat (0) and a standing place
/// (1), one rider seated at the seat.
fn riding(name: &str) -> (PeopleSim, BusNow, HashMap<BusId, usize>) {
    let (mut h, mut bus) = super::super::pax_stops::brt_tests::setup(name, None);
    bus.id = BusId::Player;
    bus.entry_open = vec![false; bus.cabin.entries.len()];
    bus.exit_open = vec![false; bus.cabin.exits.len()];
    let mut standing = bus.cabin.seats[0].clone();
    standing.seated = false;
    standing.height = 0.0;
    standing.pos.y = -3.5;
    Arc::get_mut(&mut bus.cabin).unwrap().seats.push(standing);
    h.player_cabin = Some(bus.cabin.clone());
    h.seats.insert(BusId::Player, vec![true, false]);
    {
        let p = h.pax_mut(0).unwrap();
        p.bus = Some(BusId::Player);
        p.inside = Some(BusId::Player);
        p.stop = None;
        p.seat = Some(0);
        p.task = Task::SittingInBus;
        p.dest = Some("Far away".into());
    }
    let ix: HashMap<BusId, usize> = [(BusId::Player, 0)].into_iter().collect();
    (h, bus, ix)
}

fn run(h: &mut PeopleSim, bus: &BusNow, ix: &HashMap<BusId, usize>, steps: usize) {
    let world = crate::world_data::WorldData::default();
    let at: HashMap<BusId, crate::people::pax::BusAtStops> = HashMap::new();
    let mut taken = false;
    let mut remove = Vec::new();
    for _ in 0..steps {
        h.time += 0.05;
        h.pax_tick(0, 0.05, &world, std::slice::from_ref(bus), ix, &at, None, &mut taken, &mut remove);
    }
}

#[test]
fn a_rider_moves_to_a_standing_place_while_the_bus_runs() {
    let (mut h, mut bus, ix) = riding("po-move");
    bus.speed = 10.0;
    assert_eq!(h.pax_move_to(1, PaxOrder::Seat), Err("no free place of that kind"));
    h.pax_move_to(1, PaxOrder::Stand).unwrap();
    run(&mut h, &bus, &ix, 1);
    let p = h.pax(0).unwrap();
    assert_eq!(p.seat, Some(1));
    assert_eq!(p.task, Task::InBusToPlace, "walking there");
    assert_eq!(h.seats[&BusId::Player], vec![false, true], "the seat is given back");
    run(&mut h, &bus, &ix, 600);
    let p = h.pax(0).unwrap();
    assert_eq!(p.task, Task::SittingInBus, "at the new place");
    assert_eq!(p.pax_state, 0.0, "standing");
    assert!((p.pos.y - -3.5).abs() < 0.01, "{:?}", p.pos);
}

#[test]
fn a_held_rider_stands_still_and_goes_on_after() {
    let (mut h, bus, ix) = riding("po-hold");
    h.pax_move_to(1, PaxOrder::Stand).unwrap();
    run(&mut h, &bus, &ix, 1);
    let at = h.pax(0).unwrap().pos;
    h.pax_hold(1, 2.0).unwrap();
    run(&mut h, &bus, &ix, 20);
    assert_eq!(h.pax(0).unwrap().pos, at, "held");
    run(&mut h, &bus, &ix, 600);
    assert_eq!(h.pax(0).unwrap().task, Task::SittingInBus, "let go after the hold");
    assert!(h.pax(0).unwrap().held_until.is_none());
}

#[test]
fn staying_riders_keep_their_place_and_leave_when_told() {
    let (mut h, bus, ix) = riding("po-stay");
    h.pax_mut(0).unwrap().dest = None;
    h.pax_mut(0).unwrap().ride_km = -1.0;
    h.pax_stay(1, true).unwrap();
    run(&mut h, &bus, &ix, 5);
    assert_eq!(h.pax(0).unwrap().task, Task::SittingInBus, "past their ride");
    h.pax_leave(1, true).unwrap();
    assert!(!h.pax(0).unwrap().stay);
    run(&mut h, &bus, &ix, 1);
    let p = h.pax(0).unwrap();
    assert_eq!(p.task, Task::InBusToExit);
    assert!(p.leave_anywhere);
    assert!(h.stop_request, "they ask to stop");
}

#[test]
fn orders_are_only_for_the_player_s_bus() {
    let (mut h, _bus, _ix) = riding("po-only");
    h.pax_mut(0).unwrap().bus = Some(BusId::Ai(3));
    h.pax_mut(0).unwrap().inside = Some(BusId::Ai(3));
    assert!(h.pax_hold(1, 1.0).is_err());
    assert!(h.pax_leave(1, false).is_err());
    assert_eq!(h.pax_hold(99, 1.0), Err("no such person"));
}

#[test]
fn somebody_boarding_turns_back_and_does_not_board_that_bus_again() {
    let (mut h, bus, ix) = riding("po-cancel");
    {
        let p = h.pax_mut(0).unwrap();
        p.inside = None;
        p.task = Task::WalkingToBus;
        p.stop = Some(42);
        p.pos = glam::DVec3::new(-3.0, 4.0, 0.0);
    }
    h.pax_cancel(1).unwrap();
    run(&mut h, &bus, &ix, 1);
    let p = h.pax(0).unwrap();
    assert_eq!(p.task, Task::WalkingToBusstop);
    assert_eq!(p.refused, Some(BusId::Player));
    assert_eq!(p.seat, None);
    assert_eq!(h.seats[&BusId::Player], vec![false, false], "the place is given back");
    // in the bus already: too late
    let (mut h, _bus, _ix) = riding("po-cancel2");
    assert_eq!(h.pax_cancel(1), Err("already in the bus"));
}
