//! Pre-server animation test box (dev-only, no session layer). A chip in the launcher corner
//! loads a small scene — carrion worm left, sworded Hume right, both from retail DATs — plus a
//! panel that fires ROM/0/0.DAT's dam0 cascade through kuluu-render's production routine,
//! particle and audio systems. Every press logs its path (what dam0 picked, which stages ran)
//! to this window AND stderr so both sides see it.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use kuluu_render::components::WorldEntity;
use kuluu_render::ffxi_actor_render::{ActorSubject, FfxiRenderActor, LoadActorRequest};
use kuluu_render::scene::TrackedEntities;
use kuluu_render::scheduler_runtime::{
    enqueue_routine, evaluate_switch, stage_summary, ActionDatRoot, ActionTarget, ActiveScheduler,
    GlobalEffectDir, HitContext, RoutineLookup, UnknownFieldPolicy, LEVEL_UP_EFFECT_DAT_ID,
};
use kuluu_render::snapshot::SceneState;
use kuluu_snapshot::EntityKind;

/// Carrion Worm family model (kuluu-render/tests/rabbit_tester.rs S12 load).
const WORM_FILE: u32 = 1724;
/// HumeM main-hand weapon row (kuluu-render/tests/rabbit_tester.rs load_humem).
const HUME_MAIN_WEAPON: u32 = 8392;

const WORM_ID: u32 = 1;
const HUME_ID: u32 = 2;

/// The worm's `dead` fall-over hides the model after this long; respawn brings it back.
const WORM_HIDE_AFTER_SECS: f32 = 2.5;

#[derive(Resource, Default)]
struct TestLog {
    lines: Vec<String>,
}

/// Both sinks at once: the on-screen panel and stderr (the terminal next to the window).
fn log_line(log: &mut TestLog, msg: String) {
    eprintln!("[animationtest] {msg}");
    log.lines.push(msg);
    if log.lines.len() > 400 {
        log.lines.drain(0..log.lines.len() - 400);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Case {
    PlayerNhIt,
    PlayerChit,
    PlayerDhit,
    MobNhIt,
    MobChit,
    MobRespawn,
    LevelUp,
}

impl Case {
    fn label(self) -> &'static str {
        match self {
            Self::PlayerNhIt => "player normal hit",
            Self::PlayerChit => "player crit hit",
            Self::PlayerDhit => "player death hit",
            Self::MobNhIt => "mob normal hit",
            Self::MobChit => "mob crit hit",
            Self::MobRespawn => "mob respawn",
            Self::LevelUp => "player level up",
        }
    }

    fn info_bits(self) -> u8 {
        match self {
            Self::PlayerNhIt | Self::MobNhIt => 0,
            Self::PlayerChit | Self::MobChit => ffxi_proto::melee::INFO_CRITICAL_HIT,
            Self::PlayerDhit => ffxi_proto::melee::INFO_DEFEATED,
            _ => 0,
        }
    }

    fn is_crit(self) -> bool {
        matches!(self, Self::PlayerChit | Self::MobChit)
    }
}

#[derive(Resource, Default)]
struct PendingCase(Option<Case>);

/// Worm death bookkeeping: dhit runs the `dead` fall-over, then hides the model.
#[derive(Resource, Default)]
struct WormState {
    dead_at: Option<Instant>,
}

/// Everything the test scene spawns (3D + UI), so teardown takes it all down at once.
#[derive(Component)]
struct TestSceneScoped;

/// Set by the launcher's AnimationTest titlebar button (gated on the
/// `enhanced-animationtest` feature); consumed by handle_toggle.
#[derive(Resource, Default)]
pub(crate) struct PendingToggle(pub bool);

#[derive(Component)]
struct CloseBox;

#[derive(Component)]
struct CaseButton(Case);

#[derive(Component)]
struct LogText;

pub struct AnimationTestScenePlugin;

impl Plugin for AnimationTestScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TestLog>()
            .init_resource::<PendingCase>()
            .init_resource::<WormState>()
            .init_resource::<PendingToggle>()
            .add_systems(OnExit(super::AppPhase::Launcher), tear_down_test_scene)
            .add_systems(
                Update,
                (
                    handle_toggle,
                    handle_close_press,
                    handle_case_presses,
                    run_pending_case,
                    worm_death_watch,
                    sync_log_text,
                )
                    .chain()
                    .run_if(in_state(super::AppPhase::Launcher)),
            );
    }
}

