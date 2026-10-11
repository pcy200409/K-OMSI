//! K-OMSI: where a boarding passenger goes in the bus. Instead of a free place at random,
//! the ride decides between sitting and standing (a short one stands by the doors, a long
//! one sits), the elderly sit near the door they came in by, and a seat on an empty bench or
//! by the window, a standing place clear of others, are taken first.

use super::*;

/// From this age a passenger takes a seat whenever one is free.
pub const ELDERLY: f32 = 65.0;

/// What a boarding passenger brings to the choice of a place (bus frame).
#[derive(Debug, Clone, Default)]
pub struct Rider {
    /// How far they mean to ride (km).
    pub ride_km: f32,
    pub age: f32,
    /// The entry they come in by, and the exits of the bus.
    pub entry: Option<Vec3>,
    pub exits: Vec<Vec3>,
}

/// How short the ride is: 1 at 1 km and below, 0 from 4 km on.
fn shortness(km: f32) -> f32 {
    ((4.0 - km) / 3.0).clamp(0.0, 1.0)
}

/// The chance of standing although a seat is free. `base`: the setting's chance (somebody
/// on a long ride); a short ride stands by up to 3 in 4, and once a fifth of the seats or
/// fewer are left those are kept for the ones riding far.
pub fn stand_wish(base: f32, r: &Rider, free_seats: usize, seats: usize) -> f32 {
    if r.age >= ELDERLY {
        return 0.0;
    }
    let short = shortness(r.ride_km);
    let mut p = base + (0.75 - base).max(0.0) * short;
    if seats > 0 && free_seats * 5 <= seats {
        p = p.max(short);
    }
    p.clamp(0.0, 1.0)
}

fn flat(v: Vec3) -> glam::Vec2 {
    v.truncate()
}

/// How much a free place `k` suits the rider (higher is better), before chance.
pub fn place_score(places: &[Seat], taken: &[bool], k: usize, r: &Rider) -> f32 {
    let s = &places[k];
    let here = flat(s.pos);
    let short = shortness(r.ride_km);
    let mut score = 0.0;
    // the nearest place somebody already has
    let near = (0..places.len())
        .filter(|j| *j != k && taken.get(*j).copied().unwrap_or(false))
        .map(|j| (flat(places[j].pos) - here).length())
        .fold(f32::INFINITY, f32::min);
    if s.seated {
        // the bench beside is empty
        if near < 0.9 {
            score -= 1.0;
        }
        // by the window: as far out as the outermost seats
        let out = places.iter().filter(|p| p.seated).map(|p| p.pos.x.abs()).fold(0.0, f32::max);
        if s.pos.x.abs() >= out - 0.25 {
            score += 0.4;
        }
    } else if near < 1.2 {
        // standing: room round them
        score -= 1.5 * (1.2 - near) / 1.2;
    }
    let to_exit = r.exits.iter().map(|e| (flat(*e) - here).length()).fold(f32::INFINITY, f32::min);
    let to_entry = r.entry.map(|e| (flat(e) - here).length());
    if r.age >= ELDERLY {
        // not far from the door they came in by
        if let Some(d) = to_entry {
            score -= 0.2 * d;
        }
    } else if to_exit.is_finite() {
        // a short ride by the doors, a long one deeper in
        score -= 0.15 * to_exit * short;
        score += 0.05 * to_exit.min(10.0) * (1.0 - short);
    }
    score
}

impl PeopleSim {
    /// `reserve_place` for a passenger who chooses (see the module): None when nothing is
    /// free.
    pub fn reserve_place_for(&mut self, bus: BusId, places: &[Seat], off: &[bool], r: &Rider) -> Option<usize> {
        let n = places.len();
        let seats = self.seats.entry(bus).or_insert_with(|| vec![false; n]);
        if seats.len() < n {
            seats.resize(n, false);
        }
        let taken = seats.clone();
        let (sit, stand): (Vec<usize>, Vec<usize>) =
            (0..n).filter(|k| !taken[*k] && !off.get(*k).copied().unwrap_or(false)).partition(|k| places[*k].seated);
        let all_seats = (0..n).filter(|k| places[*k].seated && !off.get(*k).copied().unwrap_or(false)).count();
        let wish = stand_wish(self.stand_chance, r, sit.len(), all_seats);
        let free = if stand.is_empty() || (!sit.is_empty() && self.rand_f() as f32 >= wish) { sit } else { stand };
        let mut best: Option<(usize, f32)> = None;
        for k in free {
            let v = place_score(places, &taken, k, r) + self.rand_f() as f32 * 0.6;
            if best.is_none_or(|(_, b)| v > b) {
                best = Some((k, v));
            }
        }
        let (k, _) = best?;
        self.seats.get_mut(&bus).unwrap()[k] = true;
        Some(k)
    }
}

