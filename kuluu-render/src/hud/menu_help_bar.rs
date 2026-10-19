//! Retail top menu bar (kuluu-5ndh): whenever a menu is open, a bar spans the
//! top of the game window — menu title, an inventory used/capacity counter for
//! item lists, a one-line help string for the highlighted entry, and a "Help"
//! label at the far right; observed on the horizonxi-2023 client. The counter
//! is bag fill, NOT cursor position.

use bevy::prelude::*;

use crate::hud::item_detail::{ItemMenuFocus, SortOptionId};
use crate::hud::menu::{self, DynamicMenu, DynamicMenuAction};
use crate::hud::style::{self, theme};
use crate::input_mode::{InputMode, MenuKind, MenuLevel, MenuStack, PassiveCursorFocus};

#[derive(Component)]
pub struct MenuHelpBar;

#[derive(Component)]
pub struct MenuHelpTitle;

#[derive(Component)]
pub struct MenuHelpCounter;

#[derive(Component)]
pub struct MenuHelpHint;

/// The persistent "- : page" affordance segment; shown while a menu is open,
/// hidden during the status-ribbon buff bar. "-" flips the Command menu's two
/// pages (and toggles map/list focus on the Map screen).
#[derive(Component)]
pub struct MenuHelpPaneSwitch;

/// The bar's far-left title-box NODE — the "Status" word's framed segment. While any
/// Status-flow window is open this segment alone hides (owner ruling 2026-10-19,
/// corrected: "all you need to hide is the far left side of the top command bar" —
/// the menus themselves stay drawn and clickable). Hidden NOT disabled, x3 ruling the
/// same day: it stops drawing via `Visibility` and keeps its layout slot — `Display::None`
/// dropped the flex slot and slid every bar segment left ("it still needs to exist").
#[derive(Component)]
pub struct MenuHelpTitleBox;

/// Split part two — the framed segment holding counter, help line and the right-edge labels. (Split parts: owner ruling 2026-10-18.)
#[derive(Component)]
pub struct MenuHelpRightPart;

/// The visible gap between the band's two split parts.
const BAR_SEGMENT_GAP_PX: f32 = 4.0;

const PANE_SWITCH_HINT: &str = "- : page";

// Retail's bazaar window titles the bar itself, not the window;
// observed on the horizonxi-2023 client.
const BAZAAR_TITLE: &str = "Bazaar";
const BAZAAR_HINT: &str = "Purchase merchandise.";

pub const BAR_HEIGHT: f32 = 26.0;
/// The bar's top-left corner is persona's Status spot, so persona opening over it covers the
/// title box exactly.
pub const BAR_LEFT: f32 = crate::hud::status_panel::PERSONA_STATUS_LEFT;
pub const BAR_TOP: f32 = crate::hud::status_panel::PERSONA_STATUS_TOP;
/// The bar stops short of the network panel instead of running under it.
const BAR_RIGHT: f32 = crate::hud::network_status::NET_PANEL_W + BAR_SEGMENT_GAP_PX;
/// Retail frames the active menu title in its own boxed segment, flush left.
/// Owner ruling 2026-10-19: "the 'status' part of command bar needs to be a
/// little shorter" — exactly `menu    persona`'s record width (112), so the box ends where the
/// child window placed on it does. Shared with the statcom2 window that takes over this spot.
pub const TITLE_BOX_W: f32 = 112.0;

/// Bottom-left corner of the help line box, the bar segment right of the title box: windows
/// that hang under the help line anchor here.
pub fn help_line_bottom_left() -> Vec2 {
    Vec2::new(BAR_LEFT + TITLE_BOX_W, BAR_TOP + BAR_HEIGHT)
}

/// Single-line text that truncates instead of wrapping when the bar is narrow.
fn text_nowrap() -> TextLayout {
    TextLayout {
        linebreak: LineBreak::NoWrap,
        ..default()
    }
}

