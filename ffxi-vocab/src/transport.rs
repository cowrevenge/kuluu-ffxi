/// One crossing of a scheduled ship, in the seconds-since-Vana'diel-epoch
/// cycle vendor/server/src/map/transports/voyage.h keeps: the cycle repeats
/// every `every` seconds shifted by `offset`; boarding closes `boarding_ends`
/// seconds into it, the ship leaves the berth at `departs` (the departing
/// phase start plus the seconds until it is hidden) and riders are put ashore
/// at `disembark`, which wraps past the cycle end for every airship.
#[derive(Debug, Clone, Copy)]
pub struct Schedule {
    pub voyage_zone: u16,
    pub ship: u32,
    pub boundary: u16,
    pub offset: u32,
    pub every: u32,
    pub boarding_ends: u32,
    pub departs: u32,
    pub disembark: u32,
}

include!(concat!(env!("OUT_DIR"), "/transport_table.rs"));

pub fn voyage(zone: u16) -> Option<&'static Schedule> {
    let mut schedules = SCHEDULES
        .iter()
        .filter(|s| s.voyage_zone == zone && s.voyage_zone != 0);
    let schedule = schedules.next()?;
    schedules.next().is_none().then_some(schedule)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fallback_requires_an_unambiguous_voyage_schedule() {
        assert!(voyage(228).is_some());
        assert!(voyage(0).is_none());
        for zone in 223..=226 {
            assert!(voyage(zone).is_none());
        }
    }
}
