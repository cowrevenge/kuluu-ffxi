use bevy::prelude::*;

use crate::hud::style::{self, theme};
use crate::vana_time::{
    format_vana_time, VanaDate, VanaWeekday, EARTH_EPOCH_UNIX, EARTH_SECS_PER_VANA_DAY,
};

// Placeholder cell when no PlayerMapGrid is available: pre-load on native, and
// always on wasm, where crate::minimap (the grid's source) is compiled out
// (kuluu-ehye).
const GRID_CELL_UNKNOWN: &str = "(?-?)";

const FRAMES_GROUP: &str = "menu    frames  ";
const DAY_ORB_BASE_INDEX: usize = 106;
const ORB_SIZE_PX: f32 = 14.0;

#[derive(Resource, Debug, Clone, Copy)]
pub struct VanaClockVisible(pub bool);

impl Default for VanaClockVisible {
    fn default() -> Self {
        Self(true)
    }
}

#[derive(Component)]
pub struct VanaClockPanel;

#[derive(Component)]
pub struct VanaClockLabel;

#[derive(Component)]
pub struct VanaClockOrb;

pub fn spawn_vana_clock_as_child(p: &mut ChildSpawnerCommands) {
    p.spawn((
        VanaClockPanel,
        Node {
            flex_shrink: 0.0,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(theme::FRAME_BG),
        BorderColor::all(theme::FRAME_EDGE),
    ))
    .with_children(|p| {
        p.spawn((
            VanaClockOrb,
            Node {
                width: Val::Px(ORB_SIZE_PX),
                height: Val::Px(ORB_SIZE_PX),
                margin: UiRect::right(Val::Px(4.0)),
                display: Display::None,
                ..default()
            },
            ImageNode::new(Handle::default()),
        ));
        p.spawn((
            VanaClockLabel,
            Text::new("0:00   (?-?)"),
            style::text_font(12.0),
            TextColor(theme::TEXT),
        ));
    });
}

pub fn update_vana_clock(
    mut q: Query<&mut Text, With<VanaClockLabel>>,
    mut orb_q: Query<(&mut Node, &mut ImageNode), With<VanaClockOrb>>,
    #[cfg(not(target_arch = "wasm32"))] q_self: Query<&Transform, With<crate::components::IsSelf>>,
    #[cfg(not(target_arch = "wasm32"))] grid: Option<Res<crate::minimap::retail::PlayerMapGrid>>,
    atlas: Option<ResMut<crate::ui_element_atlas::UiElementAtlas>>,
    dat_root: Option<Res<crate::ui_element_atlas::UiElementDatRoot>>,
    mut images: ResMut<Assets<Image>>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
    vana_clock: Res<crate::vana_time::VanaClock>,
    mut prev_vana_day: Local<Option<u64>>,
) {
    let Ok(mut text) = q.single_mut() else {
        return;
    };

    let earth_now = vana_clock.earth_unix_secs_now();
    #[cfg(not(target_arch = "wasm32"))]
    let cell = player_grid_cell(grid.as_deref(), q_self.single().ok());
    #[cfg(target_arch = "wasm32")]
    let cell = GRID_CELL_UNKNOWN;
    let want = format!("{}   {}", format_vana_time(earth_now), cell);
    if **text != want {
        **text = want;
    }

    let earth_since_vana = earth_now.saturating_sub(EARTH_EPOCH_UNIX);
    let total_vana_days = earth_since_vana / EARTH_SECS_PER_VANA_DAY;
    if *prev_vana_day != Some(total_vana_days) {
        if let Some(prev) = *prev_vana_day {
            if prev != total_vana_days {
                let weekday = VanaWeekday::from_vana_day(total_vana_days).name();
                toasts.write(crate::snapshot::ToastEvent::system(format!(
                    "📅 Vana day {} — {}",
                    total_vana_days, weekday,
                )));
            }
        }
        update_day_orb(&mut orb_q, total_vana_days, atlas, dat_root, &mut images);
    }
    *prev_vana_day = Some(total_vana_days);
}

fn update_day_orb(
    orb_q: &mut Query<(&mut Node, &mut ImageNode), With<VanaClockOrb>>,
    total_vana_days: u64,
    atlas: Option<ResMut<crate::ui_element_atlas::UiElementAtlas>>,
    dat_root: Option<Res<crate::ui_element_atlas::UiElementDatRoot>>,
    images: &mut Assets<Image>,
) {
    let Ok((mut node, mut image_node)) = orb_q.single_mut() else {
        return;
    };
    let (Some(mut atlas), Some(dat_root)) = (atlas, dat_root) else {
        return;
    };
    let index = DAY_ORB_BASE_INDEX + VanaWeekday::from_vana_day(total_vana_days).element_index();
    match atlas.ensure(FRAMES_GROUP, index, &dat_root, images) {
        Some(handle) => {
            image_node.image = handle;
            node.display = Display::Flex;
        }
        None => {
            node.display = Display::None;
        }
    }
}

// Also keyed off Added<VanaClockPanel>: the panel is despawned with the
// BottomLeftStack's InGameEntity on zone change, and the respawned entity must
// pick up a hidden state whose resource change tick has already been consumed.
pub fn apply_vana_clock_visibility(
    visible: Res<VanaClockVisible>,
    added: Query<(), Added<VanaClockPanel>>,
    mut q: Query<&mut Node, With<VanaClockPanel>>,
) {
    if !visible.is_changed() && added.is_empty() {
        return;
    }
    let want = if visible.0 {
        Display::Flex
    } else {
        Display::None
    };
    for mut node in q.iter_mut() {
        if node.display != want {
            node.display = want;
        }
    }
}

// Line wording is provisional pending a retail capture of the Current Time
// menu output (bead kuluu-y5hq retail_unknowns); correct it here in one place.
pub const VANA_TIME_LINE_PREFIX: &str = "Vana'diel Time: ";
pub const EARTH_TIME_LINE_PREFIX: &str = "Earth Time: ";
const EARTH_TIME_FORMAT: &str = "%Y/%m/%d %H:%M:%S";

pub fn vana_time_chat_line(earth_unix_secs: u64) -> String {
    let date = VanaDate::from_earth_unix(earth_unix_secs);
    format!(
        "{VANA_TIME_LINE_PREFIX}{}, {}, {}/{}/{} C.E.",
        date.weekday.name(),
        format_vana_time(earth_unix_secs),
        date.day,
        date.month,
        date.year,
    )
}

pub fn earth_time_chat_line(earth_unix_secs: u64) -> String {
    format!(
        "{EARTH_TIME_LINE_PREFIX}{}",
        earth_time_text(&chrono::Local, earth_unix_secs)
    )
}

fn earth_time_text<Tz: chrono::TimeZone>(tz: &Tz, earth_unix_secs: u64) -> String
where
    Tz::Offset: std::fmt::Display,
{
    match tz.timestamp_opt(earth_unix_secs as i64, 0).single() {
        Some(dt) => dt.format(EARTH_TIME_FORMAT).to_string(),
        None => format!("unix {earth_unix_secs}"),
    }
}

pub fn current_time_chat_lines(clock: &crate::vana_time::VanaClock) -> [String; 2] {
    let earth = clock.earth_unix_secs_now();
    [vana_time_chat_line(earth), earth_time_chat_line(earth)]
}

#[cfg(not(target_arch = "wasm32"))]
fn player_grid_cell(
    grid: Option<&crate::minimap::retail::PlayerMapGrid>,
    player: Option<&Transform>,
) -> String {
    match (grid.and_then(|g| g.aabb), player) {
        (Some(aabb), Some(tf)) => {
            let (col, row) = aabb.world_to_grid(tf.translation);
            format!("({col}-{row})")
        }
        _ => GRID_CELL_UNKNOWN.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vana_time::{EARTH_SECS_PER_VANA_HOUR, VANA_DAYS_PER_MONTH};

    #[test]
    fn day_orb_index_maps_weekday_to_element_sprite() {
        // Firesday->Fire(106), Earthsday->Earth(109), Watersday->Water(111),
        // Windsday->Wind(108), Iceday->Ice(107), Lightningday->Lightning(110),
        // Lightsday->Light(112), Darksday->Dark(113). (Compass.kt:43-54)
        let expected = [106, 109, 111, 108, 107, 110, 112, 113];
        for (day, want) in expected.iter().enumerate() {
            let weekday = VanaWeekday::from_vana_day(day as u64);
            assert_eq!(DAY_ORB_BASE_INDEX + weekday.element_index(), *want);
        }
    }

    #[test]
    fn player_grid_cell_without_grid_matches_wasm_placeholder() {
        // The wasm build of update_vana_clock substitutes GRID_CELL_UNKNOWN
        // directly (crate::minimap is compiled out there, kuluu-ehye); this pins
        // the native no-grid fallback to the same string so the two targets
        // render identically before a map grid loads.
        assert_eq!(player_grid_cell(None, None), GRID_CELL_UNKNOWN);
    }

    #[test]
    fn vana_chat_line_snapshot() {
        let ts = EARTH_EPOCH_UNIX
            + VANA_DAYS_PER_MONTH * EARTH_SECS_PER_VANA_DAY
            + 13 * EARTH_SECS_PER_VANA_HOUR
            + 5 * EARTH_SECS_PER_VANA_HOUR / 60;
        assert_eq!(
            vana_time_chat_line(ts),
            "Vana'diel Time: Lightsday, 13:05, 1/2/886 C.E."
        );
        assert!(vana_time_chat_line(ts).starts_with(VANA_TIME_LINE_PREFIX));
    }

    #[test]
    fn earth_chat_line_formats_a_civil_datetime() {
        // The Vana'diel epoch is 2001-12-31 15:00:00 UTC (2002-01-01 00:00 JST,
        // vendor/server/src/common/earth_time.h:40).
        assert_eq!(
            earth_time_text(&chrono::Utc, EARTH_EPOCH_UNIX),
            "2001/12/31 15:00:00"
        );
        assert!(earth_time_chat_line(EARTH_EPOCH_UNIX).starts_with(EARTH_TIME_LINE_PREFIX));
    }

    #[test]
    fn current_time_lines_use_the_exported_prefixes() {
        let clock = crate::vana_time::VanaClock::anchored_at_hour(12.0);
        let [vana, earth] = current_time_chat_lines(&clock);
        assert!(vana.starts_with(VANA_TIME_LINE_PREFIX), "{vana:?}");
        assert!(earth.starts_with(EARTH_TIME_LINE_PREFIX), "{earth:?}");
    }

    #[test]
    fn visibility_apply_flips_display_and_covers_respawn() {
        let mut app = App::new();
        app.init_resource::<VanaClockVisible>();
        app.add_systems(Update, apply_vana_clock_visibility);
        let panel = app
            .world_mut()
            .spawn((VanaClockPanel, Node::default()))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Node>(panel).unwrap().display,
            Display::Flex
        );

        app.world_mut().resource_mut::<VanaClockVisible>().0 = false;
        app.update();
        assert_eq!(
            app.world().get::<Node>(panel).unwrap().display,
            Display::None
        );

        // A zone-change respawn arrives after the resource change tick was
        // consumed; the Added<VanaClockPanel> key must still hide it.
        let respawned = app
            .world_mut()
            .spawn((VanaClockPanel, Node::default()))
            .id();
        app.update();
        assert_eq!(
            app.world().get::<Node>(respawned).unwrap().display,
            Display::None
        );

        app.world_mut().resource_mut::<VanaClockVisible>().0 = true;
        app.update();
        assert_eq!(
            app.world().get::<Node>(panel).unwrap().display,
            Display::Flex
        );
        assert_eq!(
            app.world().get::<Node>(respawned).unwrap().display,
            Display::Flex
        );
    }
}
