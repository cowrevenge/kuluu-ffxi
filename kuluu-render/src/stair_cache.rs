//! Persistent stair-plane cache. Decouples the plane query from the live
//! character march.
//!
//! Motivation: the march (in `apply_self_prediction_system`) can only detect
//! stairs when the character's ring probes hit a riser edge. That means the
//! plane only "exists" while the character is basically already on the stairs
//! — no early snap on approach, no plane extent past what the current tick's
//! probes cover, and no memory of a staircase you were just on two seconds ago.
//!
//! This cache holds fitted stair planes independent of the character. Planes
//! are seeded by the live march the first time it detects a staircase, then
//! immediately extended via straight-line raycasts along the flight axis in
//! both directions until they reach the toe (flat ground meeting sloped rise
//! at the bottom) and the crest (top step meeting flat landing).
//!
//! Rendering then queries the cache at the character's current XZ. If any
//! cached plane's extent contains it, use that plane's Y. Otherwise fall back
//! to whatever the live march produced this tick, and finally to wire Y.
//!
//! Cache is scoped to zone: enter a new zone, planes clear. No serialization,
//! no proactive scanning — we only remember what the player has actually
//! walked over this session.

use bevy::prelude::*;

use crate::dat_mzb::MzbCollisionGeometry;

/// Extension probe spacing along the flight axis. Same 0.1-yalm stride the
/// live march uses so the two agree on what counts as a valid tread.
const PROBE_STEP: f32 = 0.1;

/// Maximum extension distance either direction from the seed plane's original
/// endpoints. A generous safety cap — real FFXI flights top out around 20
/// yalms, this handles Bastok Zeruhn Mines and similar long stairs with room.
const MAX_EXTEND: f32 = 30.0;

/// A probe's actual Y must be within this of the plane's predicted Y to count
/// as still-on-the-flight. Loose enough to absorb floating-point noise in
/// long collision meshes; tight enough that a landing (flat surface, plane
/// keeps rising) breaks the extension immediately.
const ON_PLANE_TOL: f32 = 0.20;

/// Backward extension stops once the plane predicts a Y that has dropped
/// meaningfully BELOW the actual ground — that means we're past the toe and
/// walking away from the stairs onto lower flat terrain. A small threshold
/// so the toe cell (where plane meets flat within a hair) still counts as
/// part of the plane's extent.
const TOE_STOP_TOL: f32 = 0.05;

/// Two planes are considered the same physical staircase if their origins are
/// within this distance AND their axes / slopes agree. Prevents the cache
/// from filling with near-duplicate planes as the player walks the same
/// flight and the march re-fires each tick.
const MERGE_ORIGIN_DIST: f32 = 2.0;
const MERGE_SLOPE_TOL: f32 = 0.08;
const MERGE_AXIS_DOT_MIN: f32 = 0.90;

/// Half-width perpendicular to flight axis where the plane applies. FFXI
/// staircases are typically 4-6 yalms wide, so 3 yalms half-width covers
/// them without extending onto adjacent geometry. Tune if needed.
const PLANE_HALF_WIDTH: f32 = 3.0;

/// A single fitted stair plane with a bounded footprint.
#[derive(Debug, Clone, Copy)]
pub struct StairPlane {
    /// A real anchor point on the flight: an actual measured riser XZ +
    /// its measured Y. All plane math is relative to this.
    pub origin_xz: Vec2,
    pub origin_y: f32,
    /// Unit XZ vector, points up the slope (positive `along` = uphill).
    pub axis: Vec2,
    /// Signed rise per unit distance along `axis`. Positive = ascending.
    pub slope: f32,
    /// Extent along `axis` where the plane is valid. Character positions
    /// projected onto the axis outside `[along_min, along_max]` are off
    /// the flight. Both are relative to `origin_xz` (origin sits at along=0).
    pub along_min: f32,
    pub along_max: f32,
    /// Perpendicular half-width. Positions further than this from the axis
    /// line are off the flight.
    pub half_width: f32,
}

impl StairPlane {
    /// Query: at this XZ, what Y does the plane predict? None if the XZ is
    /// outside the plane's footprint (extent along axis OR half-width across).
    pub fn y_at(&self, xz: Vec2) -> Option<f32> {
        let rel = xz - self.origin_xz;
        let along = rel.dot(self.axis);
        if along < self.along_min || along > self.along_max {
            return None;
        }
        let across_vec = rel - self.axis * along;
        if across_vec.length_squared() > self.half_width * self.half_width {
            return None;
        }
        Some(self.origin_y + along * self.slope)
    }

    /// Same as `y_at` but returns Some even when marginally outside the
    /// half-width band, used only for merge comparison.
    fn origin_close(&self, other_origin: Vec2) -> bool {
        self.origin_xz.distance(other_origin) <= MERGE_ORIGIN_DIST
    }
}

/// The zone-scoped cache. Cleared automatically by
/// [`invalidate_cache_on_zone_change_system`] when the player enters a new zone.
#[derive(Resource, Default)]
pub struct StairCache {
    pub planes: Vec<StairPlane>,
    /// Zone this cache belongs to. `None` means never seeded / cleared.
    pub zone_id: Option<u16>,
}

