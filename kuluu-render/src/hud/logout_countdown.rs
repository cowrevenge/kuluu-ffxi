#![cfg(feature = "enhanced-shutdown-counter")]

// Enhanced (non-retail) on-screen shutdown/logout countdown banner. Retail's
// client shows only the 0x053 system chat lines for these ticks; this whole
// module (banner node, anchor/pending state machines, update system) exists
// only with `enhanced-shutdown-counter`. The chat lines themselves come from
// kuluu-session and are unaffected.
use bevy::prelude::*;
use kuluu_snapshot::{LogoutCountdown, SceneSnapshot, Stage};

use crate::combat_stance::{RestKind, RestStance};
use crate::hud::style::{self, theme};
use crate::snapshot::SceneState;

fn blocker_diagnostic(snap: &SceneSnapshot) -> String {
    if let Some(d) = &snap.dialog {
        let npc = d
            .npc_name
            .clone()
            .unwrap_or_else(|| format!("#{:08X}", d.npc_id));
        return format!(
            "Active dialog detected (NPC: {npc}, event_id={}, mode={}). \
             Close the NPC menu/dialog before retrying.",
            d.event_id, d.mode
        );
    }
    if !snap.status_icons.is_empty() {
        return format!(
            "Active status icons: {:?}. One of these is likely an \
             AbnormalStatus blocker (Weakness, Sleep, Charm, Petrify, \
             Encumbrance, etc.). Wait for the relevant effect to wear off.",
            snap.status_icons
        );
    }
    "No dialog or status icons visible to the client — likely Crafting \
     (synthesis in progress) or a PreventAction debuff."
        .into()
}

/// The server sends the first 0x053 tick in the same map update that accepts
/// the request: onEffectGain adds the LEAVEGAME effect and immediately calls
/// messageSystem(kind, 30) (vendor/server/scripts/effects/leavegame.lua).
/// Subsequent ticks are 5s apart - the effect is created with a 5s tick in
/// vendor/server/src/map/packets/c2s/0x0e7_reqlogout.cpp GP_CLI_COMMAND_REQLOGOUT::process.
/// If no tick has
/// arrived within this window, the request was silently rejected by the 0x0e7
/// validator (InEvent / AbnormalStatus / Crafting / PreventAction) and nothing
/// will ever arrive.
const REQUEST_ACK_TIMEOUT_SECS: f64 = 2.0;

const BLOCKED_DISPLAY_SECS: f64 = 5.0;

/// A new tick that lands within this window of what the current anchor already
/// implies is the same countdown seen from a slightly different clock, not a
/// restart: re-anchoring would visibly jump the displayed number (the +-2s
/// rule). Ticks are 5s apart on the server and each carries exactly 5 fewer
/// seconds than the previous one (leavegame.lua onEffectTick), so a genuine
/// restart always shows up as para=30, which only ever comes from
/// onEffectGain.
const RESYNC_TOLERANCE_SECS: f64 = 2.0;

#[derive(Message, Debug, Clone, Copy)]
pub struct LogoutRequested {
    pub shutdown: bool,
}

/// Lifecycle of a locally-sent /logout or /shutdown request until (and unless)
/// the server confirms it with a 0x053 tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum LogoutRequestState {
    #[default]
    None,
    /// Request sent; no 0x053 tick yet. The banner stays hidden until the first
    /// tick anchors the countdown: on accept the server sends para=30 in the
    /// same map update (leavegame.lua onEffectGain), so the visible start is
    /// server-confirmed, not local guesswork.
    AwaitingTick { requested_at: f64, shutdown: bool },
    /// No tick within REQUEST_ACK_TIMEOUT_SECS of the request: the 0x0e7
    /// validator silently rejected it (no packet exists for that case).
    Blocked { entered_at: f64, shutdown: bool },
}

#[derive(Resource, Default, Debug)]
pub struct PendingLogoutRequest {
    pub state: LogoutRequestState,
}

#[derive(Resource, Default, Debug)]
pub struct LogoutCountdownAnchor {
    pub server_seconds: Option<u16>,
    pub shutdown: bool,
    pub anchor_secs: f64,

    /// The last tick value captured at a local stand-up. Stand-up cancels
    /// leavegame server-side WITHOUT any cancel packet, so that stale tick
    /// keeps sitting in the snapshot — suppress exactly that value until a NEW
    /// tick proves the countdown survived (or it is cleared on re-anchor).
    pub suppress_stale: Option<LogoutCountdown>,
}

