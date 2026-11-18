//! K-OMSI: paper tickets handed to the driver (`paper_ticket`).

use super::ticket_hook_tests::{boarding, human_type, player_bus};
use super::*;

/// The tests' player's bus with a money point on its cash desk.
fn desk_bus(tag: &str) -> BusNow {
    let dir = std::env::temp_dir().join(format!("omsi-paper-ticket-bus-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("test.bus"), "[boundingbox]\n2.5\n12\n3\n0\n0\n1.5\n\n[passengercabin]\ncabin.cfg\n\n[paths]\npaths.cfg\n").unwrap();
    std::fs::write(dir.join("paths.cfg"), "[pathpnt]\n1.1\n4\n0.4\n\n[pathpnt]\n0\n4\n0.5\n\n[pathpnt]\n0\n-3\n0.5\n\n[pathpnt]\n1.1\n-3\n0.4\n\n[pathlink]\n0\n1\n\n[pathlink]\n1\n2\n\n[pathlink]\n2\n3\n").unwrap();
    std::fs::write(
        dir.join("cabin.cfg"),
        "[entry]\n0\n\n[exit]\n3\n\n[ticket_sale]\n1\n0\n4\n1\n\n[ticket_sale_money_point]\n0.3\n4.2\n1.0\n0.05\n0.05\n\n[passpos]\n-0.5\n0\n0.9\n0.45\n0\n",
    )
    .unwrap();
    let def = omsi_vehicle::Vehicle::load(&dir.join("test.bus")).unwrap();
    let cabin = Arc::new(Cabin::load_train(&[(&def, Vec3::ZERO, f32::INFINITY)]).expect("cabin"));
    std::fs::remove_dir_all(&dir).ok();
    assert!(cabin.money_var.is_some(), "the cabin has its money point");
    let mut bus = player_bus(tag);
    bus.cabin = cabin;
    bus
}

/// `n` passengers at the player's cash desk with a paper ticket.
fn at_desk(tag: &str, n: u32) -> (PeopleSim, BusNow) {
    let ty = human_type(tag);
    let bus = desk_bus(tag);
    let mut h = PeopleSim::new(Path::new("/nonexistent"), 200);
    h.boarding = "pay".into();
    h.player_cabin = Some(bus.cabin.clone());
    h.people = (1..=n).map(|id| boarding(&ty, id, Task::InBusToPlace)).collect();
    for i in 0..n as usize {
        let p = h.pax_mut(i).unwrap();
        p.inside = Some(BusId::Player);
        p.ticket = TICKET_HAND;
        p.sub = 3;
    }
    (h, bus)
}

fn lay_down(h: &mut PeopleSim, bus: &BusNow, i: usize) {
    h.desk_hand(i, 0.0, bus);
    assert_eq!(h.pax(i).unwrap().sub, 4, "reaching for the desk");
    h.pax_mut(i).unwrap().timer = 0.0;
    h.desk_hand(i, 0.0, bus);
    assert_eq!(h.pax(i).unwrap().sub, 5, "the ticket lies on the desk");
}

#[test]
fn nobody_rides_before_the_driver_has_taken_the_ticket() {
    let (mut h, bus) = at_desk("wait", 1);
    lay_down(&mut h, &bus, 0);
    assert!(h.paper_on_desk(1));
    assert!(h.bodies.ops.iter().any(|o| matches!(o, BodyOp::PaperTicket { person: 1, .. })), "drawn on the desk");
    assert_eq!(h.take_ticket_events(), vec![(1, TicketEvent::Handed)]);
    // however long the driver lets them wait, they stay at the desk
    for _ in 0..600 {
        h.pax_mut(0).unwrap().timer -= 1.0;
        h.desk_hand(0, 1.0, &bus);
    }
    assert_eq!(h.pax(0).unwrap().sub, 5);
    assert_eq!(h.pax(0).unwrap().ticket, TICKET_HAND);
    assert_eq!(h.desk_busy, Some(1));
    assert_eq!(h.boarded, 0);
    // the driver takes it: off the desk, on to the place
    assert_eq!(h.take_paper(1), Ok(()));
    assert!(!h.paper_on_desk(1));
    assert!(h.bodies.ops.iter().any(|o| matches!(o, BodyOp::PaperGone { person: 1 })));
    assert_eq!(h.take_ticket_events(), vec![(1, TicketEvent::Taken)]);
    h.desk_hand(0, 0.0, &bus);
    let p = h.pax(0).unwrap();
    assert_eq!((p.sub, p.ticket), (0, TICKET_NONE));
    assert_eq!(h.desk_busy, None);
    assert_eq!(h.boarded, 1);
}

#[test]
fn the_next_one_waits_for_the_desk() {
    let (mut h, bus) = at_desk("queue", 2);
    lay_down(&mut h, &bus, 0);
    for _ in 0..10 {
        h.desk_hand(1, 0.1, &bus);
        assert_eq!(h.pax(1).unwrap().sub, 3, "the desk is the first one's");
    }
    h.take_paper(1).unwrap();
    h.desk_hand(0, 0.0, &bus);
    h.desk_hand(1, 0.0, &bus);
    assert_eq!(h.pax(1).unwrap().sub, 4, "the second one's turn");
    assert_eq!(h.desk_busy, Some(2));
}

#[test]
fn only_a_ticket_on_the_desk_can_be_taken() {
    let (mut h, bus) = at_desk("take", 1);
    assert_eq!(h.take_paper(1), Err("no ticket of theirs on the desk"));
    assert_eq!(h.take_paper(99), Err("no such person"));
    lay_down(&mut h, &bus, 0);
    assert_eq!(h.take_paper(1), Ok(()));
    assert_eq!(h.take_paper(1), Err("no ticket of theirs on the desk"), "taken once");
}

#[test]
fn in_auto_boarding_the_game_takes_the_ticket() {
    let (mut h, bus) = at_desk("auto", 1);
    h.boarding = "auto".into();
    lay_down(&mut h, &bus, 0);
    h.desk_hand(0, 0.0, &bus);
    assert!(h.paper_on_desk(1), "not at once");
    h.pax_mut(0).unwrap().timer = 0.0;
    h.desk_hand(0, 0.0, &bus);
    assert_eq!(h.pax(0).unwrap().ticket, TICKET_NONE, "taken and on to the place");
}

#[test]
fn a_plugin_makes_somebody_hand_over_a_ticket_only_where_there_is_a_desk() {
    let ty = human_type("set");
    let mut h = PeopleSim::new(Path::new("/nonexistent"), 200);
    h.boarding = "pay".into();
    h.people = vec![boarding(&ty, 1, Task::WalkingToBus)];
    let hand = TicketChange { kind: Some(TICKET_HAND), ..Default::default() };
    // (a cash desk without a money point: nowhere to lay it)
    h.player_cabin = Some(player_bus("set-none").cabin.clone());
    assert_eq!(h.set_ticket(1, &hand, "x"), Err("the bus has no cash desk"));
    h.player_cabin = Some(desk_bus("set").cabin.clone());
    assert_eq!(h.set_ticket(1, &hand, "x"), Ok(()));
    assert_eq!(h.pax(0).unwrap().ticket, TICKET_HAND);
    h.boarding = "walk".into();
    h.pax_mut(0).unwrap().ticket = TICKET_BUY;
    assert_eq!(h.set_ticket(1, &hand, "x"), Err("the bus has no cash desk"));
}

#[test]
fn a_stamp_is_heard_for_the_players_bus_only() {
    let (mut h, _) = at_desk("stamp", 1);
    h.note_stamped(0, BusId::Ai(7));
    assert!(h.take_ticket_events().is_empty());
    h.note_stamped(0, BusId::Player);
    assert_eq!(h.take_ticket_events(), vec![(1, TicketEvent::Stamped)]);
}

#[test]
fn somebody_gone_from_the_desk_frees_it() {
    let (mut h, bus) = at_desk("gone", 1);
    lay_down(&mut h, &bus, 0);
    h.release(0);
    assert_eq!(h.desk_busy, None);
}

#[test]
fn the_ticket_is_drawn_on_the_paper_the_plugin_chose() {
    let (mut h, bus) = at_desk("paper", 1);
    let bad = TicketChange { paper: Some("../x".into()), ..Default::default() };
    assert_eq!(h.set_ticket(1, &bad, "x"), Err("invalid paper: a file name without a folder"));
    let dongseoul = TicketChange { paper: Some("동서울".into()), ..Default::default() };
    assert_eq!(h.set_ticket(1, &dongseoul, "x"), Ok(()));
    lay_down(&mut h, &bus, 0);
    assert!(h.bodies.ops.iter().any(|o| matches!(o, BodyOp::PaperTicket { person: 1, paper: Some(p), .. } if p == "동서울")));
}
