//! Power cuts in a thunderstorm (K-OMSI, the "Storm power cuts" setting): a lightning
//! strike close by may put the power out for a few minutes - every traffic light of the
//! loaded map dark (the junctions left to the right of way, `traffic::set_outage`) and the
//! street lamps out - before it comes back.

/// A strike this close (m) may cut the power, at this chance.
const NEAR: f32 = 1200.0;
const CHANCE: f64 = 0.35;
/// How long the power stays out (game seconds).
const SHORTEST: f64 = 90.0;
const LONGEST: f64 = 300.0;

#[derive(Default)]
pub(crate) struct Outage {
    /// The game time (run time, s) the power comes back at.
    until: f64,
    active: bool,
    rng: u64,
    /// A message for the driver (`frame_ui` shows it).
    pub(crate) notice: Option<String>,
}

impl Outage {
    fn random(&mut self) -> f64 {
        // (xorshift64*, seeded by the first strike's time)
        let mut x = self.rng.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A strike `distance` m away at game time `now`.
    pub(crate) fn strike(&mut self, on: bool, distance: f32, now: f64) {
        if !on || distance > NEAR {
            return;
        }
        if self.rng == 0 {
            self.rng = (now * 1000.0) as u64 ^ 0x9e37_79b9_7f4a_7c15;
        }
        if self.random() >= CHANCE {
            return;
        }
        let back = now + SHORTEST + (LONGEST - SHORTEST) * self.random();
        log::info!("storm: a strike {distance:.0} m away cut the power until {back:.0} s (now {now:.0} s)");
        self.until = self.until.max(back);
    }

    /// The power at game time `now` (`on`: the setting): the traffic lights and the street
    /// lamps follow it.
    pub(crate) fn tick(&mut self, on: bool, now: f64) {
        let active = on && now < self.until;
        if active != self.active {
            self.active = active;
            omsi_sim::traffic::set_outage(active);
            self.notice = Some(if active {
                "Power cut: lightning struck nearby - the traffic lights and the street lamps are out".into()
            } else {
                "The power is back: the traffic lights work again".into()
            });
            log::info!("storm: power {}", if active { "out" } else { "back" });
        }
    }

    pub(crate) fn active(&self) -> bool {
        self.active
    }
}