#[cfg(test)]
mod place_tests {
    use super::*;

    fn place(x: f32, y: f32, seated: bool) -> Seat {
        Seat {
            pos: Vec3::new(x, y, 1.0),
            floor: Vec3::new(x, y, 0.5),
            rot: 0.0,
            seated,
            height: if seated { 0.45 } else { 0.0 },
            omsi_seat: 0,
            switch_var: None,
            taken_var: None,
            group: 0,
        }
    }

    #[test]
    fn the_elderly_always_sit_and_a_short_ride_mostly_stands() {
        let old = Rider { ride_km: 1.0, age: 80.0, ..Default::default() };
        assert_eq!(stand_wish(0.05, &old, 10, 20), 0.0);
        let short = Rider { ride_km: 1.0, age: 30.0, ..Default::default() };
        assert!(stand_wish(0.05, &short, 10, 20) > 0.7);
        let long = Rider { ride_km: 8.0, age: 30.0, ..Default::default() };
        assert!((stand_wish(0.05, &long, 10, 20) - 0.05).abs() < 1e-6);
        // the last seats: a long ride still takes one, a short one leaves it
        assert!(stand_wish(0.05, &long, 2, 20) < 0.1);
        assert_eq!(stand_wish(0.05, &short, 2, 20), 1.0);
    }

    #[test]
    fn an_empty_bench_and_the_window_come_first() {
        // two benches of two: (aisle 0.4, window 1.0) at y 0 and y 3, the aisle seat at y 0
        // already taken
        let places = vec![place(0.4, 0.0, true), place(1.0, 0.0, true), place(0.4, 3.0, true), place(1.0, 3.0, true)];
        let taken = [true, false, false, false];
        let r = Rider { ride_km: 8.0, age: 30.0, ..Default::default() };
        let s: Vec<f32> = (1..4).map(|k| place_score(&places, &taken, k, &r)).collect();
        assert!(s[2] > s[1], "the window of the empty bench before its aisle seat");
        assert!(s[2] > s[0], "the empty bench before the seat beside somebody");
    }

    #[test]
    fn a_short_ride_stands_by_the_exit_and_the_elderly_by_the_entry() {
        let places = vec![place(0.0, 0.0, false), place(0.0, -6.0, false), place(0.9, 4.0, true), place(0.9, -5.0, true)];
        let taken = [false; 4];
        let short = Rider { ride_km: 0.8, age: 30.0, entry: Some(Vec3::new(1.2, 5.0, 0.5)), exits: vec![Vec3::new(1.2, -6.0, 0.5)] };
        assert!(place_score(&places, &taken, 1, &short) > place_score(&places, &taken, 0, &short));
        let old = Rider { age: 75.0, ..short };
        assert!(place_score(&places, &taken, 2, &old) > place_score(&places, &taken, 3, &old));
    }

    #[test]
    fn a_full_bus_takes_nobody() {
        let places = vec![place(0.4, 0.0, true), place(0.0, 2.0, false)];
        let mut h = PeopleSim::new(Path::new("/nonexistent"), 200);
        let r = Rider { ride_km: 5.0, age: 40.0, ..Default::default() };
        let a = h.reserve_place_for(BusId::Player, &places, &[false; 2], &r).unwrap();
        let b = h.reserve_place_for(BusId::Player, &places, &[false; 2], &r).unwrap();
        assert_ne!(a, b);
        assert_eq!(h.reserve_place_for(BusId::Player, &places, &[false; 2], &r), None);
    }
}
