//! Canonical FFXI packet id ↔ name tables, scraped at build time from LSB's
//! authoritative `PacketS2C` / `PacketC2S` enums
//! (`vendor/server/src/map/enums/packet_{s2c,c2s}.h`). LSB has absorbed the
//! atom0s/XiPackets layouts; consult that repo for per-field semantics.
//! Names are the canonical suffix (e.g. `0x00D` → `"CHAR_PC"`).

include!(concat!(env!("OUT_DIR"), "/packet_names_s2c_table.rs"));
include!(concat!(env!("OUT_DIR"), "/packet_names_c2s_table.rs"));

fn lookup(table: &'static [(u16, &'static str)], id: u16) -> Option<&'static str> {
    table
        .binary_search_by_key(&id, |&(k, _)| k)
        .ok()
        .map(|i| table[i].1)
}

pub fn s2c_name(id: u16) -> Option<&'static str> {
    lookup(PACKET_NAMES_S2C, id)
}

pub fn c2s_name(id: u16) -> Option<&'static str> {
    lookup(PACKET_NAMES_C2S, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scraped_tables_are_nonempty_and_sorted() {
        assert!(PACKET_NAMES_S2C.len() > 100);
        assert!(PACKET_NAMES_C2S.len() > 100);
        assert!(PACKET_NAMES_S2C.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(PACKET_NAMES_C2S.windows(2).all(|w| w[0].0 < w[1].0));
    }
}