fn set_launcher_ui_visibility(
    commands: &mut Commands,
    q_ui: &Query<(Entity, Option<&ChildOf>), (With<Node>, Without<TestSceneScoped>)>,
    visible: bool,
) {
    for (e, parent) in q_ui.iter() {
        if parent.is_some() {
            continue;
        }
        let vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        commands.entity(e).insert(vis);
    }
}

fn handle_toggle(
    mut pending: ResMut<PendingToggle>,
    q_scoped: Query<Entity, With<TestSceneScoped>>,
    q_ui: Query<(Entity, Option<&ChildOf>), (With<Node>, Without<TestSceneScoped>)>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    moon_materials: ResMut<Assets<kuluu_render::moon_material::MoonMaterial>>,
    images: ResMut<Assets<Image>>,
    settings: Res<kuluu_render::graphics_settings::GraphicsSettings>,
    mut load_tx: MessageWriter<LoadActorRequest>,
    mut tracked: ResMut<TrackedEntities>,
    mut scene: ResMut<SceneState>,
    actor_root: Res<ActionDatRoot>,
    mut log: ResMut<TestLog>,
) {
    if !pending.0 {
        return;
    }
    pending.0 = false;

    if q_scoped.iter().next().is_some() {
        tear_down(&mut commands, &q_scoped, &q_ui, &mut tracked, &mut scene);
        log_line(&mut log, "scene down".into());
        return;
    }

    activate_test_scene(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut load_tx,
        &mut tracked,
        &mut scene,
        &actor_root,
        &mut log,
    );

    // Hide the launcher menu while the box is up; the X button (or leaving the Launcher phase)
    // restores it.
    set_launcher_ui_visibility(&mut commands, &q_ui, false);

    // Last: it consumes the owned params. poll_load_actor_tasks parks tasks without EntityMesh,
    // and the Update chain that resource unlocks (sync_entities_system & co.) reads the world
    // resources setup_world inserts on InGame entry — so run it here: real orb meshes/materials
    // for the wire placeholders, no defaults.
    kuluu_render::setup_world(
        commands,
        meshes,
        materials,
        moon_materials,
        images,
        settings,
    );
}

fn handle_close_press(
    q_close: Query<&Interaction, (With<CloseBox>, With<bevy::ui::widget::Button>)>,
    q_scoped: Query<Entity, With<TestSceneScoped>>,
    q_ui: Query<(Entity, Option<&ChildOf>), (With<Node>, Without<TestSceneScoped>)>,
    mut commands: Commands,
    mut tracked: ResMut<TrackedEntities>,
    mut scene: ResMut<SceneState>,
    mut log: ResMut<TestLog>,
) {
    let Ok(interaction) = q_close.single() else {
        return;
    };
    if !matches!(interaction, Interaction::Pressed) {
        return;
    }
    tear_down(&mut commands, &q_scoped, &q_ui, &mut tracked, &mut scene);
    log_line(&mut log, "scene down".into());
}

