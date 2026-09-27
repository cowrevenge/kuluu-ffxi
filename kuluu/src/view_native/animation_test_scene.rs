//! Pre-server animation test box (dev-only, no session layer). A chip in the launcher corner
//! loads a small scene — carrion worm left, sworded Hume right, both from retail DATs — plus a
//! panel that fires ROM/0/0.DAT's dam0 cascade through kuluu-render's production routine,
//! particle and audio systems. Every press logs its path (what dam0 picked, which stages ran)
//! to this window AND stderr so both sides see it.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use kuluu_render::components::{InGameEntity, WorldEntity};
use kuluu_render::ffxi_actor_render::{
    ActorSubject, FfxiActorMeshChild, FfxiRenderActor, FfxiRenderRoot, LoadActorRequest,
};
use kuluu_render::scene::TrackedEntities;
use kuluu_render::scheduler_runtime::{
    enqueue_routine, stage_summary, ActionDatRoot, ActiveScheduler, GlobalEffectDir,
    ParticleSpawnTrace, RoutineLookup, VfxTrace, LEVEL_UP_EFFECT_DAT_ID,
};
use kuluu_render::snapshot::{EventLog, SceneState};
use kuluu_snapshot::EntityKind;

/// Carrion Worm family model (kuluu-render/tests/rabbit_tester.rs S12 load).
const WORM_FILE: u32 = 1724;
// The equipment table's main-hand (8392) row is a bare-hand stub: its VertexOs2 chunk carries
// joint-mapped vertices but no polygon instructions, so nothing draws from it. Production fills
// the slot from the equipped item; the box pins a real sword file with geometry.
const TEST_SWORD_FILE: u32 = 8397;

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

    fn damage(self) -> u32 {
        match self {
            Self::PlayerNhIt | Self::MobNhIt => 5,
            Self::PlayerChit | Self::MobChit => 10,
            Self::PlayerDhit => TEST_MAX_HP * 2,
            _ => 0,
        }
    }
}

#[derive(Resource, Default)]
struct PendingCase(Option<Case>);

/// Worm death bookkeeping: dhit runs the `dead` fall-over, then hides the model.
#[derive(Resource, Default)]
struct WormState {
    dead_at: Option<Instant>,
}

const TEST_MAX_HP: u32 = 10_000;

/// Absolute-HP bookkeeping for the simulated combat flushes: the snapshot only carries a
/// percentage (the 0x0E wire shape), so damage is tracked here and converted on each hit.
#[derive(Resource, Default)]
struct TestHp {
    hume: u32,
    worm: u32,
}

/// One case at a time: while set, presses are rejected until the animation window elapses.
#[derive(Resource, Default)]
struct CaseLock {
    until: Option<Instant>,
}

