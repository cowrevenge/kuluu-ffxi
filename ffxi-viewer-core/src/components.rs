use bevy::prelude::*;
use ffxi_viewer_wire::{EntityKind, EntityLook};

#[derive(Component, Debug, Clone, Copy)]
pub struct WorldEntity {
    pub id: u32,
    pub act_index: u16,
    pub kind: EntityKind,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct IsSelf;

#[derive(Component, Debug, Clone, Copy)]
pub struct InGameEntity;

#[derive(Component, Debug, Clone, Copy)]
pub struct Nameplate {
    pub entity_id: u32,
    pub kind: EntityKind,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct HpIndicator;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookComp(pub EntityLook);

/// The look a model was loaded for, plus whether it was loaded in its mounted
/// form. Mounting swaps in a whole extra animation DAT, so it re-keys the model
/// exactly like a gear change does.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityModel {
    pub look: EntityLook,
    pub mounted: bool,
}

/// The mount whose model is currently loaded onto a mount actor entity. Memoises
/// the dispatch the way [`EntityModel`] does for looks: a rider can swap mounts
/// without the entity ever going away.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountModel(pub ffxi_viewer_wire::Mount);

/// Model-load transition: grows the actor in while a transient orb stretches
/// into a light-column and dissolves. Both child entities are torn down on
/// completion (or with the parent, recursively).
#[derive(Component, Debug, Clone)]
pub struct MorphIn {
    pub elapsed: f32,
    pub actor_root: Entity,
    pub orb: Option<Entity>,
    pub orb_mat: Option<Handle<StandardMaterial>>,
    pub orb_emissive: LinearRgba,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct CameraOccluder;
