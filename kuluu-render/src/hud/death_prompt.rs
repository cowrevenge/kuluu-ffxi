use bevy::prelude::*;

use crate::hud::style::{self, theme};
use crate::snapshot::resolve_self;
use crate::snapshot::SceneState;

#[derive(Component)]
pub struct DeathPromptPanel;

#[derive(Component)]
pub struct DeathPromptInstructionText;

#[derive(Component)]
pub struct DeathPromptChoicesText;

#[derive(Resource, Debug, Default)]
pub struct DeathPromptSelection {
    observed_offer: Option<kuluu_snapshot::DeathMenuOffer>,
    accept: bool,
}

impl DeathPromptSelection {
    pub fn sync(&mut self, offer: Option<kuluu_snapshot::DeathMenuOffer>) {
        if self.observed_offer != offer {
            self.observed_offer = offer;
            self.accept = false;
        }
    }

    pub fn toggle(&mut self) {
        self.accept = !self.accept;
    }

    pub fn accepts_offer(&self) -> bool {
        self.accept
    }
}

const PANEL_WIDTH_PX: f32 = 380.0;

pub fn spawn_death_prompt(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            DeathPromptPanel,
            Node {
                position_type: PositionType::Absolute,

                top: Val::Percent(35.0),
                left: Val::Percent(50.0),
                margin: UiRect {
                    left: Val::Px(-PANEL_WIDTH_PX / 2.0),
                    ..default()
                },
                width: Val::Px(PANEL_WIDTH_PX),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                display: Display::None,
                ..default()
            },
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::DANGER),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("You were defeated."),
                style::text_font(16.0),
                TextColor(theme::DANGER),
            ));
            p.spawn((
                DeathPromptInstructionText,
                Text::new(String::new()),
                style::text_font(13.0),
                TextColor(theme::TEXT),
            ));
            p.spawn((
                DeathPromptChoicesText,
                Text::new(String::new()),
                style::text_font(13.0),
                TextColor(theme::TEXT),
            ));
            // The numeric clock is Enhanced: retail shows the home-point menu
            // and no visible KO countdown
            // (.agents/skills/retail-observe/references/death-ko-behavior.md).
            #[cfg(feature = "enhanced-death-countdown")]
            p.spawn(crate::hud::death_countdown::countdown_bundle());
        });
}

pub fn update_death_prompt_system(
    state: Res<SceneState>,
    mut selection: ResMut<DeathPromptSelection>,
    mut panel_q: Query<&mut Node, With<DeathPromptPanel>>,
    mut instruction_q: Query<
        &mut Text,
        (
            With<DeathPromptInstructionText>,
            Without<DeathPromptChoicesText>,
        ),
    >,
    mut choices_q: Query<
        &mut Text,
        (
            With<DeathPromptChoicesText>,
            Without<DeathPromptInstructionText>,
        ),
    >,
) {
    let snap = &state.snapshot;

    let dead = resolve_self(&snap.party, snap.self_char_id)
        .map(|m| m.hp_pct == 0)
        .unwrap_or(false);

    if let Ok(mut panel_node) = panel_q.single_mut() {
        let want = if dead { Display::Flex } else { Display::None };
        if panel_node.display != want {
            panel_node.display = want;
        }
    }

    let offer = dead.then_some(snap.death_menu_offer).flatten();
    selection.sync(offer);

    if let Ok(mut text) = instruction_q.single_mut() {
        let label = prompt_instruction(offer);
        if **text != label {
            **text = label.into();
        }
    }
    if let Ok(mut text) = choices_q.single_mut() {
        let label = prompt_choices(offer, selection.accepts_offer());
        if **text != label {
            **text = label;
        }
    }
}

fn prompt_instruction(offer: Option<kuluu_snapshot::DeathMenuOffer>) -> &'static str {
    match offer {
        None => "Press [Enter] to return to your home point.",
        Some(kuluu_snapshot::DeathMenuOffer::Raise) => "Accept Raise or Reraise?",
        Some(kuluu_snapshot::DeathMenuOffer::Tractor) => "Accept Tractor?",
    }
}