fn activate_test_scene(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    load_tx: &mut MessageWriter<LoadActorRequest>,
    tracked: &mut TrackedEntities,
    scene: &mut SceneState,
    actor_root: &ActionDatRoot,
    log: &mut TestLog,
) {
    if actor_root.0.is_none() {
        log_line(
            log,
            "no retail install wired — pick one in Settings first".into(),
        );
        return;
    }

    // Camera + light + ground. The camera clears its own color so the launcher backdrop zone
    // does not show around the test floor.
    commands.spawn((
        TestSceneScoped,
        Camera3d::default(),
        // Order 3: above the launcher backdrop (-2) and any default-order (gizmo) camera;
        // 1-2 are the in-game nameplate overlay/composite slots.
        Camera {
            order: 3,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.05, 0.06, 0.08)),
            ..default()
        },
        Transform::from_translation(Vec3::new(0.0, 2.6, 7.5))
            .looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        TestSceneScoped,
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
            ..default()
        },
    ));
    commands.insert_resource(bevy::light::GlobalAmbientLight {
        color: Color::srgb(1.0, 1.0, 1.0),
        brightness: 400.0,
        ..default()
    });
    let plane: Mesh = Plane3d::new(Vec3::Z, Vec2::splat(40.0)).into();
    commands.spawn((
        TestSceneScoped,
        Mesh3d(meshes.add(plane)),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.22, 0.26, 0.22),
            ..default()
        })),
    ));

    // Worm left, sworded Hume right, facing each other. The snapshot entries keep the wires
    // alive: sync_entities_system despawns any tracked wire missing from the snapshot.
    // Heading is the system's orientation source of truth (sync re-derives the wire quat from
    // it on respawn), so both the spawn transform and the heading agree. Both are 128 (= a PI
    // turn, live-checked): the two skeletons are authored facing opposite local ways — the
    // worm's forward is -X, the HumeM's +X — so equal headings square them onto each other.
    // Both are engaged so they stand in battle stance with weapons out, not rest pose.
    spawn_wire(
        commands,
        tracked,
        scene,
        WORM_ID,
        EntityKind::Mob,
        Vec3::new(-1.0, 0.0, 0.0),
        128,
        ffxi_proto::decode::animation::ATTACK,
        HUME_ID,
    );
    spawn_wire(
        commands,
        tracked,
        scene,
        HUME_ID,
        EntityKind::Pc,
        Vec3::new(1.0, 0.0, 0.0),
        128,
        ffxi_proto::decode::animation::ATTACK,
        WORM_ID,
    );

    // Production order: the face file first (head/hair), then slots 1..8 — load_pc reads
    // equipment[0] as the head. A weapon in that slot leaves the Hume headless.
    let mut equipment = Vec::new();
    match kuluu_render::look_resolver::resolve_face(0, 1) {
        Some(face_file) => equipment.push(face_file),
        None => log_line(log, "face file unresolved — head will not render".into()),
    }
    for slot in 1..=5u16 {
        match kuluu_render::look_resolver::resolve_equipment_slot(slot << 12, 1) {
            Some(file_id) => equipment.push(file_id),
            None => log_line(
                log,
                format!("slot {slot} unresolved — that body part will not render"),
            ),
        }
    }
    let equip_list = equipment
        .iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    load_tx.write(LoadActorRequest {
        entity_id: WORM_ID,
        subject: ActorSubject::Npc {
            file_id: WORM_FILE,
            graph_size: 0,
        },
    });
    load_tx.write(LoadActorRequest {
        entity_id: HUME_ID,
        subject: ActorSubject::Pc {
            race: 1,
            mounted: false,
            equipment,
            body: None,
            main_weapon: Some(HUME_MAIN_WEAPON),
            sub_weapon: None,
        },
    });

    spawn_panel(commands);

    // X in the top-right corner closes the box back to the launcher menu.
    commands
        .spawn((
            TestSceneScoped,
            CloseBox,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.35, 0.12, 0.12, 0.9)),
            bevy::ui::widget::Button,
        ))
        .with_child((
            Text::new("X"),
            TextFont {
                font_size: 14.0.into(),
                ..default()
            },
            TextColor(Color::WHITE),
        ));

    log_line(
        log,
        format!(
            "scene up — worm (file {WORM_FILE}) left, HumeM + sword ({HUME_MAIN_WEAPON}) right; equipment [{equip_list}]; models loading"
        ),
    );
}

