//! K-OMSI: paper tickets handed to the driver (`TICKET_HAND`) - the Korean intercity and
//! express buses' printed tickets. A plugin makes a passenger of the player's bus one who
//! hands over a ticket (`people.set_ticket{kind = "hand"}`); at the cash desk they reach
//! for the money point and lay the ticket down, and wait there - the people behind them
//! too - until the driver takes it (a click on the ticket, or `people.take_ticket`). Only
//! then do they go on to their place: nobody rides without handing it over.
//!
//! The plugins hear of it (`ticket_handed`, `ticket_taken`), and of a ticket stamped at a
//! validator of the player's bus (`ticket_stamped`: a card or a paper ticket scanned at the
//! reader), so a terminal can count them.

use super::*;

/// What the plugins hear of the tickets of the player's bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketEvent {
    /// The passenger laid their paper ticket on the desk.
    Handed,
    /// The driver took it.
    Taken,
    /// The passenger stamped (scanned) a ticket at a validator.
    Stamped,
}

impl TicketEvent {
    pub fn name(self) -> &'static str {
        match self {
            TicketEvent::Handed => "ticket_handed",
            TicketEvent::Taken => "ticket_taken",
            TicketEvent::Stamped => "ticket_stamped",
        }
    }
}

/// The paper tickets at the player's cash desk.
#[derive(Debug, Default, Clone)]
pub struct PaperDesk {
    /// Whose ticket lies on the desk now.
    pub on_desk: Vec<u32>,
    /// Whose ticket the driver has taken (the passenger goes on in their next step).
    pub taken: Vec<u32>,
    /// What happened since the app last looked (`take_ticket_events`): the person and what.
    pub events: Vec<(u32, TicketEvent)>,
}

/// How long the passenger reaches for the desk before the ticket lies there (s).
const LAY_DOWN: f32 = 0.6;
/// In `auto` boarding the game plays the driver: it takes the ticket after this (s).
const AUTO_TAKE: f32 = 1.5;

impl PeopleSim {
    /// A ticket of the player's bus was stamped at a validator by passenger `i`.
    pub fn note_stamped(&mut self, i: usize, bus: BusId) {
        if bus == BusId::Player {
            let id = self.people[i].id;
            self.paper.events.push((id, TicketEvent::Stamped));
        }
    }

    /// What the plugins hear of the tickets since the last call.
    pub fn take_ticket_events(&mut self) -> Vec<(u32, TicketEvent)> {
        std::mem::take(&mut self.paper.events)
    }

    /// Whether `person`'s paper ticket lies on the desk now.
    pub fn paper_on_desk(&self, person: u32) -> bool {
        self.paper.on_desk.contains(&person)
    }

    /// The driver takes `person`'s paper ticket off the desk (`people.take_ticket`, a click
    /// on it). The reason when there is none of theirs.
    pub fn take_paper(&mut self, person: u32) -> Result<(), &'static str> {
        let Some(k) = self.paper.on_desk.iter().position(|p| *p == person) else {
            return Err(if self.people.iter().any(|p| p.id == person) { "no ticket of theirs on the desk" } else { "no such person" });
        };
        self.paper.on_desk.remove(k);
        self.paper.taken.push(person);
        self.paper.events.push((person, TicketEvent::Taken));
        self.bodies.ops.push(BodyOp::PaperGone { person });
        Ok(())
    }

    /// The name and price of `person`'s ticket as the desk asks for it (empty for nobody).
    pub fn ticket_of(&self, person: u32) -> (String, f32) {
        self.people
            .iter()
            .find(|p| p.id == person)
            .and_then(|p| if let State::Pax(x) = &p.state { Some(self.ticket_request(x)) } else { None })
            .map(|(n, v)| (n.trim().to_string(), v))
            .unwrap_or_default()
    }

    /// The passenger waiting at the desk with their paper ticket laid down, for the HUD:
    /// the name of the ticket (from `ticket_request`).
    pub fn paper_waiting(&self) -> Option<(u32, String)> {
        self.paper.on_desk.iter().find_map(|&id| {
            let p = self.people.iter().find(|p| p.id == id)?;
            let State::Pax(x) = &p.state else { return None };
            Some((id, self.ticket_request(x).0))
        })
    }

    /// The cash desk with a paper ticket (case 4 for `TICKET_HAND`): wait for the desk,
    /// reach for the money point and lay the ticket down, wait for the driver to take it,
    /// on to the place.
    pub fn desk_hand(&mut self, i: usize, dt: f32, bn: &BusNow) {
        let _ = dt;
        let id = self.people[i].id;
        let p = self.pax(i).unwrap().clone();
        let auto = self.boarding.eq_ignore_ascii_case("auto");
        if p.sub == 3 {
            if self.ticket_pending(i) || self.held(i) {
                return;
            }
            if self.desk_busy.is_some_and(|d| d != id) {
                return;
            }
            // (somebody's coins still on the desk: the driver clears it first)
            let money_on_desk = self.money.as_ref().is_some_and(|m| m.placed.iter().any(|(_, _, _, change, _)| !*change));
            if money_on_desk && !auto {
                return;
            }
            self.desk_busy = Some(id);
            let pp = self.pax_mut(i).unwrap();
            pp.reach = true;
            pp.look_driver = true;
            pp.timer = LAY_DOWN;
            pp.sub = 4;
        } else if p.sub == 4 {
            if p.timer > 0.0 {
                return;
            }
            if let Some((pos, var, parent)) = bn.cabin.money_var.clone() {
                let paper = p.plugin().paper.clone();
                self.bodies.ops.push(BodyOp::PaperTicket { person: id, point: pos, var, parent, paper });
            }
            self.paper.on_desk.retain(|q| *q != id);
            self.paper.on_desk.push(id);
            self.paper.taken.retain(|q| *q != id);
            self.paper.events.push((id, TicketEvent::Handed));
            let pp = self.pax_mut(i).unwrap();
            pp.reach = false;
            pp.timer = AUTO_TAKE;
            pp.sub = 5;
        } else if p.sub == 5 {
            if auto && p.timer <= 0.0 && self.paper_on_desk(id) {
                let _ = self.take_paper(id);
            }
            let Some(k) = self.paper.taken.iter().position(|q| *q == id) else { return };
            self.paper.taken.remove(k);
            self.desk_busy = None;
            self.boarded += 1;
            self.served += 1;
            self.route_to_place(i, bn);
            let pp = self.pax_mut(i).unwrap();
            pp.reach = false;
            pp.talking = false;
            pp.look_driver = false;
            pp.sub = 0;
            pp.ticket = TICKET_NONE;
            pp.pt = bn.cabin.sale.and_then(|s| s.0);
        }
    }

    /// Somebody of the desk has gone (off the bus, gone from the world) with their ticket
    /// still lying there or the desk still theirs: the desk is freed and the ticket kept for
    /// the driver to take (it is drawn until taken).
    pub fn paper_forget(&mut self, person: u32) {
        self.paper.taken.retain(|q| *q != person);
        if self.desk_busy == Some(person) {
            self.desk_busy = None;
        }
    }
}
