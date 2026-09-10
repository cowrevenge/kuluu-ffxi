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

    /// The last snapshot value the anchor logic has processed. state.rs
    /// level-holds that value until a new 0x053 tick arrives, so only a change
    /// against it is a new tick; re-running the resync math on an already
    /// processed value would re-anchor it once the implied remaining drifts
    /// RESYNC_TOLERANCE_SECS past it (the oscillation).
    pub last_seen_tick: Option<u16>,

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

/// One frame of anchor decision for the logout/shutdown countdown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnchorStep {
    /// The new (server_seconds, anchor_secs) pair; None clears the countdown.
    pub server: Option<(u16, f64)>,
    /// The kind flag to store alongside it.
    pub shutdown: bool,
    /// True when a live tick anchored this frame (first anchor or re-anchor):
    /// the caller spends suppress_stale and resolves any AwaitingTick request.
    pub anchored_tick: bool,
}

/// One frame of anchor decision, extracted so the system and the tests run the
/// same code. `snapshot` is the value kuluu-session holds (state.rs level-holds
/// the last 0x053 tick until a new one arrives), `suppress_stale` is the
/// stand-up-suppressed value, `last_seen_tick` is the last snapshot value this
/// logic processed, and `server`/`anchor_secs` are the current anchor.
fn step_anchor(
    snapshot: Option<LogoutCountdown>,
    suppress_stale: Option<LogoutCountdown>,
    server: Option<u16>,
    shutdown: bool,
    last_seen_tick: Option<u16>,
    anchor_secs: f64,
    now: f64,
) -> AnchorStep {
    // The stand-up-suppressed stale tick must not re-anchor.
    let live = match snapshot {
        Some(c) if suppress_stale == Some(c) => None,
        other => other,
    };
    match (live, server) {
        // No value this frame: clear the anchor and forget what was seen (the
        // ZoneChanged / Disconnected folds in state.rs drop it).
        (None, _) => AnchorStep {
            server: None,
            shutdown,
            anchored_tick: false,
        },
        // Value-changed guard: state.rs level-holds the last tick between the
        // 5s server ticks, so a value that was already processed is not a new
        // tick. This must compare against last_seen_tick, NOT server_seconds:
        // a within-tolerance tick keeps the OLD anchor, so for the next 5s the
        // held value differs from server_seconds and comparing them would run
        // the resync math every frame, re-anchoring once the implied remaining
        // drifts RESYNC_TOLERANCE_SECS past the held value (the oscillation).
        (Some(c), _) if last_seen_tick == Some(c.seconds_remaining) => AnchorStep {
            server: server.map(|prev| (prev, anchor_secs)),
            shutdown,
            anchored_tick: false,
        },
        // First live tick: always anchors.
        (Some(c), None) => AnchorStep {
            server: Some((c.seconds_remaining, now)),
            shutdown: c.shutdown,
            anchored_tick: true,
        },
        // A new value against a live anchor: the +-2s resync rule.
        (Some(c), Some(prev)) => {
            let implied = prev as f64 - (now - anchor_secs);
            if (implied - c.seconds_remaining as f64).abs() <= RESYNC_TOLERANCE_SECS {
                // Same countdown, different clock: keep the running anchor. The
                // kind may have switched mid-countdown; refresh the label flag.
                AnchorStep {
                    server: Some((prev, anchor_secs)),
                    shutdown: c.shutdown,
                    anchored_tick: false,
                }
            } else {
                AnchorStep {
                    server: Some((c.seconds_remaining, now)),
                    shutdown: c.shutdown,
                    anchored_tick: true,
                }
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

    // One frame of anchor logic, shared verbatim with the tests (step_anchor):
    // a stand-up-suppressed stale tick never re-anchors; a value that
    // state.rs is still level-holding from the last 0x053 tick keeps the
    // running anchor untouched; only a CHANGED value runs the +-2s resync rule,
    // where a live tick within RESYNC_TOLERANCE_SECS of what the current anchor
    // already implies is the same countdown seen from a slightly different
    // clock (keep the running anchor so the display does not jump, but refresh
    // the kind flag - /shutdown during a /logout just re-powers the existing
    // effect and keeps ticking, 0x0e7_reqlogout.cpp SetPower branch) and
    // anything further away is a fresh countdown (re-anchor).
    let step = step_anchor(
        scene_state.snapshot.logout_countdown,
        anchor.suppress_stale,
        anchor.server_seconds,
        anchor.shutdown,
        anchor.last_seen_tick,
        anchor.anchor_secs,
        now,
    );
    match step.server {
        Some((secs, anchored_at)) => {
            anchor.server_seconds = Some(secs);
            anchor.anchor_secs = anchored_at;
            // This frame processed a live tick (new or held): remember its
            // value so the level-held repeats do not re-run the resync math.
            if let Some(c) = scene_state.snapshot.logout_countdown {
                anchor.last_seen_tick = Some(c.seconds_remaining);
            }
        }
        None => {
            anchor.server_seconds = None;
            anchor.last_seen_tick = None;
        }
    }
    anchor.shutdown = step.shutdown;
    if step.anchored_tick {
        // A live tick just anchored: the stand-up suppression is spent, and a
        // request that was awaiting its first tick has been acknowledged.
        anchor.suppress_stale = None;

        if matches!(pending.state, LogoutRequestState::AwaitingTick { .. }) {
            pending.state = LogoutRequestState::None;
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
        let s = step_anchor(Some(tick(25, false)), None, Some(30), false, None, 0.0, 6.0);
        assert_eq!(s.server, Some((30, 0.0)));
        assert!(!s.anchored_tick);

        // And the one after: carries 20 at t=11 (implied = 19, diff 1) -> keep.
        let s = step_anchor(
            Some(tick(20, false)),
            None,
            Some(30),
            false,
            None,
            0.0,
            11.0,
        );
        assert_eq!(s.server, Some((30, 0.0)));

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
        let s = step_anchor(Some(tick(30, false)), None, Some(15), false, None, 0.0, 2.0);
        assert_eq!(s.server, Some((30, 2.0)));
        assert!(s.anchored_tick);
    }

    /// The +-2s boundary itself: exactly 2 apart keeps the anchor (the rule is
    /// "within +-2 seconds").
    #[test]
    fn tick_exactly_at_tolerance_keeps_the_anchor() {
        // Anchor 30 @ t=0; at t=4.5 implied = 25.5, incoming 23 -> diff 2.5 > 2:
        // re-anchor. At t=4.0 implied = 26, incoming 24 -> diff exactly 2: keep.
        let s = step_anchor(Some(tick(24, false)), None, Some(30), false, None, 0.0, 4.0);
        assert_eq!(s.server, Some((30, 0.0)));

        let s = step_anchor(Some(tick(23, false)), None, Some(30), false, None, 0.0, 4.5);
        assert_eq!(s.server, Some((23, 4.5)));
    }

    /// /shutdown during a /logout (or vice versa) re-powers the existing effect
    /// without restarting it: the tick stays within tolerance and only the kind
    /// flag flips.
    #[test]
    fn kind_switch_within_tolerance_refreshes_flag_without_reanchor() {
        // Anchor 30/logout @ t=0; at t=5 a shutdown-kind tick carries 25
        // (implied = 25, diff 0) -> same anchor, flag flips to shutdown.
        let s = step_anchor(Some(tick(25, true)), None, Some(30), false, None, 0.0, 5.0);
        assert!(s.shutdown);
        assert_eq!(s.server, Some((30, 0.0)));
        assert!(!s.anchored_tick);

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
        let s = step_anchor(Some(tick(30, true)), None, None, false, None, 0.0, 0.4);
        assert_eq!(s.server, Some((30, 0.4)));
        assert!(s.shutdown);
        assert!(s.anchored_tick);
    }

    /// The oscillation regression: state.rs level-holds the last 0x053 tick,
    /// so between two server ticks every frame sees the SAME value. Feeding
    /// that held value through the anchor logic must never move the anchor;
    /// without the value-changed guard the resync math re-anchored it once the
    /// implied remaining drifted RESYNC_TOLERANCE_SECS past the held value and
    /// the display bounced back up on a loop.
    #[test]
    fn held_tick_200_frames_never_moves_the_anchor() {
        let frame = 1.0 / 60.0;
        // The first tick anchors at t=0; the system then remembers it as seen.
        let s = step_anchor(Some(tick(30, false)), None, None, false, None, 0.0, 0.0);
        assert_eq!(s.server, Some((30, 0.0)));

        for i in 1..=200 {
            let now = i as f64 * frame;
            // The snapshot still holds the t=0 tick (no new server tick yet),
            // and it was already processed: last_seen_tick matches.
            let s = step_anchor(
                Some(tick(30, false)),
                None,
                Some(30),
                false,
                Some(30),
                0.0,
                now,
            );
            assert_eq!(
                s.server,
                Some((30, 0.0)),
                "frame {i}: anchor moved on a held tick"
            );
            assert!(
                !s.anchored_tick,
                "frame {i}: held tick reported as an anchor"
            );
        }
    }

    /// A real countdown: para=30 at t=0 (onEffectGain), then the server's 5s
    /// ticks. The display must count down smoothly through both held segments,
    /// with no re-anchor bounce when the 25 tick lands.
    #[test]
    fn thirty_to_twenty_five_sequence_counts_down_without_reanchor_bounce() {
        let frame = 1.0 / 60.0;
        // The anchor state exactly as update_logout_countdown applies it.
        let mut server_seconds: Option<u16> = None;
        let mut last_seen_tick: Option<u16> = None;
        let mut anchor_secs = 0.0_f64;
        let mut shutdown = false;

        for i in 0..=540 {
            let now = i as f64 * frame;
            // The snapshot holds the last tick that actually arrived: 30 until
            // t=5, then 25.
            let held = if now >= 5.0 {
                Some(tick(25, false))
            } else {
                Some(tick(30, false))
            };
            let s = step_anchor(
                held,
                None,
                server_seconds,
                shutdown,
                last_seen_tick,
                anchor_secs,
                now,
            );
            match s.server {
                Some((secs, at)) => {
                    server_seconds = Some(secs);
                    anchor_secs = at;
                    if let Some(c) = held {
                        last_seen_tick = Some(c.seconds_remaining);
                    }
                }
                None => {
                    server_seconds = None;
                    last_seen_tick = None;
                }
            }
            shutdown = s.shutdown;

            // Sample once per second: the display must render 30,29,28,27,26,
            // 25, ... with no re-anchor bounce when the 25 tick lands at t=5.
            if i % 60 == 0 {
                let mode = compute_display(
                    now,
                    server_seconds.map(|v| (v, shutdown, anchor_secs)),
                    LogoutRequestState::None,
                );
                match (i / 60, &mode) {
                    // t=0..4s: the held-30 segment renders 30 down to 26.
                    (0, DisplayMode::Counting { seconds: 30, .. })
                    | (1, DisplayMode::Counting { seconds: 29, .. })
                    | (2, DisplayMode::Counting { seconds: 28, .. })
                    | (3, DisplayMode::Counting { seconds: 27, .. })
                    | (4, DisplayMode::Counting { seconds: 26, .. }) => {}
                    // t=5..9s: the held-25 segment continues 25 down to 21
                    // with no bounce back up when the tick lands.
                    (5, DisplayMode::Counting { seconds: 25, .. })
                    | (6, DisplayMode::Counting { seconds: 24, .. })
                    | (7, DisplayMode::Counting { seconds: 23, .. })
                    | (8, DisplayMode::Counting { seconds: 22, .. })
                    | (9, DisplayMode::Counting { seconds: 21, .. }) => {}
                    _ => panic!("frame {i} (t={now}s): unexpected display {mode:?}"),
                }
            }
        }
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