fn prompt_choices(offer: Option<kuluu_snapshot::DeathMenuOffer>, accept: bool) -> String {
    if offer.is_none() {
        return String::new();
    }
    if accept {
        "> Yes\n  No".into()
    } else {
        "  Yes\n> No".into()
    }
}

pub fn drain_death_prompt_selection(mut selection: ResMut<DeathPromptSelection>) {
    *selection = DeathPromptSelection::default();
}

pub fn is_dead(state: &SceneState) -> bool {
    let snap = &state.snapshot;
    resolve_self(&snap.party, snap.self_char_id)
        .map(|m| m.hp_pct == 0)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kuluu_snapshot::PartyMember;

    fn member(id: u32, hp_pct: u8) -> PartyMember {
        PartyMember {
            id,
            act_index: 1,
            name: None,
            hp: 0,
            mp: 0,
            tp: 0,
            hp_pct,
            mp_pct: 100,
            zone_no: 0,
            main_job: 0,
            main_job_lv: 0,
            sub_job: 0,
            sub_job_lv: 0,
            is_party_leader: false,
            is_alliance_leader: false,
            in_mog_house: false,
            party_no: 0,
        }
    }

    #[test]
    fn is_dead_true_when_self_party_row_at_zero() {
        let mut state = SceneState::default();
        state.snapshot.self_char_id = Some(7);
        state.snapshot.party = vec![member(7, 0)];
        assert!(is_dead(&state));
    }

    #[test]
    fn is_dead_false_when_self_alive() {
        let mut state = SceneState::default();
        state.snapshot.self_char_id = Some(7);
        state.snapshot.party = vec![member(7, 50)];
        assert!(!is_dead(&state));
    }

    #[test]
    fn is_dead_false_when_party_empty() {
        let state = SceneState::default();
        assert!(!is_dead(&state));
    }

    #[test]
    fn is_dead_falls_back_to_first_member_when_self_id_unknown() {
        let mut state = SceneState::default();
        state.snapshot.self_char_id = None;
        state.snapshot.party = vec![member(99, 0)];
        assert!(is_dead(&state));
    }

    /// The panel's vanilla rows: the "You were defeated." header, the
    /// instruction line, and the Yes/No choices. Retail shows the home-point
    /// menu and no numeric KO clock
    /// (.agents/skills/retail-observe/references/death-ko-behavior.md), so the
    /// countdown row exists only in an `enhanced-death-countdown` build.
    const VANILLA_PROMPT_ROWS: usize = 3;

    #[test]
    fn the_numeric_countdown_row_exists_only_in_an_enhanced_build() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world
            .run_system_once(spawn_death_prompt)
            .expect("spawner runs");

        let panel = world
            .query_filtered::<Entity, With<DeathPromptPanel>>()
            .single(&world)
            .expect("one death prompt panel");
        let rows = world
            .entity(panel)
            .get::<Children>()
            .map(|c| c.len())
            .unwrap_or(0);

        assert_eq!(
            rows,
            VANILLA_PROMPT_ROWS + cfg!(feature = "enhanced-death-countdown") as usize
        );
    }

    #[test]
    fn raise_and_tractor_offers_render_safe_default_choices() {
        use kuluu_snapshot::DeathMenuOffer;

        assert_eq!(
            prompt_instruction(Some(DeathMenuOffer::Raise)),
            "Accept Raise or Reraise?"
        );
        assert_eq!(
            prompt_instruction(Some(DeathMenuOffer::Tractor)),
            "Accept Tractor?"
        );
        assert_eq!(
            prompt_choices(Some(DeathMenuOffer::Raise), false),
            "  Yes\n> No"
        );
        assert_eq!(
            prompt_choices(Some(DeathMenuOffer::Raise), true),
            "> Yes\n  No"
        );
    }

    #[test]
    fn selection_resets_to_no_when_the_server_offer_changes() {
        use kuluu_snapshot::DeathMenuOffer;

        let mut selection = DeathPromptSelection::default();
        selection.sync(Some(DeathMenuOffer::Raise));
        assert!(!selection.accepts_offer());
        selection.toggle();
        assert!(selection.accepts_offer());

        selection.sync(Some(DeathMenuOffer::Raise));
        assert!(selection.accepts_offer());
        selection.sync(Some(DeathMenuOffer::Tractor));
        assert!(!selection.accepts_offer());
    }
}