pub fn spawn_menu_help_bar(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            MenuHelpBar,
            // The band paints nothing itself — each split part carries its own frame and fill
            // (owner ruling 2026-10-18: the LEFT and RIGHT are separate parts).
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(BAR_TOP),
                left: Val::Px(BAR_LEFT),
                right: Val::Px(BAR_RIGHT),
                height: Val::Px(BAR_HEIGHT),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Stretch,
                display: Display::None,
                ..default()
            },
            GlobalZIndex(style::WINDOW_Z),
        ))
        .with_children(|p| {
            // Split part one — the title box, framed and filled on its own.
            p.spawn((
                MenuHelpTitleBox,
                Node {
                    width: Val::Px(TITLE_BOX_W),
                    flex_shrink: 0.0,
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(2.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    align_items: AlignItems::Center,
                    ..default()
                },
                // Draw-state only — the node stays in flex so nothing shifts (x3 ruling).
                Visibility::Visible,
                BackgroundColor(theme::FRAME_BG),
                BorderColor::all(theme::FRAME_EDGE),
            ))
            .with_children(|seg| {
                seg.spawn((
                    MenuHelpTitle,
                    Text::new(""),
                    text_nowrap(),
                    style::text_font(14.0),
                    TextColor(theme::TITLE),
                ));
            });
            // Split part two — framed and filled on its own, behind a gap; the counter +
            // help text clip instead of wrapping over the world view.
            p.spawn((
                MenuHelpRightPart,
                Node {
                    flex_grow: 1.0,
                    flex_shrink: 1.0,
                    min_width: Val::Px(0.0),
                    overflow: Overflow::clip_x(),
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(2.0)),
                    margin: UiRect::left(Val::Px(BAR_SEGMENT_GAP_PX)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(theme::FRAME_BG),
                BorderColor::all(theme::FRAME_EDGE),
            ))
            .with_children(|mid| {
                mid.spawn((
                    MenuHelpCounter,
                    Text::new(""),
                    text_nowrap(),
                    style::text_font(12.0),
                    TextColor(theme::MUTED),
                ));
                mid.spawn((
                    MenuHelpHint,
                    Text::new(""),
                    text_nowrap(),
                    style::text_font(14.0),
                    TextColor(theme::TEXT),
                ));
                // The right-edge labels ride inside split part two: a spacer pushes them
                // to its end, keeping "Help" clear of the pinned network S/R panel.
                mid.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });
                // Persistent pane-toggle affordance (retail's two-pane menus map
                // it to the Minus key); shown only while a menu is open.
                mid.spawn((
                    MenuHelpPaneSwitch,
                    Node {
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        margin: UiRect::right(Val::Px(12.0)),
                        ..default()
                    },
                ))
                .with_children(|seg| {
                    seg.spawn((
                        Text::new(PANE_SWITCH_HINT),
                        text_nowrap(),
                        style::text_font(12.0),
                        TextColor(theme::MUTED),
                    ));
                });
                mid.spawn(Node {
                    flex_shrink: 0.0,
                    align_items: AlignItems::Center,
                    margin: UiRect::right(Val::Px(12.0)),
                    ..default()
                })
                .with_children(|seg| {
                    seg.spawn((
                        Text::new("Help"),
                        text_nowrap(),
                        style::text_font(12.0),
                        TextColor(theme::MUTED),
                    ));
                });
            });
        });
}

/// One-line help for the highlighted entry. "Use an item." / "Select an item."
/// are retail-confirmed (kuluu-5ndh capture); the rest are provisional
/// retail-style phrasing pending captures (tracked on kuluu-5ndh).
fn root_entry_help(label: &str) -> &'static str {
    match label {
        menu::ROOT_MAGIC_LIST => "Cast magic.",
        "Abilities" => "Use abilities.",
        "Items" => "Use an item.",
        "Key Items" => "Check key items.",
        "Equipment" => "Change equipment.",
        "Status" => "Check current status.",
        "Party" => "Organize your party.",
        "Search" => "Search for players.",
        "Map" => "Display the map.",
        "Synthesis" => "Craft items.",
        "Trade" => "Offer a trade to your target.",
        "Linkshell" => "Organize linkshells.",
        "Region Info" => "Display region information.",
        "Missions" => "Check your current mission.",
        "Quests" => "Check quests.",
        "View House" => "Visit another player's house.",
        menu::ROOT_SET_BAZAAR => "Sell items from your storage.",
        "Help Desk" => "Contact the Help Desk.",
        menu::ROOT_COMMUNICATION => "Communicate with other players.",
        menu::ROOT_MACRO_PALETTE => "Edit macros.",
        "Graphics" => "Adjust graphics settings.",
        "Config" => "Change the configuration.",
        "Interface" => "Adjust interface settings.",
        "Controls" => "Rebind control keys.",
        menu::ROOT_CURRENT_TIME => "Check the current time.",
        "Debug" => "Toggle debug panels.",
        menu::ROOT_LOG_OUT => "Log out of the game.",
        menu::ROOT_SHUT_DOWN => "Shut down and exit the game.",
        _ => "",
    }
}

fn entry_help(kind: MenuKind, cursor: usize, dynamic: &DynamicMenu) -> String {
    match kind {
        MenuKind::Root => root_entry_help(menu::entry_label(kind, cursor, dynamic)).to_string(),
        MenuKind::Items | MenuKind::UsableItems => "Select an item.".to_string(),
        MenuKind::Magic => "Select a spell.".to_string(),
        MenuKind::Abilities => "Select an ability.".to_string(),
        MenuKind::KeyItems => "Select a key item.".to_string(),
        MenuKind::Equipment | MenuKind::EquipSlot(_) => "Select equipment.".to_string(),
        // Use / Drop wording per .agents/skills/retail-observe/references/2026-09-11-items-window.md;
        // the move rows have no retail counterpart in that capture's bag layout.
        MenuKind::ItemAction { .. } => match menu::entry_action(kind, cursor, dynamic) {
            Some(DynamicMenuAction::UseItem { .. }) => "Use an item.".to_string(),
            Some(DynamicMenuAction::DropItem { .. }) => ITEM_DROP_HELP.to_string(),
            Some(DynamicMenuAction::MoveItem { .. }) => "Move this item.".to_string(),
            _ => "Select an action.".to_string(),
        },
        MenuKind::ItemDropConfirm { .. } => ITEM_DROP_CONFIRM_HELP.to_string(),
        // The remodel prompt's question is GetText table-12 text, read from the DAT at load — so it
        // renders nothing until that catalog loads instead of being typed into the client (.text
        // FFXiMain.dll retail-2026-09 0x1D6B85; bead kuluu-o9qt7.13.5.2).
        MenuKind::RoomListConfirm { .. } => dynamic.roomlist_prompt.clone().unwrap_or_default(),
        // While the swap state word is pending, retail replaces this whole help line
        // with the DAT table-0x0E prompts (ids 931/932 through the caption catalog;
        // FFXiMain.dll retail-2026-09 `.text 0x225FA2`; bead kuluu-o9qt7.34) — so does
        // kuluu, reading the strings [`DynamicMenu`] copied from the DAT.
        MenuKind::EmoteList => match dynamic.emote_sort_state {
            menu::EmoteSortState::AwaitFirst => dynamic
                .emote_sort_prompts
                .first()
                .cloned()
                .flatten()
                .unwrap_or_default(),
            menu::EmoteSortState::AwaitSecond(_) => dynamic
                .emote_sort_prompts
                .get(1)
                .cloned()
                .flatten()
                .unwrap_or_default(),
            menu::EmoteSortState::Off => "Select an emote.".to_string(),
        },
        MenuKind::Status => "Select a category.".to_string(),
        // .agents/skills/retail-observe/references/2026-10-15-unity-and-fp-windows.md, "Unity
        // leader variant": DAT text, empty until the catalog loads.
        MenuKind::UnityLeaders => dynamic.unity_fp.leader_help.clone().unwrap_or_default(),
        // .agents/skills/retail-observe/references/2026-10-15-merit-and-job-point-windows.md:
        // `sortyn` shows the raise or lower help line from DAT, empty until the catalog loads.
        MenuKind::Merit(crate::hud::merit_windows::MeritWindow::SortConfirm { raise, .. }) => {
            dynamic.merit.sort_help[usize::from(!raise)]
                .clone()
                .unwrap_or_default()
        }
        _ => "Select an option.".to_string(),
    }
}

