//! Self-heading wire basis. The remote-side half of the heading-basis verification lives in
//! `combat_stance::tests` (worldAngle grid, byte round trip, StepTo direction, same-snapshot
//! application). This file holds the self side: it crosses into kuluu-session (a dev-dependency),
//! and on this pinned nightly referencing that crate from inside the lib-test target trips a rustc
//! resolution bug that misattributes E0277 to unrelated code in scheduler_runtime.

use bevy::prelude::*;

#[test]
fn self_heading_byte_matches_lsb_facing() {
    // The byte kuluu sends for a given facing must be what LSB reads back: for every byte b, the
    // wire direction the walker moves along (kuluu_session::state::heading_to_forward) must equal
    // CPathFind::StepTo's travel direction for that same byte (vendor/server/src/map/ai/helpers/
    // pathfind.cpp, pinned vendor/server): radians = (1 - rotation / 256) * 2 * PI, move by
    // (cosf(radians), sinf(radians)). worldAngle of any position pair along it quantizes to b or a
    // neighbor.
    for b in 0u8..=255 {
        let (fx, fy) = kuluu_session::state::heading_to_forward(b);
        let wire_fwd = Vec2::new(fx, fy).normalize();
        let radians = (1.0 - b as f32 / 256.0) * std::f32::consts::TAU;
        let step_dir = Vec2::new(radians.cos(), radians.sin()).normalize();
        assert!(
            wire_fwd.dot(step_dir) > 1.0 - 1e-6,
            "byte {b}: the sent facing is not LSB's travel direction"
        );
    }
}
