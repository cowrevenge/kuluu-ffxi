use super::*;

/// UV moved per arrow press by the Markers placement crosshair.
const MAP_CURSOR_STEP_UV: f32 = 0.02;

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_map_key(
    key: &Key,
    key_code: KeyCode,
    bindings: &Bindings,
    stack: &mut MenuStack,
    scene_state: &mut SceneState,
    cmd_tx: &Sender<AgentCommand>,
    map_state: &mut kuluu_render::hud::map_screen::MapScreenState,
    map_markers: Mut<kuluu_render::hud::map_screen::MapMarkers>,
    map_view: &kuluu_render::hud::map_screen::MapView,
    minimap_state: &kuluu_render::minimap::MinimapState,
) -> Option<InputMode> {
    use kuluu_render::hud::map_screen::{
        change_map_targets, widescan_rows, MapSubMode, COMMAND_ROWS,
    };

    // Zoom the map with the camera-zoom keys in any submode, except while a
    // marker label is being typed: the default binds are Period/Comma, which
    // the label needs as literal '.'/',' text. Matched on the raw keycode
    // because the logical-key path never resolves symbol keys (kuluu-kzxp).
    // The Map keeps its own zoom, independent of the minimap (kuluu-bi1s.3).
    if map_state.marker_entry.is_none() {
        let zone_half =
            kuluu_render::minimap::zone_half_span(minimap_state.retail_aabb.or(minimap_state.aabb));
        if bindings.matches_keycode(Action::CameraZoomIn, key_code) {
            map_state.zoom_by(1.0 / kuluu_render::minimap::ZOOM_STEP_FACTOR, zone_half);
            return None;
        }
        if bindings.matches_keycode(Action::CameraZoomOut, key_code) {
            map_state.zoom_by(kuluu_render::minimap::ZOOM_STEP_FACTOR, zone_half);
            return None;
        }
    }

    match map_state.mode {
        MapSubMode::Command => {
            if let Some(dir) = nav_dir(bindings, key) {
                map_state.cursor = wrap_cursor(map_state.cursor, COMMAND_ROWS.len(), dir);
                return None;
            }
            if bindings.matches_logical(Action::NavConfirm, key) {
                if let Some((_, sub)) = COMMAND_ROWS.get(map_state.cursor) {
                    enter_submode(*sub, map_state, cmd_tx);
                }
                return None;
            }
            if bindings.matches_logical(Action::NavCancel, key) {
                return close_map_screen(stack, map_state, cmd_tx);
            }
            None
        }
        MapSubMode::WideScan => {
            let rows = widescan_rows(&scene_state.snapshot);
            if let Some(dir) = nav_dir(bindings, key) {
                map_state.cursor = wrap_cursor(map_state.cursor, rows.len(), dir);
                return None;
            }
            if bindings.matches_logical(Action::NavConfirm, key) {
                if let Some(row) = rows.get(map_state.cursor) {
                    if let Err(e) = cmd_tx.try_send(AgentCommand::WidescanTrack {
                        act_index: row.act_index,
                    }) {
                        push_system_chat_line(
                            scene_state,
                            format!("[widescan] track dropped: {e}"),
                        );
                    }
                }
                return None;
            }
            if bindings.matches_logical(Action::NavCancel, key) {
                let _ = cmd_tx.try_send(AgentCommand::WidescanEnd);
                return_to_command(map_state);
                None
            } else {
                None
            }
        }
        MapSubMode::Markers => handle_markers_key(
            key,
            bindings,
            scene_state,
            map_state,
            map_markers,
            map_view.visible_aabb,
        ),
        MapSubMode::ChangeMap => {
            let targets = change_map_targets(map_state, &scene_state.snapshot);
            if let Some(dir) = nav_dir(bindings, key) {
                map_state.cursor = wrap_cursor(map_state.cursor, targets.len(), dir);
                return None;
            }
            if bindings.matches_logical(Action::NavConfirm, key) {
                if let Some(&(zone, idx)) = targets.get(map_state.cursor) {
                    map_state.viewed = Some((zone, idx));
                    map_state.cursor = 0;
                }
                return None;
            }
            if bindings.matches_logical(Action::NavCancel, key) {
                map_state.viewed = None;
                return_to_command(map_state);
                None
            } else {
                None
            }
        }
    }
}