pub const ITEM_DROP_HELP: &str = "Dispose of item. Most items will be placed in the recycle bin. \
                                  If the recycle bin is full, the oldest item in the recycle bin \
                                  will be deleted.";

/// Provisional: retail's Drop flow past the submenu row is unobserved.
pub const ITEM_DROP_CONFIRM_HELP: &str = "Drop this item?";

/// Help-bar title and hint while the Items window's Options box has focus
/// (.agents/skills/retail-observe/references/2026-09-11-items-window.md).
pub fn option_help(id: SortOptionId) -> (&'static str, &'static str) {
    match id {
        SortOptionId::Auto => ("Auto-sort", "Automatically rearrange items."),
        SortOptionId::Manual => ("Manual Sort", "Manually rearrange items."),
        SortOptionId::RecycleBin => (
            "Recycling Bin",
            "Display thrown-away items. They will disappear for good when you change \
             areas, log out, or disconnect.",
        ),
    }
}

/// The bar keeps the list's title while the per-item submenu is up (retail:
/// `Items | Use an item.`), even though the submenu pane itself says "Item".
fn help_bar_title(kind: MenuKind) -> &'static str {
    if crate::hud::item_screen::is_item_submenu(kind) {
        menu::menu_title(MenuKind::Items)
    } else {
        menu::menu_title(kind)
    }
}

/// Retail shows bag fill (used/capacity, not cursor position) beside the title
/// for item lists (kuluu-5ndh: "20/58" fixed while the cursor scrolled), and
/// drops it while the per-item submenu is up.
fn bag_counter(
    kind: MenuKind,
    snap: &kuluu_snapshot::SceneSnapshot,
    active_bag: u8,
) -> Option<(usize, u16)> {
    let container = match kind {
        MenuKind::Items => active_bag,
        MenuKind::UsableItems => ffxi_proto::map::container::LOC_INVENTORY,
        _ => return None,
    };
    snap.container(container)
        .map(|c| (c.items.len(), c.capacity))
}

