use bevy::prelude::*;

use crate::hud::overlay::ActiveOverlay;
use crate::hud::style::{self, theme};
use crate::snapshot::SceneState;

#[derive(Debug, Clone, Copy)]
pub struct StatusEntry {
    pub kind: StatusEntryKind,
    pub label: &'static str,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEntryKind {
    Profile,
    JobLevels,
    MasterLevels,
    CombatSkill,
    MagicSkill,
    CraftSkill,
    Currencies,
    Currencies2,
    Unity,
    PlayTime,
    MeritPoints,
    JobPoints,
    /// statcom2 id 13 (`fp_cat` — the record's caption reads "Displays
    /// Categories for which alternate...", vault main_menu.md §59): the Facet
    /// Points category browser. Kuluu has no facet store yet, so its detail
    /// window shows the grounded empty readout.
    FacetCategories,
}

/// statcom2's live rows in DISPLAY order (block y), paired with each row's
/// handler-visible item id: the selector dispatches on these ids (FFXiMain.dll
/// retail-2026-09 select handler RVA 0x148D00 over detail table RVA 0x3716B8)
/// while rows draw where their block y says — so Master Levels (id 12) sits
/// third and Play Time (id 9) tenth, the record read in `ffxi-dat`
/// `menu_windows` pins (vault main_menu.md §59). STATUS_ENTRIES follows this
/// order row for row.
pub const STATCOM2_DISPLAY_ITEM_IDS: [i16; 13] = [1, 2, 12, 3, 4, 5, 6, 10, 11, 9, 7, 8, 13];

/// The focus law (vault main_menu.md §59): display-only windows never receive keys,
/// so while one is open the selector underneath keeps navigating. These are
/// exactly retail's display-only statcom2 companions — profile, joblevel,
/// btlskill, mgcskill, trdskill, masterle (detail table entries behind
/// FFXiMain.dll retail-2026-09 RVA 0x148D00; the close-details helper at
/// RVA 0x148AC0 only ever closes these six).
pub fn status_detail_display_only(kind: StatusEntryKind) -> bool {
    matches!(
        kind,
        StatusEntryKind::Profile
            | StatusEntryKind::JobLevels
            | StatusEntryKind::MasterLevels
            | StatusEntryKind::CombatSkill
            | StatusEntryKind::MagicSkill
            | StatusEntryKind::CraftSkill
    )
}

pub const STATUS_ENTRIES: &[StatusEntry] = &[
    StatusEntry {
        kind: StatusEntryKind::Profile,
        label: "Profile",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::JobLevels,
        label: "Job Levels",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::MasterLevels,
        label: "Master Levels",
        enabled: false,
    },
    StatusEntry {
        kind: StatusEntryKind::CombatSkill,
        label: "Combat Skill",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::MagicSkill,
        label: "Magic Skill",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::CraftSkill,
        label: "Craft Skill",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::Currencies,
        label: "Currencies",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::Currencies2,
        label: "Currencies 2",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::Unity,
        label: "Unity",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::PlayTime,
        label: "Play Time",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::MeritPoints,
        label: "Merit Points",
        enabled: false,
    },
    StatusEntry {
        kind: StatusEntryKind::JobPoints,
        label: "Job Points",
        enabled: true,
    },
    StatusEntry {
        kind: StatusEntryKind::FacetCategories,
        // The record's description (catalog table 11 id 1096) reads "Displays
        // Categories for which alter ego points may be spent to increase
        // attributes" — the label follows that wording (vault main_menu.md §59).
        label: "Alter Ego Points",
        enabled: true,
    },
];

pub fn status_entry_count() -> usize {
    STATUS_ENTRIES.len()
}

const PANEL_WIDTH_PX: f32 = 280.0;

/// Left-column host for the persona panel and its `level` box, at the spot the
/// menu flow opens them (vault main_menu.md §59 addendum 10): opening Status
/// code-moves persona to design **(16,16)** (mode-1 move; record frame
/// `(16,48)`) and draws `level` right under it — both display-only while
/// statcom2 holds the keys.
#[derive(Component)]
pub struct StatusLeftStack;

#[derive(Component)]
pub struct StatusPanel;

/// The framed `level` box (retail window `menu    level  `): Level / EXP /
/// NEXTLevel, from the s2c 0x061 pair kuluu folds into
/// [`kuluu_snapshot::CharStats::exp_now`] (vendor/server/src/map/packets/s2c/
/// 0x061_clistatus.h CLISTATUS exp_now/exp_next).
#[derive(Component)]
pub struct StatusLevelBox;

