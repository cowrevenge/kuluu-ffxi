//! Zone id -> event-bytecode DAT file id.

use crate::archive::DatRoot;
use crate::zone_dat::ZONE_DAT_THRESHOLD;

/// Added to a zone id below [`ZONE_DAT_THRESHOLD`] to address its
/// event-bytecode DAT. research/xi-tools/docs/reference/ps2_decomp_crosscheck.md
/// "Per-zone DAT ids" recovers it from the PS2 client's `xievent.cpp` as
/// `eventnum + 0x16BC`.
pub const EVENT_DAT_LO_OFFSET: u32 = 5820;

/// The same addition for zone ids from [`ZONE_DAT_THRESHOLD`] up, whose file ids
/// sit in the expansion block the PS2 decompile predates. Measured against both
/// [`crate::client_profile::KNOWN_CLIENTS`] installs, where zone 256 addresses
/// file id 84991 (ROM9/5/53.DAT).
pub const EVENT_DAT_HI_OFFSET: u32 = 84735;

/// VTABLE/FTABLE file id of a zone's event-bytecode DAT. Resolve it through
/// [`crate::archive::DatRoot::resolve`] at every use rather than caching the
/// path: a client build can re-home a base file id and leave the superseded copy
/// on disk, where a cached path still finds it.
pub fn event_dat_file_id(zone_id: u16) -> u32 {
    let id = u32::from(zone_id);
    if zone_id < ZONE_DAT_THRESHOLD {
        id + EVENT_DAT_LO_OFFSET
    } else {
        id + EVENT_DAT_HI_OFFSET
    }
}

/// Highest zone id carrying an event-bytecode DAT on either
/// [`crate::client_profile::KNOWN_CLIENTS`] install; every id above it is marked
/// missing in VTABLE on both. `ffxi-dat/tests/event_dat_locate.rs` pins that,
/// so a build that adds zones fails the pin rather than losing them silently.
pub const EVENT_DAT_ZONE_ID_MAX: u16 = 299;

/// The zone ids `root` actually carries an event-bytecode DAT for, ascending.
/// VTABLE is the authority for presence, so a zone the build omits is simply
/// absent here.
pub fn event_dat_zones(root: &DatRoot) -> Vec<u16> {
    (0..=EVENT_DAT_ZONE_ID_MAX)
        .filter(|&zone| root.resolve(event_dat_file_id(zone)).is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_switch_at_the_zone_dat_threshold() {
        assert_eq!(event_dat_file_id(0), EVENT_DAT_LO_OFFSET);
        assert_eq!(
            event_dat_file_id(ZONE_DAT_THRESHOLD - 1),
            u32::from(ZONE_DAT_THRESHOLD - 1) + EVENT_DAT_LO_OFFSET
        );
        assert_eq!(
            event_dat_file_id(ZONE_DAT_THRESHOLD),
            u32::from(ZONE_DAT_THRESHOLD) + EVENT_DAT_HI_OFFSET
        );
    }
}