/// Markers submode: a placement crosshair (arrow keys), Confirm to name+place a
/// marker (text entry), and the placed-marker list (delete with Confirm on a row
/// once naming is done). Cancel exits text entry, then the submode.
fn handle_markers_key(
    key: &Key,
    bindings: &Bindings,
    scene_state: &mut SceneState,
    map_state: &mut kuluu_render::hud::map_screen::MapScreenState,
    // `Mut` so change detection (which triggers the markers.json rewrite in
    // marker_store::sync_markers) fires only on the branch that actually
    // mutates a marker, not on every keypress routed through here (kuluu-df0x).
    mut map_markers: Mut<kuluu_render::hud::map_screen::MapMarkers>,
    visible_aabb: Option<kuluu_render::minimap::MinimapAabb>,
) -> Option<InputMode> {
    use kuluu_render::hud::map_screen::MapMarker;

    // Text-entry step: naming the marker being placed at the crosshair.
    if let Some(entry) = map_state.marker_entry.as_mut() {
        if bindings.matches_logical(Action::ChatSubmit, key)
            || bindings.matches_logical(Action::NavConfirm, key)
        {
            let label = std::mem::take(entry);
            map_state.marker_entry = None;
            let label = if label.trim().is_empty() {
                "Marker".to_string()
            } else {
                label.trim().to_string()
            };
            if let (Some(zone), Some(uv), Some(aabb)) = (
                scene_state.snapshot.zone_id,
                map_state.map_cursor,
                visible_aabb,
            ) {
                let xz = aabb.uv_to_world(uv);
                map_markers
                    .by_zone
                    .entry(zone)
                    .or_default()
                    .push(MapMarker {
                        world: kuluu_snapshot::Vec3 {
                            x: xz.x,
                            y: 0.0,
                            z: xz.y,
                        },
                        label,
                    });
            }
            return None;
        }
        if bindings.matches_logical(Action::ChatExit, key)
            || bindings.matches_logical(Action::NavCancel, key)
        {
            map_state.marker_entry = None;
            return None;
        }
        if bindings.matches_logical(Action::ChatBackspace, key) {
            entry.pop();
            return None;
        }
        match key {
            Key::Space => entry.push(' '),
            Key::Character(s) => {
                for c in s.chars() {
                    if !c.is_control() {
                        entry.push(c);
                    }
                }
            }
            _ => {}
        }
        return None;
    }

    // Crosshair movement across the map (UV space, clamped).
    let cursor = map_state
        .map_cursor
        .get_or_insert(bevy::math::Vec2::splat(0.5));
    let delta = if bindings.matches_logical(Action::NavLeft, key) {
        Some(bevy::math::Vec2::new(-MAP_CURSOR_STEP_UV, 0.0))
    } else if bindings.matches_logical(Action::NavRight, key) {
        Some(bevy::math::Vec2::new(MAP_CURSOR_STEP_UV, 0.0))
    } else if bindings.matches_logical(Action::NavUp, key) {
        Some(bevy::math::Vec2::new(0.0, -MAP_CURSOR_STEP_UV))
    } else if bindings.matches_logical(Action::NavDown, key) {
        Some(bevy::math::Vec2::new(0.0, MAP_CURSOR_STEP_UV))
    } else {
        None
    };
    if let Some(d) = delta {
        *cursor = (*cursor + d).clamp(bevy::math::Vec2::ZERO, bevy::math::Vec2::ONE);
        return None;
    }
    if bindings.matches_logical(Action::NavConfirm, key) {
        map_state.marker_entry = Some(String::new());
        return None;
    }
    if bindings.matches_logical(Action::NavCancel, key) {
        map_state.map_cursor = None;
        return_to_command(map_state);
    }
    None
}

/// A single up/down navigation step, or `None` if the key isn't up/down.
fn nav_dir(bindings: &Bindings, key: &Key) -> Option<i32> {
    if bindings.matches_logical(Action::NavUp, key) {
        Some(-1)
    } else if bindings.matches_logical(Action::NavDown, key) {
        Some(1)
    } else {
        None
    }
}

/// Move a wrapping list cursor by `dir` (±1); empty lists park at 0.
fn wrap_cursor(cursor: usize, len: usize, dir: i32) -> usize {
    if len == 0 {
        return 0;
    }
    let n = len as i32;
    (((cursor as i32 + dir) % n + n) % n) as usize
}

/// Drill from the command submenu into a submode, initializing its cursor and
/// firing the wide-scan request (0x0F4) only when Wide Scan is chosen.
fn enter_submode(
    sub: kuluu_render::hud::map_screen::MapSubMode,
    map_state: &mut kuluu_render::hud::map_screen::MapScreenState,
    cmd_tx: &Sender<AgentCommand>,
) {
    use kuluu_render::hud::map_screen::MapSubMode;
    map_state.mode = sub;
    map_state.cursor = 0;
    match sub {
        MapSubMode::WideScan => {
            let _ = cmd_tx.try_send(AgentCommand::WidescanRequest);
        }
        MapSubMode::Markers => {
            map_state.map_cursor = Some(bevy::math::Vec2::splat(0.5));
        }
        _ => {}
    }
}

/// Return from a submode to the command submenu, restoring the command cursor.
fn return_to_command(map_state: &mut kuluu_render::hud::map_screen::MapScreenState) {
    map_state.mode = kuluu_render::hud::map_screen::MapSubMode::Command;
    map_state.cursor = 0;
    map_state.map_cursor = None;
    map_state.marker_entry = None;
}

/// Close the Map screen: stop tracking (0x0F6), reset the submode, and pop back
/// to the menu it opened from, or to the world if it was the only level.
fn close_map_screen(
    stack: &mut MenuStack,
    map_state: &mut kuluu_render::hud::map_screen::MapScreenState,
    cmd_tx: &Sender<AgentCommand>,
) -> Option<InputMode> {
    let _ = cmd_tx.try_send(AgentCommand::WidescanEnd);
    map_state.reset();
    if stack.pop() {
        None
    } else {
        Some(InputMode::World)
    }
}