fn spawn_wire(
    commands: &mut Commands,
    tracked: &mut TrackedEntities,
    scene: &mut SceneState,
    id: u32,
    kind: EntityKind,
    pos: Vec3,
    heading: u8,
    animation: u8,
    bt_target_id: u32,
) {
    // The snapshot pos is FFXI space and the prediction tween pulls wires toward it, so it must
    // agree with the bevy-space wire transform (inverse of ffxi_to_bevy).
    let wire_pos = kuluu_snapshot::Vec3 {
        x: pos.x,
        y: -pos.z,
        z: -pos.y,
    };
    // Same formula as scene.rs heading_to_quat, so a sync respawn cannot re-orient the wire.
    let rot = Quat::from_rotation_y(-(heading as f32) * std::f32::consts::TAU / 256.0);
    // Visibility on the wire (as in sync_entities_system's spawn): Bevy then attaches
    // InheritedVisibility to the parent, so the model children don't trip B0004.
    let parent = commands
        .spawn((
            TestSceneScoped,
            WorldEntity {
                id,
                act_index: 0,
                kind,
            },
            Transform::from_translation(pos).with_rotation(rot),
            Visibility::default(),
        ))
        .id();
    tracked.by_id.insert(id, parent);
    scene.snapshot.entities.push(kuluu_snapshot::Entity {
        id,
        act_index: 0,
        kind,
        name: None,
        pos: wire_pos,
        heading,
        hp_pct: Some(100),
        bt_target_id,
        face_target: 0,
        claim_id: 0,
        speed: 0,
        speed_base: 0,
        look: None,
        animation,
        animationsub: 0,
        mount: None,
        status: 0,
        char_flags: Default::default(),
        monstrosity: false,
        name_vis: None,
    });
}

fn spawn_panel(commands: &mut Commands) {
    let panel = commands
        .spawn((
            TestSceneScoped,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(215.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                column_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.05, 0.09, 0.92)),
            GlobalZIndex(6),
        ))
        .id();

    for case in [
        Case::PlayerNhIt,
        Case::PlayerChit,
        Case::PlayerDhit,
        Case::MobNhIt,
        Case::MobChit,
        Case::MobRespawn,
        Case::LevelUp,
    ] {
        let button = commands
            .spawn((
                TestSceneScoped,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.16, 0.2, 0.3)),
                bevy::ui::widget::Button,
                CaseButton(case),
            ))
            .with_child((
                Text::new(case.label()),
                TextFont {
                    font_size: 13.0.into(),
                    ..default()
                },
                TextColor(Color::WHITE),
            ))
            .id();
        commands.entity(panel).add_child(button);
    }

    let log_node = commands
        .spawn((
            TestSceneScoped,
            Node {
                flex_grow: 1.0,
                width: Val::Percent(100.0),
                align_items: AlignItems::FlexStart,
                ..default()
            },
        ))
        .with_child((
            Text::new("(log)"),
            TextFont {
                font_size: 10.5.into(),
                ..default()
            },
            TextColor(Color::srgb(0.75, 0.85, 0.75)),
            LogText,
        ))
        .id();
    commands.entity(panel).add_child(log_node);
}

fn handle_case_presses(
    q_buttons: Query<(&CaseButton, &Interaction)>,
    mut pending: ResMut<PendingCase>,
) {
    for (case_button, interaction) in q_buttons.iter() {
        if !matches!(interaction, Interaction::Pressed) {
            continue;
        }
        pending.0 = Some(case_button.0);
    }
}

