//! Scraped from `vendor/server/src/map/zone_entities.cpp` at build time. The radius LSB
//! streams entities to a client within, measured player-to-entity
//! (`isWithinDistance(PEntity->loc.p, PCurrentChar->loc.p, ENTITY_RENDER_DISTANCE)`), so it is
//! also the outer bound on anything the client can know about. Edit the scraper or the LSB
//! pin, never the value.

include!(concat!(env!("OUT_DIR"), "/entity_stream_table.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    // Relative to the package root, which is cargo's cwd for a test binary. build.rs reads the
    // same file through the same relative path, so a tree that compiled has it.
    const LSB_ZONE_ENTITIES_CPP: &str = "../vendor/server/src/map/zone_entities.cpp";

    #[test]
    fn scraped_render_distance_still_matches_the_lsb_declaration() {
        let src = std::fs::read_to_string(LSB_ZONE_ENTITIES_CPP)
            .unwrap_or_else(|e| panic!("reading {LSB_ZONE_ENTITIES_CPP}: {e}"));
        let declared: Vec<f32> = src
            .lines()
            .filter_map(|line| {
                let (decl, rhs) = line.split_once('=')?;
                let decl = decl.trim();
                if !decl.starts_with("constexpr") || !decl.ends_with("ENTITY_RENDER_DISTANCE") {
                    return None;
                }
                rhs.trim()
                    .trim_end_matches(';')
                    .trim()
                    .trim_end_matches(['f', 'F'])
                    .parse::<f32>()
                    .ok()
            })
            .collect();

        assert_eq!(
            declared.len(),
            1,
            "expected exactly one ENTITY_RENDER_DISTANCE declaration, found {declared:?}"
        );
        assert_eq!(
            declared[0], ENTITY_RENDER_DISTANCE_YALMS,
            "the generated constant drifted from the LSB source it is scraped from"
        );
    }
}
