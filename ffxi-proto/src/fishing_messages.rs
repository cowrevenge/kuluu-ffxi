//! Which fishing message a zone-dialog `MesNum` is.
//!
//! LSB adds a per-zone base (`FISHING_MESSAGE_OFFSET`, a text id in the zone's
//! `IDs.lua`) to a FISHMESSAGEOFFSET constant and sends the sum as the TALKNUM
//! family's `MesNum` (vendor/server/src/map/utils/fishingutils.cpp). The dialog
//! DAT lookup that renders the line needs only the sum; recovering *which*
//! message it was — to label the mini-game bar "Small Fish" vs "Large Fish" the
//! way retail does — needs the base subtracted back out.
//!
//! Both halves are scraped: the per-zone bases from LSB's zone scripts, the
//! constants from `fishingutils.h`.

include!(concat!(env!("OUT_DIR"), "/fishing_zone_offset_table.rs"));

/// `FISHMESSAGEOFFSET_*` with the prefix stripped — `kind::HOOKED_LARGE_FISH`
/// and friends.
pub mod kind {
    include!(concat!(env!("OUT_DIR"), "/fishing_message_consts.rs"));
}

/// The zone's fishing-message base, or `None` for a zone LSB declares no
/// fishing messages for.
pub fn zone_offset(zone_id: u16) -> Option<u16> {
    FISHING_ZONE_OFFSET
        .binary_search_by_key(&zone_id, |&(k, _)| k)
        .ok()
        .map(|i| FISHING_ZONE_OFFSET[i].1)
}

/// Which fishing message `mes_num` is in `zone_id`, or `None` when it is not a
/// fishing message at all (any other zone-dialog line, or an unmapped zone).
pub fn classify(zone_id: u16, mes_num: u16) -> Option<u8> {
    let base = zone_offset(zone_id)?;
    let delta = mes_num.checked_sub(base)?;
    u8::try_from(delta).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the two scrapes against each other and against LSB's own comments:
    /// Port San d'Oria's base is 7264 and `_NOROD` sits at +1, so the "you can't
    /// fish without a rod" line is 7265 there.
    #[test]
    fn port_san_doria_base_and_offsets_compose() {
        const PORT_SAN_DORIA: u16 = 232;
        assert_eq!(zone_offset(PORT_SAN_DORIA), Some(7264));
        assert_eq!(kind::NOROD, 0x01);
        assert_eq!(classify(PORT_SAN_DORIA, 7265), Some(kind::NOROD));
    }

    /// The two "something caught the hook" lines retail sizes the mini-game bar
    /// off (research/xim FishHppUi.kt). Pinned because they are far apart in the
    /// enum and a scrape that mismatched them would mislabel every catch.
    #[test]
    fn hook_message_kinds_keep_their_lsb_values() {
        assert_eq!(kind::HOOKED_SMALL_FISH, 0x08);
        assert_eq!(kind::HOOKED_LARGE_FISH, 0x32);
        assert_ne!(kind::HOOKED_ITEM, kind::HOOKED_SMALL_FISH);
    }

    #[test]
    fn a_message_below_the_zone_base_is_not_a_fishing_message() {
        const PORT_SAN_DORIA: u16 = 232;
        assert_eq!(classify(PORT_SAN_DORIA, 24), None, "HOMEPOINT_SET");
        assert_eq!(classify(u16::MAX, 7265), None, "unmapped zone");
    }

    /// Every zone LSB gives a fishing base to must land in the table; a parse
    /// that silently matched nothing would still pass a single-zone check.
    #[test]
    fn the_scrape_covers_the_whole_zone_set() {
        assert!(
            FISHING_ZONE_OFFSET.len() > 100,
            "only {} zones scraped",
            FISHING_ZONE_OFFSET.len()
        );
        assert!(FISHING_ZONE_OFFSET.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
