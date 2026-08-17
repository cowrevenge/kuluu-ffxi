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

include!(concat!(env!("OUT_DIR"), "/fishing_message_tables.rs"));

/// Whether `delta` past a zone's fishing base is a FISHMESSAGEOFFSET LSB can
/// put on the wire.
pub fn is_known_offset(delta: u16) -> bool {
    u8::try_from(delta)
        .ok()
        .is_some_and(|d| OFFSETS.binary_search(&d).is_ok())
}

/// The trailing `//` comment LSB's header records for `offset`, when it
/// records one. Only some are verbatim retail lines fit for landmark-matching
/// against an installed dialog DAT (the era reconciliation uses a few, pinned
/// by `landmark_texts_are_scraped`); others are templates or editorial notes,
/// so verify an offset's text before matching on it.
pub fn offset_text(offset: u8) -> Option<&'static str> {
    TEXTS
        .binary_search_by_key(&offset, |&(o, _)| o)
        .ok()
        .map(|i| TEXTS[i].1)
}

/// Whether LSB routes the fishing message `offset` through the TALKNUM-family
/// opcode `opcode`. The packet choice is fixed per message
/// (vendor/server/src/map/utils/fishingutils.cpp — every `pushPacket` /
/// `PushPacket` call site): TALKNUMWORK2's fishing constructor carries only
/// the three catch announcements (0x027_talknumwork2.cpp:30 — "this is how
/// it's used for Fishing messages currently"), TALKNUMNAME only the monster
/// and chest broadcasts, TALKNUMWORK only keen angler's sense, and TALKNUM
/// everything else.
pub fn carried_by(opcode: u16, offset: u8) -> bool {
    use crate::map::s2c;
    let work2 = matches!(
        offset,
        kind::CATCH | kind::CATCH_MULTI | kind::CATCH_INV_FULL
    );
    let name = matches!(offset, kind::MONSTER | kind::CATCH_CHEST);
    let work = offset == kind::KEEN_ANGLERS_SENSE;
    if opcode == s2c::TALKNUMWORK2 {
        work2
    } else if opcode == s2c::TALKNUMNAME {
        name
    } else if opcode == s2c::TALKNUMWORK {
        work
    } else if opcode == s2c::TALKNUM {
        !work2 && !name && !work && is_known_offset(u16::from(offset))
    } else {
        false
    }
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

    /// The opcode routing the era reconciliation depends on: catch
    /// announcements are the only TALKNUMWORK2 fishing traffic, and no
    /// TALKNUM-carried offset may collide with another opcode's family.
    #[test]
    fn opcode_families_partition_the_offsets() {
        use crate::map::s2c;
        assert!(carried_by(s2c::TALKNUMWORK2, kind::CATCH));
        assert!(carried_by(s2c::TALKNUMWORK2, kind::CATCH_MULTI));
        assert!(carried_by(s2c::TALKNUMWORK2, kind::CATCH_INV_FULL));
        assert!(!carried_by(s2c::TALKNUMWORK2, kind::NOCATCH));
        assert!(carried_by(s2c::TALKNUMNAME, kind::MONSTER));
        assert!(carried_by(s2c::TALKNUMNAME, kind::CATCH_CHEST));
        assert!(carried_by(s2c::TALKNUMWORK, kind::KEEN_ANGLERS_SENSE));
        assert!(carried_by(s2c::TALKNUM, kind::NOCATCH));
        assert!(carried_by(s2c::TALKNUM, kind::HOOKED_SMALL_FISH));
        assert!(!carried_by(s2c::TALKNUM, kind::CATCH));
        assert!(!carried_by(s2c::TALKNUM, kind::KEEN_ANGLERS_SENSE));
        assert!(!carried_by(s2c::TALKNUMWORK, kind::CATCH));
        assert!(!carried_by(s2c::TALKNUMNAME, kind::NOCATCH));
        assert!(!carried_by(0xFFFF, kind::NOCATCH));
    }

    /// A delta past `u8::MAX` must never alias down onto a real offset.
    #[test]
    fn an_offset_beyond_a_byte_is_not_known() {
        assert!(is_known_offset(u16::from(kind::NOCATCH)));
        assert!(!is_known_offset(256 + u16::from(kind::NOCATCH)));
    }

    /// The landmark texts the era reconciliation scans for must survive the
    /// scrape verbatim — they are prefix-matched against installed DAT entries.
    #[test]
    fn landmark_texts_are_scraped() {
        assert_eq!(
            offset_text(kind::NOROD),
            Some("You can't fish without a rod in your hands.")
        );
        assert_eq!(
            offset_text(kind::NOCATCH),
            Some("You didn't catch anything.")
        );
        assert_eq!(
            offset_text(kind::HOOKED_SMALL_FISH),
            Some("Something caught the hook!")
        );
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