fn run_pending_case(
    mut pending: ResMut<PendingCase>,
    mut log: ResMut<TestLog>,
    tracked: Res<TrackedEntities>,
    q_children: Query<&Children>,
    q_render: Query<&FfxiRenderActor>,
    global: Option<Res<GlobalEffectDir>>,
    root: Res<ActionDatRoot>,
    mut worm_state: ResMut<WormState>,
    mut scene: ResMut<SceneState>,
    q_root: Query<&kuluu_render::ffxi_actor_render::FfxiRenderRoot>,
    mut q_vis: Query<&mut Visibility>,
    mut commands: Commands,
) {
    let Some(case) = pending.0.take() else {
        return;
    };
    log_line(&mut log, format!("case: {}", case.label()));

    match case {
        Case::MobRespawn => respawn_worm(
            &mut worm_state,
            &mut scene,
            &mut log,
            &tracked,
            &q_root,
            &mut q_vis,
        ),
        Case::LevelUp => fire_level_up(
            &root,
            &tracked,
            &q_children,
            &q_render,
            global.as_deref(),
            &mut log,
            &mut commands,
        ),
        _ => fire_hit(
            case,
            &tracked,
            &q_children,
            &q_render,
            global.as_deref(),
            &mut worm_state,
            &mut log,
            &mut commands,
        ),
    }
}

fn actor_routines(
    entity: Entity,
    q_children: &Query<&Children>,
    q_render: &Query<&FfxiRenderActor>,
) -> Option<std::collections::HashMap<ffxi_dat::datid::DatId, ffxi_dat::scheduler::Scheduler>> {
    q_children
        .get(entity)
        .ok()?
        .iter()
        .find_map(|child| q_render.get(child).ok())
        .map(|a| a.routines().clone())
}

fn fourcc(name: &[u8; 4]) -> String {
    std::str::from_utf8(name).unwrap_or("????").to_string()
}

