//! The plugins' say in what a passenger of the player's bus does (K-OMSI): hold them where
//! they stand (in the doorway, at the validator or the cash desk, in the aisle), keep them
//! in the bus past their stop or the terminus, get them off at the next stop - or wherever
//! the doors open - and move them to another place while the bus runs (`people.hold`,
//! `people.stay`, `people.leave`, `people.move` of the Lua plugins).
//!
//! Only the riders and the people boarding the player's bus take orders: the town's other
//! buses are none of the plugins' affair. A move is carried out in the people's next step,
//! where the bus's cabin is at hand (`apply_order`).

use super::HashMap;

use super::model::{BusId, PeopleSim};
use super::pax::Task;
use super::cabin::BusNow;

/// The longest a plugin may hold somebody at once (s of game time).
pub const HOLD_MAX: f64 = 120.0;

/// Where a plugin moves a rider to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaxOrder {
    /// A free seat.
    Seat,
    /// A free standing place.
    Stand,
    /// The free standing place nearest an exit (getting ready to get off, without asking
    /// to stop).
    Door,
}

impl PaxOrder {
    pub fn parse(s: &str) -> Option<PaxOrder> {
        match s {
            "seat" => Some(PaxOrder::Seat),
            "stand" => Some(PaxOrder::Stand),
            "door" => Some(PaxOrder::Door),
            _ => None,
        }
    }
}