// One-shot per activation: exp_* are the buffer counts from the load-time check; *_done mark
// the post-spawn drawn count already logged.
#[derive(Resource, Default)]
struct DrawnCheck {
    hume_done: bool,
    worm_done: bool,
    parts_ok: bool,
    exp_hume: usize,
    exp_worm: usize,
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
            .init_resource::<DrawnCheck>()
            .init_resource::<PendingCase>()
            .init_resource::<WormState>()
            .init_resource::<TestHp>()
            .init_resource::<CaseLock>()
            .init_resource::<PendingToggle>()
            .add_systems(OnExit(super::AppPhase::Launcher), tear_down_test_scene)
            .add_systems(
                Update,
                (
                    handle_toggle,
                    handle_close_press,
                    handle_case_presses,
                    run_pending_case,
                    verify_drawn,
                    worm_death_watch,
                    zone_backdrop_visibility,
                    collect_spawn_traces,
                    sync_case_buttons,
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
    mut drawn_check: ResMut<DrawnCheck>,
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
    mut hp: ResMut<TestHp>,
) {
    if !pending.0 {
        return;
    }
    pending.0 = false;

    if q_scoped.iter().next().is_some() {
        tear_down(&mut commands, &q_scoped, &q_ui, &mut tracked, &mut scene);
        // The dispatch funnel's info! traces (routine resolution, particle defs/meshes) are
        // gated on this; the box is where they earn their keep.
        commands.insert_resource(VfxTrace(false));
        log_line(&mut log, "scene down".into());
        return;
    }

    drawn_check.hume_done = false;
    drawn_check.worm_done = false;
    drawn_check.parts_ok = true;
    hp.hume = TEST_MAX_HP;
    hp.worm = TEST_MAX_HP;
    commands.insert_resource(VfxTrace(true));
    activate_test_scene(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut load_tx,
        &mut tracked,
        &mut scene,
        &actor_root,
        &mut log,
        &mut drawn_check,
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
    check: &mut DrawnCheck,
) {
    let Some(dat_root) = actor_root.0.as_ref() else {
        log_line(
            log,
            "no retail install wired — pick one in Settings first".into(),
        );
        return;
    };

    // Camera + light + ground. The camera clears its own color so the launcher backdrop zone
    // does not show around the test floor.
    commands.spawn((
        TestSceneScoped,
        Camera3d::default(),
        // Order 3: above the launcher backdrop (-2) and any default-order (gizmo) camera;
        // 1-2 are the in-game nameplate overlay/composite slots.
        Camera {
            order: 3,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
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
    // +Y normal: a +Z plane is a vertical wall at z=0 that hides everything behind it.
    let plane: Mesh = Plane3d::new(Vec3::Y, Vec2::splat(40.0)).into();
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
    // it on respawn), so both the spawn transform and the heading agree. Both skeletons face
    // local +X at identity (live-checked), so opposite headings square them onto each other:
    // worm 0 faces +X toward the Hume, Hume 128 faces -X back.
    // Both are engaged so they stand in battle stance with weapons out, not rest pose.
    spawn_wire(
        commands,
        tracked,
        scene,
        WORM_ID,
        EntityKind::Mob,
        Vec3::new(-1.0, 0.0, 0.0),
        0,
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
    let mut parts: Vec<(&str, u32)> = Vec::new();
    match kuluu_render::look_resolver::resolve_face(0, 1) {
        Some(face_file) => {
            equipment.push(face_file);
            parts.push(("face", face_file));
        }
        None => log_line(
            log,
            "ERROR: face file unresolved — head will not render".into(),
        ),
    }
    // Slots 2..5 come from the race's default equipment table (real geometry); slot 6 is pinned
    // to a real sword because its table row is a bare stub; slot 1 stays empty — the face file
    // carries hair and face, so no headgear.
    const SLOT_NAMES: [&str; 6] = ["head", "body", "hands", "legs", "feet", "sword"];
    for (slot, name) in (1u16..=6).zip(SLOT_NAMES) {
        if slot == 1 {
            continue;
        }
        let file_id = match slot {
            6 => Some(TEST_SWORD_FILE),
            _ => kuluu_render::look_resolver::resolve_equipment_slot(slot << 12, 1),
        };
        match file_id {
            Some(file_id) => {
                equipment.push(file_id);
                parts.push((name, file_id));
            }
            None => log_line(
                log,
                format!("ERROR: slot {slot} ({name}) unresolved — that body part will not render"),
            ),
        }
    }
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
            main_weapon: Some(TEST_SWORD_FILE),
            sub_weapon: None,
        },
    });

    verify_parts(
        dat_root,
        &parts,
        kuluu_render::dat_vos2::skeleton_file_id_for_race(None, 1),
        WORM_FILE,
        log,
        check,
    );

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
            "loaded: worm={WORM_FILE} hume=[{}]",
            parts
                .iter()
                .map(|(name, file_id)| format!("{name}={file_id}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    );
}

// Non-empty mesh buffers in a DAT: what load_pc turns into drawn mesh entities. None = unreadable.
fn part_mesh_count(dat_root: &ffxi_dat::DatRoot, file_id: u32) -> Option<usize> {
    let loc = dat_root.resolve(file_id).ok()?;
    let bytes = std::fs::read(loc.path_under(dat_root)).ok()?;
    Some(
        ffxi_dat::resource_dir::ResourceDir::from_bytes(bytes)
            .collect_skel_meshes()
            .iter()
            .flat_map(|m| m.meshes.iter())
            .filter(|b| !b.vertices.is_empty())
            .count(),
    )
}

// Every part file must be readable with mesh buffers, else load_pc drops it and the part
// never renders; also seeds DrawnCheck's expected counts for verify_drawn.
fn verify_parts(
    dat_root: &ffxi_dat::DatRoot,
    parts: &[(&str, u32)],
    skel_file: Option<u32>,
    worm_file: u32,
    log: &mut TestLog,
    check: &mut DrawnCheck,
) {
    let mut ok_bits = Vec::new();
    for (name, file_id) in parts {
        match part_mesh_count(dat_root, *file_id) {
            Some(n) if n > 0 => ok_bits.push(format!("{name}={file_id}({n})")),
            _ => {
                check.parts_ok = false;
                log_line(
                    log,
                    format!(
                        "ERROR: {name}={file_id} unreadable or 0 mesh buffers — will not render"
                    ),
                );
            }
        }
    }
    if !ok_bits.is_empty() {
        log_line(log, format!("check ok: {}", ok_bits.join(" ")));
    }

    check.exp_hume = skel_file
        .and_then(|f| part_mesh_count(dat_root, f))
        .unwrap_or(0)
        + parts
            .iter()
            .map(|(_, file_id)| part_mesh_count(dat_root, *file_id).unwrap_or(0))
            .sum::<usize>();
    check.exp_worm = part_mesh_count(dat_root, worm_file).unwrap_or(0);
    if check.exp_worm == 0 {
        log_line(
            log,
            format!("ERROR: worm={worm_file} unreadable or 0 mesh buffers — will not render"),
        );
    }
}

fn verify_drawn(
    mut check: ResMut<DrawnCheck>,
    tracked: Res<TrackedEntities>,
    q_root: Query<&FfxiRenderRoot>,
    q_children: Query<&Children>,
    q_mesh: Query<Entity, With<FfxiActorMeshChild>>,
    mut log: ResMut<TestLog>,
) {
    let count_drawn = |wire: Entity| -> Option<usize> {
        let root = q_root.get(wire).ok()?.0;
        let children = q_children.get(root).ok()?;
        Some(children.iter().filter(|&c| q_mesh.get(c).is_ok()).count())
    };

    if !check.hume_done {
        if let Some(hume) = tracked.by_id.get(&HUME_ID).copied() {
            if let Some(n) = count_drawn(hume) {
                check.hume_done = true;
                if n == 0 {
                    log_line(&mut log, "ERROR: hume drew no mesh parts".into());
                } else if n < check.exp_hume {
                    let exp = check.exp_hume;
                    log_line(
                        &mut log,
                        format!("ERROR: hume drew {n} of {exp} expected mesh parts"),
                    );
                } else if check.parts_ok {
                    log_line(
                        &mut log,
                        format!(
                            "drawn on player: {n} mesh parts (all checked parts, sword included)"
                        ),
                    );
                } else {
                    log_line(&mut log, format!("drawn on player: {n} mesh parts"));
                }
            }
        }
    }

    if !check.worm_done {
        if let Some(worm) = tracked.by_id.get(&WORM_ID).copied() {
            if let Some(n) = count_drawn(worm) {
                check.worm_done = true;
                if n == 0 {
                    log_line(&mut log, "ERROR: worm drew no mesh parts".into());
                } else {
                    log_line(&mut log, format!("drawn: worm {n} mesh parts"));
                }
            }
        }
    }
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

// Per-case animation windows: swing + impact reaction; death adds the fall-over, level-up
// runs to its frame-170 tail (lvup `main` in effect DAT 3310), respawn is near-instant.
fn case_duration(case: Case) -> std::time::Duration {
    match case {
        Case::PlayerNhIt | Case::MobNhIt => std::time::Duration::from_millis(1200),
        Case::PlayerChit | Case::MobChit => std::time::Duration::from_millis(1500),
        Case::PlayerDhit => std::time::Duration::from_millis(2000),
        Case::LevelUp => std::time::Duration::from_millis(3500),
        Case::MobRespawn => std::time::Duration::from_millis(300),
    }
}

fn handle_case_presses(
    q_buttons: Query<(&CaseButton, &Interaction)>,
    mut pending: ResMut<PendingCase>,
    lock: Res<CaseLock>,
) {
    for (case_button, interaction) in q_buttons.iter() {
        if !matches!(interaction, Interaction::Pressed) {
            continue;
        }
        // Bevy holds Pressed for the whole mouse-down, so a locked press is ignored silently
        // (the buttons grey out while the case window runs) instead of logging per frame.
        if lock.until.is_some_and(|until| Instant::now() < until) {
            continue;
        }
        pending.0 = Some(case_button.0);
    }
}

// The dispatch funnel's per-generator trace lines (route + timing), mirrored into the panel so
// they sit next to the case log instead of only in stderr.
fn collect_spawn_traces(
    mut traces: MessageReader<ParticleSpawnTrace>,
    q_scoped: Query<Entity, With<TestSceneScoped>>,
    mut log: ResMut<TestLog>,
) {
    if q_scoped.iter().next().is_none() {
        return;
    }
    for t in traces.read() {
        log_line(&mut log, t.0.clone());
    }
}

// Grey the case buttons out while a case window runs; restore them when it elapses.
fn sync_case_buttons(
    lock: Res<CaseLock>,
    mut q_btns: Query<(&mut BackgroundColor, &Children), With<CaseButton>>,
    mut q_text: Query<&mut TextColor>,
) {
    let locked = lock.until.is_some_and(|until| Instant::now() < until);
    for (mut bg, children) in &mut q_btns {
        *bg = if locked {
            BackgroundColor(Color::srgb(0.10, 0.12, 0.16))
        } else {
            BackgroundColor(Color::srgb(0.16, 0.2, 0.3))
        };
        for child in children {
            if let Ok(mut tc) = q_text.get_mut(*child) {
                *tc = TextColor(if locked {
                    Color::srgb(0.45, 0.48, 0.55)
                } else {
                    Color::WHITE
                });
            }
        }
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
    mut events: ResMut<EventLog>,
    mut hp: ResMut<TestHp>,
    mut lock: ResMut<CaseLock>,
) {
    let Some(case) = pending.0.take() else {
        return;
    };
    log_line(&mut log, format!("case: {}", case.label()));

    // A dead worm can neither swing nor react: bring it back before the hit so the impact
    // lands on a live model.
    if matches!(
        case,
        Case::PlayerNhIt | Case::PlayerChit | Case::PlayerDhit | Case::MobNhIt | Case::MobChit
    ) && hp.worm == 0
    {
        log_line(&mut log, "worm dead — respawning before hit".into());
        respawn_worm(
            &mut worm_state,
            &mut scene,
            &mut log,
            &tracked,
            &q_root,
            &mut q_vis,
            &q_children,
            &mut commands,
            &mut hp,
        );
    }

    match case {
        Case::MobRespawn => respawn_worm(
            &mut worm_state,
            &mut scene,
            &mut log,
            &tracked,
            &q_root,
            &mut q_vis,
            &q_children,
            &mut commands,
            &mut hp,
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
        _ => fire_hit(case, &tracked, &mut log, &mut events, &mut scene, &mut hp),
    }

    lock.until = Some(Instant::now() + case_duration(case));
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

// The button is the server: one BATTLE2-shaped ActionStarted into the EventLog. Production
// reacts from there — dispatch_action_overlay plays the attacker's swing clip (ati0), and
// dispatch_melee_action_started enqueues the effects and arms the victim reaction that the
// swing's DamageCallback fires at its impact frame.
fn fire_hit(
    case: Case,
    tracked: &TrackedEntities,
    log: &mut TestLog,
    events: &mut EventLog,
    scene: &mut SceneState,
    hp: &mut TestHp,
) {
    let (attacker_id, victim_id) = match case {
        Case::PlayerNhIt | Case::PlayerChit | Case::PlayerDhit => (HUME_ID, WORM_ID),
        _ => (WORM_ID, HUME_ID),
    };
    if tracked.by_id.get(&attacker_id).is_none() || tracked.by_id.get(&victim_id).is_none() {
        log_line(log, "actors not loaded yet — try again in a second".into());
        return;
    }
    events.push(kuluu_snapshot::ViewerEvent::ActionStarted {
        actor_id: attacker_id,
        action_id: u32::from_le_bytes(*b"atk0"),
        action_kind: ffxi_proto::melee::CATEGORY_BASIC_ATTACK,
        target_id: Some(victim_id),
        result: Some((0, 0)), // resolution Hit, animation RightAttack
        animation: Some(0),
        outcome: Some((case.info_bits(), 0, 0)),
    });
    // The 0x0E ships in the same flush as BATTLE2: apply the damage to the tracked HP and
    // publish the percentage, so death rides on wire state like real combat.
    let victim_hp = if victim_id == WORM_ID {
        &mut hp.worm
    } else {
        &mut hp.hume
    };
    *victim_hp = victim_hp.saturating_sub(case.damage());
    let pct = ((*victim_hp as u128) * 100 / TEST_MAX_HP as u128).min(100) as u8;
    if let Some(e) = scene
        .snapshot
        .entities
        .iter_mut()
        .find(|e| e.id == victim_id)
    {
        e.hp_pct = Some(pct);
    }
    log_line(
        log,
        format!(
            "action started: {} -> {} ({}): hp {}",
            if attacker_id == HUME_ID {
                "hume"
            } else {
                "worm"
            },
            if victim_id == WORM_ID { "worm" } else { "hume" },
            case.label(),
            *victim_hp
        ),
    );
}

fn respawn_worm(
    worm_state: &mut WormState,
    scene: &mut SceneState,
    log: &mut TestLog,
    tracked: &TrackedEntities,
    q_root: &Query<&kuluu_render::ffxi_actor_render::FfxiRenderRoot>,
    q_vis: &mut Query<&mut Visibility>,
    q_children: &Query<&Children>,
    commands: &mut Commands,
    hp: &mut TestHp,
) {
    let Some(worm) = tracked.by_id.get(&WORM_ID).copied() else {
        return;
    };
    if let Ok(root) = q_root.get(worm) {
        if let Ok(mut vis) = q_vis.get_mut(root.0) {
            *vis = Visibility::Visible;
        }
    }
    // Cancel the death path so the pose falls back to idle: drop the Defeated latch and any
    // running `dead` scheduler (its cor0 hold is what keeps a respawned worm on the ground).
    if let Ok(children) = q_children.get(worm) {
        for child in children {
            commands
                .entity(*child)
                .remove::<kuluu_render::scheduler_runtime::DeadFromAction>();
            commands
                .entity(*child)
                .remove::<kuluu_render::scheduler_runtime::ActiveSchedulers>();
        }
    }
    worm_state.dead_at = None;
    hp.worm = TEST_MAX_HP;
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

// The launcher backdrop mirrors a live zone into the same world space; its meshes carry
// InGameEntity and would show through around the test floor. While the box is up, hide every
// InGameEntity not under it — effect particles are children of the test actors, so they stay.
// Restores visibility on teardown.
fn zone_backdrop_visibility(
    q_scoped: Query<Entity, With<TestSceneScoped>>,
    mut q_vis: Query<
        (&mut Visibility, Option<&ChildOf>),
        (With<InGameEntity>, Without<TestSceneScoped>),
    >,
    q_anc: Query<(Option<&ChildOf>, Has<TestSceneScoped>)>,
) {
    let active = q_scoped.iter().next().is_some();
    for (mut vis, parent) in &mut q_vis {
        if active {
            if matches!(*vis, Visibility::Hidden) {
                continue;
            }
            let mut under_test = false;
            let mut cur = parent.map(|p| p.parent());
            while let Some(p) = cur {
                let (next, scoped) = match q_anc.get(p) {
                    Ok(v) => v,
                    Err(_) => break,
                };
                if scoped {
                    under_test = true;
                    break;
                }
                cur = next.map(|n| n.parent());
            }
            if !under_test {
                *vis = Visibility::Hidden;
            }
        } else if matches!(*vis, Visibility::Hidden) {
            *vis = Visibility::default();
        }
    }
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