#[derive(Component)]
pub struct StatusLevelText;

/// True while the stack holds statcom2 or one of its detail windows — retail
/// opens persona + level WITH the list and cascades them closed together, so
/// kuluu shows both panels for exactly as long as a `Status`/detail frame is open.
pub fn status_flow_open(mode: &crate::input_mode::InputMode) -> bool {
    let crate::input_mode::InputMode::Menu(stack) = mode else {
        return false;
    };
    stack.levels.iter().any(|l| {
        matches!(
            l.kind,
            crate::input_mode::MenuKind::Status | crate::input_mode::MenuKind::StatusDetail { .. }
        )
    })
}

#[derive(Component)]
pub struct StatusHeaderRow;

#[derive(Component)]
pub struct StatusVitalsRow;

#[derive(Component)]
pub struct StatusItemLevelRow;

#[derive(Component)]
pub struct StatusAttrRow {
    pub attr_index: usize,
}

pub const PRIMARY_ATTRS: &[&str] = &["STR", "DEX", "VIT", "AGI", "INT", "MND", "CHR"];

pub fn spawn_status_panel(mut commands: Commands) {
    let stack = commands
        .spawn((
            crate::components::InGameEntity,
            StatusLeftStack,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                left: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    let panel = commands
        .spawn((
            StatusPanel,
            Node {
                width: Val::Px(PANEL_WIDTH_PX),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::FRAME_EDGE),
        ))
        .id();
    commands.entity(stack).add_child(panel);
    commands.entity(stack).with_children(|s| {
        s.spawn((
            StatusLevelBox,
            Node {
                width: Val::Px(150.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::FRAME_EDGE),
        ))
        .with_children(|b| {
            b.spawn((
                StatusLevelText,
                Text::new(""),
                style::text_font(13.0),
                TextColor(theme::TEXT),
            ));
        });
    });
    commands.entity(panel).with_children(|p| {
        p.spawn((
            StatusHeaderRow,
            Text::new(""),
            style::text_font(14.0),
            TextColor(theme::TITLE),
        ));
        p.spawn((
            StatusVitalsRow,
            Text::new(""),
            style::text_font(13.0),
            TextColor(theme::TEXT),
        ));
        p.spawn((
            StatusItemLevelRow,
            Text::new(""),
            style::text_font(13.0),
            TextColor(theme::MUTED),
        ));
        for attr_index in 0..PRIMARY_ATTRS.len() {
            p.spawn((
                StatusAttrRow { attr_index },
                Text::new(""),
                style::text_font(13.0),
                TextColor(theme::TEXT),
            ));
        }
    });
}

/// The `level` box body: Level / EXP / NEXTLevel (labels follow the retail
/// window's on-screen words captured in the owner's screenshot; values from
/// s2c 0x061 — no wire value, no invented number).
pub fn level_box_lines(snap: &kuluu_snapshot::SceneSnapshot) -> String {
    let me = crate::snapshot::resolve_self(&snap.party, snap.self_char_id);
    let Some(lv) = me.map(|m| m.main_job_lv) else {
        return "Level —".to_string();
    };
    match snap.stats.as_ref() {
        Some(s) if s.exp_next != 0 => {
            format!("Level {lv}\nEXP {}\nNEXTLevel {}", s.exp_now, s.exp_next)
        }
        _ => format!("Level {lv}"),
    }
}

pub fn update_status_panel(
    mode: Res<crate::input_mode::InputMode>,
    state: Res<SceneState>,
    mut stack_q: Query<&mut Node, With<StatusLeftStack>>,
    mut level_text_q: Query<&mut Text, With<StatusLevelText>>,
    // Pairwise-disjoint per the cycle-57 B0001 law: every &mut Text query here
    // must exclude EVERY other marker in the system (StatusLevelText included —
    // schedule-time conflict detection killed the release client without it).
    mut header_q: Query<
        &mut Text,
        (
            With<StatusHeaderRow>,
            Without<StatusLevelText>,
            Without<StatusVitalsRow>,
            Without<StatusItemLevelRow>,
            Without<StatusAttrRow>,
        ),
    >,
    mut vitals_q: Query<
        &mut Text,
        (
            With<StatusVitalsRow>,
            Without<StatusHeaderRow>,
            Without<StatusLevelText>,
            Without<StatusItemLevelRow>,
            Without<StatusAttrRow>,
        ),
    >,
    mut ilvl_q: Query<
        &mut Text,
        (
            With<StatusItemLevelRow>,
            Without<StatusHeaderRow>,
            Without<StatusVitalsRow>,
            Without<StatusLevelText>,
            Without<StatusAttrRow>,
        ),
    >,
    mut attr_q: Query<
        (&StatusAttrRow, &mut Text),
        (
            Without<StatusHeaderRow>,
            Without<StatusVitalsRow>,
            Without<StatusItemLevelRow>,
            Without<StatusLevelText>,
        ),
    >,
) {
    let Ok(mut stack_node) = stack_q.single_mut() else {
        return;
    };
    let open = status_flow_open(&mode);
    if !open && stack_node.display != Display::None {
        stack_node.display = Display::None;
        return;
    }
    if open && stack_node.display == Display::None {
        stack_node.display = Display::Flex;
    }

    let snap = &state.snapshot;
    let me = crate::snapshot::resolve_self(&snap.party, snap.self_char_id);

    if let Ok(mut text) = header_q.single_mut() {
        let want = profile_header(snap, me);
        if **text != want {
            **text = want;
        }
    }

    if let Ok(mut text) = vitals_q.single_mut() {
        let want = match me {
            Some(m) => format!("HP {}   MP {}   TP {}", m.hp, m.mp, m.tp),
            None => "HP —   MP —   TP —".to_string(),
        };
        if **text != want {
            **text = want;
        }
    }

    if let Ok(mut text) = ilvl_q.single_mut() {
        let want = match snap.stats.as_ref().filter(|s| s.item_level > 0) {
            Some(s) => format!("Item Level: {}", s.item_level),
            None => "Item Level: —".to_string(),
        };
        if **text != want {
            **text = want;
        }
    }

    if let Ok(mut text) = level_text_q.single_mut() {
        let want = level_box_lines(snap);
        if **text != want {
            **text = want;
        }
    }

    for (row, mut text) in attr_q.iter_mut() {
        let Some(name) = PRIMARY_ATTRS.get(row.attr_index) else {
            continue;
        };
        let want = match snap.stats.as_ref() {
            Some(s) => {
                let base = attr_value(s, row.attr_index).unwrap_or(0);
                let bonus = s.bonus.get(row.attr_index).copied().unwrap_or(0);
                if bonus != 0 {
                    format!("{name:<4}{base:>4} {bonus:+}")
                } else {
                    format!("{name:<4}{base:>4}")
                }
            }
            None => format!("{name:<4}   —"),
        };
        if **text != want {
            **text = want;
        }
    }
}

fn profile_header(
    snap: &kuluu_snapshot::SceneSnapshot,
    me: Option<&kuluu_snapshot::PartyMember>,
) -> String {
    let name = me
        .and_then(|m| m.name.clone())
        .or_else(|| snap.char_name.clone())
        .unwrap_or_else(|| "—".to_string());
    match me {
        Some(m) => {
            let main = job_abbrev(m.main_job);
            if m.sub_job != 0 {
                let sub = job_abbrev(m.sub_job);
                format!("{name}  {main}{} / {sub}{}", m.main_job_lv, m.sub_job_lv)
            } else {
                format!("{name}  {main}{}", m.main_job_lv)
            }
        }
        None => name,
    }
}

pub(crate) fn job_abbrev(job_id: u8) -> String {
    if job_id == 0 {
        return "---".to_string();
    }
    // Canonical 3-letter code from LSB (2 → "MNK"); truncating "Monk" gives "MON".
    match ffxi_vocab::job_names::abbrev(job_id as u16) {
        Some(code) => code.to_string(),
        None => format!("J{job_id}"),
    }
}

fn attr_value(stats: &kuluu_snapshot::CharStats, attr_index: usize) -> Option<u16> {
    Some(match attr_index {
        0 => stats.str_,
        1 => stats.dex,
        2 => stats.vit,
        3 => stats.agi,
        4 => stats.int_,
        5 => stats.mnd,
        6 => stats.chr,
        _ => return None,
    })
}

/// The detail window's rows for one Status selector row (vault main_menu.md §59).
/// Every line is either live snapshot data or a grounded empty readout — the
/// display-only companions in retail show exactly what their source object
/// holds, and kuluu shows nothing more than that from a window it has not
/// built packet readers for yet. Selecting any of these rows is inert
/// ([`crate::hud::menu::DynamicMenuAction::DisplayInfo`]); the windows carry no
/// commands (statcom2's detail table maps ids to OPEN actions only, FFXiMain.dll
/// retail-2026-09 RVA 0x148D00).
pub fn status_detail_lines(entry_idx: usize, snap: &kuluu_snapshot::SceneSnapshot) -> Vec<String> {
    let Some(entry) = STATUS_ENTRIES.get(entry_idx) else {
        return vec!["(no such status row)".to_string()];
    };
    let me = crate::snapshot::resolve_self(&snap.party, snap.self_char_id);
    match entry.kind {
        StatusEntryKind::Profile => {
            let mut lines = vec![profile_header(snap, me)];
            lines.push(match me {
                Some(m) => format!("HP {}   MP {}   TP {}", m.hp, m.mp, m.tp),
                None => "HP —   MP —   TP —".to_string(),
            });
            lines.push(match snap.stats.as_ref().filter(|s| s.item_level > 0) {
                Some(s) => format!("Item Level: {}", s.item_level),
                _ => "Item Level: —".to_string(),
            });
            for (attr_index, name) in PRIMARY_ATTRS.iter().enumerate() {
                lines.push(match snap.stats.as_ref() {
                    Some(s) => {
                        let base = attr_value(s, attr_index).unwrap_or(0);
                        let bonus = s.bonus.get(attr_index).copied().unwrap_or(0);
                        if bonus != 0 {
                            format!("{name:<4}{base:>4} {bonus:+}")
                        } else {
                            format!("{name:<4}{base:>4}")
                        }
                    }
                    None => format!("{name:<4}   —"),
                });
            }
            lines
        }
        StatusEntryKind::JobLevels => match snap.jobs.as_ref() {
            Some(list) => {
                let mut lines: Vec<String> = list
                    .job_levels
                    .iter()
                    .enumerate()
                    .filter(|(job_id, _)| *job_id != 0)
                    .filter(|(_, &level)| level > 0)
                    .map(|(job_id, &level)| {
                        format!("{:<16} {}", job_abbrev(job_id as u8), level)
                    })
                    .collect();
                if lines.is_empty() {
                    lines.push("(no job levels reported yet)".to_string());
                }
                lines
            }
            None => vec!["(job roster not received yet)".to_string()],
        },
        StatusEntryKind::MasterLevels => vec![
            "(master-level roster has no wire source in LSB yet — the window opens empty like retail's with nothing to show)".to_string(),
        ],
        StatusEntryKind::CombatSkill | StatusEntryKind::MagicSkill | StatusEntryKind::CraftSkill => {
            vec!["(skill lists arrive with the login skill packet — not captured yet)".to_string()]
        }
        StatusEntryKind::Currencies | StatusEntryKind::Currencies2 => {
            vec!["(event-item currency list not received yet)".to_string()]
        }
        StatusEntryKind::Unity => vec!["(unity campaign data not received yet)".to_string()],
        StatusEntryKind::PlayTime => vec![play_time_chat_line(snap)],
        StatusEntryKind::MeritPoints => vec!["(merit points have no wire source in LSB yet — the window opens empty like retail's with nothing to show)".to_string()],
        StatusEntryKind::JobPoints => vec!["(job-point category not received yet)".to_string()],
        StatusEntryKind::FacetCategories => {
            vec!["(facet-point categories have no wire source in LSB yet — the window opens empty like retail's with nothing to show)".to_string()]
        }
    }
}

pub fn play_time_chat_line(snap: &kuluu_snapshot::SceneSnapshot) -> String {
    let total = snap.play_time_s;
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let mins = (total % 3_600) / 60;
    if days > 0 {
        format!("Play time: {days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("Play time: {hours}h {mins}m")
    } else {
        format!("Play time: {mins}m")
    }
}

pub fn status_entries_for(_overlay: &ActiveOverlay) -> &'static [StatusEntry] {
    STATUS_ENTRIES
}

#[derive(Debug, Clone)]
pub struct JobLevelRow {
    pub job_id: u8,
    pub name: &'static str,
    pub level: u8,
}

pub fn job_level_rows(overlay: &ActiveOverlay, levels: &[(u8, u8)]) -> Vec<JobLevelRow> {
    levels
        .iter()
        .filter(|(job_id, _)| overlay.0.job_allowed(*job_id))
        .filter_map(|&(job_id, level)| {
            ffxi_vocab::job_names::lookup(job_id as u16).map(|name| JobLevelRow {
                job_id,
                name,
                level,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression pin (release-client panic, 2026-10-14): the real schedule runs
    /// `update_status_panel`, and Bevy's B0001 conflict check fires at system
    /// init — every &mut Text parameter pair must be provably disjoint via
    /// Without<..>. Unit tests never touched the scheduler, so this only ever
    /// exploded in the built game; ticking it once here pins that class.
    #[test]
    fn update_status_panel_runs_in_a_real_schedule_without_query_conflicts() {
        use bevy::app::App;
        let mut app = App::new();
        app.add_systems(bevy::app::Update, update_status_panel);
        app.insert_resource(crate::input_mode::InputMode::default());
        app.insert_resource(SceneState::default());
        // No entities: the system returns early, but the queries are already
        // registered — a B0001 conflict panics inside this same call.
        app.update();
    }

    /// The `level` box under the persona panel answers from s2c 0x061 EXP
    /// (vault main_menu.md §59 addendum 10 screenshot law): Level / EXP /
    /// NEXTLevel, no invented values when the stats block is absent.
    #[test]
    fn level_box_answers_from_the_wire_exp_pair() {
        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        assert_eq!(level_box_lines(&snap), "Level —", "no self yet");

        snap.self_char_id = Some(7);
        snap.party.push(kuluu_snapshot::PartyMember {
            id: 7,
            act_index: 0x100,
            name: Some("Cowrevenge".into()),
            hp: 98,
            mp: 55,
            tp: 572,
            hp_pct: 100,
            mp_pct: 100,
            zone_no: 104,
            main_job: 22,
            main_job_lv: 6,
            sub_job: 0,
            sub_job_lv: 0,
            is_party_leader: true,
            is_alliance_leader: false,
            in_mog_house: false,
            party_no: 0,
        });
        assert_eq!(level_box_lines(&snap), "Level 6", "no stats block yet");

        snap.stats = Some(kuluu_snapshot::CharStats {
            exp_now: 806,
            exp_next: 1750,
            ..Default::default()
        });
        assert_eq!(level_box_lines(&snap), "Level 6\nEXP 806\nNEXTLevel 1750");
    }

    #[test]
    fn status_entries_have_disabled_post_classic_screens() {
        let ml = STATUS_ENTRIES
            .iter()
            .find(|e| e.kind == StatusEntryKind::MasterLevels)
            .unwrap();
        assert!(!ml.enabled, "Master Levels disabled");
        let mp = STATUS_ENTRIES
            .iter()
            .find(|e| e.kind == StatusEntryKind::MeritPoints)
            .unwrap();
        assert!(!mp.enabled, "Merit Points disabled");
    }

    #[test]
    fn status_entry_order_matches_retail() {
        assert_eq!(STATUS_ENTRIES[0].kind, StatusEntryKind::Profile);
        assert_eq!(STATUS_ENTRIES[1].kind, StatusEntryKind::JobLevels);
        // statcom2 displays 13 live rows (ffxi-dat record pin, vault
        // main_menu.md §59); the engine list follows that display order.
        assert_eq!(status_entry_count(), STATCOM2_DISPLAY_ITEM_IDS.len());
        assert_eq!(
            STATUS_ENTRIES.last().unwrap().kind,
            StatusEntryKind::FacetCategories
        );
    }

    #[test]
    fn statcom_display_only_set_matches_retail_detail_table() {
        // The retail set is profile/joblevel/btlskill/mgcskill/trdskill/
        // masterle: exactly what FFXiMain.dll retail-2026-09 close-details
        // helper RVA 0x148AC0 acts on (vault main_menu.md §59).
        for kind in [
            StatusEntryKind::Profile,
            StatusEntryKind::JobLevels,
            StatusEntryKind::MasterLevels,
            StatusEntryKind::CombatSkill,
            StatusEntryKind::MagicSkill,
            StatusEntryKind::CraftSkill,
        ] {
            assert!(status_detail_display_only(kind), "{kind:?} display-only");
        }
        for kind in [
            StatusEntryKind::Currencies,
            StatusEntryKind::MeritPoints,
            StatusEntryKind::JobPoints,
            StatusEntryKind::Unity,
            StatusEntryKind::FacetCategories,
        ] {
            assert!(!status_detail_display_only(kind), "{kind:?} interactive");
        }
    }

    #[test]
    fn job_abbrev_uses_canonical_codes() {
        assert_eq!(job_abbrev(0), "---");
        assert_eq!(job_abbrev(1), "WAR");
        assert_eq!(job_abbrev(2), "MNK"); // not "MON"
        assert_eq!(job_abbrev(3), "WHM"); // not "WHI"
        assert_eq!(job_abbrev(4), "BLM"); // not "BLA"
    }

    #[test]
    fn primary_attrs_count_is_seven() {
        assert_eq!(PRIMARY_ATTRS.len(), 7);
        assert_eq!(PRIMARY_ATTRS[0], "STR");
        assert_eq!(PRIMARY_ATTRS[6], "CHR");
    }
}
