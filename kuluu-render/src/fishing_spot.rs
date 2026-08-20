//! Retail's client-side fishing gate: whether the player could cast right now.
//!
//! Retail decides this locally, before any packet leaves — the action menu only
//! grows a "Fish" entry when it passes (research/xim UiState.kt
//! `getCurrentActions`, Scene.kt `canFish`, AssetViewer.kt `canFish`). The
//! server re-checks with its own `fishing_area` bounds
//! (vendor/server/src/map/utils/fishingutils.cpp `GetFishingArea`), so this is a
//! UI affordance, not the authority.

use bevy::prelude::*;

/// Snapshot equipment slot ids, LSB `SLOT_RANGED` / `SLOT_AMMO`
/// (vendor/server/src/map/entities/battleentity.h:179).
const SLOT_RANGE: usize = crate::equip_slot::EquipmentIndex::Range as usize;
const SLOT_AMMO: usize = crate::equip_slot::EquipmentIndex::Ammo as usize;

/// Why the player cannot cast, or that they can. The reason is what `/fish`
/// echoes back; the menu only asks whether it is [`FishingGate::Ready`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FishingGate {
    Ready,
    /// Already fishing, engaged, in a cutscene, or dead.
    #[default]
    Busy,
    NoRod,
    NoBait,
    /// Rod and bait in hand, but nothing castable ahead.
    NoWater,
}

impl FishingGate {
    pub fn is_ready(self) -> bool {
        self == FishingGate::Ready
    }

    /// The line retail prints when a cast is refused. The rod/bait wording is
    /// LSB's `FISHMESSAGEOFFSET_NOROD` / `_NOBAIT`; the no-water case never
    /// reaches the server at all, so it borrows `_CANNOTFISH_MOMENT`
    /// (vendor/server/src/map/utils/fishingutils.h:513-516).
    pub fn refusal(self) -> Option<&'static str> {
        Some(match self {
            FishingGate::Ready => return None,
            FishingGate::Busy => "You cannot use that command at this time.",
            FishingGate::NoRod => "You can't fish without a rod in your hands.",
            FishingGate::NoBait => "You can't fish without bait on the hook.",
            FishingGate::NoWater => "You can't fish here.",
        })
    }
}

/// Recomputed each frame from the snapshot and the loaded zone collision, so
/// menu construction and `/fish` read one answer instead of each deriving it.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct FishingSpot(pub FishingGate);

/// The equipment and busy-state half of the gate — everything that does not need
/// zone collision, so it stays testable without a loaded zone.
pub fn gate_from_snapshot(snapshot: &kuluu_snapshot::SceneSnapshot) -> FishingGate {
    let self_dead = snapshot
        .self_char_id
        .and_then(|id| snapshot.entities.iter().find(|e| e.id == id))
        .is_some_and(|e| e.is_dead());
    let engaged = matches!(
        snapshot.current_goal,
        Some(kuluu_snapshot::ReactorGoal::Engaged { .. })
    );
    if self_dead || engaged || snapshot.self_fishing.is_some() || snapshot.dialog.is_some() {
        return FishingGate::Busy;
    }

    let is_gear = |slot: usize| {
        snapshot.equipped[slot].is_some_and(ffxi_vocab::weapon_skill::is_fishing_gear)
    };
    if !is_gear(SLOT_RANGE) {
        return FishingGate::NoRod;
    }
    if !is_gear(SLOT_AMMO) {
        return FishingGate::NoBait;
    }
    FishingGate::Ready
}

/// Fails open when the zone collision has not landed yet: a gate that refuses
/// while the geometry loads would make fishing unusable right after a zone-in,
/// and the server validates the spot regardless.
#[cfg(not(target_arch = "wasm32"))]
pub fn update_fishing_spot(
    state: Res<crate::snapshot::SceneState>,
    geom: Res<crate::dat_mzb::MzbCollisionGeometry>,
    mut spot: ResMut<FishingSpot>,
) {
    use crate::combat_stance::heading_forward;
    use crate::dat_mzb::facing_water;

    let snapshot = &state.snapshot;
    let mut gate = gate_from_snapshot(snapshot);
    if gate.is_ready() && geom.tri_count() > 0 {
        let pos = Vec3::new(
            snapshot.self_pos.pos.x,
            snapshot.self_pos.pos.y,
            snapshot.self_pos.pos.z,
        );
        if !facing_water(&geom, pos, heading_forward(snapshot.self_pos.heading)) {
            gate = FishingGate::NoWater;
        }
    }
    if spot.0 != gate {
        spot.0 = gate;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuluu_snapshot::SceneSnapshot;

    /// vendor/server/sql/fishing_rod.sql:58 "Ebisu Fishing Rod" and
    /// fishing_bait.sql:44 "Slice of Bluetail". Both are SKILL_FISHING in
    /// item_weapon.sql, which is what the gate actually reads.
    const ROD: u16 = 17011;
    const BAIT: u16 = 16992;

    fn rigged() -> SceneSnapshot {
        let mut s = SceneSnapshot::default();
        s.equipped[SLOT_RANGE] = Some(ROD);
        s.equipped[SLOT_AMMO] = Some(BAIT);
        s
    }

    #[test]
    fn rod_and_bait_pass_the_equipment_gate() {
        assert!(ffxi_vocab::weapon_skill::is_fishing_gear(ROD));
        assert!(ffxi_vocab::weapon_skill::is_fishing_gear(BAIT));
        assert_eq!(gate_from_snapshot(&rigged()), FishingGate::Ready);
    }

    #[test]
    fn missing_rod_and_bait_are_reported_apart() {
        let mut s = rigged();
        s.equipped[SLOT_RANGE] = None;
        assert_eq!(gate_from_snapshot(&s), FishingGate::NoRod);

        let mut s = rigged();
        s.equipped[SLOT_AMMO] = None;
        assert_eq!(gate_from_snapshot(&s), FishingGate::NoBait);
    }

    #[test]
    fn a_sword_in_the_ranged_slot_is_not_a_rod() {
        let mut s = rigged();
        s.equipped[SLOT_RANGE] = Some(16537);
        assert_eq!(gate_from_snapshot(&s), FishingGate::NoRod);
    }

    #[test]
    fn an_active_cast_blocks_a_second_one() {
        let mut s = rigged();
        s.self_fishing = Some(kuluu_snapshot::SelfFishing {
            phase: 0,
            fish_max: 0,
            fish_hp: 0,
            arrow: None,
            size: None,
        });
        assert_eq!(gate_from_snapshot(&s), FishingGate::Busy);
    }

    #[test]
    fn every_refusal_but_ready_has_a_line() {
        assert_eq!(FishingGate::Ready.refusal(), None);
        for g in [
            FishingGate::Busy,
            FishingGate::NoRod,
            FishingGate::NoBait,
            FishingGate::NoWater,
        ] {
            assert!(g.refusal().is_some(), "{g:?} has no refusal line");
        }
    }
}
