use bevy::prelude::*;
use kuluu_snapshot::Weather;

use crate::hud::style::{self, theme};
use crate::snapshot::SceneState;
use crate::ui_element_atlas::{UiElementAtlas, UiElementDatRoot};

// Retail's weather indicator: "font    usgaiji " elements 0-7 are the eight
// element icons (textures elfire..eldark, ROM/119/51.DAT); the single weather
// of an element draws one icon, its double draws two
// (research/xim/src/jsMain/kotlin/xim/poc/ui/Compass.kt:69-96).
const USGAIJI_GROUP: &str = "font    usgaiji ";
const ICON_SIZE_PX: f32 = 14.0;
const MAX_ICONS: usize = 2;

#[derive(Component)]
pub struct WeatherIconPanel;

#[derive(Component)]
pub struct WeatherIconSlot(usize);

#[derive(Component)]
pub struct WeatherIconLabel;

pub fn weather_sprite(w: Weather) -> Option<(usize, usize)> {
    match w {
        Weather::None | Weather::Sunshine | Weather::Clouds | Weather::Fog => None,
        Weather::HotSpell => Some((0, 1)),
        Weather::HeatWave => Some((0, 2)),
        Weather::Snow => Some((1, 1)),
        Weather::Blizzards => Some((1, 2)),
        Weather::Wind => Some((2, 1)),
        Weather::Gales => Some((2, 2)),
        Weather::DustStorm => Some((3, 1)),
        Weather::SandStorm => Some((3, 2)),
        Weather::Thunder => Some((4, 1)),
        Weather::Thunderstorms => Some((4, 2)),
        Weather::Rain => Some((5, 1)),
        Weather::Squall => Some((5, 2)),
        Weather::Auroras => Some((6, 1)),
        Weather::StellarGlare => Some((6, 2)),
        Weather::Gloom => Some((7, 1)),
        Weather::Darkness => Some((7, 2)),
    }
}

pub fn weather_label(w: Weather) -> &'static str {
    match w {
        Weather::None => "",
        Weather::Sunshine => "Sunshine",
        Weather::Clouds => "Clouds",
        Weather::Fog => "Fog",
        Weather::HotSpell => "Hot Spell",
        Weather::HeatWave => "Heat Wave",
        Weather::Rain => "Rain",
        Weather::Squall => "Squall",
        Weather::DustStorm => "Dust Storm",
        Weather::SandStorm => "Sand Storm",
        Weather::Wind => "Wind",
        Weather::Gales => "Gales",
        Weather::Snow => "Snow",
        Weather::Blizzards => "Blizzards",
        Weather::Thunder => "Thunder",
        Weather::Thunderstorms => "Thunderstorms",
        Weather::Auroras => "Auroras",
        Weather::StellarGlare => "Stellar Glare",
        Weather::Gloom => "Gloom",
        Weather::Darkness => "Darkness",
    }
}

pub fn spawn_weather_icon_as_child(p: &mut ChildSpawnerCommands) {
    p.spawn((
        WeatherIconPanel,
        Node {
            flex_shrink: 0.0,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            border: UiRect::all(Val::Px(1.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            column_gap: Val::Px(3.0),
            display: Display::None,
            ..default()
        },
        BackgroundColor(theme::FRAME_BG),
        BorderColor::all(theme::FRAME_EDGE),
    ))
    .with_children(|p| {
        for slot in 0..MAX_ICONS {
            p.spawn((
                WeatherIconSlot(slot),
                Node {
                    width: Val::Px(ICON_SIZE_PX),
                    height: Val::Px(ICON_SIZE_PX),
                    display: Display::None,
                    ..default()
                },
                ImageNode::new(Handle::default()),
            ));
        }
        p.spawn((
            WeatherIconLabel,
            Text::new(""),
            style::text_font(14.0),
            TextColor(theme::TEXT),
        ));
    });
}

pub fn update_weather_icon(
    state: Res<SceneState>,
    mut panel_q: Query<&mut Node, (With<WeatherIconPanel>, Without<WeatherIconSlot>)>,
    mut slot_q: Query<(&WeatherIconSlot, &mut Node, &mut ImageNode)>,
    mut text_q: Query<&mut Text, With<WeatherIconLabel>>,
    mut atlas: ResMut<UiElementAtlas>,
    dat_root: Res<UiElementDatRoot>,
    mut images: ResMut<Assets<Image>>,
) {
    if !state.dirty {
        return;
    }
    let weather = state.snapshot.weather.unwrap_or(Weather::None);
    let label = weather_label(weather);

    let Ok(mut panel) = panel_q.single_mut() else {
        return;
    };
    let Ok(mut text) = text_q.single_mut() else {
        return;
    };

    if label.is_empty() {
        if panel.display != Display::None {
            panel.display = Display::None;
        }
        return;
    }

    if panel.display != Display::Flex {
        panel.display = Display::Flex;
    }
    if **text != label {
        **text = label.to_string();
    }

    let sprite = weather_sprite(weather);
    for (slot, mut node, mut image) in slot_q.iter_mut() {
        let handle = sprite
            .filter(|(_, count)| slot.0 < *count)
            .and_then(|(index, _)| atlas.ensure(USGAIJI_GROUP, index, &dat_root, &mut images));
        match handle {
            Some(handle) => {
                image.image = handle;
                node.display = Display::Flex;
            }
            None => {
                node.display = Display::None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fine_weather_family_has_no_icon() {
        for w in [
            Weather::None,
            Weather::Sunshine,
            Weather::Clouds,
            Weather::Fog,
        ] {
            assert_eq!(weather_sprite(w), None, "{w:?}");
        }
        assert_eq!(weather_label(Weather::None), "");
    }

    #[test]
    fn elemental_weathers_pair_single_and_double_on_one_index() {
        // research/xim/src/jsMain/kotlin/xim/poc/ui/Compass.kt:74-89
        let pairs = [
            (Weather::HotSpell, Weather::HeatWave, 0),
            (Weather::Snow, Weather::Blizzards, 1),
            (Weather::Wind, Weather::Gales, 2),
            (Weather::DustStorm, Weather::SandStorm, 3),
            (Weather::Thunder, Weather::Thunderstorms, 4),
            (Weather::Rain, Weather::Squall, 5),
            (Weather::Auroras, Weather::StellarGlare, 6),
            (Weather::Gloom, Weather::Darkness, 7),
        ];
        for (single, double, index) in pairs {
            assert_eq!(weather_sprite(single), Some((index, 1)), "{single:?}");
            assert_eq!(weather_sprite(double), Some((index, 2)), "{double:?}");
            assert!(!weather_label(single).is_empty(), "{single:?}");
            assert!(!weather_label(double).is_empty(), "{double:?}");
        }
    }

    #[test]
    fn icon_counts_fit_the_spawned_slots() {
        let all = [
            Weather::Sunshine,
            Weather::Clouds,
            Weather::Fog,
            Weather::HotSpell,
            Weather::HeatWave,
            Weather::Rain,
            Weather::Squall,
            Weather::DustStorm,
            Weather::SandStorm,
            Weather::Wind,
            Weather::Gales,
            Weather::Snow,
            Weather::Blizzards,
            Weather::Thunder,
            Weather::Thunderstorms,
            Weather::Auroras,
            Weather::StellarGlare,
            Weather::Gloom,
            Weather::Darkness,
        ];
        for w in all {
            if let Some((index, count)) = weather_sprite(w) {
                assert!(index < 8, "{w:?}");
                assert!((1..=MAX_ICONS).contains(&count), "{w:?}");
            }
        }
    }
}