/// The /check window is titled with the target's name and their jobs in
/// retail; observed on the horizonxi-2023 client.
#[allow(clippy::type_complexity)]
pub fn update_menu_help_bar(
    mode: Res<InputMode>,
    dynamic: Res<DynamicMenu>,
    scene: Res<crate::snapshot::SceneState>,
    active_bag: Res<crate::hud::item_screen::ItemScreenContainer>,
    item_focus: Res<ItemMenuFocus>,
    check: Res<crate::hud::check_view::CheckTarget>,
    auction_screen: Res<crate::hud::auction::AuctionScreenState>,
    shop_screen: Res<crate::hud::shop::ShopScreenState>,
    auction_inv: Res<crate::hud::auction::AuctionSellInventory>,
    mut bar_q: Query<&mut Node, With<MenuHelpBar>>,
    mut title_q: Query<
        &mut Text,
        (
            With<MenuHelpTitle>,
            Without<MenuHelpCounter>,
            Without<MenuHelpHint>,
        ),
    >,
    mut counter_q: Query<
        &mut Text,
        (
            With<MenuHelpCounter>,
            Without<MenuHelpTitle>,
            Without<MenuHelpHint>,
        ),
    >,
    mut hint_q: Query<
        &mut Text,
        (
            With<MenuHelpHint>,
            Without<MenuHelpTitle>,
            Without<MenuHelpCounter>,
        ),
    >,
    // Pairwise-disjoint Node queries per the B0001 law.
    mut pane_switch_q: Query<&mut Node, (With<MenuHelpPaneSwitch>, Without<MenuHelpBar>)>,
    mut title_box_q: Query<
        &mut Visibility,
        (
            With<MenuHelpTitleBox>,
            Without<MenuHelpBar>,
            Without<MenuHelpPaneSwitch>,
        ),
    >,
    config_rows: Res<menu::ConfigDatRows>,
) {
    let Ok(mut node) = bar_q.single_mut() else {
        return;
    };

    // Owner ruling 2026-10-19 (x2 corrected): the ONLY thing a status flow hides is this
    // far-left title-box segment — its spot belongs to whichever status window is open.
    // The commands windows themselves stay drawn and clickable, always.

    // Owner ruling 2026-10-18 ("that's how the system works"): while the Status FLOW is open —
    // list or any sub-menu — its word never stands in the top bar; the status windows carry
    // their own captions there. Other menus keep theirs.
    let status_word_hidden = matches!(&*mode, InputMode::Menu(stack)
    if stack.levels.iter().any(|lv| {
        matches!(lv.kind, MenuKind::Status | MenuKind::StatusDetail { .. })
    }));

    // The help line follows the active pane's level: at Root the left column,
    // after a drill the right column, and the parent again once focus toggles.
    for mut visibility in title_box_q.iter_mut() {
        let want = if status_word_hidden {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        if *visibility != want {
            *visibility = want;
        }
    }

    let (content, is_menu) = match &*mode {
        InputMode::Menu(stack) => (
            focused_level(stack).map(|l| {
                if let MenuKind::Arrange(window) = l.kind {
                    return BarContent {
                        title: dynamic.arrange.help_title(),
                        counter: String::new(),
                        hint: dynamic.arrange.window_help(window),
                    };
                }
                let option = (l.kind == MenuKind::Items)
                    .then(|| item_focus.sort_selection())
                    .flatten();
                match option {
                    Some(id) => {
                        let (title, hint) = option_help(id);
                        BarContent {
                            title: title.to_string(),
                            counter: String::new(),
                            hint: hint.to_string(),
                        }
                    }
                    None => BarContent {
                        title: if status_word_hidden {
                            String::new()
                        } else {
                            help_bar_title(l.kind).to_string()
                        },
                        counter: match bag_counter(l.kind, &scene.snapshot, active_bag.0) {
                            Some((used, capacity)) => format!("{used}/{capacity}"),
                            None => String::new(),
                        },
                        hint: profile_detail_help(&mode, &scene.snapshot)
                            .or_else(|| dat_row_help(l.kind, l.cursor, &config_rows))
                            .unwrap_or_else(|| entry_help(l.kind, l.cursor, &dynamic)),
                    },
                }
            }),
            true,
        ),
        InputMode::PassiveCursor(s) if s.focus == PassiveCursorFocus::StatusIcons => {
            (buff_bar_content(&scene.snapshot, s.status_cursor), false)
        }
        InputMode::Check => (
            check.target_id.map(|id| BarContent {
                title: crate::hud::check_view::target_name(&scene.snapshot, id),
                counter: String::new(),
                hint: crate::hud::check_view::job_ribbon(
                    scene.snapshot.check.as_ref().filter(|c| c.target_id == id),
                ),
            }),
            false,
        ),
        // .agents/skills/retail-observe/references/2026-10-15-mog-house-arrange.md B1: layout mode's
        // help bar title and help are DAT text, empty until the catalogs load.
        InputMode::Arrange => (
            Some(BarContent {
                title: dynamic.arrange.help_title(),
                counter: String::new(),
                hint: dynamic.arrange.help_text(),
            }),
            false,
        ),
        InputMode::Bazaar => (
            Some(BarContent {
                title: BAZAAR_TITLE.to_string(),
                counter: String::new(),
                hint: BAZAAR_HINT.to_string(),
            }),
            false,
        ),
        InputMode::Shop => {
            let (title, hint) = crate::hud::shop::help_bar_content(&shop_screen, &scene.snapshot);
            (
                Some(BarContent {
                    title,
                    counter: String::new(),
                    hint,
                }),
                false,
            )
        }
        InputMode::Auction => {
            let (title, hint) = crate::hud::auction::help_bar_content(
                &auction_screen,
                &scene.snapshot,
                &auction_inv,
            );
            (
                Some(BarContent {
                    title,
                    counter: String::new(),
                    hint,
                }),
                false,
            )
        }
        _ => (None, false),
    };

    let Some(content) = content else {
        if node.display != Display::None {
            node.display = Display::None;
        }
        return;
    };
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }

    if let Ok(mut pane_node) = pane_switch_q.single_mut() {
        let want = if is_menu {
            Display::Flex
        } else {
            Display::None
        };
        if pane_node.display != want {
            pane_node.display = want;
        }
    }

    if let Ok(mut text) = title_q.single_mut() {
        if **text != content.title {
            **text = content.title;
        }
    }

    if let Ok(mut text) = counter_q.single_mut() {
        if **text != content.counter {
            **text = content.counter;
        }
    }

    if let Ok(mut text) = hint_q.single_mut() {
        if **text != content.hint {
            **text = content.hint;
        }
    }
}

/// The level holding the keys: a display-only Status detail on top leaves them on the selector
/// under it (.agents/skills/retail-observe/references/2026-10-15-menu-window-stacking.md).
fn focused_level(stack: &MenuStack) -> Option<&MenuLevel> {
    if stack.active_level_index() + 1 == stack.levels.len() {
        if let Some(index) = crate::hud::status_panel::status_selector_under_detail(stack) {
            return stack.levels.get(index);
        }
    }
    stack.active_level()
}

