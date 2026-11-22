//! K-OMSI: the stop the player's riders watch for.
//!
//! The riders decide where to ring the bell for their stop once the bus leaves the last
//! one: a random point between 100 m before it and the whole way there
//! (`Pax::wants_stop_at`). Given the next *calling* stop, a timetable with stations passed
//! through on a motorway (Korean maps put them there to keep the times) made that way
//! tens of kilometres, and riders rang and stood at the door in the middle of the motorway.
//! So the riders get the next stop of the timetable again, stations passed through
//! included, as before openOMSI #3833; only a stop closed by a plugin is skipped. The
//! navigator, the announcements and the plugins keep `next_calling_stop`.

use super::{next_calling_stop, PlannedStop};

/// The index of the stop the riders watch for: the next stop that is not closed.
pub fn next_rider_stop(stops: &[PlannedStop], next: usize) -> usize {
    (next..stops.len())
        .find(|&i| stops[i].closed.is_none())
        .unwrap_or_else(|| next_calling_stop(stops, next))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(id: i64, stops: bool, closed: bool) -> PlannedStop {
        PlannedStop {
            object_id: id,
            name: format!("stop {id}"),
            arr: 0.0,
            dep: 0.0,
            position: None,
            dir: Default::default(),
            stops,
            closed: closed.then(|| "plugin diversion".to_string()),
        }
    }

    #[test]
    fn riders_watch_a_station_passed_through() {
        let stops = vec![stop(1, true, false), stop(2, false, false), stop(3, false, false), stop(4, true, false)];
        assert_eq!(next_rider_stop(&stops, 1), 1);
        assert_eq!(next_rider_stop(&stops, 2), 2);
        assert_eq!(next_rider_stop(&stops, 3), 3);
        // the others still skip it
        assert_eq!(next_calling_stop(&stops, 1), 3);
    }

    #[test]
    fn riders_skip_a_closed_stop() {
        let stops = vec![stop(1, true, false), stop(2, true, true), stop(3, false, false), stop(4, true, false)];
        assert_eq!(next_rider_stop(&stops, 1), 2);
        let stops = vec![stop(1, true, false), stop(2, true, true), stop(3, true, true)];
        assert_eq!(next_rider_stop(&stops, 1), next_calling_stop(&stops, 1));
    }
}