impl StairCache {
    /// Look up a Y at this XZ. Returns the Y from the first plane whose
    /// footprint contains the XZ. Vec is small (a handful of staircases per
    /// zone at most), so linear scan is fine.
    pub fn y_at(&self, xz: Vec2) -> Option<f32> {
        self.planes.iter().find_map(|p| p.y_at(xz))
    }

    /// Add a freshly measured plane to the cache. Extends it via collision
    /// probes to cover the whole flight, then either merges it into an
    /// existing plane covering the same physical staircase or appends it.
    ///
    /// `seed_origin_xz` / `seed_origin_y`: an actual riser position measured
    /// by the live march.
    /// `axis`: unit vector along the flight (positive = up-slope direction).
    /// `signed_slope`: rise per unit along `axis` (positive ascending).
    pub fn add_from_march(
        &mut self,
        collision: &MzbCollisionGeometry,
        seed_origin_xz: Vec2,
        seed_origin_y: f32,
        axis: Vec2,
        signed_slope: f32,
    ) {
        // Reject garbage: axis must be a unit vector-ish, slope must be finite
        // and non-trivial. The live march already gates these, but be defensive.
        if !axis.is_finite() || axis.length_squared() < 0.9 {
            return;
        }
        if !signed_slope.is_finite() || signed_slope.abs() < 0.01 {
            return;
        }

        let axis = axis.normalize();
        let mut plane = StairPlane {
            origin_xz: seed_origin_xz,
            origin_y: seed_origin_y,
            axis,
            slope: signed_slope,
            along_min: 0.0,
            along_max: 0.0,
            half_width: PLANE_HALF_WIDTH,
        };

        // Extend forward (positive along = up the slope). Keep going as long
        // as the actual ground stays close to plane-predicted Y. Stop on
        // deviation (landing, wall, cliff, drop into water, off-mesh).
        let mut along = 0.0f32;
        while along < MAX_EXTEND {
            along += PROBE_STEP;
            let probe_xz = plane.origin_xz + plane.axis * along;
            let expected_y = plane.origin_y + along * plane.slope;
            match collision.ground_raycast(probe_xz, expected_y + 2.0) {
                Some(actual_y) if (actual_y - expected_y).abs() <= ON_PLANE_TOL => {
                    plane.along_max = along;
                }
                _ => break,
            }
        }

        // Extend backward. Stop when the plane's predicted Y has dropped
        // below the actual ground (i.e., we've walked past the toe onto
        // flat approach that sits above the extrapolated slope line).
        let mut along = 0.0f32;
        while along > -MAX_EXTEND {
            along -= PROBE_STEP;
            let probe_xz = plane.origin_xz + plane.axis * along;
            let expected_y = plane.origin_y + along * plane.slope;
            match collision.ground_raycast(probe_xz, expected_y.max(0.0) + 5.0) {
                Some(actual_y) => {
                    // Ground is still close to plane prediction: keep extending.
                    if (actual_y - expected_y).abs() <= ON_PLANE_TOL {
                        plane.along_min = along;
                        continue;
                    }
                    // Plane dropped below actual ground: past the toe. Include
                    // this last step (the toe cell where flat meets slope
                    // within TOE_STOP_TOL) and stop.
                    if expected_y < actual_y - TOE_STOP_TOL {
                        plane.along_min = along;
                        break;
                    }
                    // Otherwise (actual dropped below plane): a cliff, stop
                    // without extending.
                    break;
                }
                None => break,
            }
        }

        // If extension produced a degenerate plane (didn't cover at least a
        // couple of ticks worth of travel), don't cache it — it's not going
        // to help anyone.
        if plane.along_max - plane.along_min < 1.0 {
            return;
        }

        // Merge into an existing cached plane covering the same physical
        // staircase, or append.
        if let Some(existing) = self
            .planes
            .iter_mut()
            .find(|p| p.matches(&plane))
        {
            // Absorb: extend the existing plane's extent to cover both.
            // Convert new plane's endpoints into existing plane's axis frame
            // and take the union of ranges.
            let a_min = (plane.origin_xz + plane.axis * plane.along_min
                - existing.origin_xz)
                .dot(existing.axis);
            let a_max = (plane.origin_xz + plane.axis * plane.along_max
                - existing.origin_xz)
                .dot(existing.axis);
            existing.along_min = existing.along_min.min(a_min).min(a_max);
            existing.along_max = existing.along_max.max(a_min).max(a_max);
        } else {
            self.planes.push(plane);
        }
    }
}

impl StairPlane {
    /// Do two planes describe the same physical staircase?
    fn matches(&self, other: &StairPlane) -> bool {
        self.origin_close(other.origin_xz)
            && self.axis.dot(other.axis) >= MERGE_AXIS_DOT_MIN
            && (self.slope - other.slope).abs() <= MERGE_SLOPE_TOL
    }
}

/// Clears the cache when the player enters a new zone. Runs in Update,
/// cheap: a HashMap lookup and (very rarely) a Vec::clear.
pub fn invalidate_cache_on_zone_change_system(
    scene: Res<crate::snapshot::SceneState>,
    mut cache: ResMut<StairCache>,
) {
    let zone_id = scene.snapshot.zone_id;
    if cache.zone_id == zone_id {
        return;
    }
    cache.zone_id = zone_id;
    cache.planes.clear();
}