/// While retail's Status Profile window stands open this bar carries the words
/// `Title: <worn title>` (owner ruling 2026-10-19: those words belong in the bar across the top of
/// the screen, not on the window's own border). `None` everywhere else, and also while no title is
/// known yet, so every other window keeps its own help line.
fn profile_detail_help(mode: &InputMode, snap: &kuluu_snapshot::SceneSnapshot) -> Option<String> {
    let stack = match &*mode {
        InputMode::Menu(stack) => stack,
        _ => return None,
    };
    let profile_row = crate::hud::status_panel::profile_entry_index();
    if !stack.levels.iter().any(|lv| {
        matches!(lv.kind, MenuKind::StatusDetail { entry } if usize::from(entry) == profile_row)
    }) {
        return None;
    }
    let stats = snap.stats.as_ref()?;
    // One formatter of record for these words: `Title: <worn title>`.
    ffxi_vocab::title_names::lookup(stats.title_id)?;
    Some(crate::hud::status_panel::profile_caption(snap))
}

/// The DAT item a window's cursor row is, for windows whose rows are their record's items.
fn dat_row_item_id(kind: MenuKind, cursor: usize) -> Option<i16> {
    match kind {
        MenuKind::Status => crate::hud::status_panel::STATCOM2_DISPLAY_ITEM_IDS
            .get(cursor)
            .copied(),
        _ => None,
    }
}

/// The help line the record gives the row under the cursor; `None` keeps kuluu's own line.
fn dat_row_help(kind: MenuKind, cursor: usize, rows: &menu::ConfigDatRows) -> Option<String> {
    let item_id = dat_row_item_id(kind, cursor)?;
    rows.row_help(menu::registry_window(kind)?, item_id)
        .map(str::to_string)
}

struct BarContent {
    title: String,
    counter: String,
    hint: String,
}