impl PeopleSim {
    fn player_pax(&self, person: u32) -> Result<usize, &'static str> {
        let Some(i) = self.people.iter().position(|p| p.id == person) else { return Err("no such person") };
        let Some(p) = self.pax(i) else { return Err("not a passenger") };
        if p.bus != Some(BusId::Player) && p.inside != Some(BusId::Player) {
            return Err("not a passenger of the player's bus");
        }
        Ok(i)
    }

    /// Passenger `i` is held by a plugin now.
    pub fn held(&self, i: usize) -> bool {
        self.pax(i).and_then(|p| p.held_until).is_some_and(|t| self.time < t)
    }

    /// `people.hold`: they stop where they are for `seconds` (0 lets them go).
    pub fn pax_hold(&mut self, person: u32, seconds: f64) -> Result<(), &'static str> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err("invalid seconds");
        }
        let i = self.player_pax(person)?;
        let until = (seconds > 0.0).then(|| self.time + seconds.min(HOLD_MAX));
        self.pax_mut(i).unwrap().held_until = until;
        Ok(())
    }

    /// `people.stay`: they do not get off - not at their stop, not at the terminus - until
    /// told to go (`false`, or `people.leave`).
    pub fn pax_stay(&mut self, person: u32, stay: bool) -> Result<(), &'static str> {
        let i = self.player_pax(person)?;
        let p = self.pax_mut(i).unwrap();
        if p.inside != Some(BusId::Player) {
            return Err("not in the player's bus");
        }
        p.stay = stay;
        Ok(())
    }

    /// `people.leave`: they get up and ask to stop; off at the next stop, or - `anywhere` -
    /// as soon as the driver opens their door.
    pub fn pax_leave(&mut self, person: u32, anywhere: bool) -> Result<(), &'static str> {
        let i = self.player_pax(person)?;
        let p = self.pax_mut(i).unwrap();
        if p.inside != Some(BusId::Player) {
            return Err("not in the player's bus");
        }
        if !matches!(p.task, Task::SittingInBus | Task::InBusToExit) {
            return Err("not at their place yet");
        }
        p.stay = false;
        p.leave_anywhere = anywhere;
        p.order = None;
        if p.task == Task::SittingInBus {
            p.leave_now = true;
        }
        Ok(())
    }

    /// `people.move`: to another place of the bus, walking there while it runs.
    pub fn pax_move_to(&mut self, person: u32, to: PaxOrder) -> Result<(), &'static str> {
        let i = self.player_pax(person)?;
        let p = self.pax(i).unwrap();
        if p.inside != Some(BusId::Player) {
            return Err("not in the player's bus");
        }
        if p.task != Task::SittingInBus {
            return Err("not at their place");
        }
        // (a free place of the kind asked for: the cabin's places as the people took them)
        if let Some(cabin) = self.player_cabin.as_ref() {
            let taken = self.seats.get(&BusId::Player);
            let free = cabin.seats.iter().enumerate().any(|(k, s)| {
                !taken.is_some_and(|t| t.get(k).copied().unwrap_or(false))
                    && match to {
                        PaxOrder::Seat => s.seated,
                        PaxOrder::Stand | PaxOrder::Door => !s.seated,
                    }
            });
            if !free {
                return Err("no free place of that kind");
            }
        }
        self.pax_mut(i).unwrap().order = Some(to);
        Ok(())
    }

    /// The order a plugin gave rider `i`, carried out (from `pax_tick`, the bus at hand):
    /// to leave - the stop asked for, the walk to the exit - or to another place.
    pub(crate) fn apply_order(&mut self, i: usize, buses: &[BusNow], bus_ix: &HashMap<BusId, usize>, world: &dyn super::World) {
        let Some(p) = self.pax(i) else { return };
        if p.task != Task::SittingInBus {
            if p.task != Task::InBusToPlace {
                let pp = self.pax_mut(i).unwrap();
                pp.order = None;
                pp.leave_now = false;
            }
            return;
        }
        let Some(bn) = p.inside.and_then(|b| bus_ix.get(&b).map(|k| &buses[*k])) else { return };
        if p.leave_now {
            self.pax_mut(i).unwrap().leave_now = false;
            self.set_task(i, Task::InBusToExit, buses, bus_ix, world);
            return;
        }
        let Some(order) = p.order else { return };
        let old = p.seat;
        let from = old.and_then(|k| bn.cabin.seats.get(k)).map(|s| s.pos).unwrap_or(p.pos.as_vec3());
        // (the place they have now counts as taken while another is chosen)
        let off: Vec<bool> = bn.cabin.seats.iter().enumerate().map(|(k, s)| {
            bn.places_off.get(k).copied().unwrap_or(false)
                || Some(k) == old
                || match order {
                    PaxOrder::Seat => !s.seated,
                    PaxOrder::Stand | PaxOrder::Door => s.seated,
                }
        }).collect();
        let new = match order {
            PaxOrder::Door => self.reserve_by_a_door(bn.id, &bn.cabin.seats, &off, &bn.cabin.exits),
            PaxOrder::Seat | PaxOrder::Stand => {
                // (`reserve_place` takes a seat first; only seats or only standing places
                // are left open here)
                let keep = self.stand_chance;
                self.stand_chance = if order == PaxOrder::Stand { 1.0 } else { 0.0 };
                let k = self.reserve_place(bn.id, &bn.cabin.seats, &off);
                self.stand_chance = keep;
                k
            }
        };
        self.pax_mut(i).unwrap().order = None;
        let Some(new) = new else { return };
        if let Some(k) = old {
            self.free_seat(bn.id, k);
        }
        let all = bn.cabin.all_points();
        let start = bn.cabin.omsi_nearest(from, &all, false, true, None, None);
        {
            let pp = self.pax_mut(i).unwrap();
            pp.seat = Some(new);
            pp.task = Task::InBusToPlace;
            pp.pax_state = 1.0;
            pp.pt = start;
            if let Some(q) = start.and_then(|k| bn.cabin.graph.points.get(k)) {
                pp.pos = q.as_dvec3();
            }
            pp.ticket = super::pax::TICKET_NONE;
            pp.sub = 0;
        }
        self.route_to_place(i, bn);
        if self.pax(i).unwrap().pt_target.is_none() {
            // (no way there along the paths: at it at once)
            self.pax_mut(i).unwrap().task = Task::InBusToPlace;
            self.set_task(i, Task::SittingInBus, buses, bus_ix, world);
        }
    }
}

#[cfg(test)]
#[path = "pax_orders_tests.rs"]
mod tests;