fn fire_hit(
    case: Case,
    tracked: &TrackedEntities,
    q_children: &Query<&Children>,
    q_render: &Query<&FfxiRenderActor>,
    global: Option<&GlobalEffectDir>,
    worm_state: &mut WormState,
    log: &mut TestLog,
    commands: &mut Commands,
) {
    let (attacker_id, victim_id) = match case {
        Case::PlayerNhIt | Case::PlayerChit | Case::PlayerDhit => (HUME_ID, WORM_ID),
        _ => (WORM_ID, HUME_ID),
    };
    let Some(attacker) = tracked.by_id.get(&attacker_id).copied() else {
        log_line(
            log,
            "attacker not loaded yet — try again in a second".into(),
        );
        return;
    };
    let Some(victim) = tracked.by_id.get(&victim_id).copied() else {
        log_line(log, "victim not loaded yet — try again in a second".into());
        return;
    };

    // The reaction runs on the VICTIM with target = attacker (production semantics: the victim's
    // own damg shadows the global one and links chit back onto the attacker).
    let Some(victim_routines) = actor_routines(victim, q_children, q_render) else {
        log_line(
            log,
            "victim has no routines yet — model still loading".into(),
        );
        return;
    };
    // The attacker's own swing, mirroring dispatch_melee_action_started: atk0 voice merged with
    // the ati0 motion from the attacker's routines; target = victim so the pose faces them.
    if let Some(att_routines) = actor_routines(attacker, q_children, q_render) {
        let mut att_lookup = RoutineLookup::new().with_actor(&att_routines);
        if let Some(g) = global {
            att_lookup = att_lookup.with_dat(&g.schedulers);
        }
        match ActiveScheduler::effects_only_merged(&att_lookup, &[*b"atk0", *b"ati0"]) {
            Some(active) => {
                log_line(
                    log,
                    format!("swing on attacker: {}", stage_summary(&active)),
                );
                enqueue_routine(commands, attacker, active);
                commands
                    .entity(attacker)
                    .try_insert(ActionTarget(Some(victim)));
            }
            None => log_line(log, "no atk0/ati0 swing routine on the attacker".into()),
        }
    } else {
        log_line(
            log,
            "attacker has no routines yet — model still loading".into(),
        );
    }

    let mut lookup = RoutineLookup::new().with_actor(&victim_routines);
    if let Some(g) = global {
        lookup = lookup.with_dat(&g.schedulers);
    }

    let ctx = HitContext {
        resolution: 0,
        animation: 0,
        info: u32::from(case.info_bits()),
    };
    log_line(
        log,
        format!(
            "dam0 switch on victim (res=Hit, info={:#x}) — {}",
            case.info_bits(),
            if global.is_some() {
                "global effect dir present"
            } else {
                "NO global effect dir loaded!"
            }
        ),
    );

    for name in evaluate_switch(&lookup, b"dam0", &ctx, UnknownFieldPolicy::Random) {
        let fourcc = fourcc(&name);
        match ActiveScheduler::from_routine(&lookup, &name) {
            Some(active) => {
                log_line(log, format!("dam0 -> {fourcc}: {}", stage_summary(&active)));
                enqueue_routine(commands, victim, active);
                commands
                    .entity(victim)
                    .try_insert(ActionTarget(Some(attacker)));
            }
            None => log_line(
                log,
                format!("dam0 -> {fourcc}: UNRESOLVED (no such routine in victim+global)"),
            ),
        }
    }

    if case.is_crit() {
        for name in evaluate_switch(&lookup, b"crtl", &ctx, UnknownFieldPolicy::Match) {
            let fourcc = fourcc(&name);
            match ActiveScheduler::from_routine(&lookup, &name) {
                Some(active) => {
                    log_line(
                        log,
                        format!(
                            "crtl -> {fourcc} (spark on attacker): {}",
                            stage_summary(&active)
                        ),
                    );
                    enqueue_routine(commands, attacker, active);
                    commands
                        .entity(attacker)
                        .try_insert(ActionTarget(Some(victim)));
                }
                None => log_line(log, format!("crtl -> {fourcc}: UNRESOLVED")),
            }
        }
    }

    if case == Case::PlayerDhit {
        // Production's Defeated frame also runs the victim's `dead` fall-over; mirror it.
        match ActiveScheduler::from_routine(&lookup, b"dead") {
            Some(active) => {
                log_line(
                    log,
                    format!("defeated -> dead (fall-over): {}", stage_summary(&active)),
                );
                enqueue_routine(commands, victim, active);
            }
            None => log_line(log, "defeated: no `dead` routine on the victim".into()),
        }
        worm_state.dead_at = Some(Instant::now());
    }
}

fn respawn_worm(
    worm_state: &mut WormState,
    scene: &mut SceneState,
    log: &mut TestLog,
    tracked: &TrackedEntities,
    q_root: &Query<&kuluu_render::ffxi_actor_render::FfxiRenderRoot>,
    q_vis: &mut Query<&mut Visibility>,
) {
    let Some(worm) = tracked.by_id.get(&WORM_ID).copied() else {
        return;
    };
    if let Ok(root) = q_root.get(worm) {
        if let Ok(mut vis) = q_vis.get_mut(root.0) {
            *vis = Visibility::Visible;
        }
    }
    worm_state.dead_at = None;
    if let Some(e) = scene.snapshot.entities.iter_mut().find(|e| e.id == WORM_ID) {
        e.hp_pct = Some(100);
    }
    log_line(log, "worm respawned (visible again, hp 100)".into());
}