/// Retail's status window names the highlighted buff in the top bar. We mirror
/// it while the F-cycle cursor sits on the ribbon: title "Status", a
/// position/total counter, and the buff's name (from the scraped effect table)
/// as the help line. Returns `None` when the player has no active effects.
fn buff_bar_content(snap: &kuluu_snapshot::SceneSnapshot, cursor: usize) -> Option<BarContent> {
    let icons = &snap.status_icons;
    if icons.is_empty() {
        return None;
    }
    let cursor = cursor.min(icons.len() - 1);
    let icon = icons[cursor];
    let name = ffxi_vocab::status_names::lookup(icon).unwrap_or("(Unknown effect)");
    Some(BarContent {
        title: menu::menu_title(MenuKind::Status).to_string(),
        counter: format!("{}/{}", cursor + 1, icons.len()),
        hint: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_help_covers_every_root_entry() {
        // A root entry without a help line renders an empty bar segment —
        // catch new entries that forget to add one.
        let dynamic = DynamicMenu::default();
        for idx in 0..menu::entry_count(MenuKind::Root, &dynamic, true) {
            let label = menu::entry_label(MenuKind::Root, idx, &dynamic);
            assert!(
                !root_entry_help(label).is_empty(),
                "root entry {label:?} has no help line"
            );
        }
    }

    #[test]
    fn item_lists_count_bag_fill_not_cursor() {
        use kuluu_snapshot::{ContainerView, InventoryItem, SceneSnapshot};
        let inv = ffxi_proto::map::container::LOC_INVENTORY;
        let item = |index: u8, item_no: u16| InventoryItem {
            container: inv,
            index,
            item_no,
            quantity: 1,
            locked: false,
            unselectable: false,
            charges_remaining: None,
            next_use_vana_ts: None,
            use_delay_end_vana_ts: None,
            ready: None,

            extdata: None,
        };
        let snap = SceneSnapshot {
            containers: vec![ContainerView {
                id: inv,
                capacity: 58,
                items: vec![item(1, 4096), item(2, 4097)],
            }],
            ..Default::default()
        };
        assert_eq!(bag_counter(MenuKind::Items, &snap, inv), Some((2, 58)));
        assert_eq!(
            bag_counter(MenuKind::UsableItems, &snap, inv),
            Some((2, 58))
        );
        assert_eq!(bag_counter(MenuKind::Root, &snap, inv), None);
    }

    #[test]
    fn buff_bar_names_the_cursor_slot() {
        use kuluu_snapshot::SceneSnapshot;
        // 40 = Protect, 41 = Shell (status_names scrape).
        let snap = SceneSnapshot {
            status_icons: vec![40, 41],
            ..Default::default()
        };
        let c = buff_bar_content(&snap, 1).expect("has effects");
        assert_eq!(c.title, "Status");
        assert_eq!(c.counter, "2/2");
        assert_eq!(c.hint, "Shell");
    }

    #[test]
    fn ribbon_focus_drives_the_live_bar_system() {
        use crate::hud::item_screen::ItemScreenContainer;
        use crate::input_mode::{PassiveCursorFocus, PassiveCursorState};
        use crate::snapshot::SceneState;
        use kuluu_snapshot::SceneSnapshot;

        let mut app = App::new();
        app.init_resource::<InputMode>()
            .init_resource::<DynamicMenu>()
            .init_resource::<ItemScreenContainer>()
            .init_resource::<ItemMenuFocus>()
            .init_resource::<crate::hud::check_view::CheckTarget>()
            .init_resource::<crate::hud::auction::AuctionScreenState>()
            .init_resource::<crate::hud::auction::AuctionSellInventory>()
            .init_resource::<crate::hud::shop::ShopScreenState>()
            .init_resource::<menu::ConfigDatRows>();

        let scene = SceneState {
            snapshot: SceneSnapshot {
                status_icons: vec![40, 41],
                ..Default::default()
            },
            ..Default::default()
        };
        app.insert_resource(scene);
        app.insert_resource(InputMode::PassiveCursor(PassiveCursorState {
            focus: PassiveCursorFocus::StatusIcons,
            status_cursor: 1,
            chat_expanded: false,
        }));

        app.add_systems(Startup, spawn_menu_help_bar);
        app.add_systems(Update, update_menu_help_bar);
        app.update();

        // The bar unhides and its title/counter/hint reflect the highlighted buff.
        let bar_shown = app
            .world_mut()
            .query_filtered::<&Node, With<MenuHelpBar>>()
            .single(app.world())
            .map(|n| n.display == Display::Flex)
            .unwrap();
        assert!(
            bar_shown,
            "bar must be visible while ribbon holds the cursor"
        );

        let title = app
            .world_mut()
            .query_filtered::<&Text, With<MenuHelpTitle>>()
            .single(app.world())
            .unwrap()
            .0
            .clone();
        let hint = app
            .world_mut()
            .query_filtered::<&Text, With<MenuHelpHint>>()
            .single(app.world())
            .unwrap()
            .0
            .clone();
        assert_eq!(title, "Status");
        assert_eq!(hint, "Shell", "cursor 1 names the second effect");
    }

    #[test]
    fn buff_bar_clamps_stale_cursor_and_hides_when_empty() {
        use kuluu_snapshot::SceneSnapshot;
        let snap = SceneSnapshot {
            status_icons: vec![40],
            ..Default::default()
        };
        // Cursor past the end (e.g. a buff expired) clamps to the last slot.
        assert_eq!(buff_bar_content(&snap, 9).unwrap().hint, "Protect");
        let empty = SceneSnapshot::default();
        assert!(buff_bar_content(&empty, 0).is_none());
    }

    /// Help-line replacement law (bead kuluu-o9qt7.34): while the emote window's swap
    /// state word is pending, retail swaps this line for its DAT prompts — and back when
    /// settled (.text FFXiMain.dll retail-2026-09 0x225FA2; cancel arm 0x225E34).
    #[test]
    fn emote_list_help_line_follows_the_swap_state() {
        let mut dynamic = DynamicMenu::default();
        assert_eq!(
            entry_help(MenuKind::EmoteList, 0, &dynamic),
            "Select an emote."
        );
        dynamic.emote_sort_prompts = [Some("P1".to_string()), Some("P2".to_string())];
        dynamic.emote_sort_state = menu::EmoteSortState::AwaitFirst;
        assert_eq!(entry_help(MenuKind::EmoteList, 0, &dynamic), "P1");
        dynamic.emote_sort_state = menu::EmoteSortState::AwaitSecond(6);
        assert_eq!(entry_help(MenuKind::EmoteList, 3, &dynamic), "P2");
        dynamic.emote_sort_state = menu::EmoteSortState::Off;
        assert_eq!(
            entry_help(MenuKind::EmoteList, 0, &dynamic),
            "Select an emote."
        );
    }

    /// With Status focused the help line is the record's help text of the row under the cursor,
    /// also while a display-only detail sits on top; rows without one keep kuluu's line.
    #[test]
    fn status_help_line_follows_the_focused_row() {
        use crate::hud::status_panel::{STATCOM2_DISPLAY_ITEM_IDS, STATUS_ENTRIES};
        use crate::input_mode::{MenuLevel, MenuStack};
        const HELPED_ROW: usize = 2;
        let mut rows = menu::ConfigDatRows::default();
        rows.row_help.insert(
            menu::STATUS_SELECTOR_WINDOW.into(),
            [
                (STATCOM2_DISPLAY_ITEM_IDS[0], "first row help".to_string()),
                (
                    STATCOM2_DISPLAY_ITEM_IDS[HELPED_ROW],
                    "third row help".to_string(),
                ),
            ]
            .into(),
        );
        assert_eq!(
            dat_row_help(MenuKind::Status, 0, &rows).as_deref(),
            Some("first row help")
        );
        assert_eq!(
            dat_row_help(MenuKind::Status, HELPED_ROW, &rows).as_deref(),
            Some("third row help")
        );
        assert_eq!(dat_row_help(MenuKind::Status, 1, &rows), None);
        assert_eq!(
            dat_row_help(MenuKind::Status, 0, &menu::ConfigDatRows::default()),
            None,
            "no install: kuluu's own line"
        );

        let mut stack = MenuStack::root();
        stack
            .levels
            .push(MenuLevel::new(MenuKind::Status, HELPED_ROW));
        let profile = STATUS_ENTRIES
            .iter()
            .position(|e| e.kind == crate::hud::status_panel::StatusEntryKind::Profile)
            .unwrap();
        stack.levels.push(MenuLevel::new(
            MenuKind::StatusDetail {
                entry: profile as u8,
            },
            0,
        ));
        let focused = focused_level(&stack).expect("a focused level");
        assert_eq!(
            (focused.kind, focused.cursor),
            (MenuKind::Status, HELPED_ROW),
            "the display-only detail leaves the keys on Status"
        );

        let mut app = App::new();
        app.init_resource::<DynamicMenu>()
            .init_resource::<crate::hud::item_screen::ItemScreenContainer>()
            .init_resource::<ItemMenuFocus>()
            .init_resource::<crate::hud::check_view::CheckTarget>()
            .init_resource::<crate::hud::auction::AuctionScreenState>()
            .init_resource::<crate::hud::auction::AuctionSellInventory>()
            .init_resource::<crate::hud::shop::ShopScreenState>()
            .init_resource::<crate::snapshot::SceneState>()
            .insert_resource(rows)
            .insert_resource(InputMode::Menu(stack));
        app.add_systems(Startup, spawn_menu_help_bar);
        app.add_systems(Update, update_menu_help_bar);
        app.update();
        let hint = app
            .world_mut()
            .query_filtered::<&Text, With<MenuHelpHint>>()
            .single(app.world())
            .unwrap()
            .0
            .clone();
        assert_eq!(hint, "third row help");
    }

    /// The title law (owner ruling 2026-10-19): with the Profile window open, this bar shows
    /// `Title: <worn title>`. With no title known it falls back through the usual chain, so the
    /// selector's own line survives.
    #[test]
    fn profile_detail_puts_the_title_in_the_top_bar() {
        use crate::hud::status_panel::{StatusEntryKind, STATUS_ENTRIES};
        use crate::input_mode::{MenuLevel, MenuStack};

        let hint_with_title = |title_id: u16| -> String {
            let mut app = App::new();
            app.init_resource::<DynamicMenu>()
                .init_resource::<crate::hud::item_screen::ItemScreenContainer>()
                .init_resource::<ItemMenuFocus>()
                .init_resource::<crate::hud::check_view::CheckTarget>()
                .init_resource::<crate::hud::auction::AuctionScreenState>()
                .init_resource::<crate::hud::auction::AuctionSellInventory>()
                .init_resource::<crate::hud::shop::ShopScreenState>()
                .init_resource::<menu::ConfigDatRows>();
            app.init_resource::<crate::snapshot::SceneState>();
            {
                let mut scene = app
                    .world_mut()
                    .resource_mut::<crate::snapshot::SceneState>();
                scene.snapshot.stats = Some(kuluu_snapshot::CharStats {
                    title_id,
                    ..Default::default()
                });
            }
            let profile_row = STATUS_ENTRIES
                .iter()
                .position(|entry| entry.kind == StatusEntryKind::Profile)
                .expect("profile is a status row") as u8;
            let mut stack = MenuStack::root();
            stack.levels.push(MenuLevel::new(MenuKind::Status, 0));
            stack.levels.push(MenuLevel::new(
                MenuKind::StatusDetail { entry: profile_row },
                0,
            ));
            app.insert_resource(InputMode::Menu(stack));
            app.add_systems(Startup, spawn_menu_help_bar);
            app.add_systems(Update, update_menu_help_bar);
            for _ in 0..2 {
                app.update();
            }
            app.world_mut()
                .query_filtered::<&Text, With<MenuHelpHint>>()
                .single(app.world())
                .unwrap()
                .0
                .clone()
        };

        assert_eq!(
            hint_with_title(120),
            "Title: Star Onion Brigade Member",
            "the words the owner capture shows, now on this bar"
        );
        assert_eq!(
            hint_with_title(0),
            entry_help(MenuKind::Status, 0, &DynamicMenu::default()),
            "no title known: the selector's own line stays"
        );
    }

    /// Owner ruling 2026-10-18 ("that's how the system works"): while ANY part of the Status
    /// flow is open — its list or a sub-menu — the word "Status" never stands in the top bar;
    /// other menus keep their words, and leaving the flow restores them.
    #[test]
    fn status_word_hidden_across_the_status_flow() {
        use crate::input_mode::{MenuLevel, MenuStack};

        let mut app = App::new();
        app.init_resource::<DynamicMenu>()
            .init_resource::<crate::hud::item_screen::ItemScreenContainer>()
            .init_resource::<ItemMenuFocus>()
            .init_resource::<crate::hud::check_view::CheckTarget>()
            .init_resource::<crate::hud::auction::AuctionScreenState>()
            .init_resource::<crate::hud::auction::AuctionSellInventory>()
            .init_resource::<crate::hud::shop::ShopScreenState>()
            .init_resource::<menu::ConfigDatRows>()
            .init_resource::<crate::snapshot::SceneState>();
        app.add_systems(Startup, spawn_menu_help_bar);
        app.add_systems(Update, update_menu_help_bar);

        let title_of = |app: &mut App| -> String {
            for _ in 0..2 {
                app.update();
            }
            app.world_mut()
                .query_filtered::<&Text, With<MenuHelpTitle>>()
                .single(app.world())
                .unwrap()
                .0
                .clone()
        };

        let mut other = MenuStack::root();
        other.levels.push(MenuLevel::new(MenuKind::Items, 0));
        app.insert_resource(InputMode::Menu(other.clone()));
        assert_eq!(
            title_of(&mut app),
            menu::menu_title(MenuKind::Items),
            "other menus keep their word"
        );

        let mut stack = MenuStack::root();
        stack.levels.push(MenuLevel::new(MenuKind::Status, 0));
        app.insert_resource(InputMode::Menu(stack.clone()));
        assert_eq!(title_of(&mut app), "", "the flow itself hides the word");

        // Owner ruling 2026-10-19 (corrected): not just the word — the whole far-left
        // title-box segment is coded hidden while any status window holds its spot.
        // Hidden NOT disabled (owner x3, 2026-10-19): the box stops drawing but keeps
        // its flex slot — a Display::None here slid every bar segment left.
        let mut vis_q = app
            .world_mut()
            .query_filtered::<&Visibility, With<MenuHelpTitleBox>>();
        assert_eq!(
            *vis_q.single(app.world()).expect("title box"),
            Visibility::Hidden,
            "the title-box segment stops drawing with the word"
        );
        let mut disp_q = app
            .world_mut()
            .query_filtered::<&Node, With<MenuHelpTitleBox>>();
        assert_eq!(
            disp_q.single(app.world()).unwrap().display,
            Display::Flex,
            "the box keeps its layout slot while hidden"
        );

        let mut with_detail = stack.clone();
        with_detail
            .levels
            .push(MenuLevel::new(MenuKind::StatusDetail { entry: 0 }, 0));
        app.insert_resource(InputMode::Menu(with_detail));
        assert_eq!(title_of(&mut app), "", "sub-menu keeps it hidden");

        app.insert_resource(InputMode::Menu(other));
        assert_eq!(
            title_of(&mut app),
            menu::menu_title(MenuKind::Items),
            "flow closed: words work again"
        );
    }

    /// The help line box starts where the title box ends, its bottom is the bar's, and the bar sits at persona's spot.
    #[test]
    fn help_line_corner_matches_the_spawned_bar() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.run_system_once(spawn_menu_help_bar).unwrap();
        let bar = world
            .query_filtered::<&Node, With<MenuHelpBar>>()
            .single(&world)
            .unwrap()
            .clone();
        let (Val::Px(left), Val::Px(top), Val::Px(height)) = (bar.left, bar.top, bar.height) else {
            panic!("bar placed in pixels");
        };
        let title_parent = world
            .query_filtered::<&ChildOf, With<MenuHelpTitle>>()
            .single(&world)
            .unwrap()
            .parent();
        let title_box_width = world.get::<Node>(title_parent).unwrap().width;
        let corner = help_line_bottom_left();
        assert_eq!(title_box_width, Val::Px(corner.x - left));
        assert_eq!(top + height, corner.y);
        assert_eq!(
            (left, top),
            (
                crate::hud::status_panel::PERSONA_STATUS_LEFT,
                crate::hud::status_panel::PERSONA_STATUS_TOP
            ),
            "persona opens exactly over the title box"
        );
    }

    /// The band is two framed parts — each with its own border and fill — the title box
    /// left, the counter/help box right, split by a visible gap (owner ruling 2026-10-18:
    /// "ensure the LEFT and RIGHT are split parts").
    #[test]
    fn command_band_is_two_split_parts() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.run_system_once(spawn_menu_help_bar).unwrap();

        // The band itself paints nothing (bevy seeds every node with a transparent
        // BackgroundColor, so the check is on alpha), leaving no strip behind the parts.
        let root_bg = world
            .query_filtered::<&BackgroundColor, With<MenuHelpBar>>()
            .single(&world)
            .expect("one band");
        assert!(
            root_bg.0.alpha() == 0.0,
            "the band root paints nothing: {:?}",
            root_bg.0
        );

        let all_edges = |node: &Node| {
            [
                node.border.left,
                node.border.right,
                node.border.top,
                node.border.bottom,
            ]
        };

        // Left part — the title box frames and fills itself.
        let left_parent = world
            .query_filtered::<&ChildOf, With<MenuHelpTitle>>()
            .single(&world)
            .unwrap()
            .parent();
        let left_node = world.get::<Node>(left_parent).expect("title box node");
        assert_eq!(
            all_edges(left_node),
            [Val::Px(1.0); 4],
            "the title box frames itself"
        );
        assert_eq!(
            world.get::<BackgroundColor>(left_parent).unwrap().0,
            theme::FRAME_BG
        );

        // Right part — framed, filled, grows across the rest of the band behind a gap.
        let (right_node, right_bg) = world
            .query_filtered::<(&Node, &BackgroundColor), With<MenuHelpRightPart>>()
            .single(&world)
            .expect("the hint box");
        assert_eq!(
            all_edges(right_node),
            [Val::Px(1.0); 4],
            "hint box frames itself"
        );
        assert_eq!(right_bg.0, theme::FRAME_BG);
        assert!(right_node.flex_grow > 0.0, "takes the rest of the band");
        assert!(
            matches!(right_node.margin.left, Val::Px(gap) if gap >= BAR_SEGMENT_GAP_PX),
            "split by a visible gap: {:?}",
            right_node.margin.left
        );
    }

    #[test]
    fn unity_leader_rows_share_the_dat_help_line() {
        let mut dynamic = DynamicMenu::default();
        assert_eq!(entry_help(MenuKind::UnityLeaders, 0, &dynamic), "");
        dynamic.unity_fp.leader_help = Some("leader help".to_string());
        for cursor in [0, crate::hud::unity_fp::layout::LEADER_ROWS - 1] {
            assert_eq!(
                entry_help(MenuKind::UnityLeaders, cursor, &dynamic),
                "leader help"
            );
        }
    }
}