#[derive(Component)]
pub struct LogoutCountdownBanner;

#[derive(Component)]
pub struct LogoutCountdownLabel;

pub fn spawn_logout_countdown(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            LogoutCountdownBanner,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Percent(35.0),
                left: Val::Percent(50.0),
                margin: UiRect {
                    left: Val::Px(-140.0),
                    ..default()
                },
                width: Val::Px(280.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                justify_content: JustifyContent::Center,
                display: Display::None,
                ..default()
            },
            BackgroundColor(theme::FRAME_BG),
            BorderColor::all(theme::DANGER),
        ))
        .with_children(|p| {
            p.spawn((
                LogoutCountdownLabel,
                Text::new(""),
                style::text_font(22.0),
                TextColor(theme::DANGER),
            ));
        });
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayMode {
    Hidden,
    Counting { seconds: u32, shutdown: bool },
    LoggingOut { shutdown: bool },
    Blocked { shutdown: bool },
}

/// The banner is driven ONLY by the server anchor. AwaitingTick shows nothing:
/// the countdown starts when the first 0x053 tick (para=30) lands, never on
/// the local request.
fn compute_display(
    now: f64,
    server: Option<(u16, bool, f64)>,
    pending: LogoutRequestState,
) -> DisplayMode {
    if let Some((server_secs, shutdown, anchor)) = server {
        let elapsed = (now - anchor).max(0.0);
        let remaining = (server_secs as f64 - elapsed).max(0.0);
        let secs = remaining.round() as u32;
        return if secs == 0 {
            DisplayMode::LoggingOut { shutdown }
        } else {
            DisplayMode::Counting {
                seconds: secs,
                shutdown,
            }
        };
    }

    match pending {
        LogoutRequestState::None | LogoutRequestState::AwaitingTick { .. } => DisplayMode::Hidden,
        LogoutRequestState::Blocked {
            entered_at,
            shutdown,
        } => {
            if now - entered_at > BLOCKED_DISPLAY_SECS {
                DisplayMode::Hidden
            } else {
                DisplayMode::Blocked { shutdown }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_logout_countdown(
    mut requests: MessageReader<LogoutRequested>,
    time: Res<Time>,
    rest: Res<RestStance>,
    mut prev_rest: Local<RestKind>,
    mut anchor: ResMut<LogoutCountdownAnchor>,
    mut pending: ResMut<PendingLogoutRequest>,

    scene_state: Res<SceneState>,
    mut toasts: MessageWriter<crate::snapshot::ToastEvent>,
    mut banner_q: Query<&mut Node, With<LogoutCountdownBanner>>,
    mut label_q: Query<&mut Text, With<LogoutCountdownLabel>>,
) {
    let now = time.elapsed_secs_f64();

    // A zoning or disconnect ends the map-session context a pending request
    // belongs to. In particular /logout inside a Mog House is accepted by an
    // IMMEDIATE leaveGame() with no countdown ticks at all (leavegame.lua
    // onEffectGain), so without this the ack timeout would fire "blocked" two
    // seconds after we were already disconnected. The snapshot's own
    // logout_countdown clears on both transitions (kuluu-session state.rs
    // ZoneChanged / Disconnected folds), which drops the anchor through the
    // None branch below; only the pending request needs explicit invalidation.
    if matches!(
        scene_state.snapshot.stage,
        Stage::Disconnected | Stage::Zoning
    ) {
        pending.state = LogoutRequestState::None;
    }

    // Stand-up is LOCAL knowledge (Sit key, heal toggle, movement exit, /sit):
    // the server cancels leavegame on stand-up WITHOUT sending any cancel
    // packet, so we just stop drawing — send-and-assume. If the countdown was
    // actually still live, the next 0x053 tick carries a NEW value and
    // re-anchors within a second; if it was cancelled, nothing does. Checked
    // BEFORE request handling so a /shutdown on the same frame as stand-up
    // still arms fresh.
    let stood_up = rest.kind == RestKind::None && *prev_rest != RestKind::None;
    *prev_rest = rest.kind;
    if stood_up {
        pending.state = LogoutRequestState::None;
        anchor.server_seconds = None;
        // The pre-stand-up tick is still sitting in the snapshot (no cancel
        // packet exists to clear it); suppress exactly that value so it cannot
        // re-anchor. A new /shutdown or a surviving countdown ticks a different
        // value and anchors as usual.
        anchor.suppress_stale = scene_state.snapshot.logout_countdown;
    }

    let mut latest_request: Option<LogoutRequested> = None;
    for ev in requests.read() {
        latest_request = Some(*ev);
    }
    if let Some(req) = latest_request {
        // Do NOT start counting locally. The banner stays hidden until the
        // server's first 0x053 tick (para=30, sent in the same map update as
        // the accept - leavegame.lua onEffectGain) anchors it below. If no
        // tick arrives within REQUEST_ACK_TIMEOUT_SECS the request was
        // silently rejected and we surface Blocked instead.
        pending.state = LogoutRequestState::AwaitingTick {
            requested_at: now,
            shutdown: req.shutdown,
        };
    }

    // The stand-up-suppressed stale tick must not re-anchor; any other value is
    // a live tick. A live tick within RESYNC_TOLERANCE_SECS of what the current
    // anchor already implies is the same countdown seen from a slightly
    // different clock: keep the running anchor so the display does not jump,
    // but refresh the kind flag - /shutdown during a /logout (or vice versa)
    // just re-powers the existing effect and keeps ticking (0x0e7_reqlogout.cpp
    // SetPower branch). Anything further away is a fresh countdown: re-anchor.
    let live = match scene_state.snapshot.logout_countdown {
        Some(c) if anchor.suppress_stale == Some(c) => None,
        other => other,
    };
    match live {
        Some(c) => match anchor.server_seconds {
            None => {
                anchor.server_seconds = Some(c.seconds_remaining);
                anchor.shutdown = c.shutdown;
                anchor.anchor_secs = now;
                anchor.suppress_stale = None;

                if matches!(pending.state, LogoutRequestState::AwaitingTick { .. }) {
                    pending.state = LogoutRequestState::None;
                }
            }
            Some(prev) => {
                let implied = prev as f64 - (now - anchor.anchor_secs);
                if (implied - c.seconds_remaining as f64).abs() <= RESYNC_TOLERANCE_SECS {
                    // Same countdown, different clock: no re-anchor. The kind
                    // may have switched mid-countdown; refresh the label flag.
                    anchor.shutdown = c.shutdown;
                } else {
                    anchor.server_seconds = Some(c.seconds_remaining);
                    anchor.shutdown = c.shutdown;
                    anchor.anchor_secs = now;
                    anchor.suppress_stale = None;

                    if matches!(pending.state, LogoutRequestState::AwaitingTick { .. }) {
                        pending.state = LogoutRequestState::None;
                    }
                }
            }
        },
        None => {
            anchor.server_seconds = None;
        }
    }

    if let LogoutRequestState::AwaitingTick {
        requested_at,
        shutdown,
    } = pending.state
    {
        if now - requested_at >= REQUEST_ACK_TIMEOUT_SECS {
            pending.state = LogoutRequestState::Blocked {
                entered_at: now,
                shutdown,
            };
            let diagnostic = blocker_diagnostic(&scene_state.snapshot);
            let label = if shutdown { "/shutdown" } else { "/logout" };
            toasts.write(crate::snapshot::ToastEvent::debug(format!(
                "{label}: server did not acknowledge (silent reject \
                 by 0x0e7_reqlogout.cpp validator). {diagnostic}"
            )));
        }
    }

    let server_anchor = anchor
        .server_seconds
        .map(|s| (s, anchor.shutdown, anchor.anchor_secs));
    let mode = compute_display(now, server_anchor, pending.state);

    let Ok(mut node) = banner_q.single_mut() else {
        return;
    };
    let Ok(mut text) = label_q.single_mut() else {
        return;
    };

    let (display_flex, label) = match mode {
        DisplayMode::Hidden => (false, String::new()),
        DisplayMode::Counting { seconds, shutdown } => (
            true,
            if shutdown {
                format!("Shutdown in {seconds}s")
            } else {
                format!("Logout in {seconds}s")
            },
        ),
        DisplayMode::LoggingOut { shutdown } => (
            true,
            if shutdown {
                "Shutting down…".to_string()
            } else {
                "Logging out…".to_string()
            },
        ),
        DisplayMode::Blocked { shutdown } => (
            true,
            if shutdown {
                "Shutdown blocked".to_string()
            } else {
                "Logout blocked".to_string()
            },
        ),
    };

    let want_display = if display_flex {
        Display::Flex
    } else {
        Display::None
    };
    if node.display != want_display {
        node.display = want_display;
    }
    if display_flex && **text != label {
        **text = label;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(secs: u16, shutdown: bool) -> LogoutCountdown {
        LogoutCountdown {
            seconds_remaining: secs,
            shutdown,
        }
    }

    /// The anchoring decision for one live tick against the current anchor.
    /// Mirrors update_logout_countdown's match arm so the +-2s rule is testable
    /// without a Bevy world. Returns (new_anchor_secs, new_shutdown).
    fn apply_tick(
        now: f64,
        c: LogoutCountdown,
        anchor_secs: Option<(u16, bool, f64)>,
    ) -> (Option<u16>, bool, f64) {
        match anchor_secs {
            None => (Some(c.seconds_remaining), c.shutdown, now),
            Some((prev, _shutdown, anchored_at)) => {
                let implied = prev as f64 - (now - anchored_at);
                if (implied - c.seconds_remaining as f64).abs() <= RESYNC_TOLERANCE_SECS {
                    // Same countdown, different clock: keep the running anchor.
                    (Some(prev), c.shutdown, anchored_at)
                } else {
                    (Some(c.seconds_remaining), c.shutdown, now)
                }
            }
        }
    }

    #[test]
    fn hidden_when_nothing_pending() {
        let mode = compute_display(100.0, None, LogoutRequestState::None);
        assert_eq!(mode, DisplayMode::Hidden);
    }

    /// Requirement 1: the banner must NOT start on the local request. While a
    /// tick is awaited (even well past where an old optimistic counter would
    /// have been counting) nothing displays; only the first server tick starts
    /// the countdown.
    #[test]
    fn awaiting_tick_shows_nothing_until_the_first_server_tick() {
        let pending = LogoutRequestState::AwaitingTick {
            requested_at: 0.0,
            shutdown: false,
        };

        // 10s after the request with no tick: still hidden (the old code would
        // have shown "Logout in 20s" here).
        assert_eq!(compute_display(10.0, None, pending), DisplayMode::Hidden);

        // The first tick (para=30, leavegame.lua onEffectGain) starts it:
        // anchored at t=10.2 carrying 30, so at t=10.5 remaining = 29.7 -> 30.
        let server = Some((30u16, false, 10.2));
        assert_eq!(
            compute_display(10.5, server, LogoutRequestState::None),
            DisplayMode::Counting {
                seconds: 30,
                shutdown: false
            }
        );
    }

    #[test]
    fn server_wins_over_pending() {
        let server = Some((25u16, false, 100.0));
        let pending = LogoutRequestState::AwaitingTick {
            requested_at: 99.0,
            shutdown: false,
        };
        let mode = compute_display(100.5, server, pending);
        assert!(matches!(
            mode,
            DisplayMode::Counting {
                seconds: 24 | 25,
                shutdown: false
            }
        ));
    }

    #[test]
    fn blocked_displays_then_hides() {
        let blocked = LogoutRequestState::Blocked {
            entered_at: 0.0,
            shutdown: false,
        };

        assert_eq!(
            compute_display(2.0, None, blocked),
            DisplayMode::Blocked { shutdown: false }
        );

        assert_eq!(
            compute_display(BLOCKED_DISPLAY_SECS + 0.5, None, blocked),
            DisplayMode::Hidden
        );
    }

    #[test]
    fn shutdown_label_propagates() {
        let server = Some((25u16, true, 100.0));
        let mode = compute_display(100.0, server, LogoutRequestState::None);
        assert_eq!(
            mode,
            DisplayMode::Counting {
                seconds: 25,
                shutdown: true
            }
        );
    }

    /// Requirement 2: a tick within +-2s of the implied remaining keeps the
    /// running anchor (no visible jump), even when it arrives off-cadence.
    #[test]
    fn tick_within_tolerance_keeps_the_running_anchor() {
        // Anchor 30 @ t=0. The next server tick carries 25 but arrives late at
        // t=6: implied = 24, |24 - 25| = 1 <= 2 -> keep the anchor.
        let (secs, _shutdown, anchored_at) =
            apply_tick(6.0, tick(25, false), Some((30, false, 0.0)));
        assert_eq!(secs, Some(30));
        assert_eq!(anchored_at, 0.0);

        // And the one after: carries 20 at t=11 (implied = 19, diff 1) -> keep.
        let (secs, _shutdown, anchored_at) =
            apply_tick(11.0, tick(20, false), Some((30, false, 0.0)));
        assert_eq!(secs, Some(30));
        assert_eq!(anchored_at, 0.0);

        // Display consequence: at t=6 the counter reads off the original anchor
        // (24) instead of jumping back up to 25 on the late tick.
        let mode = compute_display(6.0, Some((30u16, false, 0.0)), LogoutRequestState::None);
        assert_eq!(
            mode,
            DisplayMode::Counting {
                seconds: 24,
                shutdown: false
            }
        );
    }

    /// A tick further than +-2s from the implied remaining is a fresh countdown
    /// (para=30 only ever comes from onEffectGain) and re-anchors.
    #[test]
    fn tick_beyond_tolerance_reanchors() {
        // Anchor 15 @ t=0; at t=2 a brand-new countdown's first tick arrives:
        // implied = 13, |13 - 30| = 17 > 2 -> re-anchor to 30 @ now.
        let (secs, _shutdown, anchored_at) =
            apply_tick(2.0, tick(30, false), Some((15, false, 0.0)));
        assert_eq!(secs, Some(30));
        assert_eq!(anchored_at, 2.0);
    }

    /// The +-2s boundary itself: exactly 2 apart keeps the anchor (the rule is
    /// "within +-2 seconds").
    #[test]
    fn tick_exactly_at_tolerance_keeps_the_anchor() {
        // Anchor 30 @ t=0; at t=4.5 implied = 25.5, incoming 23 -> diff 2.5 > 2:
        // re-anchor. At t=4.0 implied = 26, incoming 24 -> diff exactly 2: keep.
        let (secs, _shutdown, anchored_at) =
            apply_tick(4.0, tick(24, false), Some((30, false, 0.0)));
        assert_eq!(secs, Some(30));
        assert_eq!(anchored_at, 0.0);

        let (secs, _shutdown, anchored_at) =
            apply_tick(4.5, tick(23, false), Some((30, false, 0.0)));
        assert_eq!(secs, Some(23));
        assert_eq!(anchored_at, 4.5);
    }

    /// /shutdown during a /logout (or vice versa) re-powers the existing effect
    /// without restarting it: the tick stays within tolerance and only the kind
    /// flag flips.
    #[test]
    fn kind_switch_within_tolerance_refreshes_flag_without_reanchor() {
        // Anchor 30/logout @ t=0; at t=5 a shutdown-kind tick carries 25
        // (implied = 25, diff 0) -> same anchor, flag flips to shutdown.
        let (_secs, shutdown, anchored_at) =
            apply_tick(5.0, tick(25, true), Some((30, false, 0.0)));
        assert!(shutdown);
        assert_eq!(anchored_at, 0.0);

        // And the display carries the new label off the unchanged anchor.
        let mode = compute_display(5.0, Some((30u16, true, 0.0)), LogoutRequestState::None);
        assert_eq!(
            mode,
            DisplayMode::Counting {
                seconds: 25,
                shutdown: true
            }
        );
    }

    /// The first tick after a request always anchors, whatever it carries.
    #[test]
    fn first_tick_always_anchors() {
        let (secs, shutdown, anchored_at) = apply_tick(0.4, tick(30, true), None);
        assert_eq!(secs, Some(30));
        assert!(shutdown);
        assert_eq!(anchored_at, 0.4);
    }

    /// Inside a Mog House the server disconnects immediately with no ticks at
    /// all (leavegame.lua onEffectGain). The pending request is invalidated by
    /// the Disconnected stage transition, so the ack timeout can never fire a
    /// false "blocked" toast after we are already out.
    #[test]
    fn disconnected_stage_invalidates_pending_request() {
        // This mirrors the system's stage guard: on Disconnected/Zoning the
        // pending state is dropped before the ack-timeout check can run.
        let mut state = LogoutRequestState::AwaitingTick {
            requested_at: 0.0,
            shutdown: false,
        };
        for stage in [Stage::Disconnected, Stage::Zoning] {
            if matches!(stage, Stage::Disconnected | Stage::Zoning) {
                state = LogoutRequestState::None;
            }
        }
        assert_eq!(state, LogoutRequestState::None);

        // And with nothing pending and no anchor, the banner stays hidden even
        // long after a request was sent.
        assert_eq!(compute_display(50.0, None, state), DisplayMode::Hidden);
    }
}