fn fire_level_up(
    root: &ActionDatRoot,
    tracked: &TrackedEntities,
    q_children: &Query<&Children>,
    q_render: &Query<&FfxiRenderActor>,
    global: Option<&GlobalEffectDir>,
    log: &mut TestLog,
    commands: &mut Commands,
) {
    let Some(hume) = tracked.by_id.get(&HUME_ID).copied() else {
        log_line(log, "hume not loaded yet".into());
        return;
    };
    let Some(dat_root) = root.0.as_ref() else {
        log_line(log, "no install wired".into());
        return;
    };
    let Ok(loc) = dat_root.resolve(LEVEL_UP_EFFECT_DAT_ID) else {
        log_line(
            log,
            format!("level-up effect DAT {LEVEL_UP_EFFECT_DAT_ID} not found in the install"),
        );
        return;
    };
    let Ok(bytes) = std::fs::read(loc.path_under(dat_root)) else {
        log_line(log, "failed to read the level-up effect DAT".into());
        return;
    };
    let (schedulers, _assets, _cameras) =
        kuluu_render::scheduler_runtime::parse_action_bytes(&bytes);
    log_line(
        log,
        format!(
            "level-up effect DAT file {LEVEL_UP_EFFECT_DAT_ID}: {} routines",
            schedulers.len()
        ),
    );
    let hume_routines = actor_routines(hume, q_children, q_render);
    let mut lookup = RoutineLookup::new().with_dat(&schedulers);
    if let Some(r) = &hume_routines {
        lookup = lookup.with_actor(r);
    }
    if let Some(g) = global {
        lookup = lookup.with_dat(&g.schedulers);
    }
    match ActiveScheduler::from_routine(&lookup, b"main") {
        Some(active) => {
            log_line(
                log,
                format!("lvup main on hume: {}", stage_summary(&active)),
            );
            enqueue_routine(commands, hume, active);
        }
        None => log_line(log, "lvup `main` UNRESOLVED".into()),
    }
}

fn worm_death_watch(
    mut worm_state: ResMut<WormState>,
    tracked: Res<TrackedEntities>,
    q_root: Query<&kuluu_render::ffxi_actor_render::FfxiRenderRoot>,
    mut q_vis: Query<&mut Visibility>,
) {
    let Some(dead_at) = worm_state.dead_at else {
        return;
    };
    if dead_at.elapsed() < Duration::from_secs_f32(WORM_HIDE_AFTER_SECS) {
        return;
    }
    let Some(worm) = tracked.by_id.get(&WORM_ID).copied() else {
        return;
    };
    if let Ok(root) = q_root.get(worm) {
        if let Ok(mut vis) = q_vis.get_mut(root.0) {
            *vis = Visibility::Hidden;
        }
    }
    worm_state.dead_at = None;
}

fn sync_log_text(log: Res<TestLog>, mut node: Query<&mut Text, With<LogText>>) {
    if !log.is_changed() {
        return;
    }
    let recent: Vec<String> = log.lines.iter().rev().take(18).map(|s| s.clone()).collect();
    let text = recent.into_iter().rev().collect::<Vec<_>>().join("\n");
    if let Ok(mut t) = node.single_mut() {
        *t = Text::new(text);
    }
}

fn tear_down(
    commands: &mut Commands,
    q_scoped: &Query<Entity, With<TestSceneScoped>>,
    q_ui: &Query<(Entity, Option<&ChildOf>), (With<Node>, Without<TestSceneScoped>)>,
    tracked: &mut TrackedEntities,
    scene: &mut SceneState,
) {
    for e in q_scoped.iter() {
        // try_despawn: despawn() is recursive, so a parent earlier in the query may have
        // already freed this entity (same fix as launcher_backdrop's teardown).
        commands.entity(e).try_despawn();
    }
    set_launcher_ui_visibility(commands, q_ui, true);
    tracked.by_id.remove(&WORM_ID);
    tracked.by_id.remove(&HUME_ID);
    scene
        .snapshot
        .entities
        .retain(|e| e.id != WORM_ID && e.id != HUME_ID);
}

fn tear_down_test_scene(
    mut commands: Commands,
    q_scoped: Query<Entity, With<TestSceneScoped>>,
    q_ui: Query<(Entity, Option<&ChildOf>), (With<Node>, Without<TestSceneScoped>)>,
    mut tracked: ResMut<TrackedEntities>,
    mut scene: ResMut<SceneState>,
) {
    tear_down(&mut commands, &q_scoped, &q_ui, &mut tracked, &mut scene);
}
