use bevy::prelude::*;

use crate::graphics_settings::{
    config_rows, dlss_config_rows, graphics_rows, GraphicsField, GraphicsSection, GraphicsSettings,
    MenuRow, CONFIG_SECTIONS, DLSS_CONFIG_SECTIONS, GRAPHICS_SECTIONS,
};
use crate::hud::style::{self, theme};
use crate::input_mode::{InputMode, MenuKind, MenuStack};

pub const ROOT_LOG_OUT: &str = "Log Out";
pub const ROOT_SHUT_DOWN: &str = "Shut Down";
// Caption strings verbatim from the client's UI caption DAT (ROM/165/76.DAT
// d_msg entries), which also carries `Commands` as the window title.
pub const ROOT_CURRENT_TIME: &str = "Current Time";
pub const ROOT_COMMUNICATION: &str = "Communication";
pub const COMM_EMOTE_LIST: &str = "Emote List";
/// Friend List row — the `com1` DAT carries windowps330 with caption token 61
/// = catalog string `Friend List`, listed before windowps928's `Emote List`
/// (token 180) (vault main_menu.md §37).
pub const COMM_FRIEND_LIST: &str = "Friend List";
/// Page-2 row 4. The caption is catalog id 107, which the `mogcont` window
/// record itself carries as its comwin title token (vault main_menu.md §32).
pub const ROOT_VIEW_HOUSE: &str = "View House";

/// Retail's Command menu (windows `menuwind`/`socialme`) shows twelve rows per
/// page across two pages; kuluu appends a third page of its own entries. "-"
/// and Right advance 1→2→3→1, Left steps back. A flip between the retail pages
/// keeps the cursor on the same screen row — retail carries it through an
/// explicit pair table (FFXiMain.dll retail-2026-09 RVA 0x36FF98), whose rows
/// line up with display order, so in this layout both retail pages share the
/// row index; entering or leaving page 3 keeps the row number clamped to its
/// length. ROOT_ENTRIES is page 1, then page 2 (retail display order, pinned by
/// `root_pages_match_retail_pair_table`), then page 3; the page a cursor is on
/// follows from its index (root_page_bounds), so no separate page state exists.
/// The Debug entry renders only when built with `--features debug-menu` —
/// opt-in and off by default, so retail parity holds as long as shipped builds
/// do not enable the flag; local test batches (build_cowland.bat) pass it on
/// RELEASE builds on purpose.
const ROOT_ENTRIES: &[&str] = &[
    // Page 1 (`menuwind`): Status, Equipment, Magic, Items, Synthesis,
    // Abilities, Party, Trade, Search, Linkshell, Region Info, Map.
    "Status",
    "Equipment",
    "Magic",
    "Items",
    "Synthesis",
    "Abilities",
    "Party",
    "Trade",
    "Search",
    "Linkshell",
    "Region Info",
    "Map",
    // Page 2 (`socialme`): Missions, Quests, Key Items, View House, Bazaar,
    // Edit Macros (the caption for the mcrmenu row - no bare `Macros` exists
    // in the client's caption catalog), Config, Help Desk, Current Time,
    // Communication, Shut Down, Log Out.
    "Missions",
    "Quests",
    "Key Items",
    ROOT_VIEW_HOUSE,
    "Bazaar",
    "Edit Macros",
    "Config",
    "Help Desk",
    ROOT_CURRENT_TIME,
    ROOT_COMMUNICATION,
    ROOT_SHUT_DOWN,
    ROOT_LOG_OUT,
    // Page 3 (kuluu-only).
    "Graphics",
    "Interface",
    "Controls",
    #[cfg(feature = "debug-menu")]
    "Debug",
];

const ROOT_PAGE_1_LEN: usize = 12;
const ROOT_PAGE_2_LEN: usize = 12;

/// The Command menu cycles three pages: retail's two plus kuluu's.
pub const ROOT_PAGE_COUNT: usize = 3;

fn root_page_ranges() -> [(usize, usize); ROOT_PAGE_COUNT] {
    let p2 = ROOT_PAGE_1_LEN;
    let p3 = ROOT_PAGE_1_LEN + ROOT_PAGE_2_LEN;
    [(0, p2), (p2, p3), (p3, ROOT_ENTRIES.len())]
}

fn root_page_index(cursor: usize) -> usize {
    if cursor < ROOT_PAGE_1_LEN {
        0
    } else if cursor < ROOT_PAGE_1_LEN + ROOT_PAGE_2_LEN {
        1
    } else {
        2
    }
}

/// The half-open `[start, end)` range of `ROOT_ENTRIES` the page containing
/// `cursor` occupies. The Command menu shows one page at a time; "-" and Right
/// flip forward, Left steps back, Up/Down wrap within the current page.
pub fn root_page_bounds(cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(ROOT_ENTRIES.len() - 1);
    root_page_ranges()[root_page_index(cursor)]
}

/// The cursor index one page forward ('-' / Right): page 1→2→3→1.
pub fn root_next_page_cursor(cursor: usize) -> usize {
    root_row_on_target_page(cursor, (root_page_index(cursor) + 1) % ROOT_PAGE_COUNT)
}

/// The cursor index one page back (Left), wrapping backwards through the pages.
pub fn root_prev_page_cursor(cursor: usize) -> usize {
    root_row_on_target_page(
        cursor,
        (root_page_index(cursor) + ROOT_PAGE_COUNT - 1) % ROOT_PAGE_COUNT,
    )
}

fn root_row_on_target_page(cursor: usize, target_page: usize) -> usize {
    let ranges = root_page_ranges();
    let row_in_page = cursor - ranges[root_page_index(cursor)].0;
    let (start, end) = ranges[target_page];
    start + row_in_page.min(end - start - 1)
}

/// The Root command list, for callers that enumerate the top-level entries
/// (e.g. the drill parity guard) without matching each label.
pub fn root_entries() -> &'static [&'static str] {
    ROOT_ENTRIES
}

// commenu's two list widgets in DAT order: windowps330 `Friend List` (61),
// then windowps928 `Emote List` (180) — vault main_menu.md §37.
const COMMUNICATION_ENTRIES: &[&str] = &[COMM_FRIEND_LIST, COMM_EMOTE_LIST];

const ITEMS_ENTRIES_STUB: &[&str] = &["(Items — Stage 3: pending inventory submenu)"];

pub const DROP_CONFIRM_YES: &str = "Yes";

pub const DROP_CONFIRM_NO: &str = "No";

/// The drop confirm's rows; the cursor opens on No (`DROP_CONFIRM_DEFAULT`),
/// like retail's Log Out prompt.
pub const DROP_CONFIRM_ENTRIES: &[&str] = &[DROP_CONFIRM_YES, DROP_CONFIRM_NO];

pub const DROP_CONFIRM_DEFAULT: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicMenuRow {
    pub label: String,

    pub action: DynamicMenuAction,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DynamicMenuAction {
    CastSpell {
        spell_id: u16,
    },

    JobAbility {
        ability_id: u16,
    },

    Weaponskill {
        skill_id: u16,
    },

    PetAbility {
        ability_id: u16,
    },

    /// Ranged attack (c2s action 0x10). Carries no id — only a target, picked
    /// through the same sub-target cursor as a weapon skill.
    RangedAttack,

    /// The two answers `prtyjoin` offers over a pending invite; sent as c2s
    /// GROUP_SOLICIT_RES (Res 1/0). Retail opens that window from the Party row's
    /// helper whenever its static state holds an unresolved solicit
    /// (FFXiMain.dll retail-2026-09 RVA 0x2016E0; vault main_menu.md section 24).
    AcceptPartyInvite,
    DeclinePartyInvite,

    /// One roster slot of the party window. Selecting a slot is inert until its
    /// member sub-action mapping is dug out of the prtyN select family
    /// (0x2059A0.., vault main_menu.md section 24 [I]).
    PartyMemberRow {
        char_id: u32,
    },

    /// One automaton slot row of the pupequip variant. `slot` is the LSB
    /// AutomatonSlot id (Head=0, Frame=1, Attachment1..12 = 2..13 — vendor/
    /// server/src/map/packets/c2s/0x102_extended_job.h). Selecting stays inert
    /// until the pupequip select family's equip flow is dug out and wired to
    /// c2s 0x102 pup_data_t (vault main_menu.md §39 [I]).
    AutomatonSlotRow {
        slot: u8,
    },

    /// One filled macro slot row of the mcrselec select window (vault
    /// main_menu.md §41). Selecting stays inert until the macro-use seam is
    /// lawed end to end — the DAT grids prove capacity (2 blocks x 20 keytops),
    /// the store proves content, but which widget press runs a macro is an
    /// open-map item on bead .19.1.
    MacroSlotRow {
        block: u8,
        index: u8,
    },

    /// Answer an incoming trade request YES — c2s TRADE_RES Kind Start. Retail
    /// answers through the gift window's rows once s2c 0x021 lands (FFXiMain.dll
    /// retail-2026-09 RVA 0x1515E0; vault main_menu.md §25).
    TradeAcceptInvite,
    /// Answer NO — c2s TRADE_RES Kind Cancell. The server confirms nothing back
    /// to the decliner, so kuluu clears its pending window locally too (the same
    /// law accepted for party answers in §24).
    TradeDeclineInvite,
    /// The trade window's button row id 9 = Confirm: sends c2s TRADE_RES Kind
    /// Make while unconfirmed; retail sets its state to confirmed at send time
    /// (vault main_menu.md §25 select law).
    TradeConfirmRow,
    /// Button row id 10 = Exit: retail closes BOTH window names when nothing is
    /// pending (predicate 0x151340); kuluu sends TRADE_RES Kind Cancell and
    /// clears its own state.
    TradeCancelRow,
    /// One of our own offer slots — select ids 1..8 through the byte table at
    /// .rdata 0x371D60: an empty row opens the Add Item picker, a filled row
    /// releases via the qty-0 TRADE_LIST path (player_trade.cpp setSlot).
    TradeOwnSlot {
        trade_index: u8,
    },
    /// Button row id 11 = Add Gil. Retail posts an async keypad dialog
    /// (`call 0x140D90`, callback 0x1515C0) before staging slot 0 with the
    /// 0xFFFF sentinel; the keypad itself is [I-pending], so this row stays
    /// display-only for now (vault main_menu.md §25).
    TradeGilRow,
    /// Partner-side offer slot rows (`gift` window): display only — retail gives
    /// them no select arm at all.
    TradePartnerOfferRow {
        trade_index: u8,
    },
    /// Pick from the Add Item bag picker: c2s TRADE_LIST stages `bag_slot`'s
    /// contents into our offer at `trade_index` (player_trade.cpp setSlot).
    TradeStageFromBag {
        trade_index: u8,
        item_no: u16,
        qty: u32,
        bag_slot: u8,
    },

    /// Get off the mount (c2s action 0x12).
    /// GP_CLI_COMMAND_ACTION_ACTIONID,
    /// vendor/server/src/map/packets/c2s/0x01a_action.h.
    Dismount,

    /// Dig with the chocobo for a buried item (c2s action 0x11). Self-targeted
    /// like Dismount. GP_CLI_COMMAND_ACTION_ACTIONID,
    /// vendor/server/src/map/packets/c2s/0x01a_action.h.
    ChocoboDig,

    UseItem {
        container: u8,
        index: u8,
        item_no: u16,
    },

    /// c2s 0x029 ITEM_MOVE (full stack, server-picked destination slot).
    MoveItem {
        quantity: u32,
        from_container: u8,
        from_slot: u8,
        to_container: u8,
        item_no: u16,
    },

    /// Open the per-item context submenu for a slot.
    OpenItemAction {
        container: u8,
        index: u8,
        item_no: u16,
    },

    /// Item submenu "Drop": opens the Yes/No confirm for the whole stack
    /// (c2s 0x028 ITEM_DUMP on Yes;
    /// vendor/server/src/map/packets/c2s/0x028_item_dump.cpp).
    DropItem {
        container: u8,
        index: u8,
        item_no: u16,
        quantity: u32,
    },

    EquipItem {
        container: u8,
        container_index: u8,
        equip_slot: u8,
        item_no: u16,
    },

    /// Terminal Key Items row; selecting it echoes the name to chat, and
    /// cursoring over it marks the key item seen on menu close (c2s 0x064).
    KeyItem {
        id: u16,
    },

    /// Emote List row: dispatches the same AgentCommand::Emote as the slash
    /// command, at the current target.
    Emote {
        emote_id: u8,
    },

    /// The `searchma` run row — select id 1 hands to the query object's method
    /// 0x20EE70 (FFXiMain.dll retail-2026-09 law at handler RVA 0x20D340; vault
    /// main_menu.md section 26). Fires AgentCommand::PlayerSearch; merged
    /// CSearchListPacket frames land the results in SceneSnapshot.search.
    SearchRun,
    /// One hit row of the results list (retail's `scresult` model window). Selecting is inert
    /// until a search-to-target mapping exists - retail's own hit-row semantics stay [I] (vault
    /// main_menu.md section 26).
    SearchHitRow {
        index: usize,
    },

    /// One shell-slot row of the Link Shell window (retail `menu    link5`, opened by the page-1
    /// arm FFXiMain.dll retail-2026-09 RVA 0x1405EE through helper
    /// FFXiMain.dll retail-2026-09 RVA 0x1E4470; vault main_menu.md section 27).
    /// `linkshell_slot` is LSB's `LinkshellId` (1/2). Selecting stays inert while that window's own
    /// select table is [I-pending] (section 27.6): the row makes the s2c 0x0E0 state legible, it
    /// does not guess at equip/unequip UI.
    LinkShellSlot {
        linkshell_slot: u8,
    },

    /// One mode row of the Region Info window: items 1..4 install display modes
    /// 0..3 on that window itself (FFXiMain.dll retail-2026-09 RVA 0x140818;
    /// vault main_menu.md §29).
    RegionInfoMode {
        mode: u8,
    },

    /// One row of the `missionm` list opening its `miss00` detail (the handler's
    /// select tail name-opens miss00; FFXiMain.dll retail-2026-09 RVA 0x22DB40;
    /// vault main_menu.md §30).
    MissionOpenLog {
        port: u16,
    },

    /// A legible-only mission-log line (`miss00`'s paging semantics are
    /// [I-pending], vault main_menu.md section 30).
    MissionLogRow {
        port: u16,
    },

    /// One stall-picked row of the Bazaar window (the `bazaar` browser is
    /// data-driven chrome — FFXiMain.dll retail-2026-09 RVA 0x1F6C70; vault
    /// main_menu.md §33): select browses that seller's stall via c2s 0x105. The
    /// empty-list hint row carries `char_id: u32::MAX` and never resolves.
    BazaarStall {
        char_id: u32,
    },

    /// One row of the `quest00` list opening its `quest01` detail (the handler's
    /// select tail name-opens quest01 with the row value; FFXiMain.dll retail-2026-09
    /// RVA 0x2300EE; vault main_menu.md section 31).
    QuestOpenDetail {
        log: u8,
        id: u16,
    },

    /// A legible-only quest detail line (`quest01`'s paging semantics stay
    /// [I-pending], vault main_menu.md section 31).
    QuestDetailRow {
        log: u8,
        id: u16,
    },

    /// One craftable recipe row of the kuluu craft list (the `cmbmenu` row-2 branch opens this
    /// window pair; FFXiMain.dll retail-2026-09 RVA 0x1E29C1, vault main_menu.md §22/§28). The plan is resolved
    /// against the bag at render time: selecting sends exactly this c2s 0x096 ask, ingredients in
    /// the recipe row's own positional order.
    SynthesizeRecipe {
        crystal_id: u16,
        crystal_slot: u8,
        /// Entries in recipe column order; `None` fills the empty trailing columns
        /// (the ask's Items byte is the number of Somes).
        ingredients: [Option<ffxi_proto::synth::SynthIngredient>; 8],
    },

    /// One job row of the home-view pick list (`jobchang` → `jobcsel` family,
    /// vault main_menu.md §53): selecting sends c2s 0x100 MYROOM_JOB carrying
    /// this id on its own side and zero ("no change") on the other — exactly
    /// what LSB's validator walks before applying it (vendor/server/src/map/
    /// packets/c2s/0x100_myroom_job.cpp).
    HomeJobRow {
        job_id: u8,
        as_main: bool,
    },
}

impl DynamicMenuAction {
    /// The item id a row acts on, for rows that carry one (item list, usable
    /// list, per-item context, equip). Lets the item window resolve icon +
    /// detail without matching each variant at every call site.
    pub fn item_no(&self) -> Option<u16> {
        match *self {
            DynamicMenuAction::UseItem { item_no, .. }
            | DynamicMenuAction::MoveItem { item_no, .. }
            | DynamicMenuAction::OpenItemAction { item_no, .. }
            | DynamicMenuAction::DropItem { item_no, .. }
            | DynamicMenuAction::EquipItem { item_no, .. } => Some(item_no),
            _ => None,
        }
    }

    /// The source `(container, index)` of an item-bearing action, so per-slot
    /// state (charges, recast) resolves the exact instance rather than the
    /// first item of the same id.
    pub fn item_slot(&self) -> Option<(u8, u8)> {
        match *self {
            DynamicMenuAction::UseItem {
                container, index, ..
            }
            | DynamicMenuAction::OpenItemAction {
                container, index, ..
            }
            | DynamicMenuAction::DropItem {
                container, index, ..
            } => Some((container, index)),
            DynamicMenuAction::EquipItem {
                container,
                container_index,
                ..
            } => Some((container, container_index)),
            _ => None,
        }
    }
}

/// One item row's label: the name, suffixed " xN" only for a real stack. Shared
/// by the inventory list and the usable-item list so partial stacks read the
/// same wherever an item appears.
pub fn item_qty_label(name: &str, quantity: u32) -> String {
    if quantity > 1 {
        format!("{name} x{quantity}")
    } else {
        name.to_string()
    }
}

/// Unseen ("new") key-item indicator suffix. Retail shows a yellow-bubble
/// glyph whose exact appearance is unverified (bead kuluu-h7x
/// retail_unknowns); a text marker stands in until a retail capture.
pub const KEY_ITEM_UNSEEN_SUFFIX: &str = " (new)";

pub fn key_item_row_label(id: u16, seen: bool) -> String {
    let name = ffxi_vocab::key_item_names::lookup(id)
        .map(str::to_string)
        .unwrap_or_else(|| format!("Key Item #{id}"));
    if seen {
        name
    } else {
        format!("{name}{KEY_ITEM_UNSEEN_SUFFIX}")
    }
}

#[derive(bevy::prelude::Resource, Debug, Clone, Default)]
pub struct DynamicMenu {
    pub rows: Vec<DynamicMenuRow>,
}

pub const DYNAMIC_VISIBLE_ROWS: usize = 22;

const EQUIPMENT_ENTRIES: &[&str] = &[
    "Main", "Sub", "Ranged", "Ammo", "Head", "Body", "Hands", "Legs", "Feet", "Neck", "Waist",
    "L.Ear", "R.Ear", "L.Ring", "R.Ring", "Back",
];

const STATUS_LABELS: &[&str] = &[
    "Profile",
    "Job Levels",
    "Master Levels",
    "Combat Skill",
    "Magic Skill",
    "Craft Skill",
    "Currencies",
    "Currencies 2",
    "Unity",
    "Play Time",
    "Merit Points",
    "Job Points",
];

const CONTROLS_ENTRIES: &[&str] = &[
    "Standard",
    "Compact 1",
    "Compact 2",
    "Reset to defaults",
    "Show current bindings",
];

pub const DEBUG_PERF: &str = "Perf";
pub const DEBUG_TARGET_CYCLE: &str = "Target Cycle";
pub const DEBUG_MESH: &str = "Mesh Debug";
pub const DEBUG_NET_STATUS: &str = "Net Status";
pub const DEBUG_NOCLIP: &str = "NoClip";
/// Enhanced build only: forces the retail weapon draw/sheathe movement hold
/// back on (the gated build lifts it by default). Default off.
#[cfg(feature = "enhanced-engage-move-lock-off")]
pub const DEBUG_ENGAGE_ANIM_LOCK: &str = "Engage_anim_lock";
/// Debug auto-enter row (enternity-style): [on] = event-dialog message frames
/// advance themselves after their read time instead of parking on Enter;
/// choice frames, item lines, text-entry frames, server custom menus, and the
/// enternity blacklist (Paintbrush of Souls, Geomantic Reservoir) stay manual.
/// Off by default.
pub const DEBUG_AUTO_ENTER_CS: &str = "Auto-Enter CS";
/// Debug weather gate row: [on] = the active-weather modifier (ambient tint,
/// sun mul, lightning) and precipitation particles are applied; toggling it
/// off suppresses them for isolating scene-graphic errors. Default on.
pub const DEBUG_WEATHER: &str = "Weather";
/// Debug fog gate row: [on] = every fog layer (DAT distance fog, volumetric
/// ground haze) renders; toggling it off strips all of them. Default on.
pub const DEBUG_FOG: &str = "Fog";
/// Debug clock-gate row: [on] = the Vana clock is frozen at 18:00 (lamp
/// night-scene testing); off thaws it back to live time.
pub const DEBUG_FORCE_18: &str = "Force_18:00";
/// Debug Entity List overlay row: [on] = the scrollable live-entity dump
/// (id/name/kind/pos/status/hp/invis flags from the EntityTable) is shown.
/// Mouse wheel scrolls. Default off.
pub const DEBUG_ENTITY_LIST: &str = "Entity List";
pub const DEBUG_SOUND: &str = "Sound";
pub const DEBUG_VOLUME: &str = "Volume";
pub const DEBUG_PRINT_POS: &str = "Print POS";
pub const DEBUG_STAIR_DRAW: &str = "Draw Stair Climber";
pub const DEBUG_STAIR_STATUS: &str = "Show Stair Status";
pub const DEBUG_GRAPHICS_DEBUG: &str = "Graphics Debug";
pub const DEBUG_POSITION_LOG: &str = "Panel Pos Log";
pub const DEBUG_NAMEPLATES: &str = "Nameplate Debug";
pub const DEBUG_UI_SETTINGS: &str = "UI Settings";

/// Retail+ section (dev-only Debug menu): a separator row, the section label,
/// then the DLSS On/Off toggle plus — in enhanced builds only — Mob HP Under /
/// Job Display. All persist in GraphicsSettings, so each choice survives
/// restarts.
pub const DEBUG_RETAIL_SEPARATOR: &str = "────────────────────────────";
pub const DEBUG_RETAIL_LABEL: &str = "Retail+";
/// Makes DLSS selectable in the Graphics menu. Does NOT turn DLSS on — with
/// it off (the default) every DLSS row reads N/A even on capable machines.
pub const RETAIL_DLSS_MENU: &str = "DLSS On/Off";
/// Gates the mob/pet HP readout on nameplates (billboard bar + pct suffix;
/// default off). The row exists only with `enhanced-mob-hp-under`.
pub const RETAIL_MOB_HP_UNDER: &str = "Mob HP Under";
/// Gates the party-frame Job column (retail shows none; default off). The row
/// exists only with `enhanced-job-display`.
pub const RETAIL_JOB_DISPLAY: &str = "Job Display";
/// Makes the Status panel's rows actionable. Retail opens persona, which answers no menu command
/// events (FFXiMain.dll retail-2026-09 RVA 0x12AC60 is a ret-8 stub) — display-only. Default off.
pub const RETAIL_STATUS_ROWS: &str = "Status Rows";
/// Makes re-selecting the already-equipped item in an equip slot unequip it. Retail's equipment
/// handler equips on every select case - no toggle-off branch (FFXiMain.dll retail-2026-09 RVA
/// 0x1322B0, jump table 0x132608; vault main_menu.md §20). Default off = retail re-equip.
pub const RETAIL_RESELECT_UNEQUIP: &str = "Reselect Unequip";
/// Debug dev row: [on] = the player character takes knockback displacement
/// (the retail behavior, the default); off drops the travel only — flinch,
/// facing and the movement lock still play. The row exists only with
/// `enhanced-ignore-knockback-self`.
#[cfg(feature = "enhanced-ignore-knockback-self")]
pub const RETAIL_KNOCKBACK: &str = "Knockback";
/// Retail+ camera leash row: the chase camera's focus dead zone in yalms
/// (camera_collision.rs resolve_camera). Left/Right step it by
/// CAMERA_LEASH_STEP_YALMS; 0 turns the leash off. Persists in
/// GraphicsSettings.
pub const RETAIL_CAMERA_LEASH: &str = "Camera_leash";
/// Retail+ magic row: open the kuluu flat spell list instead of retail's
/// mgcmenu category picker. Kuluu's single-step list has no retail counterpart -
/// the real Magic flow is two windows (FFXiMain.dll retail-2026-09 RVA 0x13F750
/// opens the `magic` spell list through the category rows; vault main_menu.md §21)
/// - so the flat list survives as an off-by-default option.
pub const RETAIL_MAGIC_FLAT_LIST: &str = "Flat Magic List";

/// The client-side school numbers mgcmenu filters on: the internal ids its
/// handler pushes into the `magic` window (FFXiMain.dll retail-2026-09 RVA
/// 0x13F750 jump table 0x13F7CC maps item ids {4..B} to esi ∈ {1,2,5,4,3,6,7,8};
/// vault main_menu.md §21), which are the spell DAT's magicType numbers
/// (ffxi-dat spell_info::MagicType).
const MAGIC_CAT_WHITE: u8 = 1;
const MAGIC_CAT_BLACK: u8 = 2;
const MAGIC_CAT_SUMMONING: u8 = 3;
const MAGIC_CAT_NINJUTSU: u8 = 4;
const MAGIC_CAT_SONGS: u8 = 5;
const MAGIC_CAT_BLUE: u8 = 6;
const MAGIC_CAT_GEOMANCY: u8 = 7;
const MAGIC_CAT_TRUST: u8 = 8;

/// mgcmenu's rows as (client school id, caption) in retail display order - the
/// jump-table shuffle above says row 3 is Songs (5) and row 5 Summoning (3),
/// not numeric order (§21). Captions are d_msg catalog strings: ROM/165/77.DAT
/// indices 246-250 and 252, Blue Magic from ROM/165/76.DAT index 141 (the §10
/// caption catalogs). No short Trust caption exists in any ROM/165 d_msg table,
/// so "Trust Magic" follows the LSB wording [I].
pub const MAGIC_CATEGORY_ROWS: &[(u8, &str)] = &[
    (MAGIC_CAT_WHITE, "White Magic"),
    (MAGIC_CAT_BLACK, "Black Magic"),
    (MAGIC_CAT_SONGS, "Songs"),
    (MAGIC_CAT_NINJUTSU, "Ninjutsu"),
    (MAGIC_CAT_SUMMONING, "Summoning"),
    (MAGIC_CAT_BLUE, "Blue Magic"),
    (MAGIC_CAT_GEOMANCY, "Geomancy"),
    (MAGIC_CAT_TRUST, "Trust Magic"),
];

/// The same labels for the generic static-list path; pairing with
/// [`MAGIC_CATEGORY_ROWS`] is pinned by a test.
const MAGIC_CATEGORY_LABELS: &[&str] = &[
    "White Magic",
    "Black Magic",
    "Songs",
    "Ninjutsu",
    "Summoning",
    "Blue Magic",
    "Geomancy",
    "Trust Magic",
];

/// The client school of the mgcmenu row at display index `row`.
pub fn magic_category_at(row: usize) -> Option<u8> {
    MAGIC_CATEGORY_ROWS.get(row).map(|(cat, _)| *cat)
}

/// The client school a picked mgcmenu caption drills into.
pub fn magic_category_for_label(label: &str) -> Option<u8> {
    MAGIC_CATEGORY_ROWS
        .iter()
        .find(|(_, cap)| *cap == label)
        .map(|(cat, _)| *cat)
}

/// Retail+ abilities row: open the kuluu flat ability list instead of retail's
/// abimenu category picker. Kuluu's single-step list has no retail counterpart -
/// the real Abilities flow is two windows (FFXiMain.dll retail-2026-09 RVA
/// 0x13FA70 opens the `ability` list through its rows, row 6 opening `mount`;
/// vault main_menu.md §23) - so the flat list survives as an off-by-default
/// option.
pub const RETAIL_ABILITIES_FLAT_LIST: &str = "Flat Abilities List";

/// abimenu's five category rows in item-id order (vault main_menu.md §23:
/// items {1,2,3,5} open the `ability` list with filter triples and item 6 opens
/// `menu    mount`). The category semantics per row are [I-pending] on the DLL
/// side; this set is grounded by kuluu's existing target-action groups
/// (AbilityGroup::ALL in hud/action_model.rs, indices referenced positionally)
/// - noted on bead kuluu-o9qt7.7 per the ambiguity rule.
pub const ABILITIES_CATEGORY_LABELS: &[&str] = &[
    "Job Abilities",
    "Weapon Skills",
    "Ranged Attack",
    "Mount",
    "Pet Commands",
];

/// The picker row at display index `idx` as a positional index into
/// `AbilityGroup::ALL` (pairing pinned by a test).
pub fn abilities_category_index(label: &str) -> Option<usize> {
    ABILITIES_CATEGORY_LABELS
        .iter()
        .position(|cap| *cap == label)
}

const DEBUG_ENTRIES: &[&str] = &[
    DEBUG_PERF,
    DEBUG_TARGET_CYCLE,
    DEBUG_MESH,
    DEBUG_NET_STATUS,
    DEBUG_NOCLIP,
    #[cfg(feature = "enhanced-engage-move-lock-off")]
    DEBUG_ENGAGE_ANIM_LOCK,
    DEBUG_AUTO_ENTER_CS,
    DEBUG_WEATHER,
    DEBUG_FOG,
    DEBUG_FORCE_18,
    DEBUG_ENTITY_LIST,
    DEBUG_SOUND,
    DEBUG_VOLUME,
    DEBUG_PRINT_POS,
    DEBUG_STAIR_DRAW,
    DEBUG_STAIR_STATUS,
    DEBUG_GRAPHICS_DEBUG,
    DEBUG_POSITION_LOG,
    DEBUG_NAMEPLATES,
    DEBUG_UI_SETTINGS,
    DEBUG_RETAIL_SEPARATOR,
    DEBUG_RETAIL_LABEL,
    RETAIL_DLSS_MENU,
    #[cfg(feature = "enhanced-mob-hp-under")]
    RETAIL_MOB_HP_UNDER,
    #[cfg(feature = "enhanced-job-display")]
    RETAIL_JOB_DISPLAY,
    RETAIL_STATUS_ROWS,
    RETAIL_RESELECT_UNEQUIP,
    RETAIL_MAGIC_FLAT_LIST,
    RETAIL_ABILITIES_FLAT_LIST,
    #[cfg(feature = "enhanced-ignore-knockback-self")]
    RETAIL_KNOCKBACK,
    RETAIL_CAMERA_LEASH,
];

/// The settings pages whose rows are derived from `GraphicsSection` lists.
/// `None` for every other menu, which uses `static_entries`.
pub fn settings_rows(kind: MenuKind, dlss_supported: bool) -> Option<Vec<MenuRow>> {
    Some(match kind {
        MenuKind::Graphics => graphics_rows(dlss_supported),
        MenuKind::Config => config_rows(),
        MenuKind::GraphicsDlss => dlss_config_rows(),
        _ => return None,
    })
}

pub fn settings_row_at(kind: MenuKind, slot: usize, dlss_supported: bool) -> Option<MenuRow> {
    settings_rows(kind, dlss_supported)?.get(slot).copied()
}

/// The cyclable field at a cursor slot, or `None` on a header or action row.
pub fn settings_field_at(
    kind: MenuKind,
    slot: usize,
    dlss_supported: bool,
) -> Option<GraphicsField> {
    settings_row_at(kind, slot, dlss_supported)?.field()
}

/// Nudge a cursor off a header row in the direction it was already moving,
/// wrapping. Headers are chrome, so the cursor does not rest on one.
pub fn settle_cursor(kind: MenuKind, dlss_supported: bool, cursor: usize, down: bool) -> usize {
    let Some(rows) = settings_rows(kind, dlss_supported) else {
        return cursor;
    };
    if rows.is_empty() {
        return cursor;
    }
    let mut at = cursor.min(rows.len() - 1);
    for _ in 0..rows.len() {
        if rows[at].is_selectable() {
            return at;
        }
        at = if down {
            (at + 1) % rows.len()
        } else {
            at.checked_sub(1).unwrap_or(rows.len() - 1)
        };
    }
    cursor
}

/// Upper bound on a section page: one header per section plus its fields.
/// Callers add their own action rows.
const fn section_row_count(sections: &[GraphicsSection]) -> usize {
    let mut total = 0;
    let mut i = 0;
    while i < sections.len() {
        total += sections[i].fields.len() + 1;
        i += 1;
    }
    total
}

const MAX_ENTRY_COUNT: usize = {
    let r = ROOT_ENTRIES.len();
    let c = section_row_count(CONFIG_SECTIONS) + 1;
    let g = section_row_count(GRAPHICS_SECTIONS) + 2;
    let e = EQUIPMENT_ENTRIES.len();
    let s = STATUS_LABELS.len();

    let gd = section_row_count(DLSS_CONFIG_SECTIONS) + 1;

    let d = DYNAMIC_VISIBLE_ROWS;
    let rc = if r >= c { r } else { c };
    let rcg = if rc >= g { rc } else { g };
    let rcgd = if rcg >= gd { rcg } else { gd };
    let rcge = if rcgd >= e { rcgd } else { e };
    let rcges = if rcge >= s { rcge } else { s };
    if rcges >= d {
        rcges
    } else {
        d
    }
};

pub fn is_dynamic(kind: MenuKind) -> bool {
    matches!(
        kind,
        MenuKind::Magic
            | MenuKind::MagicGroup(_)
            | MenuKind::Abilities
            | MenuKind::AbilitiesGroup(_)
            | MenuKind::Party
            | MenuKind::Trade
            | MenuKind::TradePick { .. }
            | MenuKind::Items
            | MenuKind::KeyItems
            | MenuKind::UsableItems
            | MenuKind::ItemAction { .. }
            | MenuKind::EquipSlot(_)
            | MenuKind::EmoteList
            | MenuKind::Search
            | MenuKind::Linkshell
            | MenuKind::CraftList
            | MenuKind::RegionInfo { .. }
            | MenuKind::MissionList
            | MenuKind::MissionDetail { .. }
            | MenuKind::BazaarStalls
            | MenuKind::QuestList
            | MenuKind::QuestDetail { .. }
            | MenuKind::AutomatonEquip
            | MenuKind::MacroSelect
            | MenuKind::JobPick { .. }
    )
}

pub fn entry_count(kind: MenuKind, dynamic: &DynamicMenu, dlss_supported: bool) -> usize {
    if is_dynamic(kind) {
        dynamic.rows.len().max(1)
    } else {
        entries_for(kind, dlss_supported).len()
    }
}

/// Rows to jump per Left/Right page in a vertical list menu — the visible-row
/// count of whichever panel renders `kind`, so one page-press scrolls a single
/// screenful (retail Left/Right paging). Items and the action-ring Usable list
/// ride the item_screen panel; everything else the generic panel.
pub fn list_page_rows(kind: MenuKind) -> usize {
    match kind {
        MenuKind::Items | MenuKind::UsableItems => crate::hud::item_screen::ITEM_LIST_ROWS,
        _ => DYNAMIC_VISIBLE_ROWS,
    }
}

/// New cursor after a Left/Right page press: jump `rows` toward `forward`
/// (Right/Down = true), clamped to the list — retail pages with no wrap, unlike
/// Up/Down which wrap one row at a time.
pub fn page_cursor(cursor: usize, entry_count: usize, rows: usize, forward: bool) -> usize {
    if forward {
        (cursor + rows).min(entry_count.saturating_sub(1))
    } else {
        cursor.saturating_sub(rows)
    }
}

/// The label at a cursor slot. Dynamic menus read the live row; every other
/// menu reads `static_entries`, which is empty for the section-derived pages
/// (Config/Graphics/GraphicsDlss) — callers that need those rows use
/// `entries_for`, which resolves them through `settings_rows`.
pub fn entry_label(kind: MenuKind, idx: usize, dynamic: &DynamicMenu) -> &str {
    if is_dynamic(kind) {
        if dynamic.rows.is_empty() {
            return empty_dynamic_hint(kind);
        }
        return dynamic
            .rows
            .get(idx)
            .map(|r| r.label.as_str())
            .unwrap_or("<unknown>");
    }
    static_entries(kind)
        .get(idx)
        .copied()
        .unwrap_or("<unknown>")
}

pub fn entry_action(
    kind: MenuKind,
    idx: usize,
    dynamic: &DynamicMenu,
) -> Option<DynamicMenuAction> {
    if !is_dynamic(kind) {
        return None;
    }
    dynamic.rows.get(idx).map(|r| r.action)
}

/// Remaining recast (seconds) for a menu action, or `None` if ready. `now` must
/// be [`kuluu_snapshot::recast_now_unix`] — the clock the 0x119 ABIL_RECAST
/// expiries are stamped with. The single
/// source of truth for both greying a menu row and blocking its confirm. Spells
/// have no snapshot recast field yet (LSB never sends magic recasts), so only
/// job/pet abilities resolve here.
pub fn action_recast_remaining(
    ability_recasts: &[(u16, u32)],
    action: &DynamicMenuAction,
    now: u32,
) -> Option<u32> {
    let ability_id = match action {
        DynamicMenuAction::JobAbility { ability_id }
        | DynamicMenuAction::PetAbility { ability_id } => *ability_id,
        _ => return None,
    };
    let recast_id = ffxi_vocab::recast::ability_recast_id(ability_id)?;
    let expiry = ability_recasts
        .iter()
        .find(|(rid, _)| *rid == recast_id)
        .map(|(_, e)| *e)?;
    let remaining = expiry.saturating_sub(now);
    (remaining > 0).then_some(remaining)
}

fn empty_dynamic_hint(kind: MenuKind) -> &'static str {
    match kind {
        MenuKind::Magic => "(no spells learned yet)",
        MenuKind::MagicGroup(_) => "(no spells learned in this school)",
        MenuKind::Abilities => "(no abilities available — wrong job?)",
        MenuKind::AbilitiesGroup(group) => crate::hud::action_model::AbilityGroup::ALL
            .get(group as usize)
            .map(|g| ability_group_empty_hint(*g))
            .unwrap_or("(empty)"),
        MenuKind::Party => "(not in a party)",
        // A trade window with no transaction behind it (the invite timed out on
        // the server and already sent RES{Cancell}) keeps the §25 hint state.
        MenuKind::Trade => "(not trading)",
        MenuKind::TradePick { .. } => "(nothing to offer)",
        MenuKind::Items => "(bag empty)",
        MenuKind::KeyItems => "(no key items)",
        MenuKind::UsableItems => "(no usable items)",
        MenuKind::ItemAction { .. } => "(item no longer in this bag)",
        MenuKind::EquipSlot(_) => "(no equippable items for this slot)",
        MenuKind::EmoteList => "(emote table unavailable)",
        // The Run row is always present; this hint covers the no-results state
        // (scresult empty before a query has run, vault main_menu.md section 26).
        MenuKind::Search => "(no search results yet)",
        _ => "(empty)",
    }
}

/// Every row label of a page, whichever model backs it.
pub fn entries_for(kind: MenuKind, dlss_supported: bool) -> Vec<&'static str> {
    match settings_rows(kind, dlss_supported) {
        Some(rows) => rows.iter().map(|r| r.label()).collect(),
        None => static_entries(kind).to_vec(),
    }
}

fn static_entries(kind: MenuKind) -> &'static [&'static str] {
    match kind {
        MenuKind::Root => ROOT_ENTRIES,
        MenuKind::Config => &[],
        MenuKind::Controls => CONTROLS_ENTRIES,
        MenuKind::Debug => DEBUG_ENTRIES,
        MenuKind::Graphics => &[],

        // Both magic lists are snapshot-built (`spell_rows` / `magic_school_rows`):
        // an empty one renders its `empty_dynamic_hint`, never a stub.
        MenuKind::Magic => &[],
        MenuKind::MagicGroups => MAGIC_CATEGORY_LABELS,
        MenuKind::MagicGroup(_) => &[],
        // Both ability lists are snapshot-built (`ability_flat_rows` /
        // `ability_group_rows`, fed by the s2c COMMAND_DATA bitmaps), so an empty one
        // renders its `empty_dynamic_hint`, never a stub.
        MenuKind::Abilities => &[],
        MenuKind::AbilitiesGroups => ABILITIES_CATEGORY_LABELS,
        MenuKind::AbilitiesGroup(_) => &[],
        MenuKind::Party => &[],
        MenuKind::Trade => &[],
        MenuKind::TradePick { .. } => &[],
        MenuKind::Search => &[],
        MenuKind::Items => ITEMS_ENTRIES_STUB,
        MenuKind::KeyItems => &[],
        MenuKind::UsableItems => &[],

        MenuKind::Equipment => EQUIPMENT_ENTRIES,

        MenuKind::Status => STATUS_LABELS,

        MenuKind::ItemAction { .. } => &[],
        MenuKind::ItemDropConfirm { .. } => DROP_CONFIRM_ENTRIES,
        MenuKind::PowerPrompt { .. } => DROP_CONFIRM_ENTRIES,
        // The pick list is always snapshot-built (equip_slot_rows); an empty bag
        // renders the `empty_dynamic_hint` for this kind, never a loading stub.
        MenuKind::EquipSlot(_) => &[],

        MenuKind::Communication => COMMUNICATION_ENTRIES,
        // Retail law: the friend roster is POL-era account data with no modern
        // wire — LSB carries SearchType Friend 0x0C enum-only (no handler case)
        // and ships no friends table, so on any LSB server the list is always
        // empty (vault main_menu.md §38.5; the live system of that family is the
        // black list, which kuluu may surface separately — not via this row).
        MenuKind::FriendList => &[],
        // The pupequip variant's fourteen rows come from s2c 0x044 (dynamic;
        // automaton_equip_rows, vault main_menu.md §39).
        MenuKind::AutomatonEquip => &[],
        // `mp_mncst` chrome carries only three unnamed buttons and no captions
        // (DAT mp_1, vault main_menu.md §39) — the grounded empty display.
        MenuKind::MpCustomize => &[],
        // The myroom home-view ops panel: fourteen captioned widgets read from
        // the DAT chunk (vault main_menu.md §40 as corrected by §53).
        MenuKind::HomeView => HOME_VIEW_ROWS,
        // jobchang's two widgets — catalog tokens 65/66 in DAT record 'jobc'
        // (vault main_menu.md §53); the pick lists are roster-built.
        MenuKind::JobChange => JOB_CHANGE_ROWS,
        MenuKind::JobPick { .. } => &[],
        MenuKind::EmoteList => &[],
        MenuKind::GraphicsDlss => &[],
        // The Map screen renders its own bespoke panes; it has no generic list.
        MenuKind::Map => &[],
        // Slot rows come from the server state, never a static table (vault main_menu.md section 27).
        MenuKind::Linkshell => &[],
        // The cmbmenu launcher's two fixed rows: select ids 1-2 of handler FFXiMain.dll
        // retail-2026-09 RVA 0x1E29A0 (bag-pick, then craft list; vault main_menu.md §22/§28). No DAT
        // caption was recovered for either row (§22 caption law), so kuluu's labels are pinned by tests.
        MenuKind::SynthLauncher => SYNTH_LAUNCHER_ENTRIES,
        // The mcrmenu launcher's two fixed rows (handler FFXiMain.dll
        // retail-2026-09 RVA 0x1A64B0; vault main_menu.md §34).
        MenuKind::MacrosPalette => MACRO_LAUNCHER_ENTRIES,
        // mcrselec rows arrive from the store (dynamic; macro_select_rows and
        // vault main_menu.md §41). mcres20 shows run results, of which kuluu
        // has none until macros execute — the grounded empty display.
        MenuKind::MacroSelect => &[],
        MenuKind::MacroResult => &[],
        // The configwi hub's twelve page rows (handler FFXiMain.dll
        // retail-2026-09 RVA 0x196770; vault main_menu.md §35).
        MenuKind::RetailConfig => RETAIL_CONFIG_ENTRIES,
        // A page window's rows are its own DAT option widgets (handler per
        // vault main_menu.md §47; the empty pages carry only anonymous records).
        MenuKind::RetailConfigPage(page) => match usize::from(page) {
            idx if idx < RETAIL_CONFIG_PAGE_ROWS.len() => RETAIL_CONFIG_PAGE_ROWS[idx],
            _ => &[],
        },
        MenuKind::RetailConfigSub(sub) => match usize::from(sub) {
            idx if idx < RETAIL_CONFIG_SUB_ROWS.len() => RETAIL_CONFIG_SUB_ROWS[idx],
            _ => &[],
        },
        // The faqsub Help Desk panel rows (vault main_menu.md §36: the
        // big-widget caption builder loads exactly these destination strings).
        MenuKind::HelpDeskPanel => HELPDESK_PANEL_ENTRIES,
        // cmbhlst-style rows come from the recipe table filtered against the bag (vault main_menu.md §28).
        MenuKind::CraftList => &[],
        MenuKind::RegionInfo { .. } => &[],
        MenuKind::MissionList => &[],
        MenuKind::MissionDetail { .. } => &[],
        // Stall rows come from the zone entity stream (vault main_menu.md §33).
        MenuKind::BazaarStalls => &[],
        MenuKind::QuestList => &[],
        MenuKind::QuestDetail { .. } => &[],
    }
}

pub fn menu_title(kind: MenuKind) -> &'static str {
    match kind {
        MenuKind::Root => "Commands",
        MenuKind::Config => "Interface",
        MenuKind::Controls => "Controls",
        MenuKind::Debug => "Debug",
        MenuKind::Graphics => "Graphics",
        MenuKind::Equipment => "Equipment",
        MenuKind::Magic => "Magic",
        // The picker keeps the Magic row's own caption; the spell list it opens is
        // titled `Magic List` (ROM/165/76.DAT catalog index 76, §10 - which one the
        // root row uses stays [I], the list title is the grounded side).
        MenuKind::MagicGroups => "Magic",
        MenuKind::MagicGroup(_) => "Magic List",
        MenuKind::Abilities => "Abilities",
        // Like Magic's pair: the picker carries the row caption, the filtered
        // list window is titled by the row it answers (vault §23).
        MenuKind::AbilitiesGroups => "Abilities",
        MenuKind::AbilitiesGroup(_) => "Ability List",
        // The prtyN family's DAT records carry no inline captions (§24); the
        // Root-row caption is the grounded title choice. Neither does `trade`
        // or `gift` (vault main_menu.md §25), so kuluu titles its mirrored
        // pair-window the same way.
        MenuKind::Party => "Party",
        MenuKind::Trade => "Trade",
        MenuKind::Search => "Search",
        // Neither `lscswin` nor the `linkN` records carry inline captions (vault main_menu.md
        // section 27.5), so kuluu titles its slot window after the Root row, like Party/Trade/Search.
        MenuKind::Linkshell => "Link Shell",
        MenuKind::SynthLauncher => "Synthesis",
        // DAT `mcrm` carries comwin title token 80 = catalog id 80
        // `MacroPalette` (vault main_menu.md §34) — the grounded title.
        MenuKind::MacrosPalette => "MacroPalette",
        // Neither `mcrselec` nor `mcres20` carries a DAT title token (both
        // comwin records read -1; vault main_menu.md §41), so both inherit the
        // parent row's caption like the other token-less windows here.
        MenuKind::MacroSelect => "Macros",
        MenuKind::MacroResult => "Macros",
        // DAT `conf` carries comwin title token 54 = catalog id 54 `Config`
        // (vault main_menu.md §35) — the grounded title.
        MenuKind::RetailConfig => "Config",
        // No config page window carries a usable comwin title token (vault
        // main_menu.md §47), so each inherits the hub's grounded title.
        MenuKind::RetailConfigPage(_) => "Config",
        MenuKind::RetailConfigSub(_) => "Config",
        // The `faqs` DAT record carries no title (token -1); the grounded
        // caption is catalog string 60 `Help Desk` in ROM/165/76.DAT — same as
        // the Root row's own label (§36).
        MenuKind::HelpDeskPanel => "Help Desk",
        MenuKind::CraftList => "Craft List",
        MenuKind::RegionInfo { .. } => "Region Info",
        MenuKind::MissionList => "Missions",
        MenuKind::MissionDetail { .. } => "Mission Log",
        MenuKind::BazaarStalls => "Bazaar",
        MenuKind::QuestList => "Quests",
        MenuKind::QuestDetail { .. } => "Quest Log",
        // The bag picker hung off the trade window's Add-Item button row; its
        // own caption is [I] like the rest of the family (§25).
        MenuKind::TradePick { .. } => "Add Item",
        MenuKind::Items => "Items",
        MenuKind::KeyItems => "Key Items",
        MenuKind::UsableItems => "Items",
        MenuKind::ItemAction { .. } => "Item",
        MenuKind::ItemDropConfirm { .. } => "Drop",
        // Retail titles the prompt after the command row that opened it.
        MenuKind::PowerPrompt {
            shut_down: true, ..
        } => ROOT_SHUT_DOWN,
        MenuKind::PowerPrompt {
            shut_down: false, ..
        } => ROOT_LOG_OUT,
        MenuKind::Status => "Status",
        MenuKind::EquipSlot(_) => "Equip",
        MenuKind::Communication => "Communication",
        MenuKind::FriendList => "Friend List",
        // The pupequip DAT comwin record carries no title token (vault
        // main_menu.md §39), so the variant keeps the parent row's caption.
        MenuKind::AutomatonEquip => "Equipment",
        // DAT mp_1's comwin record carries title token 165, and the caption
        // catalog resolves it to `Customization` (vault main_menu.md §39).
        MenuKind::MpCustomize => "Customization",
        // The `myro` DAT record carries no title token (comwin `anc anc_s -1`,
        // vault main_menu.md §40), so — as with the pupequip variant — kuluu
        // inherits the parent row's caption.
        MenuKind::HomeView => "Map",
        // jobchang's frame carries no title token of its own in DAT record
        // 'jobc' (vault main_menu.md §53); catalog caption 67 grounds both it
        // and the pick lists drilled from its rows.
        MenuKind::JobChange => "Change Jobs",
        MenuKind::JobPick { .. } => "Change Jobs",
        MenuKind::EmoteList => "Emote List",
        MenuKind::GraphicsDlss => "DLSS Config",
        MenuKind::Map => "Map",
    }
}

/// MenuKind::Party rows, following the Party-row helper's state dispatch
/// (FFXiMain.dll retail-2026-09 RVA 0x2016E0; vault main_menu.md §24): while a
/// solicit is unresolved it opens `prtyjoin`, whose answer rows are YES/NO
/// (c2s GROUP_SOLICIT_RES Res=1/0, vendor/server/src/map/packets/c2s/
/// 0x074_group_solicit_res.h); otherwise the window shows the member roster,
/// and a solo player sees the display-only empty state (`prty1`).
pub fn party_window_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    if let Some(invite) = &snap.party_invite {
        return vec![
            DynamicMenuRow {
                label: format!("Yes (join {})", invite.name),
                action: DynamicMenuAction::AcceptPartyInvite,
            },
            DynamicMenuRow {
                label: "No".to_string(),
                action: DynamicMenuAction::DeclinePartyInvite,
            },
        ];
    }
    snap.party
        .iter()
        .map(|m| DynamicMenuRow {
            label: m.name.clone().unwrap_or_else(|| format!("{:08X}", m.id)),
            action: DynamicMenuAction::PartyMemberRow { char_id: m.id },
        })
        .collect()
}

/// Label for one cell of the trade slot grid from its s2c view row. Neither
/// `trade` nor `gift` carries inline DAT captions (vault main_menu.md §25), so
/// these labels are kuluu's choice, pinned by tests; the 0xFFFF sentinel IS the
/// gil cell per player_trade.cpp.
fn trade_cell_label(view: Option<kuluu_snapshot::TradeOffer>, is_gil_slot: bool) -> String {
    match view {
        Some(offer) if is_gil_slot => format!("Gil x{}", offer.qty),
        Some(offer) => {
            let name = ffxi_vocab::item_names::lookup(offer.item_no)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{:#06X}", offer.item_no));
            if offer.qty > 1 {
                format!("{name} x{}", offer.qty)
            } else {
                name
            }
        }
        None if is_gil_slot => "Gil".to_string(),
        None => "(empty)".to_string(),
    }
}

/// Rows for the trade window pair — retail opens `trade` (own side, selectable)
/// and `gift` (partner view, display-only) together as one state machine
/// (FFXiMain.dll retail-2026-09 vtable 0x335060 handler RVA 0x1515E0; vault
/// main_menu.md §25). kuluu mirrors the pair as one menu: while s2c 0x021 is
/// unanswered it shows the answer rows, else gil (trade slot 0 per LSB's
/// `GilSlot = 0`), own item slots 1..8 (select byte table .rdata 0x371D60),
/// then the button law {9=Confirm, 10=Exit}, and finally the partner-side
/// views appended as display rows.
pub fn trade_window_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let Some(trade) = snap.trade.as_deref() else {
        return Vec::new();
    };
    if trade.incoming_pending {
        let name = snap
            .entities
            .iter()
            .find(|e| e.id == trade.partner_id)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| format!("{:08X}", trade.partner_id));
        return vec![
            DynamicMenuRow {
                label: format!("Yes (accept {})", name),
                action: DynamicMenuAction::TradeAcceptInvite,
            },
            DynamicMenuRow {
                label: "No".to_string(),
                action: DynamicMenuAction::TradeDeclineInvite,
            },
        ];
    }
    let mut rows = Vec::new();
    rows.push(DynamicMenuRow {
        label: trade_cell_label(trade.my_slots.first().copied().flatten(), true),
        action: DynamicMenuAction::TradeGilRow,
    });
    for idx in 1..ffxi_proto::decode::TRADE_SLOT_COUNT {
        rows.push(DynamicMenuRow {
            label: trade_cell_label(trade.my_slots.get(idx).copied().flatten(), false),
            action: DynamicMenuAction::TradeOwnSlot {
                trade_index: idx as u8,
            },
        });
    }
    rows.push(DynamicMenuRow {
        label: if trade.my_confirmed {
            "Confirm (waiting for partner)".to_string()
        } else {
            "Confirm".to_string()
        },
        action: DynamicMenuAction::TradeConfirmRow,
    });
    rows.push(DynamicMenuRow {
        label: "Exit".to_string(),
        action: DynamicMenuAction::TradeCancelRow,
    });
    for idx in 0..ffxi_proto::decode::TRADE_SLOT_COUNT {
        rows.push(DynamicMenuRow {
            label: trade_cell_label(trade.partner_slots.get(idx).copied().flatten(), idx == 0),
            action: DynamicMenuAction::TradePartnerOfferRow {
                trade_index: idx as u8,
            },
        });
    }
    rows
}

/// Add-Item bag picker rows for one own-side slot (retail opens this from an
/// empty slot row; vault main_menu.md §25 [I-pending] keeps the qty keypad
/// undecoded, so kuluu stages ONE per pick — the minimum legal ItemNum under
/// player_trade.cpp setSlot). Locked and server-unselectable slots are skipped:
/// LSB's own validation refuses them (Exclusive flag / ItemLockFlg::NoSelect).
pub fn trade_pick_rows(
    snap: &kuluu_snapshot::SceneSnapshot,
    trade_index: u8,
) -> Vec<DynamicMenuRow> {
    snap.inventory_main()
        .iter()
        .filter(|slot| !slot.locked && !slot.unselectable)
        .filter_map(|slot| {
            let name = ffxi_vocab::item_names::lookup(slot.item_no)?;
            Some(DynamicMenuRow {
                label: if slot.quantity > 1 {
                    format!("{name} x{}", slot.quantity)
                } else {
                    name.to_string()
                },
                action: DynamicMenuAction::TradeStageFromBag {
                    trade_index,
                    item_no: slot.item_no,
                    qty: 1,
                    bag_slot: slot.index,
                },
            })
        })
        .collect()
}

/// Rows of the Search surface — one menu for retail's `searchma` form and its
/// `scresult` model list (vault main_menu.md §26). The Run row leads (select id
/// 1 → query-object method 0x20EE70), then one row per hit in server order.
/// Label law follows the AddPlayer fields: name, job abbrev + levels (`???` for
/// the hidden shape behind flags1 & 0x4000 — those entries vanish from the
/// frame), and zone id. kuluu has no filter editor yet, so Run sends an empty
/// entry list, which LSB's _HandleSearchRequest defaults read as "everyone".
/// The form sub-windows (scname/searchjo/searchna/… select ids 2-7,17) are the
/// documented follow-ups for this bead family.
pub fn search_window_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut rows = vec![DynamicMenuRow {
        label: "Run Search".to_string(),
        action: DynamicMenuAction::SearchRun,
    }];
    let Some(results) = snap.search.as_deref() else {
        return rows;
    };
    for (index, hit) in results.players.iter().enumerate() {
        let shape = match (hit.main_job, hit.main_level) {
            (Some(job), Some(level)) => {
                let job = ffxi_vocab::job_names::abbrev(u16::from(job)).unwrap_or("??");
                let sub = match (hit.sub_job, hit.sub_level) {
                    (Some(sjob), Some(slvl)) if slvl > 0 => format!(
                        "/{}{slvl}",
                        ffxi_vocab::job_names::abbrev(u16::from(sjob)).unwrap_or("??")
                    ),
                    _ => String::new(),
                };
                format!("{job}{level}{sub}")
            }
            _ => "???".to_string(),
        };
        rows.push(DynamicMenuRow {
            label: format!("{}  {shape}  Z{}", hit.name, hit.zone),
            action: DynamicMenuAction::SearchHitRow { index },
        });
    }
    rows
}

/// The submenu a Root category drills into, and thus the level the right pane
/// previews while that category is highlighted at Root. Single source for the
/// Root → submenu mapping: `text_input::resolve_menu_entry` drills through this
/// so the client dispatch and the preview can't disagree (guarded by a test in
/// the client). `None` = a Root entry with no browsable submenu (Macros isn't
/// built; Current Time/Key Items run their retail text commands, Log Out/Shut
/// Down act directly), so the right pane shows no preview.
/// `flat_magic` is the Retail+ "Flat Magic List" gate
/// ([`RETAIL_MAGIC_FLAT_LIST`] in GraphicsSettings): on, kuluu's one-step spell
/// list replaces retail's two-window flow (vault main_menu.md §21).
/// `flat_abilities` is the same pattern for Abilities
/// ([`RETAIL_ABILITIES_FLAT_LIST`]; vault main_menu.md §23).
/// Row law for the Link Shell window (vault main_menu.md section 27): retail's `menu    link5`
/// mirrors the two shell slots LSB tracks (`LinkshellNum` 1/2), and all the server ever says about a
/// slot is which inventory item is its pearl - s2c 0x0E0 `{ LinkshellNum, ItemIndex, Category }`,
/// zeros meaning "nothing equipped". The shell *name* lives in the pearl's exdata (which kuluu does
/// not ingest yet) and retail's own row captions are string-table ids (section 27.5), so rows stay
/// slot-labelled rather than invented.
pub fn linkshell_window_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    [1u8, 2]
        .into_iter()
        .map(|slot| {
            let state = snap.linkshells.slot(slot).copied().unwrap_or_default();
            let label = if state.is_occupied() {
                format!(
                    "Link Shell {}   pearl at container {}, index {}",
                    slot, state.category, state.item_index
                )
            } else {
                format!("Link Shell {}   (no pearl equipped)", slot)
            };
            DynamicMenuRow {
                label,
                action: DynamicMenuAction::LinkShellSlot {
                    linkshell_slot: slot,
                },
            }
        })
        .collect()
}

/// The cmbmenu launcher's two rows. Neither window carries DAT captions (§22 caption law: the
/// `cmbmenu`/`cmbhlst` mini-chunks are chrome-only and no short craft strings exist in the ROM/165
/// message catalog), so these kuluu-picked labels stand pinned by tests; their order follows
/// select ids 1-2 of handler FFXiMain.dll retail-2026-09 RVA 0x1E29A0 (vault main_menu.md §22/§28).
pub const SYNTH_ROW_PICK_MATERIAL: &str = "Synthesize from a bag item";
pub const SYNTH_ROW_OPEN_LIST: &str = "Open the craft list";
const SYNTH_LAUNCHER_ENTRIES: &[&str] = &[SYNTH_ROW_PICK_MATERIAL, SYNTH_ROW_OPEN_LIST];

/// The mcrmenu launcher's two rows. DAT `mcrm` carries no captions — the
/// windowps pair is chrome-only (vault main_menu.md §34) — so these
/// kuluu-picked labels stand pinned by tests; their order follows select ids
/// {1,2} of handler FFXiMain.dll retail-2026-09 RVA 0x1A64B0: item 1 opens
/// `mcrselec`, item 2 opens `mcres20`.
pub const MACRO_ROW_SELECT_MACRO: &str = "Select Macro";
pub const MACRO_ROW_MACRO_RESULT: &str = "Macro Result";
const MACRO_LAUNCHER_ENTRIES: &[&str] = &[MACRO_ROW_SELECT_MACRO, MACRO_ROW_MACRO_RESULT];

/// The configwi hub's twelve rows, captioned from the DAT: each `conf` chunk widget
/// `windowpsNNN` indexes catalog table 11 = ROM/165/75.DAT at its own suffix id — the
/// windowpsNNN caption law (vault main_menu.md §43; supersedes §35's chrome-only reading).
/// Row order is the chunk's widget file order [V]; the handler arms of FFXiMain.dll
/// retail-2026-09 RVA 0x196792.. name each row's page window (§35/§42 literal run
/// .data 0x37A618..), and that arm↔widget pairing stays [I] until the arms are walked.
pub const RETAIL_CONFIG_ENTRIES: &[&str] = &[
    "Adjust the keyboard settings.",
    "Adjust in-game settings exclusive to FINAL FANTASY XI.",
    "Adjust the filter settings for text in the log window.",
    "Adjust the color of different types of text in the log window.",
    "Adjust log window settings.",
    "Adjust miscellaneous settings for FINAL FANTASY XI.",
    "Remove onscreen battle effects to improve frame rate.",
    "Adjust the mouse and camera settings.",
    "Adjust global options that affect all of PlayOnline.",
    "Adjust other miscellaneous settings for FINAL FANTASY XI.",
    "Designate which messages appear in which window. Only active when multiple log windows are displayed.",
    "Adjust further miscellaneous settings for FINAL FANTASY XI.",
];

/// The twelve `configwi` page windows' option rows, indexed by hub row (0..11 = the
/// hub handler's select-id 1..12; FFXiMain.dll retail-2026-09 RVA 0x196770 jump table
/// .text 0x1968A4 → name-open). Each list is that page window's own kind-30 DAT chunk
/// in ROM/0/1.DAT, `menu    windowpsNNN` records in file order, with the caption read
/// from catalog table 11 = ROM/165/75.DAT at the suffix id (the §43 caption law, which
/// vault main_menu.md §47 shows is also the row-set law: the same ids are the option
/// ids the page handler writes through the shared option store at [0x106626CC]).
///
/// Two page windows (`conf6win`, `conf13wi`) have no record at all in the
/// ROM/0/1.DAT generation, so their lists come from the newer DAT set where each
/// root matches the DLL's name-open verbatim (vault main_menu.md §49).
///
/// Two pages stay empty — `cfilter` and `fxfilter` repeat anonymous sub-records
/// (the filter word slots / the per-effect toggles) in **both** generations, so
/// their rows have to be recovered from the shared handler's arms; open work.
pub const RETAIL_CONFIG_ROWS_CONF2WIN: &[&str] = &[
    "Automatically lock on to target at the start of battle.",
    "Do not automatically lock on to target.",
    "Play music in stereo.",
    "Play music in monaural sound.",
    "Set the volume of sound effects.",
    "Set the volume of the background music.",
    "Set the level of screen flicker reduction.",
    "Automatically sort items as they enter your inventory.",
    "Disable the autosort function, and opt to manually sort your inventory.",
    "Sort items by category when organizing them.",
    "Sort items by name when organizing them and stacking them when possible.",
];
const RETAIL_CONFIG_ROWS_EMPTY: &[&str] = &[];
const RETAIL_CONFIG_ROWS_CONFTXT: &[&str] =
    &["Set the font colors for actions that affect others."];
const RETAIL_CONFIG_ROWS_CONF5M: &[&str] = &[
    "Adjust shared window settings.",
    "Adjust individual window settings.",
];
pub const RETAIL_CONFIG_ROWS_CONF3WIN: &[&str] = &[
    "Display damage in the log window only.",
    "Display damage on the game screen and in the log window.",
    "Make FINAL FANTASY XI character data public for party searches and Friend List searches.",
    "Hide FINAL FANTASY XI character data from Friend List searches.",
    "Turn on the DUALSHOCKi2 analog controller vibration function.",
    "Turn off the DUALSHOCKi2 analog controller vibration function.",
    "Adjust the strength of the vibration function.",
    "Set display to standard TV screen size.",
    "Set display to wide-screen TV size.",
    "Display damage as pop-up graphics on the game screen only. Damage will not be shown in the log window.",
    "Adjust the screen's brightness.",
];
pub const RETAIL_CONFIG_ROWS_CONF7: &[&str] = &[
    "Use cursor as point of reference for movement.",
    "Use your character as point of reference for movement.",
    "The camera will rotate when the mouse pointer is moved to the edge of the screen.",
    "The camera will not rotate when the mouse pointer is moved to the edge of the screen.",
    "Allows unrestricted movement of the third-person camera.",
    "Locks the third-person camera behind your character.",
    "Reset third-person camera controls to the default setting.",
    "Invert third-person camera controls on the Y axis.",
    "Invert third-person camera controls on the X axis.",
    "Reset first-person camera controls to the default setting.",
    "Invert first-person camera controls on the Y axis.",
    "Invert first-person camera controls on the X axis.",
];
const RETAIL_CONFIG_ROWS_CONF4: &[&str] =
    &["Filter out words that could be interpreted as offensive."];
const RETAIL_CONFIG_ROWS_CONF11M: &[&str] = &[
    "Designate which logs appear in which window. Only active when multiple windows are displayed.",
];
pub const RETAIL_CONFIG_ROWS_CONF12WI: &[&str] = &[
    "Display party member status icons.",
    "Hide party member status icons.",
    "Display status icon timers.",
    "Hide status icon timers.",
    "Enable camera collision on Mog House furniture.",
    "Disable camera collision on Mog House furniture.",
];

/// `conf6win` — this window has no record at all in ROM/0/1.DAT; its rows come
/// from the newer DAT generation where every record root matches the DLL's
/// name-open literal (chunk 'co13' root `conf6win`, ROM/119/51.DAT file-off
/// 0x28B470 size 0x470; vault main_menu.md §49), captions from table 11.
pub const RETAIL_CONFIG_ROWS_CONF6WIN: &[&str] = &[
    "Adjust the distance of the back clipping plane of the 3D view.",
    "Display footstep effects such as footprints and ripples.",
    "Remove footstep effects to increase frame rate.",
    "Limit character animation rate to maintain frame rate.",
    "Normal input method for full-sized keyboards.",
    "Specialized input method for compact keyboards without numeric keypads.",
    "Adjust the background screen proportions to match your monitor's aspect ratio. Recommended for monitors with an aspect ratio other than 4:3.",
    "Assign keys for movement, cursor, and camera controls. (Compact keyboard only)",
    "Assign keys for shortcut commands.",
    "Return key assignments to default settings.",
    "New input method supporting scroll wheel and keyboard/mouse combination controls.",
    "Adjust the size of the macro palette.",
    "Shift macro palette to the right during combat.",
    "Lock macro palette to the right at all times.",
];
/// `conf13wi` — likewise record-less in ROM/0/1.DAT; grounded from chunk 'co22'
/// root `conf13wi` (ROM/119/51.DAT file-off 0x29D920 size 0x200) per vault
/// main_menu.md §49.
pub const RETAIL_CONFIG_ROWS_CONF13WI: &[&str] = &[
    "Disable software keyboard.",
    "Use standard software keyboard.",
    "Use compact software keyboard.",
    "Hide certain chat messages that contain terms designated by your filter settings. Presssing this button will reflect your edits.",
    "Hide certain chat messages sent by player characters designated by your filter settings. Pressing this button will reflect your edits.",
    "Term Filter disabled.",
];

/// Indexed by hub row (0-based). The two empty entries are `cfilter` and
/// `fxfilter`: those windows carry only anonymous repeated sub-records in both
/// DAT generations, so their rows have to come from the handler side
/// (`FFXiMain.dll retail-2026-09 RVA 0x197BE0`; vault main_menu.md §47/§49).
pub const RETAIL_CONFIG_PAGE_ROWS: [&[&str]; 12] = [
    RETAIL_CONFIG_ROWS_CONF2WIN,
    RETAIL_CONFIG_ROWS_EMPTY,
    RETAIL_CONFIG_ROWS_CONFTXT,
    RETAIL_CONFIG_ROWS_CONF5M,
    RETAIL_CONFIG_ROWS_CONF3WIN,
    RETAIL_CONFIG_ROWS_CONF6WIN,
    RETAIL_CONFIG_ROWS_EMPTY,
    RETAIL_CONFIG_ROWS_CONF7,
    RETAIL_CONFIG_ROWS_CONF4,
    RETAIL_CONFIG_ROWS_CONF11M,
    RETAIL_CONFIG_ROWS_CONF12WI,
    RETAIL_CONFIG_ROWS_CONF13WI,
];

pub const CONFIG_ROW_SHARED_LOG_WINDOWS: &str = "Adjust shared window settings.";
pub const CONFIG_ROW_INDIVIDUAL_LOG_WINDOWS: &str = "Adjust individual window settings.";

/// `conf5win` — its DAT chunk rows (vault main_menu.md §47, chunk 'co11' root
/// `conf5`).
pub const RETAIL_CONFIG_ROWS_CONF5WIN: &[&str] = &[
    "Display separate log windows.",
    "Display only one log window.",
    "Display time stamps down to the minute in the log window.",
    "Display time stamps down to the second in the log window.",
    "Stop displaying timestamps in the log window(s).",
    "Display opening/closing visual effects for windows.",
    "Do not display opening/closing visual effects for windows.",
];
/// `conf5w1` and `conf5w2` — the two per-window log pages (DAT chunks 'co16'
/// root `conf5p1` and 'co17' root `conf5p2`; the p1↔w1 / p2↔w2 pairing is by
/// record suffix, [I] in vault main_menu.md §47).
pub const RETAIL_CONFIG_ROWS_LOG_PAGE: &[&str] = &[
    "Automatically adjust log window's size to match amount of text.",
    "Fix log window to set size.",
    "Set the time interval for automatic log window adjustment.",
];

/// Indexed by `MenuKind::RetailConfigSub` index. `k1assign`/`k2assign` carry only
/// key-slot chrome in their DAT records, so they have no captioned rows yet.
pub const RETAIL_CONFIG_SUB_ROWS: [&[&str]; 5] = [
    RETAIL_CONFIG_ROWS_CONF5WIN,
    RETAIL_CONFIG_ROWS_LOG_PAGE,
    RETAIL_CONFIG_ROWS_LOG_PAGE,
    &[],
    &[],
];

/// Which `configwi` page window a hub row opens, per the handler's jump table
/// (FFXiMain.dll retail-2026-09 RVA 0x1968A4; vault main_menu.md §47). Returns the
/// 0-based page index for a hub caption.
pub fn retail_config_page_for_row(label: &str) -> Option<u8> {
    RETAIL_CONFIG_ENTRIES
        .iter()
        .position(|row| *row == label)
        .and_then(|idx| u8::try_from(idx).ok())
}

/// The faqsub Help Desk panel's destinations. The DAT `faqs` chunk is
/// chrome-only with an empty title token (-1), so labels come from the
/// window's own caption builder strings (vault main_menu.md §36): table 9 =
/// ROM/165/72.DAT idx 0xC3 `Q&A`, 0xCF `Mentor Search`, 0xDD `Config` — the
/// three targets named by the transfer messages at 0xD7/0xD8/0xD9.
pub const FAQ_ROW_QA: &str = "Q&A";
pub const FAQ_ROW_MENTOR: &str = "Mentor Search";
pub const FAQ_ROW_CONFIG: &str = "Config";
const HELPDESK_PANEL_ENTRIES: &[&str] = &[FAQ_ROW_QA, FAQ_ROW_MENTOR, FAQ_ROW_CONFIG];

/// The myroom home-view panel's captioned widgets, verbatim from the DAT
/// chunk `myro` (ROM/0/1.DAT file-off 0x282D80 size 0x4D0) windowps tokens,
/// each decoded against ROM/165/76.DAT. The full fourteen named widgets —
/// the §40 reading of six of them as "chrome" was wrong: their caption tokens
/// label real buttons (vault main_menu.md §53 re-read): 168 M.Ward., 3 Mog
/// Safe, 37 Storage, 138 Mog Locker, 151 Satchel, 147 Open Mog House, 166
/// Mog Case, 158 Mog Sack, 87 Delivery Box, 67 Change Jobs, 89 Gardening, 4
/// Layout, 181 Safe 2, 182 Remodel. Display order is fixed twice over by the
/// geometry y column (0x15..0xE5 stride 0x10) and the per-record prev/next
/// chain ring. The handler answers select ids 1..7
/// (FFXiMain.dll retail-2026-09 RVA 0x145460 jump table .rdata 0x1455BC):
/// id1 storage, id2 mogpost, id3 jobchang, id4 "/garden", id5 "/layout" (the
/// §40 literal pairing was swapped), id6 furniture-edit mode, id7 roomlist.
pub const HOME_VIEW_ROWS: &[&str] = &[
    "M.Ward.",
    "Mog Safe",
    "Storage",
    "Mog Locker",
    "Satchel",
    "Open Mog House",
    "Mog Case",
    "Mog Sack",
    "Delivery Box",
    "Change Jobs",
    "Gardening",
    "Layout",
    "Safe 2",
    "Remodel",
];

/// `jobchang`'s two rows, verbatim from the DAT record 'jobc' (ROM/0/1.DAT):
/// windowps273/274 with catalog tokens 65 "Main Job" and 66 "Support Job"
/// (vault main_menu.md §53).
pub const JOB_CHANGE_ROWS: &[&str] = &["Main Job", "Support Job"];

/// The container an openable home-view row browses, keyed by the DAT caption.
/// The bag identities follow the LSB CONTAINER_ID enum (vendor/server/src/map/
/// item_container.h: MOGSAFE=1, STORAGE=2, MOGLOCKER=4, MOGSATCHEL=5, MOGSACK=
/// 6, MOGCASE=7, WARDROBE=8, MOGSAFE2=9 — vault main_menu.md §53); the 'M.Ward.'
/// caption is the wardrobe bag. Which of the two window classes `storage`/
/// `storage2` (name-opened by select id 1's holder method 0x145C70) binds to
/// which bag stays unmapped — those windows have no DAT record to read it from
/// [V-negative], so kuluu maps the "Storage" caption to its same-named LSB bag.
pub fn home_view_row_container(label: &str) -> Option<u8> {
    use ffxi_proto::map::container as c;
    match label {
        "Mog Safe" => Some(c::LOC_MOGSAFE),
        "Storage" => Some(c::LOC_STORAGE),
        "Mog Locker" => Some(c::LOC_MOGLOCKER),
        "Satchel" => Some(c::LOC_MOGSATCHEL),
        "Mog Case" => Some(c::LOC_MOGCASE),
        "Mog Sack" => Some(c::LOC_MOGSACK),
        "M.Ward." => Some(c::LOC_WARDROBE),
        "Safe 2" => Some(c::LOC_MOGSAFE2),
        _ => None,
    }
}

/// The two home-view rows that are text commands (select ids 4 and 5 of handler
/// FFXiMain.dll retail-2026-09 RVA 0x145460, arms @0x145541/@0x14554A): each
/// runs its literal through the runner (0x80DC0) and closes myroom by name via
/// 0x15E910. The §40 pairing of literals to ids was swapped — fresh read: id 4
/// pushes .data 0x3715F0 "/garden", id 5 pushes .data 0x3715E8 "/layout".
/// LSB ships neither command (C++ nor scripts/commands), so like /clock the
/// literal never reaches an LSB server as anything meaningful [V-negative].
pub fn home_view_text_command(label: &str) -> Option<&'static str> {
    match label {
        "Gardening" => Some("/garden"),
        "Layout" => Some("/layout"),
        _ => None,
    }
}

/// cmbhlst-shaped rows: every recipe the server could currently resolve into a synth — crystal
/// tier substitutable by element and all positional ingredients assignable to bag slots under
/// LSB's per-stack use law (vault main_menu.md §28; `resolveRecipe`,
/// `GP_CLI_COMMAND_COMBINE_ASK::process`). The row carries its full ask plan so the select is a
/// straight send. Rows keep recipe-id order for stable paging.
pub fn craft_list_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let bag: Vec<(u8, u16, u32)> = snap
        .inventory_main()
        .iter()
        .filter(|slot| slot.item_no != 0 && slot.quantity > 0)
        .map(|slot| (slot.index, slot.item_no, slot.quantity))
        .collect();
    let mut rows = Vec::new();
    for recipe in ffxi_vocab::craft::SYNTH_RECIPES {
        // A key-item gate means the server cancels without it (`resolveRecipe`).
        if recipe.key_item != 0 && !snap.key_items.contains(&(recipe.key_item as u16)) {
            continue;
        }
        let Some((crystal_slot, held_crystal_id)) = bag.iter().find_map(|(slot, id, _)| {
            ffxi_vocab::craft::crystal_accepts(recipe.crystal, *id).then_some((*slot, *id))
        }) else {
            continue;
        };
        let needed: Vec<u16> = recipe
            .ingredients
            .iter()
            .copied()
            .filter(|&i| i != 0)
            .collect();
        let Some(entries) = ffxi_proto::synth::assign_ingredient_slots(&needed, &bag, crystal_slot)
        else {
            continue;
        };
        let mut plan: [Option<ffxi_proto::synth::SynthIngredient>; 8] = [None; 8];
        for (cell, entry) in plan.iter_mut().zip(entries) {
            *cell = Some(entry);
        }
        rows.push(DynamicMenuRow {
            label: recipe.name.to_string(),
            action: DynamicMenuAction::SynthesizeRecipe {
                crystal_id: held_crystal_id,
                crystal_slot,
                ingredients: plan,
            },
        });
    }
    rows
}

pub fn root_child_kind(label: &str, flat_magic: bool, flat_abilities: bool) -> Option<MenuKind> {
    Some(match label {
        "Magic" if flat_magic => MenuKind::Magic,
        "Magic" => MenuKind::MagicGroups,
        "Abilities" if flat_abilities => MenuKind::Abilities,
        "Abilities" => MenuKind::AbilitiesGroups,
        // Retail's Party row opens the prtyN window chosen by party state
        // (FFXiMain.dll retail-2026-09 RVA 0x2016E0; vault main_menu.md §24) —
        // one kuluu window whose rows follow that same state.
        "Party" => MenuKind::Party,
        // Retail's Trade row hands off to helper 0x13FE70, which refuses (error
        // sound) without an eligible target — kuluu keeps the window drill and
        // enforces that gate at confirm time (vault main_menu.md §25).
        "Trade" => MenuKind::Trade,
        // Retail's Search row opens `searchma` unconditionally from its menuwind
        // arm 0x140626 — no gate, just the scresult null-check on entry select
        // (FFXiMain.dll retail-2026-09; vault main_menu.md §26).
        "Search" => MenuKind::Search,
        "Linkshell" => MenuKind::Linkshell,
        // Page-1 row 11 name-opens `menu    region` ungated (FFXiMain.dll
        // retail-2026-09 RVA 0x140691; vault main_menu.md §29).
        "Region Info" => MenuKind::RegionInfo { mode: 0 },

        // Page-2 row 1 name-opens `menu    missionm` (FFXiMain.dll retail-2026-09
        // RVA 0x14E86D; vault main_menu.md §30).
        "Missions" => MenuKind::MissionList,

        // Page-2 row 5 name-opens `menu    bazaar` ungated (FFXiMain.dll retail-2026-09
        // RVA 0x14E8A1; vault main_menu.md §33): kuluu drills the stall picker its
        // data-driven reading of that browser window builds from entity flags.
        "Bazaar" => MenuKind::BazaarStalls,
        // Page-2 row 6 is the ungated name-open of `menu    mcrmenu`
        // (FFXiMain.dll retail-2026-09 RVA 0x14E8BD; vault main_menu.md
        // §34): the root row drills that launcher. The `Edit Macros`
        // caption is the catalog string (§10 idx 75/109).
        "Edit Macros" => MenuKind::MacrosPalette,
        // Page-2 row 7 is the ungated name-open of `menu    configwi`
        // (FFXiMain.dll retail-2026-09 RVA 0x14E8D9; vault main_menu.md
        // §35): the retail Config row drills that twelve-page hub. kuluu's
        // own settings pages stay separate as Interface/Controls (page 3).
        "Config" => MenuKind::RetailConfig,
        // Page-2 row 8 goes through the Help Desk trampoline: socialme item
        // id 8 (FFXiMain.dll retail-2026-09 RVA 0x14E9C0) calls 0x2174C0, which
        // name-opens `menu    faqsub` once its boot singleton exists; kuluu
        // drills the panel directly (vault main_menu.md §36).
        "Help Desk" => MenuKind::HelpDeskPanel,
        // Page-1 row 5 is the ungated name-open of `menu    cmbmenu` (FFXiMain.dll retail-2026-09
        // RVA 0x1405D2; vault main_menu.md §28): the root row drills into that launcher.
        "Synthesis" => MenuKind::SynthLauncher,
        "Items" => MenuKind::Items,
        // "Key Items" and "Quests" are absent on purpose: retail page 2 fires the
        // "/keyitem" and "/quest" text commands instead of drilling (vault
        // main_menu.md §5 row id 3, §12 runner, and §31 for the quest arm),
        // handled as TextCommands in `resolve_menu_entry`.
        "Equipment" => MenuKind::Equipment,
        "Status" => MenuKind::Status,
        ROOT_COMMUNICATION => MenuKind::Communication,
        "Graphics" => MenuKind::Graphics,
        // The page-2 `Config` row is retail's configwi window (not built yet);
        // kuluu's own settings pages live on page 3 as Interface/Controls.
        "Interface" => MenuKind::Config,
        "Controls" => MenuKind::Controls,
        "Debug" => MenuKind::Debug,
        _ => return None,
    })
}

/// Kinds that render their own framed multi-panel screen (item_screen /
/// equipment_screen) instead of the generic two-pane list, so the generic pane
/// is suppressed and its pane-toggle does not apply while one is on top.
pub fn renders_bespoke_screen(kind: MenuKind) -> bool {
    matches!(
        kind,
        MenuKind::Equipment
            | MenuKind::EquipSlot(_)
            | MenuKind::Items
            | MenuKind::UsableItems
            | MenuKind::Map
    )
}

#[derive(Component)]
pub struct MainMenu;

#[derive(Component)]
pub struct MainMenuTitle;

#[derive(Component)]
pub struct MainMenuRow {
    pub slot: usize,
}

#[derive(Message, Debug, Clone, Copy)]
pub struct MenuRowActivated {
    pub slot: usize,
}

/// The single Command-menu column width; the Map screen's wide-scan column
/// reuses it so the pane widths can't drift.
pub const MENU_PANE_WIDTH: f32 = 220.0;

/// Wider pane for the Graphics menu: its rows are `label:<16>[value]` pairs
/// (longest: `> Texture Filtering:[Aniso 16x]`, 31 chars) that overflow the
/// 220px command column at 14px and used to wrap, doubling row heights and
/// pushing the pane past the window bottom.
pub const GRAPHICS_PANE_WIDTH: f32 = 320.0;

fn pane_width_for(kind: MenuKind) -> f32 {
    match kind {
        MenuKind::Config | MenuKind::Graphics | MenuKind::GraphicsDlss => GRAPHICS_PANE_WIDTH,
        _ => MENU_PANE_WIDTH,
    }
}

/// Single-line rows: a wrapped row silently occupies two lines, which breaks
/// the `DYNAMIC_VISIBLE_ROWS` viewport math (22 logical rows must be 22 drawn
/// lines). Truncate instead of wrapping if a row ever outgrows its pane.
fn text_nowrap() -> TextLayout {
    TextLayout {
        linebreak: LineBreak::NoWrap,
        ..default()
    }
}

/// Marker for the framed pane column so `update_main_menu` can retarget its
/// width per menu kind.
#[derive(Component)]
pub struct MainMenuPane;

pub fn spawn_main_menu(mut commands: Commands) {
    commands
        .spawn((
            crate::components::InGameEntity,
            MainMenu,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(48.0),
                right: Val::Px(8.0),
                display: Display::None,
                ..default()
            },
        ))
        .with_children(spawn_pane_column);
}

fn spawn_pane_column(parent: &mut ChildSpawnerCommands) {
    let (mut node, bg, border) = style::window_frame();
    node.width = Val::Px(MENU_PANE_WIDTH);
    parent
        .spawn((MainMenuPane, node, bg, border))
        .with_children(|c| {
            c.spawn((
                MainMenuTitle,
                Text::new(""),
                text_nowrap(),
                style::text_font(14.0),
                TextColor(theme::TITLE),
            ));

            for slot in 0..MAX_ENTRY_COUNT {
                c.spawn((
                    MainMenuRow { slot },
                    Button,
                    Text::new(""),
                    text_nowrap(),
                    style::text_font(14.0),
                    TextColor(theme::TEXT),
                    BackgroundColor(Color::NONE),
                ));
            }
        });
}

/// Spell-school lookup for the Magic category filter. Retail buckets spells by
/// the spell DAT's magicType - the same numbers mgcmenu filters on (vault
/// main_menu.md §21); with no install loaded the scraped LSB `group` column
/// stands in, mapped onto those numbers.
#[derive(Resource, Default)]
pub struct MagicSchoolTable {
    loaded: bool,
    table: ffxi_dat::spell_info::SpellTable,
}

impl MagicSchoolTable {
    fn ensure_loaded(&mut self, root: Option<&ffxi_dat::DatRoot>) {
        if !self.loaded {
            if let Some(root) = root {
                self.table = ffxi_dat::spell_info::SpellTable::open_from_root(root);
                self.loaded = true;
            }
        }
    }

    fn school_of(&self, spell_id: u16) -> Option<u8> {
        if self.loaded {
            if let Some(school) = self
                .table
                .lookup(spell_id)
                .and_then(|s| s.magic_type.bucket_id())
            {
                return Some(school);
            }
        }
        ffxi_vocab::magic::group(u32::from(spell_id)).and_then(lsb_group_to_school)
    }
}

/// LSB xi.magic.spellGroup numbers onto the client school numbers of §21: the
/// two enums are bijective but shuffled (SONG 1→5, SUMMONING 5→3, WHITE 6→1,
/// BLUE 3→6; BLACK/NINJUTSU/GEOMANCY/TRUST keep their number).
fn lsb_group_to_school(group: u8) -> Option<u8> {
    use ffxi_vocab::magic as g;
    match group {
        g::GROUP_WHITE => Some(MAGIC_CAT_WHITE),
        g::GROUP_BLACK => Some(MAGIC_CAT_BLACK),
        g::GROUP_SUMMONING => Some(MAGIC_CAT_SUMMONING),
        g::GROUP_NINJUTSU => Some(MAGIC_CAT_NINJUTSU),
        g::GROUP_SONG => Some(MAGIC_CAT_SONGS),
        g::GROUP_BLUE => Some(MAGIC_CAT_BLUE),
        g::GROUP_GEOMANCY => Some(MAGIC_CAT_GEOMANCY),
        g::GROUP_TRUST => Some(MAGIC_CAT_TRUST),
        _ => None,
    }
}

/// The `magic` window's unfiltered list — kuluu's flat view of it, reached with
/// the Retail+ toggle (retail always drills a category first, vault
/// main_menu.md §21): every spell the snapshot reports as known.
pub fn spell_rows(spells_known: &[u16]) -> Vec<DynamicMenuRow> {
    spells_known
        .iter()
        .filter_map(|&id| {
            ffxi_vocab::spell_names::lookup(id).map(|name| DynamicMenuRow {
                label: name.to_string(),
                action: DynamicMenuAction::CastSpell { spell_id: id },
            })
        })
        .collect()
}

/// The `magic` spell list for one school: known spells whose bucket matches.
pub fn magic_school_rows(
    spells_known: &[u16],
    school: u8,
    school_of: impl Fn(u16) -> Option<u8>,
) -> Vec<DynamicMenuRow> {
    spells_known
        .iter()
        .filter(|&&id| school_of(id) == Some(school))
        .filter_map(|&id| {
            ffxi_vocab::spell_names::lookup(id).map(|name| DynamicMenuRow {
                label: name.to_string(),
                action: DynamicMenuAction::CastSpell { spell_id: id },
            })
        })
        .collect()
}

/// Region Info row captions. The `region` window is chrome-only — no string
/// hits for its buttons in DLL or DAT (vault main_menu.md §29) — so kuluu pins
/// these English mode names by test; naming informed by retail zoom semantics,
/// caption itself [I].
pub const REGION_MODE_LABELS_PINNED: [&str; 4] = ["World", "Nation", "Region", "Zone"];

/// Notice worn by the World row when the installed mode is 0 but the current
/// zone fails FFXiMain's predicate — kuluu's stand-in for that path's message
/// id 0x7B post (vault main_menu.md §29); retail caption text [I].
pub const REGION_UNAVAILABLE_NOTICE_PINNED: &str = "(unavailable)";

/// Zones accepted by the mode-install predicate: verbatim first-word copy of
/// the stride-8 table FFXiMain.dll retail-2026-09 RVA 0x1E6CA0 scans at RVA
/// 0x37F632 (vault main_menu.md §29), both trailing zero words kept as found.
pub const REGION_PRED_ZONES_PINNED: [u16; 146] = [
    0x00E6, 0x00E7, 0x00E8, 0x00E9, 0x00EA, 0x00EB, 0x00EC, 0x00ED, 0x00EE, 0x00EF, 0x00F0, 0x00F1,
    0x00F2, 0x00F3, 0x00F4, 0x00F5, 0x00F6, 0x0064, 0x0065, 0x008C, 0x008D, 0x008E, 0x008B, 0x00BE,
    0x00A7, 0x0067, 0x006C, 0x00C4, 0x0066, 0x00C1, 0x00F8, 0x0068, 0x0069, 0x00C3, 0x0095, 0x0096,
    0x0001, 0x0002, 0x006A, 0x006B, 0x008F, 0x0090, 0x00BF, 0x00AC, 0x00AD, 0x006D, 0x006E, 0x00C5,
    0x0093, 0x0094, 0x0073, 0x0074, 0x0091, 0x0092, 0x00A9, 0x00C0, 0x00C2, 0x00AA, 0x0076, 0x0075,
    0x00C6, 0x00F9, 0x00D5, 0x0003, 0x0004, 0x0077, 0x0078, 0x00C8, 0x0097, 0x0098, 0x0007, 0x0008,
    0x006F, 0x00A6, 0x00CC, 0x00CE, 0x00CB, 0x0009, 0x000A, 0x0070, 0x00A1, 0x00A2, 0x00A5, 0x0005,
    0x0006, 0x007E, 0x007F, 0x009D, 0x009E, 0x00B8, 0x00B3, 0x0079, 0x007A, 0x00FB, 0x0099, 0x009A,
    0x00CA, 0x0072, 0x007D, 0x00F7, 0x00D0, 0x00A8, 0x00D1, 0x0071, 0x00AE, 0x0080, 0x00D4, 0x00C9,
    0x007B, 0x00B0, 0x00FA, 0x00FC, 0x007C, 0x009F, 0x00A0, 0x00A3, 0x00CD, 0x00CF, 0x00D3, 0x0082,
    0x00B1, 0x00B2, 0x00B4, 0x00B5, 0x000B, 0x000C, 0x000D, 0x0018, 0x0019, 0x0020, 0x001B, 0x001C,
    0x001D, 0x001E, 0x001F, 0x00DF, 0x00E0, 0x00E1, 0x00DC, 0x00DD, 0x00E3, 0x00E4, 0x00E2, 0x001A,
    0x0000, 0x0000,
];

/// The mode-install predicate's first-word test (vault main_menu.md §29).
fn region_zone_known(zone: u16) -> bool {
    REGION_PRED_ZONES_PINNED.contains(&zone)
}

/// The four Region Info rows in handler item order (FFXiMain.dll retail-2026-09
/// RVA 0x140818; vault main_menu.md §29): the marker rides the installed mode,
/// and while mode 0 is installed over a zone outside FFXiMain's predicate table
/// the World row wears the unavailable notice. Four rows regardless of state —
/// the window answers only items 1..4.
pub fn region_info_rows(mode: u8, zone: Option<u16>) -> Vec<DynamicMenuRow> {
    REGION_MODE_LABELS_PINNED
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let mut label = (*label).to_string();
            if i == 0 && mode == 0 && !zone.is_some_and(region_zone_known) {
                label.push(' ');
                label.push_str(REGION_UNAVAILABLE_NOTICE_PINNED);
            }
            let marker = if u8::try_from(i).is_ok_and(|m| m == mode) {
                "\u{bb} "
            } else {
                ""
            };
            DynamicMenuRow {
                label: format!("{marker}{label}"),
                action: DynamicMenuAction::RegionInfoMode { mode: i as u8 },
            }
        })
        .collect()
}

/// Nation names in LSB's MissionLog enum order (`Sandoria`/`Bastok`/`Windurst`,
/// vendor/server/src/map/enums/mission_log.h).
pub const MISSION_NATION_NAMES_PINNED: [&str; 3] = ["Sandoria", "Bastok", "Windurst"];

/// The complete-mission ports named by `MissionComplete` (vendor/server/src/map/
/// enums/mission_log.h) — the only OTHER ports the Missions surface lists; quest
/// ports belong to the "/quest" seam (vault main_menu.md §30).
pub const MISSION_COMPLETE_PORTS_PINNED: [(u16, &str); 4] = [
    (0x00D0, "Nations"),
    (0x00D8, "ToAU/WoTG"),
    (0x0030, "Campaign I"),
    (0x0038, "Campaign II"),
];

pub fn mission_nation_name(nation: u32) -> String {
    match MISSION_NATION_NAMES_PINNED.get(nation as usize) {
        Some(name) => (*name).to_string(),
        None => format!("Nation #{nation}"),
    }
}

/// The `missionm` rows (vault main_menu.md §30): the chrome carries no captions,
/// and kuluu lists exactly the logs its s2c 0x056 state holds — the nation log
/// line plus each complete-mission port with data. Select mirrors the handler's
/// tail name-open of miss00 by pushing that port's detail.
pub fn mission_list_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut rows = Vec::new();
    if let Some(info) = &snap.missions.info {
        rows.push(DynamicMenuRow {
            label: format!(
                "{}: mission #{}",
                mission_nation_name(info.nation),
                info.nation_mission
            ),
            action: DynamicMenuAction::MissionOpenLog {
                port: ffxi_proto::mission::PORT_MISSION_MAIN,
            },
        });
    }
    for entry in snap
        .missions
        .logs
        .iter()
        .filter(|e| e.data.iter().any(|&w| w != 0))
    {
        if let Some((port, name)) = MISSION_COMPLETE_PORTS_PINNED
            .iter()
            .find(|(p, _)| *p == entry.port)
        {
            rows.push(DynamicMenuRow {
                label: format!("Completed missions — {name}"),
                action: DynamicMenuAction::MissionOpenLog { port: *port },
            });
        }
    }
    if rows.is_empty() {
        rows.push(DynamicMenuRow {
            label: "(no mission logs reported)".to_string(),
            action: DynamicMenuAction::MissionLogRow { port: 0 },
        });
    }
    rows
}

/// The Equipment row's variant law (FFXiMain.dll retail-2026-09 RVA 0x1404D5;
/// vault main_menu.md §39): the Monstrosity predicate comes first (call 0x97D50
/// → helper 0x141BA0 opens `mp_mncst`), then — when the puppet flag byte has
/// bit 0 set and either job slot is PUP (`mov al, 0x12`, RVA 0x1404FB) — a
/// name-open of `pupequip`; otherwise the ordinary `equip` window.
///
/// kuluu's stand for both client flags is data presence: LSB only ever sends
/// s2c 0x044 MON while a monstrosity session is active and s2c 0x044 PUP only
/// for characters whose main or sub job is PUP (SendExtendedJobPackets,
/// vendor/server/src/map/utils/charutils.cpp), so `monstrosity.is_some()` /
/// `automaton.is_some()` mirror the gates exactly without a job byte of their
/// own on the snapshot.
pub fn equipment_drill_kind(snap: &kuluu_snapshot::SceneSnapshot) -> MenuKind {
    if snap.monstrosity.is_some() {
        return MenuKind::MpCustomize;
    }
    if snap.automaton.is_some() {
        return MenuKind::AutomatonEquip;
    }
    MenuKind::Equipment
}

/// The Map row's variant law (FFXiMain.dll retail-2026-09 RVA 0x140653; vault
/// main_menu.md §40): when the page-1 window object field +0x14 reads 1 and
/// predicate FFXiMain.dll retail-2026-09 RVA 0xF0C50 is true — bit 8 of the
/// game-state dword at [gameStateObj+0x3c35c] — the row name-opens `myroom`
/// instead of driving the mapframe family (call to 0x1F7810(0)).
///
/// kuluu's stand for both conditions is data presence: LSB marks the login
/// packet MYROOM whenever `PChar->inMogHouse()` holds and every zone change
/// re-authenticates through 0x00A (vendor/server/src/map/packets/s2c/
/// 0x00a_login.cpp), so `snap.myroom.is_some()` mirrors the gate exactly.
pub fn map_drill_kind(snap: &kuluu_snapshot::SceneSnapshot) -> MenuKind {
    if snap.myroom.is_some() {
        return MenuKind::HomeView;
    }
    MenuKind::Map
}

/// Name vocabulary for the packet's AutomatonHead byte (0 = nothing equipped);
/// verbatim from vendor/server/src/map/enums/automaton.h.
pub fn automaton_head_name(code: u8) -> Option<&'static str> {
    match code {
        1 => Some("Harlequin"),
        2 => Some("Valoredge"),
        3 => Some("Sharpshot"),
        4 => Some("Stormwaker"),
        5 => Some("Soulsoother"),
        6 => Some("Spiritreaver"),
        _ => None,
    }
}

/// Name vocabulary for the packet's AutomatonFrame byte; verbatim from the same
/// header (values 0x20..=0x23).
pub fn automaton_frame_name(code: u8) -> Option<&'static str> {
    match code {
        0x20 => Some("Harlequin"),
        0x21 => Some("Valoredge"),
        0x22 => Some("Sharpshot"),
        0x23 => Some("Stormwaker"),
        _ => None,
    }
}

/// The pupequip variant's fourteen rows: Head, Frame, then Attachment 1..12 in
/// AutomatonSlot order (Head=0, Frame=1, Attachment1..12 = 2..13 — vendor/
/// server/src/map/packets/c2s/0x102_extended_job.h; vault main_menu.md §39).
/// The DAT `pupe` record is chrome-only, so the slot words are kuluu pins held
/// by tests; equipped names come from the s2c 0x044 PUP frame (enum vocabulary
/// for head/frame, item names for attachments — empty slots read "(empty)" like
/// the trade grid's cells). Selecting is inert until §39's equip flow lands.
pub fn automaton_equip_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let Some(automaton) = snap.automaton.as_deref() else {
        return Vec::new();
    };
    let named = |word: &str, name: Option<&'static str>, slot: u8| DynamicMenuRow {
        label: format!("{word}: {}", name.unwrap_or("(empty)")),
        action: DynamicMenuAction::AutomatonSlotRow { slot },
    };
    let mut rows = vec![
        named("Head", automaton_head_name(automaton.head), 0),
        named("Frame", automaton_frame_name(automaton.frame), 1),
    ];
    for (i, &item) in automaton.attachments.iter().enumerate() {
        let name = if item == 0 {
            None
        } else {
            ffxi_vocab::item_names::lookup(u16::from(item))
        };
        rows.push(named(
            &format!("Attachment {}", i + 1),
            name,
            u8::try_from(i + 2).unwrap_or(13),
        ));
    }
    rows
}

/// The DAT-grounded macro capacity: two palette windows (`mcr1pall`/`mcr2pall`,
/// toggled by the helper family FFXiMain.dll retail-2026-09 RVA 0x1A44DE..) and
/// twenty keytop widgets per grid chunk — 'mcrs' (ROM/0/1.DAT file-off 0x2A3CC0,
/// root mcrselec) and 'mcr7' (@0x2A8560, root mcres20) each hold exactly twenty
/// `menu    keytops3-1` children (vault main_menu.md §41).
pub const MACRO_BLOCKS: usize = 2;
pub const MACRO_SLOTS_PER_BLOCK: usize = 20;

/// The mcrselec variant's rows: every filled slot of the client-local store in
/// (block, index) order. Retail renders empty keytops as blank cells; kuluu
/// lists only filled slots, so a fresh character sees the same nothing the
/// retail grid shows (vault main_menu.md §41). Selecting is inert [I] until
/// the macro-use seam lands.
pub fn macro_select_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut slots = snap.macros.clone();
    slots.sort_by_key(|m| (m.block, m.index));
    slots
        .into_iter()
        .map(|m| DynamicMenuRow {
            label: format!("B{}/{}: {}", m.block + 1, m.index + 1, m.text),
            action: DynamicMenuAction::MacroSlotRow {
                block: m.block,
                index: m.index,
            },
        })
        .collect()
}

/// Rows of kuluu's Bazaar stall picker: every PC in the zone whose
/// `Flags1.BazaarFlag` is set — LSB pushes it on the entity stream as
/// `hasBazaar()` (`char_update.cpp`; vault main_menu.md §33). Self never
/// browses its own stall from here. Labels are kuluu pins (the retail window
/// is chrome-only, §33); selecting sends c2s 0x105 for that seller.
pub fn bazaar_stall_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut rows: Vec<DynamicMenuRow> = snap
        .entities
        .iter()
        .filter(|e| is_bazaar_stall_entity(e, snap.self_char_id))
        .map(|e| DynamicMenuRow {
            label: format!("{}'s Bazaar", e.name.as_deref().unwrap_or_default()),
            action: DynamicMenuAction::BazaarStall { char_id: e.id },
        })
        .collect();
    if rows.is_empty() {
        rows.push(DynamicMenuRow {
            label: "(no player bazaars open nearby)".to_string(),
            action: DynamicMenuAction::BazaarStall { char_id: u32::MAX },
        });
    }
    rows
}

/// The §33 stall predicate on one entity; the hint-row sentinel id never
/// matches, so those rows stay inert.
pub fn is_bazaar_stall_entity(e: &kuluu_snapshot::Entity, self_char_id: Option<u32>) -> bool {
    e.kind == kuluu_snapshot::EntityKind::Pc
        && e.char_flags.bazaar
        && Some(e.id) != self_char_id
        && (e.id as u64) < u32::MAX as u64
}

/// The `miss00` detail lines for one port (vault main_menu.md §30): the field
/// dump legible, every row inert while that window's paging semantics stay
/// [I-pending]. Labels are kuluu pins (§22/§30 chrome-only law).
pub fn mission_detail_rows(snap: &kuluu_snapshot::SceneSnapshot, port: u16) -> Vec<DynamicMenuRow> {
    let mut rows = Vec::new();
    if port == ffxi_proto::mission::PORT_MISSION_MAIN {
        if let Some(i) = &snap.missions.info {
            rows.push(format!(
                "{}: mission #{}",
                mission_nation_name(i.nation),
                i.nation_mission
            ));
            // Field-named lines: the LSB field names are [V] (the packet header); retail
            // expansion captions would be invented chrome ([I]) so kuluu keeps them out.
            for (name, value) in [
                ("RoZ", i.rotz),
                ("CoP", i.cop),
                ("ACP", u32::from(i.addon_acp)),
                ("AMK", u32::from(i.addon_amk)),
                ("ASA", u32::from(i.addon_asa)),
                ("SoA", i.soa),
                ("RoV", i.rov),
            ] {
                if value != 0 {
                    rows.push(format!("{name} progress #{value}"));
                }
            }
        }
    } else if let Some(entry) = snap.missions.logs.iter().find(|e| e.port == port) {
        for (i, w) in entry.data.iter().enumerate() {
            if *w != 0 {
                rows.push(format!("word[{i}] = {w:#010X}"));
            }
        }
    }
    let mut rows: Vec<DynamicMenuRow> = rows
        .into_iter()
        .map(|label| DynamicMenuRow {
            label,
            action: DynamicMenuAction::MissionLogRow { port },
        })
        .collect();
    if rows.is_empty() {
        rows.push(DynamicMenuRow {
            label: "(nothing recorded)".to_string(),
            action: DynamicMenuAction::MissionLogRow { port },
        });
    }
    rows
}

/// The `quest00` rows (vault main_menu.md §31): the chrome carries no captions,
/// and kuluu lists exactly the quest bits its s2c 0x056 quest-port state holds —
/// one row per `(log, id)` in log-then-id order, labelled from the scraped
/// `scripts/globals/quests.lua` lattice. A bit present in both a QuestOffer and
/// its QuestComplete port shows as complete (the window carries one row per
/// quest; which report wins is state order, filter semantics [I-pending]).
/// Select mirrors quest00's select tail name-open of quest01 by pushing detail.
pub fn quest_list_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut statuses: std::collections::BTreeMap<(u8, u16), bool> = Default::default();
    for entry in &snap.missions.logs {
        if let Some((log, complete)) = ffxi_proto::mission::quest_port(entry.port) {
            for id in ffxi_proto::mission::quest_ids(&entry.data) {
                *statuses.entry((log, id)).or_default() |= complete;
            }
        }
    }
    let mut rows: Vec<DynamicMenuRow> = statuses
        .iter()
        .map(|(&(log, id), &complete)| DynamicMenuRow {
            label: match quest_caption(log, id) {
                Some(name) if complete => format!("{name} (complete)"),
                Some(name) => name.to_string(),
                None if complete => format!("Quest #{id} (complete)"),
                None => format!("Quest #{id}"),
            },
            action: DynamicMenuAction::QuestOpenDetail { log, id },
        })
        .collect();
    if rows.is_empty() {
        rows.push(DynamicMenuRow {
            label: "(no quests reported)".to_string(),
            action: DynamicMenuAction::QuestDetailRow { log: 0, id: 0 },
        });
    }
    rows
}

fn quest_caption(log: u8, id: u16) -> Option<&'static str> {
    ffxi_vocab::quest_lattice::quest_name(log, id)
}

/// The `quest01` detail lines for one quest bit (vault main_menu.md §31): the
/// field dump legible, every row inert while that window's paging law stays
/// [I-pending]. Labels are kuluu pins (§22/§31 chrome-only law).
pub fn quest_detail_rows(
    snap: &kuluu_snapshot::SceneSnapshot,
    log: u8,
    id: u16,
) -> Vec<DynamicMenuRow> {
    let mut bits = (false, false);
    for entry in &snap.missions.logs {
        if let Some((entry_log, complete)) = ffxi_proto::mission::quest_port(entry.port) {
            if entry_log == log && ffxi_proto::mission::quest_ids(&entry.data).contains(&id) {
                bits.0 |= !complete;
                bits.1 |= complete;
            }
        }
    }
    let area = ffxi_vocab::quest_lattice::QUEST_AREA_NAMES
        .get(log as usize)
        .copied()
        .unwrap_or("Unknown");
    let mut lines: Vec<String> = Vec::new();
    match quest_caption(log, id) {
        Some(name) => lines.push(name.to_string()),
        None => lines.push(format!("Quest #{id}")),
    }
    lines.push(format!("Area: {area}"));
    if bits.1 {
        lines.push("Complete.".to_string());
    } else if bits.0 {
        lines.push("In progress.".to_string());
    }
    let mut rows: Vec<DynamicMenuRow> = lines
        .into_iter()
        .map(|label| DynamicMenuRow {
            label,
            action: DynamicMenuAction::QuestDetailRow { log, id },
        })
        .collect();
    if rows.is_empty() {
        rows.push(DynamicMenuRow {
            label: "(nothing recorded)".to_string(),
            action: DynamicMenuAction::QuestDetailRow { log, id },
        });
    }
    rows
}

/// Rows for one side of a home-view job change: every JOBTYPE id whose bit is
/// set in the s2c 0x01B roster (bit 0 and never-unlocked jobs stay out — LSB's
/// MYROOM_JOB validator ranges MainJobIndex to 0x01..=MAX-1 and tests each
/// non-zero index against `jobs.unlocked`, vendor/server/src/map/packets/c2s/
/// 0x100_myroom_job.cpp; vault main_menu.md §53). A side picks nothing while
/// the roster is unknown (`snap.jobs` None) or the sub-job grant never arrived.
pub fn job_pick_rows(snap: &kuluu_snapshot::SceneSnapshot, main_pick: bool) -> Vec<DynamicMenuRow> {
    let Some(jobs) = snap.jobs.as_ref() else {
        return Vec::new();
    };
    if !main_pick && !jobs.sub_job_unlocked {
        return Vec::new();
    }
    let max_jobs = jobs.job_levels.len().min(32);
    (1u8..max_jobs as u8)
        .filter(|id| jobs.unlocked & (1 << *id) != 0)
        .map(|job_id| {
            let level = jobs.job_levels[usize::from(job_id)];
            DynamicMenuRow {
                label: format!(
                    "{} LV{}",
                    ffxi_vocab::job_names::lookup(u16::from(job_id)).unwrap_or("???"),
                    level
                ),
                action: DynamicMenuAction::HomeJobRow {
                    job_id,
                    as_main: main_pick,
                },
            }
        })
        .collect()
}

pub fn refresh_dynamic_menu_rows(
    mode: Res<InputMode>,
    scene: Res<crate::snapshot::SceneState>,
    sort: Res<crate::hud::item_detail::SortOptions>,
    active_bag: Res<crate::hud::item_screen::ItemScreenContainer>,
    dat_root: Option<Res<crate::dat_root::SharedDatRoot>>,
    mut school_table: ResMut<MagicSchoolTable>,
    mut dynamic: ResMut<DynamicMenu>,
) {
    let active_kind = match &*mode {
        InputMode::Menu(stack) => stack.current().map(|l| l.kind),
        _ => None,
    };

    let Some(kind) = active_kind.filter(|k| is_dynamic(*k)) else {
        if !dynamic.rows.is_empty() {
            dynamic.rows.clear();
        }
        return;
    };
    let snap = &scene.snapshot;
    let rows: Vec<DynamicMenuRow> = match kind {
        MenuKind::Magic => spell_rows(&snap.spells_known),
        MenuKind::MagicGroup(school) => {
            school_table.ensure_loaded(dat_root.as_ref().and_then(|r| r.get()).map(|v| &**v));
            magic_school_rows(&snap.spells_known, school, |id| school_table.school_of(id))
        }
        // Party window (FFXiMain.dll retail-2026-09 RVA 0x2016E0; vault
        // main_menu.md §24): invite pending → prtyjoin's YES/NO; else the party
        // roster as display rows; not in a party → empty-hint state.
        MenuKind::Party => party_window_rows(snap),
        // Trade window pair (FFXiMain.dll retail-2026-09 RVA 0x1515E0; vault
        // main_menu.md §25): answer rows → gil/own slots/buttons → partner
        // views, and the Add-Item bag picker under each own slot row.
        MenuKind::Trade => trade_window_rows(snap),
        MenuKind::Search => search_window_rows(snap),
        // Link Shell slots (helper FFXiMain.dll retail-2026-09 RVA 0x1E4470 always opens
        // `menu    link5`; vault main_menu.md section 27).
        MenuKind::Linkshell => linkshell_window_rows(snap),
        // The craft list (kuluu surface for the cmbHwin/cmbhlst pair behind cmbmenu row 2;
        // FFXiMain.dll retail-2026-09 RVA 0x1E29C1, vault main_menu.md §22/§28).
        MenuKind::CraftList => craft_list_rows(snap),
        // Region Info modes (vault main_menu.md §29): the rows also carry the
        // predicate gate notice for an unresolvable zone.
        MenuKind::RegionInfo { mode } => region_info_rows(mode, snap.zone_id),
        // The missionm list follows the folded s2c 0x056 battery (vault main_menu.md §30).
        MenuKind::MissionList => mission_list_rows(snap),
        MenuKind::MissionDetail { port } => mission_detail_rows(snap, port),
        // The bazaar browser lists the zone's open player stalls (§33 wire law).
        MenuKind::BazaarStalls => bazaar_stall_rows(snap),
        // The quest00 list follows the folded s2c 0x056 quest ports (vault
        // main_menu.md §31).
        MenuKind::QuestList => quest_list_rows(snap),
        MenuKind::QuestDetail { log, id } => quest_detail_rows(snap, log, id),
        // The pupequip variant's fourteen automaton slot rows (vault
        // main_menu.md §39).
        MenuKind::AutomatonEquip => automaton_equip_rows(snap),
        MenuKind::MacroSelect => macro_select_rows(snap),
        // Home-view job pick lists — both sides come from the same roster
        // (vault main_menu.md §53); the sub side renders empty until the
        // packet's sub-job grant arrived, like LSB refusing the row.
        MenuKind::JobPick { main_pick } => job_pick_rows(snap, main_pick),
        MenuKind::TradePick { trade_index } => trade_pick_rows(snap, trade_index),
        // abimenu category list (vault main_menu.md §23): the same group rows
        // kuluu's target-action groups already build.
        MenuKind::AbilitiesGroup(group) => crate::hud::action_model::AbilityGroup::ALL
            .get(group as usize)
            .map(|g| ability_group_rows(snap, *g))
            .unwrap_or_default(),
        MenuKind::Abilities => ability_flat_rows(snap),
        MenuKind::Items => inventory_rows(snap, active_bag.0, sort.auto),
        MenuKind::KeyItems => key_item_rows(snap),
        MenuKind::EmoteList => emote_rows(snap),
        MenuKind::UsableItems => usable_item_rows(snap),
        MenuKind::ItemAction {
            container,
            index,
            item_no,
        } => item_action_rows(snap, container, index, item_no),
        MenuKind::EquipSlot(equip_slot) => equip_slot_rows(snap, equip_slot),
        _ => Vec::new(),
    };

    let mut rows = rows;
    order_dynamic_rows(kind, sort.auto, &mut rows);
    if rows != dynamic.rows {
        dynamic.rows = rows;
    }
}

/// One row per slot of `container`, labelled with the bare item name (the item
/// window draws the stack count as an icon badge). Auto-sort orders by item id
/// (usables, then weapons, then armor by their DAT id ranges); Manual keeps the
/// raw slot order.
pub fn inventory_rows(
    snap: &kuluu_snapshot::SceneSnapshot,
    container: u8,
    auto_sort: bool,
) -> Vec<DynamicMenuRow> {
    let mut rows: Vec<DynamicMenuRow> = snap
        .container(container)
        .map(|c| c.items.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|slot| {
            let name = ffxi_vocab::item_names::lookup(slot.item_no)?;
            Some(DynamicMenuRow {
                label: name.to_string(),
                action: DynamicMenuAction::OpenItemAction {
                    container: slot.container,
                    index: slot.index,
                    item_no: slot.item_no,
                },
            })
        })
        .collect();
    order_dynamic_rows(MenuKind::Items, auto_sort, &mut rows);
    rows
}

/// Order the freshly built rows for one dynamic menu. Retail's Items window
/// sorts by item id when "Auto" is on (grouping usable items, then weapons,
/// then armor by their DAT id ranges) and otherwise shows raw inventory-slot
/// order. The item context submenu keeps its built order; other dynamic menus
/// stay alphabetical.
fn order_dynamic_rows(kind: MenuKind, auto_sort: bool, rows: &mut [DynamicMenuRow]) {
    match kind {
        MenuKind::Items => {
            if auto_sort {
                rows.sort_by_key(item_row_sort_key);
            }
        }
        // Retail's Command Menu Items list is always id-sorted.
        MenuKind::UsableItems => rows.sort_by_key(item_row_sort_key),
        // Key items keep id order: ascending global id groups the 512-id
        // tables together (whether retail sections match the tables is an
        // open question, bead kuluu-h7x retail_unknowns). Emotes keep the
        // scraped-table (id) order.
        MenuKind::ItemAction { .. } | MenuKind::KeyItems | MenuKind::EmoteList => {}
        _ => rows.sort_by(|a, b| a.label.cmp(&b.label)),
    }
}

/// The equip window's slot pick list (`FFXiMain.dll retail-2026-09 RVA 0x1322B0`
/// event-id 8 state==2; vault main_menu.md §20/§50): every inventory slot whose
/// item can legally land in `equip_slot`.
///
/// The candidate predicate mirrors the server's own acceptance gate, which is the
/// authority for what a select can succeed at: `EquipArmor` refuses items whose
/// equip-slot bit (`slot`), job bit (`1 << (GetMJob() - 1)`) or `level` fails
/// (vendor/server/src/map/utils/charutils.cpp EquipArmor). Those three are the
/// columns kuluu scrapes into `ffxi_vocab::equip_info`; the two remaining server
/// gates (superior-level mod, race mod) live in item *mods*, not in
/// vendor/server/sql/item_equipment.sql, so they cannot be filtered here yet —
/// vault main_menu.md §50 records that as the pick list's known gap.
pub fn equip_slot_rows(
    snap: &kuluu_snapshot::SceneSnapshot,
    equip_slot: u8,
) -> Vec<DynamicMenuRow> {
    let (main_job, main_lv) = snap
        .self_char_id
        .and_then(|id| snap.party.iter().find(|m| m.id == id))
        .map(|m| (m.main_job, m.main_job_lv))
        .unwrap_or((0, 0));
    let rows: Vec<DynamicMenuRow> = snap
        .inventory_main()
        .iter()
        .filter_map(|slot| {
            let info = ffxi_vocab::equip_info::lookup(slot.item_no)?;
            if !ffxi_vocab::equip_info::fits_slot(&info, equip_slot) {
                return None;
            }
            if main_job != 0 && !ffxi_vocab::equip_info::fits_job(&info, main_job) {
                return None;
            }
            if main_lv != 0 && info.level > main_lv {
                return None;
            }
            let name = ffxi_vocab::item_names::lookup(slot.item_no)?;
            let label = if info.level > 0 {
                format!("{name} (Lv{})", info.level)
            } else {
                name.to_string()
            };
            Some(DynamicMenuRow {
                label,
                action: DynamicMenuAction::EquipItem {
                    container: slot.container,
                    container_index: slot.index,
                    equip_slot,
                    item_no: slot.item_no,
                },
            })
        })
        .collect();
    rows
}

/// One row per owned key item, ascending global id, name from the scraped LSB
/// table, unseen ids suffixed with [`KEY_ITEM_UNSEEN_SUFFIX`].
pub fn key_item_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut ids: Vec<u16> = snap.key_items.clone();
    ids.sort_unstable();
    ids.into_iter()
        .map(|id| DynamicMenuRow {
            label: key_item_row_label(id, snap.key_items_seen.binary_search(&id).is_ok()),
            action: DynamicMenuAction::KeyItem { id },
        })
        .collect()
}

/// One row per canned emote from the scraped LSB table (id order): label +
/// "/command" column. HELM-only ids are server-initiated and skipped; the Job
/// row appears only when the 0x11A bitfield unlocks the current main job's
/// gesture (bit = job id - 1).
pub fn emote_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    use ffxi_proto::map::emote;
    let main_job = snap
        .self_char_id
        .and_then(|id| snap.party.iter().find(|m| m.id == id))
        .map(|m| m.main_job)
        .unwrap_or(0);
    ffxi_vocab::emote_names::EMOTES
        .iter()
        .filter(|&&(id, _)| !emote::HELM_ONLY.contains(&id))
        .filter(|&&(id, _)| {
            id != emote::JOB
                || snap
                    .emote_jobs
                    .is_some_and(|bits| ffxi_proto::decode::EmoteList::job_bit_set(bits, main_job))
        })
        .map(|&(id, name)| DynamicMenuRow {
            label: format!("{name} (/{})", emote_command_word(id)),
            action: DynamicMenuAction::Emote { emote_id: id },
        })
        .collect()
}

/// The slash-command word for an emote id — the scraped name lowercased,
/// except Job whose command is /jobemote.
pub fn emote_command_word(id: u8) -> String {
    if id == ffxi_proto::map::emote::JOB {
        return "jobemote".to_string();
    }
    ffxi_vocab::emote_names::lookup(id)
        .map(str::to_lowercase)
        .unwrap_or_else(|| format!("emote{id}"))
}

fn item_row_sort_key(row: &DynamicMenuRow) -> u16 {
    match row.action {
        DynamicMenuAction::OpenItemAction { item_no, .. } => item_no,
        DynamicMenuAction::UseItem { item_no, .. } => item_no,
        _ => u16::MAX,
    }
}

/// LSB 0x037 item-use gate (vendor/server/src/map/packets/c2s — 0x037 item
/// use → CState checks + `item_usable`): an item can fire right now iff it
/// appears in `item_usable` and either
/// - it is a plain consumable (maxCharges == 0) sitting unlocked in
///   LOC_INVENTORY or LOC_TEMPITEMS, or
/// - it is charged equipment (maxCharges > 0) that is currently equipped
///   (equipping is what locks the slot; the equipped item ids are mirrored
///   in `SceneSnapshot::equipped`).
pub fn item_usable_now(
    snap: &kuluu_snapshot::SceneSnapshot,
    slot: &kuluu_snapshot::InventoryItem,
) -> bool {
    use ffxi_proto::map::container as c;

    let Some(info) = ffxi_vocab::item_usable::lookup(slot.item_no) else {
        return false;
    };
    let is_equipment = ffxi_vocab::equip_info::lookup(slot.item_no).is_some();
    if is_equipment {
        // Charged equipment: usable only while equipped. Equipment without
        // charges never fires from the menu even though it may sit in
        // item_usable (enchantment already consumed server-side).
        info.max_charges > 0
            && slot.locked
            && snap.equipped.contains(&Some(slot.item_no))
            && (slot.container == c::LOC_INVENTORY || c::is_wardrobe(slot.container))
    } else {
        // Consumables: LOC_INVENTORY / LOC_TEMPITEMS only, and never from a
        // locked (bazaar/linkshell-reserved) slot.
        (slot.container == c::LOC_INVENTORY || slot.container == c::LOC_TEMPITEMS) && !slot.locked
    }
}

/// Rows for the Command Menu "Items" submenu: every currently-usable item
/// across all containers, firing Use directly (kuluu-268h), in item-id order.
pub fn usable_item_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut rows: Vec<DynamicMenuRow> = snap
        .containers
        .iter()
        .flat_map(|cont| cont.items.iter())
        .filter(|slot| item_usable_now(snap, slot))
        .filter_map(|slot| {
            let name = ffxi_vocab::item_names::lookup(slot.item_no)?;
            Some(DynamicMenuRow {
                label: name.to_string(),
                action: DynamicMenuAction::UseItem {
                    container: slot.container,
                    index: slot.index,
                    item_no: slot.item_no,
                },
            })
        })
        .collect();
    order_dynamic_rows(MenuKind::UsableItems, true, &mut rows);
    rows
}

/// Whether the quick-menu "Items" entry should be enabled at all.
pub fn any_usable_item(snap: &kuluu_snapshot::SceneSnapshot) -> bool {
    snap.containers
        .iter()
        .flat_map(|cont| cont.items.iter())
        .any(|slot| item_usable_now(snap, slot))
}

pub const ITEM_ACTION_USE: &str = "Use";

pub const ITEM_ACTION_DROP: &str = "Drop";

/// Context rows for one slot: retail's Use and Drop (Use is listed for every
/// item and silently refused when the LSB 0x037 gate would reject it, see
/// `item_usable_now`), then the moves LSB 0x029 allows
/// (vendor/server/src/map/packets/c2s/0x029_item_move.cpp): Gil and Temporary
/// items never move, wardrobes only take equipment, and "Take Out" leads when
/// browsing a storage bag. Drop follows 0x028's refusals
/// (vendor/server/src/map/packets/c2s/0x028_item_dump.cpp process): never Gil
/// or a busy slot (vendor/server/src/map/items/item.cpp CItem::isBusy:
/// equipped, bazaar-priced, placed furniture or mid-transaction; the 0x020
/// LockFlg byte this reads mirrors all but the transient transaction state).
pub fn item_action_rows(
    snap: &kuluu_snapshot::SceneSnapshot,
    container: u8,
    index: u8,
    item_no: u16,
) -> Vec<DynamicMenuRow> {
    use ffxi_proto::map::container as c;

    let Some(slot) = snap
        .container(container)
        .and_then(|v| v.items.iter().find(|s| s.index == index))
        .filter(|s| s.item_no == item_no)
    else {
        return Vec::new();
    };

    let mut rows = vec![DynamicMenuRow {
        label: ITEM_ACTION_USE.to_string(),
        action: DynamicMenuAction::UseItem {
            container,
            index,
            item_no,
        },
    }];

    // Locked = equipped / linkshell / bazaar-reserved: the server rejects the
    // move (and the drop) silently, so don't offer them. The recycle bin is
    // the one container the 0x028 ITEM_DUMP handler does not accept as a
    // source (vendor/server/src/map/packets/c2s/0x028_item_dump.cpp).
    let droppable =
        item_no != ffxi_proto::map::GIL_ITEM_NO && !slot.locked && container != c::LOC_RECYCLEBIN;
    if droppable {
        rows.push(DynamicMenuRow {
            label: ITEM_ACTION_DROP.to_string(),
            action: DynamicMenuAction::DropItem {
                container,
                index,
                item_no,
                quantity: slot.quantity,
            },
        });
    }
    let movable =
        item_no != ffxi_proto::map::GIL_ITEM_NO && !slot.locked && container != c::LOC_TEMPITEMS;
    if movable {
        let equipable = ffxi_vocab::equip_info::lookup(item_no).is_some();
        for dest in crate::hud::item_screen::accessible_containers(snap) {
            if dest == container || dest == c::LOC_TEMPITEMS || (c::is_wardrobe(dest) && !equipable)
            {
                continue;
            }
            // 0x029 item-move: recycle-bin items only come back out to the
            // inventory or the locker
            // (vendor/server/src/map/packets/c2s/0x029_item_move.cpp).
            if container == c::LOC_RECYCLEBIN
                && dest != c::LOC_INVENTORY
                && dest != c::LOC_MOGLOCKER
            {
                continue;
            }
            let Some(name) = c::name(dest) else { continue };
            let label = if dest == c::LOC_INVENTORY {
                "Take Out".to_string()
            } else {
                format!("Put in {name}")
            };
            rows.push(DynamicMenuRow {
                label,
                action: DynamicMenuAction::MoveItem {
                    quantity: slot.quantity,
                    from_container: container,
                    from_slot: index,
                    to_container: dest,
                    item_no,
                },
            });
        }
    }
    rows
}

/// One row name from the weapon-skill id space. `COMMAND_DATA`'s three bitmaps are
/// three different id spaces: vendor/server/src/map/packets/s2c/0x0ac_command_data.cpp
/// copies WeaponSkills from `m_WeaponSkills`, whose accessors key on a WeaponSkillID
/// (`charutils::hasWeaponSkill`), while JobAbilities/PetAbilities come from
/// `m_Abilities`/`m_PetCommands`, whose accessors key on an AbilityID
/// (`charutils::hasPetAbility`). So weapon skills must be named from the scraped
/// `weapon_skills` table (`ffxi_vocab::weapon_skill_names`) and never from the abilities
/// table, where id 32 is a goblin's Warcry rather than Fast Blade.
fn weapon_skill_row_name(id: u16) -> Option<&'static str> {
    ffxi_vocab::weapon_skill_names::lookup(id)
}

/// The flat list behind kuluu's Retail+ "Flat Abilities List" option: the three
/// known-id arrays at once (job abilities, weapon skills, pet commands).
/// Retail has no such window — its abimenu always drills one category first (vault
/// main_menu.md §23) — so the suffixed labels exist here only to keep the three id
/// spaces apart in a single column.
pub fn ability_flat_rows(snap: &kuluu_snapshot::SceneSnapshot) -> Vec<DynamicMenuRow> {
    let mut out: Vec<DynamicMenuRow> = Vec::with_capacity(
        snap.job_abilities_known.len()
            + snap.weaponskills_known.len()
            + snap.pet_abilities_known.len(),
    );
    out.extend(snap.job_abilities_known.iter().filter_map(|&id| {
        ffxi_vocab::ability_names::lookup(id).map(|name| DynamicMenuRow {
            label: name.to_string(),
            action: DynamicMenuAction::JobAbility { ability_id: id },
        })
    }));
    out.extend(snap.weaponskills_known.iter().filter_map(|&id| {
        weapon_skill_row_name(id).map(|name| DynamicMenuRow {
            label: format!("{name} (WS)"),
            action: DynamicMenuAction::Weaponskill { skill_id: id },
        })
    }));
    out.extend(snap.pet_abilities_known.iter().filter_map(|&id| {
        ffxi_vocab::ability_names::lookup(id).map(|name| DynamicMenuRow {
            label: format!("{name} (Pet)"),
            action: DynamicMenuAction::PetAbility { ability_id: id },
        })
    }));
    out
}

/// One category list of the `ability` window (vault main_menu.md §23): the known ids
/// of one group, named from the server-side ability id space so a row's label and
/// its c2s 0x01A payload can never disagree.
pub fn ability_group_rows(
    snap: &kuluu_snapshot::SceneSnapshot,
    group: crate::hud::action_model::AbilityGroup,
) -> Vec<DynamicMenuRow> {
    use crate::hud::action_model::AbilityGroup as G;
    let mut rows: Vec<DynamicMenuRow> = match group {
        G::JobAbilities => snap
            .job_abilities_known
            .iter()
            .filter_map(|&id| {
                ffxi_vocab::ability_names::lookup(id).map(|name| DynamicMenuRow {
                    label: name.to_string(),
                    action: DynamicMenuAction::JobAbility { ability_id: id },
                })
            })
            .collect(),
        G::WeaponSkill => snap
            .weaponskills_known
            .iter()
            .filter_map(|&id| {
                weapon_skill_row_name(id).map(|name| DynamicMenuRow {
                    label: name.to_string(),
                    action: DynamicMenuAction::Weaponskill { skill_id: id },
                })
            })
            .collect(),
        G::PetCommand => snap
            .pet_abilities_known
            .iter()
            .filter_map(|&id| {
                ffxi_vocab::ability_names::lookup(id).map(|name| DynamicMenuRow {
                    label: name.to_string(),
                    action: DynamicMenuAction::PetAbility { ability_id: id },
                })
            })
            .collect(),
        G::RangedAttack => vec![DynamicMenuRow {
            label: "Ranged Attack".to_string(),
            action: DynamicMenuAction::RangedAttack,
        }],
        // Retail's mounted commands are exactly these two — no other 0x01A
        // action id is gated on being mounted
        // (GP_CLI_COMMAND_ACTION_ACTIONID,
        // vendor/server/src/map/packets/c2s/0x01a_action.h). They only
        // exist while riding, which is also when this group has anything in it.
        G::Mount if snap.self_mount.is_some() => vec![
            DynamicMenuRow {
                label: "Dismount".to_string(),
                action: DynamicMenuAction::Dismount,
            },
            DynamicMenuRow {
                label: "Dig".to_string(),
                action: DynamicMenuAction::ChocoboDig,
            },
        ],
        G::Mount => Vec::new(),
    };
    rows.sort_by(|a, b| a.label.cmp(&b.label));
    rows
}

pub fn ability_group_empty_hint(group: crate::hud::action_model::AbilityGroup) -> &'static str {
    use crate::hud::action_model::AbilityGroup as G;
    match group {
        G::RangedAttack => "You cannot use that command here.",
        G::Mount => "You must be mounted to use that command.",
        _ => "No abilities available.",
    }
}

/// What the single Command-menu column renders this frame: the current (top)
/// level of the stack.
struct PaneView {
    kind: MenuKind,
    cursor: usize,
}

fn current_view(stack: &MenuStack) -> Option<PaneView> {
    let level = stack.current()?;
    Some(PaneView {
        kind: level.kind,
        cursor: level.cursor,
    })
}

#[allow(clippy::type_complexity)]
pub fn update_main_menu(
    mode: Res<InputMode>,
    settings: Res<GraphicsSettings>,
    panels: Res<crate::hud::HudPanels>,
    net_status: Res<crate::hud::network_status::NetStatusVisible>,
    vana_clock: Option<Res<crate::vana_time::VanaClock>>,
    #[cfg(not(target_arch = "wasm32"))] audio_mute: Res<crate::audio::AudioMuteState>,

    scene: Res<crate::snapshot::SceneState>,
    dynamic: Res<DynamicMenu>,
    mut menu_q: Query<&mut Node, (With<MainMenu>, Without<MainMenuRow>, Without<MainMenuPane>)>,
    mut pane_q: Query<&mut Node, (With<MainMenuPane>, Without<MainMenuRow>)>,
    mut row_q: Query<
        (
            &MainMenuRow,
            &mut Node,
            &mut Text,
            &mut TextColor,
            &mut BackgroundColor,
        ),
        Without<MainMenuTitle>,
    >,
    mut title_q: Query<(&MainMenuTitle, &mut Text), Without<MainMenuRow>>,
) {
    let Ok(mut node) = menu_q.single_mut() else {
        return;
    };

    let hide = |node: &mut Node| {
        if node.display != Display::None {
            node.display = Display::None;
        }
    };

    let stack = match &*mode {
        InputMode::Menu(stack) => stack,
        _ => {
            hide(&mut node);
            return;
        }
    };
    let Some(top_kind) = stack.current().map(|l| l.kind) else {
        hide(&mut node);
        return;
    };
    // The Equipment / Items screens render their own framed multi-panel layout;
    // suppress the generic two-pane while one is on top.
    if renders_bespoke_screen(top_kind) {
        hide(&mut node);
        return;
    }
    if node.display != Display::Flex {
        node.display = Display::Flex;
    }
    let want_width = Val::Px(pane_width_for(top_kind));
    for mut pane_node in pane_q.iter_mut() {
        if pane_node.width != want_width {
            pane_node.width = want_width;
        }
    }

    let view = current_view(stack);

    for (_, mut text) in title_q.iter_mut() {
        let want = view.as_ref().map(root_aware_title).unwrap_or_default();
        if **text != want {
            **text = want;
        }
    }

    let Some(view) = view else {
        for (_, mut row_node, _, _, _) in row_q.iter_mut() {
            hide(&mut row_node);
        }
        return;
    };

    let (total, viewport_start) =
        resolve_viewport(view.kind, view.cursor, &dynamic, settings.dlss_supported);
    let window = visible_window(view.kind, total);
    let clock_debug_hold = vana_clock
        .as_ref()
        .is_some_and(|clock| clock.debug_hold_active());

    for (row, mut row_node, mut text, mut color, mut bg) in row_q.iter_mut() {
        let list_idx = viewport_start + row.slot;
        if row.slot >= window || list_idx >= total {
            hide(&mut row_node);
            continue;
        }
        if row_node.display != Display::Flex {
            row_node.display = Display::Flex;
        }

        let label_owned: String = if is_dynamic(view.kind) {
            entry_label(view.kind, list_idx, &dynamic).to_string()
        } else {
            entries_for(view.kind, settings.dlss_supported)
                .get(list_idx)
                .copied()
                .unwrap_or("<unknown>")
                .to_string()
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (sound_on, master_pct) = (!(audio_mute.bgm && audio_mute.sfx), audio_mute.master_pct());
        #[cfg(target_arch = "wasm32")]
        let (sound_on, master_pct) = (false, 0);
        let body = format_row_body(
            view.kind,
            list_idx,
            &label_owned,
            &settings,
            &panels,
            net_status.0,
            sound_on,
            clock_debug_hold,
            master_pct,
            &scene.snapshot,
        );

        let is_cursor = view.cursor == list_idx;
        let (want, want_color, want_bg) = if is_cursor {
            (format!("> {body}"), theme::CURSOR, Color::NONE)
        } else {
            (format!("  {body}"), theme::TEXT, Color::NONE)
        };
        if **text != want {
            **text = want;
        }
        if color.0 != want_color {
            color.0 = want_color;
        }
        if bg.0 != want_bg {
            bg.0 = want_bg;
        }
    }
}

/// The pane title, with a `‹1/2›` page marker appended for the Root Command
/// menu so the two-page flip (retail's "-") is discoverable.
fn root_aware_title(view: &PaneView) -> String {
    let title = menu_title(view.kind);
    if view.kind == MenuKind::Root {
        let page = root_page_index(view.cursor) + 1;
        format!("{title}  \u{2039}{page}/{ROOT_PAGE_COUNT}\u{203a}")
    } else {
        title.to_string()
    }
}

fn visible_window(kind: MenuKind, total: usize) -> usize {
    if kind == MenuKind::Root {
        total
    } else {
        total.min(DYNAMIC_VISIBLE_ROWS)
    }
}

fn resolve_viewport(
    kind: MenuKind,
    cursor: usize,
    dynamic: &DynamicMenu,
    dlss_supported: bool,
) -> (usize, usize) {
    if kind == MenuKind::Root {
        // Show only the page the cursor is on: rows [start, end).
        let (start, end) = root_page_bounds(cursor);
        return (end, start);
    }
    let total = entry_count(kind, dynamic, dlss_supported);
    if total <= DYNAMIC_VISIBLE_ROWS {
        return (total, 0);
    }
    // A list taller than the viewport (e.g. the 27-row Graphics menu) scrolls a
    // fixed window that keeps the cursor centered, so its bottom rows stay
    // reachable at the default window height instead of overflowing off-screen.
    let half = DYNAMIC_VISIBLE_ROWS / 2;
    let max_start = total.saturating_sub(DYNAMIC_VISIBLE_ROWS);
    let start = cursor.saturating_sub(half).min(max_start);
    (total, start)
}

pub fn menu_mouse_hover_system(
    mut mode: ResMut<InputMode>,
    settings: Res<GraphicsSettings>,
    dynamic: Res<DynamicMenu>,
    rows: Query<(&Interaction, &MainMenuRow), Changed<Interaction>>,
) {
    let InputMode::Menu(stack) = &mut *mode else {
        return;
    };
    let Some(level_idx) = stack.levels.len().checked_sub(1) else {
        return;
    };
    let kind = stack.levels[level_idx].kind;
    for (interaction, row) in &rows {
        if !matches!(interaction, Interaction::Hovered | Interaction::Pressed) {
            continue;
        }
        let cursor = stack.levels[level_idx].cursor;
        let (total, viewport_start) =
            resolve_viewport(kind, cursor, &dynamic, settings.dlss_supported);
        let window = visible_window(kind, total);
        let list_idx = viewport_start + row.slot;
        if list_idx >= total || row.slot >= window {
            continue;
        }
        if stack.levels[level_idx].cursor != list_idx {
            stack.levels[level_idx].cursor = list_idx;
        }
    }
}

pub fn menu_mouse_click_system(
    mode: Res<InputMode>,
    settings: Res<GraphicsSettings>,
    dynamic: Res<DynamicMenu>,
    rows: Query<(&Interaction, &MainMenuRow), Changed<Interaction>>,
    mut out: MessageWriter<MenuRowActivated>,
) {
    let InputMode::Menu(stack) = &*mode else {
        return;
    };
    let Some(level) = stack.current() else {
        return;
    };
    let kind = level.kind;
    for (interaction, row) in &rows {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let (total, viewport_start) =
            resolve_viewport(kind, level.cursor, &dynamic, settings.dlss_supported);
        let window = visible_window(kind, total);
        let list_idx = viewport_start + row.slot;
        if list_idx < total && row.slot < window {
            out.write(MenuRowActivated { slot: list_idx });
        }
    }
}

fn format_row_body(
    kind: MenuKind,
    slot: usize,
    label: &str,
    settings: &GraphicsSettings,
    panels: &crate::hud::HudPanels,
    net_status_on: bool,
    sound_on: bool,
    clock_debug_hold: bool,
    master_pct: i32,
    snapshot: &kuluu_snapshot::SceneSnapshot,
) -> String {
    match kind {
        MenuKind::Config | MenuKind::Graphics | MenuKind::GraphicsDlss => {
            match settings_field_at(kind, slot, settings.dlss_supported) {
                Some(field) => format!(
                    "{:<16}[{}]",
                    format!("{}:", field.label()),
                    settings.value_label(field)
                ),
                None => label.to_string(),
            }
        }
        MenuKind::Debug => {
            if label == DEBUG_VOLUME {
                format!("{label:<14}[{master_pct:>3}]")
            } else if label == DEBUG_PRINT_POS {
                format!("{label:<14}[enter]")
            } else if label == DEBUG_RETAIL_SEPARATOR || label == DEBUG_RETAIL_LABEL {
                label.to_string()
            } else if label == RETAIL_DLSS_MENU {
                format!(
                    "{label:<14}[{}]",
                    if !settings.dlss_supported {
                        "N/A"
                    } else if settings.dlss_menu_enabled {
                        "on"
                    } else {
                        "off"
                    }
                )
            } else {
                #[cfg(feature = "enhanced-mob-hp-under")]
                if label == RETAIL_MOB_HP_UNDER {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.mob_hp_under { "on" } else { "off" }
                    );
                }
                #[cfg(feature = "enhanced-job-display")]
                if label == RETAIL_JOB_DISPLAY {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.job_display { "on" } else { "off" }
                    );
                }
                #[cfg(feature = "enhanced-ignore-knockback-self")]
                if label == RETAIL_KNOCKBACK {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.ignore_knockback_self {
                            "off"
                        } else {
                            "on"
                        }
                    );
                }
                if label == RETAIL_STATUS_ROWS {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.interactive_status_rows {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
                if label == RETAIL_RESELECT_UNEQUIP {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.reselect_unequip {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
                if label == RETAIL_MAGIC_FLAT_LIST {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.magic_flat_list {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
                if label == RETAIL_ABILITIES_FLAT_LIST {
                    return format!(
                        "{label:<14}[{}]",
                        if settings.abilities_flat_list {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
                if label == RETAIL_CAMERA_LEASH {
                    return format!("{label:<14}[{:.1}]", settings.camera_leash_yalms);
                }
                let on =
                    debug_panel_state(label, panels, net_status_on, sound_on, clock_debug_hold);
                format!("{label:<14}[{}]", if on { "on" } else { "off" })
            }
        }
        MenuKind::Equipment => {
            let item_name = snapshot
                .equipped
                .get(slot)
                .copied()
                .flatten()
                .and_then(ffxi_vocab::item_names::lookup)
                .unwrap_or("—");
            format!("{label:<7}: {item_name}")
        }
        _ => label.to_string(),
    }
}

pub fn debug_panel_state(
    label: &str,
    panels: &crate::hud::HudPanels,
    net_status_on: bool,
    sound_on: bool,
    clock_debug_hold: bool,
) -> bool {
    match label {
        DEBUG_PERF => panels.perf,
        DEBUG_TARGET_CYCLE => panels.target_cycle,
        DEBUG_MESH => panels.mesh_debug,
        DEBUG_NOCLIP => panels.noclip,
        #[cfg(feature = "enhanced-engage-move-lock-off")]
        DEBUG_ENGAGE_ANIM_LOCK => panels.engage_anim_lock,
        DEBUG_AUTO_ENTER_CS => panels.auto_enter_cs,
        DEBUG_WEATHER => !panels.weather_off,
        DEBUG_FOG => !panels.fog_off,
        DEBUG_FORCE_18 => clock_debug_hold,
        DEBUG_ENTITY_LIST => panels.entity_list,
        DEBUG_NET_STATUS => net_status_on,
        DEBUG_SOUND => sound_on,
        DEBUG_STAIR_DRAW => panels.stair_draw,
        DEBUG_STAIR_STATUS => panels.stair_debug,
        DEBUG_GRAPHICS_DEBUG => panels.graphics_debug,
        DEBUG_POSITION_LOG => panels.position_log,
        DEBUG_NAMEPLATES => panels.nameplate_debug,
        DEBUG_UI_SETTINGS => panels.ui_settings,
        _ => false,
    }
}

/// The Mog-house capital zones whose View House arm is gated (vault main_menu.md
/// §32): the `socialme` arm runs its veto predicate FFXiMain.dll retail-2026-09
/// RVA 0xF9680 only through this zone table — Chateau d'Oraguille and Metalworks
/// are allow-always, every other capital needs the in-house byte, all non-
/// capitals are unconditionally allowed. Same set as LSB's
/// `scripts/globals/moghouse.lua` moghouseZones ∩ capitals.
pub const MOGHOUSE_CAPITAL_ZONE_MIN: u16 = 230;
pub const MOGHOUSE_CAPITAL_ZONE_MAX: u16 = 241;
pub const CHATEAU_DORAGUILLE_ZONE: u16 = 233;
pub const METALWORKS_ZONE: u16 = 237;

/// View House (page-2 row 4) refuses with the arm's error sound when standing
/// in a Mog-house capital without being inside your own Mog House — the
/// FFXiMain.dll retail-2026-09 RVA 0x14E8F5 veto law read off handler 0x14E7B0
/// (vault main_menu.md §32). `self_in_mog_house` is the snapshot reading of the
/// predicate's byte operand.
pub fn view_house_blocked(zone_id: Option<u16>, self_in_mog_house: bool) -> bool {
    zone_id.is_some_and(|zone| {
        (MOGHOUSE_CAPITAL_ZONE_MIN..=MOGHOUSE_CAPITAL_ZONE_MAX).contains(&zone)
            && zone != CHATEAU_DORAGUILLE_ZONE
            && zone != METALWORKS_ZONE
            && !self_in_mog_house
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_recast_remaining_gates_on_cooldown_abilities() {
        let provoke = DynamicMenuAction::JobAbility { ability_id: 35 };
        let rid = ffxi_vocab::recast::ability_recast_id(35).expect("provoke has a recast group");
        // On cooldown: expiry 30s in the future -> Some(remaining).
        assert_eq!(
            action_recast_remaining(&[(rid, 1030)], &provoke, 1000),
            Some(30)
        );
        // Ready: expiry in the past -> None.
        assert_eq!(action_recast_remaining(&[(rid, 990)], &provoke, 1000), None);
        // No recast entry -> None.
        assert_eq!(action_recast_remaining(&[], &provoke, 1000), None);
        // Non-ability action -> None.
        assert_eq!(
            action_recast_remaining(
                &[(rid, 1030)],
                &DynamicMenuAction::CastSpell { spell_id: 1 },
                1000
            ),
            None
        );
    }

    #[test]
    fn recast_expiry_stamped_with_recast_now_unix_reads_back_undrifted() {
        // Guard for kuluu-t815: expiries are absolute seconds on the
        // recast_now_unix() clock, so a consumer passing that same clock as
        // `now` must read back the stamped duration (to within one tick).
        // The gate (text_input) and display (target_action_menu) both do; a
        // consumer on any other clock (e.g. VanaClock) would drift by the
        // client/server skew this test cannot see — keep them on this helper.
        const REMAINING_SECS: u32 = 45;
        let provoke = DynamicMenuAction::JobAbility { ability_id: 35 };
        let rid = ffxi_vocab::recast::ability_recast_id(35).expect("provoke has a recast group");
        let expiry = kuluu_snapshot::recast_now_unix() + REMAINING_SECS;

        let remaining = action_recast_remaining(
            &[(rid, expiry)],
            &provoke,
            kuluu_snapshot::recast_now_unix(),
        )
        .expect("ability still on recast");
        assert!(
            (REMAINING_SECS - 1..=REMAINING_SECS).contains(&remaining),
            "remaining {remaining} drifted from stamped {REMAINING_SECS}"
        );
    }

    #[test]
    fn root_pages_split_three_ways_by_cursor() {
        let p2 = ROOT_PAGE_1_LEN;
        let p3 = ROOT_PAGE_1_LEN + ROOT_PAGE_2_LEN;
        assert_eq!(root_page_bounds(0), (0, ROOT_PAGE_1_LEN));
        assert_eq!(root_page_bounds(p2 - 1), (0, ROOT_PAGE_1_LEN));
        assert_eq!(root_page_bounds(p2), (p2, p3));
        assert_eq!(root_page_bounds(p3), (p3, ROOT_ENTRIES.len()));

        // "Map" lives on page 1, retail's "Config" row on page 2.
        let map = ROOT_ENTRIES.iter().position(|e| *e == "Map").unwrap();
        let config = ROOT_ENTRIES.iter().position(|e| *e == "Config").unwrap();
        assert!(map < p2 && (p2..p3).contains(&config));

        // '-' / Right: 1->2->3->1, Left steps back; page 3's first row wraps to
        // page 1 keeping its (row-0) cursor.
        assert_eq!(root_next_page_cursor(0), p2);
        assert_eq!(root_prev_page_cursor(p2), 0);
        assert_eq!(root_next_page_cursor(p3), 0);

        // Same-row carry between the retail pages, and a clamp to page 3's last
        // row when its length is shorter.
        for row in 0..ROOT_PAGE_1_LEN.min(ROOT_PAGE_2_LEN) {
            assert_eq!(root_next_page_cursor(row), p2 + row);
            assert_eq!(root_prev_page_cursor(p2 + row), row);
        }
        let p3_last = ROOT_ENTRIES.len() - 1;
        let row_in_p3 = p3_last - p3;
        assert_eq!(
            root_next_page_cursor(p3_last),
            row_in_p3.min(ROOT_PAGE_1_LEN - 1)
        );
        // Page 1 -> Left enters page 3 at the same row number, clamped: its
        // first row is page 3's first; page 1's deepest rows land on page 3's last.
        assert_eq!(root_prev_page_cursor(0), p3);
        assert_eq!(
            root_prev_page_cursor(ROOT_PAGE_1_LEN - 1),
            p3_last,
            "page-1 last row clamps onto page 3's last row"
        );
    }

    /// Retail carries the cursor across the menuwind<->socialme flip through an
    /// explicit pair table of per-page item ids (FFXiMain.dll retail-2026-09 RVA
    /// 0x36FF98, read by the page handlers at RVA 0x140423 / 0x14E7D3). The
    /// pages' display orders pair up row-for-row, which is what makes this
    /// layout's plain same-index carry correct; pin both readings.
    #[test]
    fn root_pages_match_retail_pair_table() {
        // Byte pairs (menuwind id, socialme id) at FFXiMain.dll retail-2026-09
        // RVA 0x36FF98, before the terminating 00 00.
        const PAIRS: [(u8, u8); 12] = [
            (1, 1),
            (2, 2),
            (3, 3),
            (4, 4),
            (5, 6),
            (6, 7),
            (7, 8),
            (8, 0xA),
            (9, 9),
            (0xA, 5),
            (0xB, 0xB),
            (0xC, 0xC),
        ];
        // Each screen row's handler item id per page: menuwind fires Synthesis as
        // item id 0xA (screen row 5) and linkshell as 9; socialme fires Current
        // Time as 10 (row 9) and Communication as 9. A pair (a,b) carries the
        // flip from a's row to b's row - always the same screen line.
        const P1_ROWS_BY_ID: [u8; 12] = [1, 2, 3, 4, 0xA, 5, 6, 7, 8, 9, 0xB, 0xC];
        const P2_ROWS_BY_ID: [u8; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 0xA, 9, 0xB, 0xC];
        assert!(
            P1_ROWS_BY_ID
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == 12
        );
        for (p1_id, p2_id) in PAIRS {
            let r1 = P1_ROWS_BY_ID.iter().position(|id| *id == p1_id).unwrap();
            let r2 = P2_ROWS_BY_ID.iter().position(|id| *id == p2_id).unwrap();
            assert_eq!(
                r1, r2,
                "pair ({p1_id:#x},{p2_id:#x}) must be the same screen row"
            );
        }

        // Kuluu's pages hold retail's rows in that display order (labels are the
        // client caption catalog strings).
        const RETAIL_P1: [&str; 12] = [
            "Status",
            "Equipment",
            "Magic",
            "Items",
            "Synthesis",
            "Abilities",
            "Party",
            "Trade",
            "Search",
            "Linkshell",
            "Region Info",
            "Map",
        ];
        const RETAIL_P2: [&str; 12] = [
            "Missions",
            "Quests",
            "Key Items",
            "View House",
            "Bazaar",
            "Edit Macros",
            "Config",
            "Help Desk",
            "Current Time",
            "Communication",
            "Shut Down",
            "Log Out",
        ];
        assert_eq!(&ROOT_ENTRIES[..12], &RETAIL_P1);
        assert_eq!(&ROOT_ENTRIES[12..24], &RETAIL_P2);
    }

    fn root_menu_app(cursor: usize) -> App {
        use crate::hud::network_status::NetStatusVisible;
        use crate::snapshot::SceneState;

        let mut app = App::new();
        app.init_resource::<GraphicsSettings>()
            .init_resource::<crate::hud::HudPanels>()
            .init_resource::<NetStatusVisible>()
            .init_resource::<SceneState>()
            .init_resource::<DynamicMenu>();
        #[cfg(not(target_arch = "wasm32"))]
        app.init_resource::<crate::audio::AudioMuteState>();

        let mut stack = MenuStack::root();
        stack.current_mut().unwrap().cursor = cursor;
        app.insert_resource(InputMode::Menu(stack));

        app.add_systems(Startup, spawn_main_menu);
        app.add_systems(Update, update_main_menu);
        app.update();
        app
    }

    fn title_text(app: &mut App) -> String {
        let mut q = app.world_mut().query::<(&MainMenuTitle, &Text)>();
        q.iter(app.world())
            .map(|(_, text)| text.0.clone())
            .next()
            .expect("title exists")
    }

    fn visible_rows(app: &mut App) -> Vec<(usize, String)> {
        let mut q = app.world_mut().query::<(&MainMenuRow, &Text, &Node)>();
        let mut rows: Vec<(usize, String)> = q
            .iter(app.world())
            .filter(|(_, _, node)| node.display != Display::None)
            .map(|(row, text, _)| (row.slot, text.0.clone()))
            .collect();
        rows.sort_by_key(|(slot, _)| *slot);
        rows
    }

    #[test]
    fn graphics_menu_widens_the_pane_and_rows_stay_single_line() {
        // Regression: Graphics rows (`label:<16>[value]`) used to wrap inside
        // the 220px command column, doubling row heights and pushing the pane
        // past the window bottom.
        let mut app = root_menu_app(0);
        let pane_width = |app: &mut App| {
            let mut q = app.world_mut().query::<(&MainMenuPane, &Node)>();
            q.iter(app.world())
                .map(|(_, node)| node.width)
                .next()
                .expect("pane exists")
        };
        assert_eq!(pane_width(&mut app), Val::Px(MENU_PANE_WIDTH));

        // Push Graphics on top: the pane retargets to the wide width.
        let mut stack = MenuStack::root();
        stack.push(MenuKind::Graphics);
        app.insert_resource(InputMode::Menu(stack));
        app.update();
        assert_eq!(pane_width(&mut app), Val::Px(GRAPHICS_PANE_WIDTH));

        // Back on the root, it narrows again.
        app.insert_resource(InputMode::Menu(MenuStack::root()));
        app.update();
        assert_eq!(pane_width(&mut app), Val::Px(MENU_PANE_WIDTH));

        // Rows and title must never wrap: a wrapped row breaks the
        // DYNAMIC_VISIBLE_ROWS viewport math.
        let mut q = app
            .world_mut()
            .query_filtered::<&TextLayout, With<MainMenuRow>>();
        let mut count = 0;
        for layout in q.iter(app.world()) {
            assert_eq!(layout.linebreak, LineBreak::NoWrap);
            count += 1;
        }
        assert_eq!(count, MAX_ENTRY_COUNT, "every row carries NoWrap");
        let mut q = app
            .world_mut()
            .query_filtered::<&TextLayout, With<MainMenuTitle>>();
        assert!(q
            .iter(app.world())
            .all(|l| l.linebreak == LineBreak::NoWrap));
    }

    #[test]
    fn single_pane_shows_only_the_cursors_root_page() {
        // Cursor on page 1 (Status at index 0): only page-1 entries render, and
        // the title carries the ‹1/3› marker.
        let mut app = root_menu_app(0);
        assert!(title_text(&mut app).contains("Commands"));
        assert!(title_text(&mut app).contains("1/3"));
        let rows = visible_rows(&mut app);
        assert_eq!(rows.len(), ROOT_PAGE_1_LEN, "only page 1 rows: {rows:?}");
        assert_eq!(rows[0].1, "> Status");
        assert!(rows.iter().any(|(_, t)| t.contains("Map")));
        assert!(
            !rows.iter().any(|(_, t)| t.contains("Config")),
            "page 2 entries stay hidden on page 1"
        );

        // Cursor on page 2 (Config): only page-2 entries render, marker flips.
        let config = ROOT_ENTRIES.iter().position(|e| *e == "Config").unwrap();
        let mut app = root_menu_app(config);
        assert!(title_text(&mut app).contains("2/3"));
        let rows = visible_rows(&mut app);
        assert_eq!(rows.len(), ROOT_PAGE_2_LEN);
        assert!(rows.iter().any(|(_, t)| t.contains("Config")));
        assert!(!rows.iter().any(|(_, t)| t.contains("Status")));

        // Cursor on page 3 (Graphics): kuluu's own rows only, marker ‹3/3›.
        let graphics = ROOT_ENTRIES.iter().position(|e| *e == "Graphics").unwrap();
        let mut app = root_menu_app(graphics);
        assert!(title_text(&mut app).contains("3/3"));
        let rows = visible_rows(&mut app);
        assert_eq!(
            rows.len(),
            ROOT_ENTRIES.len() - ROOT_PAGE_1_LEN - ROOT_PAGE_2_LEN
        );
        assert!(rows.iter().any(|(_, t)| t.contains("Graphics")));
        assert!(!rows.iter().any(|(_, t)| t.contains("Missions")));
    }

    #[test]
    fn list_page_rows_matches_owning_panel_viewport() {
        assert_eq!(
            list_page_rows(MenuKind::Items),
            crate::hud::item_screen::ITEM_LIST_ROWS
        );
        assert_eq!(
            list_page_rows(MenuKind::UsableItems),
            crate::hud::item_screen::ITEM_LIST_ROWS
        );
        assert_eq!(list_page_rows(MenuKind::Magic), DYNAMIC_VISIBLE_ROWS);
        assert_eq!(list_page_rows(MenuKind::KeyItems), DYNAMIC_VISIBLE_ROWS);
    }

    #[test]
    fn page_cursor_jumps_a_page_and_clamps_without_wrap() {
        // Forward jumps by `rows`, clamped to the last index (no wrap past end).
        assert_eq!(page_cursor(0, 30, 13, true), 13);
        assert_eq!(page_cursor(13, 30, 13, true), 26);
        assert_eq!(page_cursor(26, 30, 13, true), 29);
        assert_eq!(page_cursor(29, 30, 13, true), 29);
        // Backward jumps by `rows`, clamped to 0 (no wrap past start).
        assert_eq!(page_cursor(29, 30, 13, false), 16);
        assert_eq!(page_cursor(5, 30, 13, false), 0);
        assert_eq!(page_cursor(0, 30, 13, false), 0);
        // A page press moves more than one row — the whole point of the bug fix.
        assert!(page_cursor(0, 30, 13, true) > 1);
        // Empty list stays put.
        assert_eq!(page_cursor(0, 0, 13, true), 0);
    }

    #[test]
    fn item_qty_label_suffixes_only_real_stacks() {
        assert_eq!(item_qty_label("Bird Egg", 1), "Bird Egg");
        assert_eq!(item_qty_label("Bird Egg", 0), "Bird Egg");
        assert_eq!(item_qty_label("Bird Egg", 12), "Bird Egg x12");
    }

    #[test]
    fn action_item_no_covers_item_bearing_variants() {
        let use_it = DynamicMenuAction::UseItem {
            container: 0,
            index: 3,
            item_no: 4096,
        };
        let open = DynamicMenuAction::OpenItemAction {
            container: 0,
            index: 3,
            item_no: 4096,
        };
        assert_eq!(use_it.item_no(), Some(4096));
        assert_eq!(open.item_no(), Some(4096));
        assert_eq!(DynamicMenuAction::KeyItem { id: 5 }.item_no(), None);
        assert_eq!(DynamicMenuAction::CastSpell { spell_id: 1 }.item_no(), None);
    }

    fn mh_snapshot() -> kuluu_snapshot::SceneSnapshot {
        use ffxi_proto::map::container as c;
        let bag =
            |id: u8, capacity: u16, items: Vec<(u8, u16, u32)>| kuluu_snapshot::ContainerView {
                id,
                capacity,
                items: items
                    .into_iter()
                    .map(|(index, item_no, quantity)| kuluu_snapshot::InventoryItem {
                        container: id,
                        index,
                        item_no,
                        quantity,
                        locked: false,
                        unselectable: false,
                        charges_remaining: None,
                        next_use_vana_ts: None,
                        use_delay_end_vana_ts: None,
                        ready: None,
                    })
                    .collect(),
            };
        kuluu_snapshot::SceneSnapshot {
            myroom: Some(kuluu_snapshot::MyRoom {
                model: 289,
                sub_map: 0,
            }),
            containers: vec![
                // Slot 0 carries Gil (retail keeps it in inventory slot 0).
                bag(
                    c::LOC_INVENTORY,
                    30,
                    vec![(0, ffxi_proto::map::GIL_ITEM_NO, 1000), (1, 4509, 12)],
                ),
                bag(c::LOC_MOGSAFE, 50, vec![(1, 13327, 1)]),
                bag(c::LOC_STORAGE, 8, vec![]),
                bag(c::LOC_TEMPITEMS, 20, vec![(0, 4212, 1)]),
                bag(c::LOC_WARDROBE, 8, vec![]),
            ],
            ..Default::default()
        }
    }

    /// Pins LSB 0x029 isValidMovement's and 0x028's CItem::isBusy rejections:
    /// locked slots (equipped / placed furniture / bazaar-priced, the states
    /// 0x020 lockFlagFor puts on the wire) get no move rows and no Drop. Use
    /// stays listed like retail; the confirm path refuses it.
    #[test]
    fn locked_slot_offers_no_moves_or_drop() {
        use ffxi_proto::map::container as c;
        let mut snap = mh_snapshot();
        snap.containers[0].items[1].locked = true;
        let rows = item_action_rows(&snap, c::LOC_INVENTORY, 1, 4509);
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.action, DynamicMenuAction::MoveItem { .. })),
            "{rows:?}"
        );
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r.action, DynamicMenuAction::DropItem { .. })),
            "{rows:?}"
        );
        assert_eq!(labels(&rows), [ITEM_ACTION_USE]);
    }

    /// Pins the LSB validContainers Safe-2F gate (mhflag & 0x20,
    /// 0x029_item_move.cpp): capacity alone must not offer Mog Safe 2.
    #[test]
    fn safe2_requires_the_2f_flag() {
        use crate::hud::item_screen::container_accessible;
        use ffxi_proto::map::container as c;
        let mut snap = mh_snapshot();
        snap.containers.push(kuluu_snapshot::ContainerView {
            id: c::LOC_MOGSAFE2,
            capacity: 60,
            items: Vec::new(),
        });
        assert!(
            !container_accessible(&snap, c::LOC_MOGSAFE2),
            "flag unknown"
        );
        snap.mh_2f_unlocked = Some(false);
        assert!(!container_accessible(&snap, c::LOC_MOGSAFE2));
        snap.mh_2f_unlocked = Some(true);
        assert!(container_accessible(&snap, c::LOC_MOGSAFE2));
        assert!(
            container_accessible(&snap, c::LOC_MOGSAFE),
            "the 2F flag must not gate the 1F safe"
        );
    }

    #[test]
    fn storage_item_leads_with_take_out() {
        use ffxi_proto::map::container as c;
        // 13327 = an equipable ring in the retail id space; equip_info lookup
        // decides wardrobe eligibility, not this test.
        let rows = item_action_rows(&mh_snapshot(), c::LOC_MOGSAFE, 1, 13327);
        assert_eq!(
            &labels(&rows)[..3],
            [ITEM_ACTION_USE, ITEM_ACTION_DROP, "Take Out"],
            "retail order: Use, Drop, then the moves"
        );
        match rows[2].action {
            DynamicMenuAction::MoveItem {
                from_container,
                to_container,
                from_slot,
                quantity,
                ..
            } => {
                assert_eq!(from_container, c::LOC_MOGSAFE);
                assert_eq!(to_container, c::LOC_INVENTORY);
                assert_eq!(from_slot, 1);
                assert_eq!(quantity, 1);
            }
            _ => panic!("Take Out must be a MoveItem"),
        }
    }

    #[test]
    fn inventory_item_offers_use_and_put_in_bags() {
        use ffxi_proto::map::container as c;
        let rows = item_action_rows(&mh_snapshot(), c::LOC_INVENTORY, 1, 4509);
        assert_eq!(rows[0].label, ITEM_ACTION_USE);
        assert!(
            matches!(
                rows[1].action,
                DynamicMenuAction::DropItem {
                    container: c::LOC_INVENTORY,
                    index: 1,
                    item_no: 4509,
                    quantity: 12,
                }
            ),
            "Drop carries the whole stack: {rows:?}"
        );
        assert!(rows.iter().any(|r| r.label == "Put in Mog Safe"));
        assert!(rows.iter().any(|r| r.label == "Put in Storage"));
        assert!(
            !rows.iter().any(|r| r.label.contains("Temporary")),
            "temp items bag is never a move destination: {rows:?}"
        );
        // 4509 (Distilled Water) is not equipment, so no wardrobe row.
        assert!(!rows.iter().any(|r| r.label.contains("Wardrobe")));
    }

    /// Gil and Temporary items never move (LSB 0x029 isValidMovement /
    /// validContainers); Gil cannot be dropped either, Temporary items can
    /// (0x028_item_dump.cpp validContainers).
    #[test]
    fn gil_and_temp_items_cannot_move() {
        use ffxi_proto::map::container as c;
        let snap = mh_snapshot();
        let gil = item_action_rows(&snap, c::LOC_INVENTORY, 0, ffxi_proto::map::GIL_ITEM_NO);
        assert_eq!(labels(&gil), [ITEM_ACTION_USE], "{gil:?}");
        let temp = item_action_rows(&snap, c::LOC_TEMPITEMS, 0, 4212);
        assert_eq!(
            labels(&temp),
            [ITEM_ACTION_USE, ITEM_ACTION_DROP],
            "{temp:?}"
        );
    }

    #[test]
    fn stale_slot_yields_no_rows() {
        use ffxi_proto::map::container as c;
        let snap = mh_snapshot();
        assert!(item_action_rows(&snap, c::LOC_MOGSAFE, 5, 13327).is_empty());
        assert!(
            item_action_rows(&snap, c::LOC_MOGSAFE, 1, 999).is_empty(),
            "item id mismatch means the slot changed under the menu"
        );
    }

    fn war_party_member(id: u32) -> kuluu_snapshot::PartyMember {
        kuluu_snapshot::PartyMember {
            id,
            act_index: 0x100,
            name: Some("Kupo".into()),
            hp: 30,
            mp: 0,
            tp: 0,
            hp_pct: 100,
            mp_pct: 0,
            zone_no: 230,
            main_job: 1,
            main_job_lv: 1,
            sub_job: 0,
            sub_job_lv: 0,
            is_party_leader: true,
            is_alliance_leader: false,
            in_mog_house: false,
            party_no: 0,
        }
    }

    /// Pins the emote-list gating: HELM-only ids never appear (server-initiated,
    /// emote.h), and the Job row needs both a known main job and its 0x11A bit.
    #[test]
    fn emote_rows_skip_helm_and_gate_job_on_0x11a_bits() {
        use ffxi_proto::map::emote;
        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        let rows = emote_rows(&snap);
        assert!(rows.iter().any(|r| r.label == "Wave (/wave)"));
        assert!(rows.iter().any(|r| r.label == "Aim (/aim)"));
        assert!(
            !rows.iter().any(|r| r.label.contains("Logging")),
            "HELM-only emotes are server-initiated"
        );
        let is_job = |r: &DynamicMenuRow| matches!(r.action, DynamicMenuAction::Emote { emote_id } if emote_id == emote::JOB);
        assert!(
            !rows.iter().any(is_job),
            "Job row hidden until 0x11A unlocks it"
        );

        snap.self_char_id = Some(1);
        snap.party = vec![war_party_member(1)];
        snap.emote_jobs = Some(0);
        assert!(!emote_rows(&snap).iter().any(is_job), "WAR bit not set");
        snap.emote_jobs = Some(1 << 0);
        assert!(
            emote_rows(&snap).iter().any(is_job),
            "WAR main + WAR gesture bit shows the Job row"
        );

        snap.party[0].main_job = 33;
        snap.emote_jobs = Some(u32::MAX);
        assert!(
            !emote_rows(&snap).iter().any(is_job),
            "wire-supplied main job past the u32 bit width reads locked (no shift panic)"
        );
    }

    #[test]
    fn emote_command_words_lowercase_names_except_job() {
        use ffxi_proto::map::emote;
        assert_eq!(emote_command_word(8), "wave");
        assert_eq!(emote_command_word(65), "dance1");
        assert_eq!(emote_command_word(emote::JOB), "jobemote");
    }

    #[test]
    fn status_labels_match_entries() {
        use crate::hud::status_panel::STATUS_ENTRIES;
        assert_eq!(STATUS_LABELS.len(), STATUS_ENTRIES.len());
        for (i, entry) in STATUS_ENTRIES.iter().enumerate() {
            assert_eq!(STATUS_LABELS[i], entry.label, "Status row {i} label drift");
        }
    }

    #[test]
    fn static_menus_fit_pool_and_window_when_taller_than_viewport() {
        for kind in [
            MenuKind::Root,
            MenuKind::Config,
            MenuKind::Debug,
            MenuKind::Graphics,
            MenuKind::Equipment,
            MenuKind::Status,
            MenuKind::Communication,
        ] {
            let total = static_entries(kind).len();
            assert!(
                total <= MAX_ENTRY_COUNT,
                "{kind:?} has {total} rows, exceeds row pool {MAX_ENTRY_COUNT}"
            );
            // Root pages internally; every other static menu that outgrows the
            // viewport must scroll a DYNAMIC_VISIBLE_ROWS window rather than
            // render every row off the bottom of the screen (kuluu-b9kr).
            let expected = if kind == MenuKind::Root {
                total
            } else {
                total.min(DYNAMIC_VISIBLE_ROWS)
            };
            assert_eq!(
                visible_window(kind, total),
                expected,
                "{kind:?} visible window must cap at the viewport height"
            );
        }
    }

    #[test]
    fn graphics_menu_scrolls_bottom_rows_into_view() {
        let total = graphics_rows(true).len();
        assert!(
            total > DYNAMIC_VISIBLE_ROWS,
            "test presumes Graphics outgrows the viewport"
        );
        let dynamic = DynamicMenu::default();
        let last = total - 1;
        let (t, start) = resolve_viewport(MenuKind::Graphics, last, &dynamic, true);
        let window = visible_window(MenuKind::Graphics, t);
        assert!(
            last >= start && last < start + window,
            "bottom row {last} not in window [{start}, {})",
            start + window
        );
    }

    #[test]
    fn current_time_appears_exactly_once_in_root() {
        assert_eq!(
            ROOT_ENTRIES
                .iter()
                .filter(|l| **l == ROOT_CURRENT_TIME)
                .count(),
            1
        );
    }

    #[test]
    fn debug_rows_map_to_distinct_panel_state() {
        let panels = crate::hud::HudPanels {
            perf: true,
            target_cycle: false,
            mesh_debug: true,
            noclip: true,
            ..Default::default()
        };
        assert!(debug_panel_state(DEBUG_PERF, &panels, false, false, false));
        assert!(!debug_panel_state(
            DEBUG_TARGET_CYCLE,
            &panels,
            false,
            false,
            false
        ));
        assert!(debug_panel_state(DEBUG_MESH, &panels, false, false, false));
        assert!(debug_panel_state(
            DEBUG_NOCLIP,
            &panels,
            false,
            false,
            false
        ));
        assert!(debug_panel_state(
            DEBUG_NET_STATUS,
            &panels,
            true,
            false,
            false
        ));
        assert!(!debug_panel_state(
            DEBUG_NET_STATUS,
            &panels,
            false,
            false,
            false
        ));
        assert!(debug_panel_state(
            DEBUG_FORCE_18,
            &panels,
            false,
            false,
            true
        ));
        assert!(!debug_panel_state(
            DEBUG_FORCE_18,
            &panels,
            false,
            false,
            false
        ));

        for label in DEBUG_ENTRIES {
            assert_eq!(
                static_entries(MenuKind::Debug)
                    .iter()
                    .filter(|e| *e == label)
                    .count(),
                1,
                "Debug row {label:?} must appear exactly once"
            );
        }
    }

    /// Two rows whose label order (Apple, Zeta) disagrees with their item-id
    /// order (50, 10), so each ordering branch is distinguishable.
    fn conflicting_item_rows() -> Vec<DynamicMenuRow> {
        let item_row = |item_no: u16| DynamicMenuAction::OpenItemAction {
            container: 0,
            index: 0,
            item_no,
        };
        vec![
            DynamicMenuRow {
                label: "Apple".to_string(),
                action: item_row(50),
            },
            DynamicMenuRow {
                label: "Zeta".to_string(),
                action: item_row(10),
            },
        ]
    }

    fn labels(rows: &[DynamicMenuRow]) -> Vec<&str> {
        rows.iter().map(|r| r.label.as_str()).collect()
    }

    #[test]
    fn key_item_rows_sorted_by_id_with_unseen_suffix() {
        // Key items 1/8 = Zeruhn Report / Airship Pass
        // (vendor/server/scripts/enum/key_item.lua).
        let snap = kuluu_snapshot::SceneSnapshot {
            key_items: vec![8, 1],
            key_items_seen: vec![8],
            ..Default::default()
        };
        let rows = key_item_rows(&snap);
        assert_eq!(
            labels(&rows),
            [
                format!("Zeruhn Report{KEY_ITEM_UNSEEN_SUFFIX}").as_str(),
                "Airship Pass"
            ]
        );
        assert_eq!(rows[0].action, DynamicMenuAction::KeyItem { id: 1 });

        let mut rows = rows;
        order_dynamic_rows(MenuKind::KeyItems, true, &mut rows);
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::KeyItem { id: 1 },
            "key items keep id order, never alphabetical"
        );
    }

    #[test]
    fn item_rows_sort_by_id_when_auto() {
        let mut rows = conflicting_item_rows();
        order_dynamic_rows(MenuKind::Items, true, &mut rows);
        assert_eq!(labels(&rows), ["Zeta", "Apple"]);
    }

    #[test]
    fn item_rows_keep_slot_order_when_manual() {
        let mut rows = conflicting_item_rows();
        order_dynamic_rows(MenuKind::Items, false, &mut rows);
        assert_eq!(labels(&rows), ["Apple", "Zeta"]);
    }

    #[test]
    fn item_action_rows_keep_built_order() {
        let mut rows = conflicting_item_rows();
        order_dynamic_rows(
            MenuKind::ItemAction {
                container: 0,
                index: 0,
                item_no: 50,
            },
            true,
            &mut rows,
        );
        assert_eq!(labels(&rows), ["Apple", "Zeta"]);
    }

    #[test]
    fn other_dynamic_menus_sort_alphabetically() {
        let mut rows = conflicting_item_rows();
        rows.swap(0, 1); // start Zeta-first so the sort has work to do
        order_dynamic_rows(MenuKind::Magic, true, &mut rows);
        assert_eq!(labels(&rows), ["Apple", "Zeta"]);
    }

    /// Retail's Log Out / Shut Down rows answer through a yes/no prompt titled
    /// after the row, sharing the Drop confirm's Yes/No law (cursor default No).
    #[test]
    fn power_prompts_share_the_yes_no_law_and_take_the_row_name() {
        for shut_down in [false, true] {
            let kind = MenuKind::PowerPrompt { shut_down };
            assert_eq!(static_entries(kind), DROP_CONFIRM_ENTRIES);
        }
        assert_eq!(
            menu_title(MenuKind::PowerPrompt { shut_down: false }),
            ROOT_LOG_OUT
        );
        assert_eq!(
            menu_title(MenuKind::PowerPrompt { shut_down: true }),
            ROOT_SHUT_DOWN
        );
    }

    #[test]
    fn config_page_groups_fields_and_keeps_controls_reachable() {
        use crate::graphics_settings::MenuAction;
        let rows = config_rows();
        assert_eq!(
            rows.last(),
            Some(&MenuRow::Action(MenuAction::Controls)),
            "Controls must stay the last row of the settings page"
        );
        assert!(matches!(rows.first(), Some(MenuRow::Header(_))));
        // The kuluu settings page hangs off the page-3 "Interface" row; the
        // page-2 "Config" row is retail's configwi hub (vault main_menu.md §35),
        // built now as MenuKind::RetailConfig.
        assert_eq!(
            root_child_kind("Interface", false, false),
            Some(MenuKind::Config)
        );
        assert_eq!(
            root_child_kind("Config", false, false),
            Some(MenuKind::RetailConfig)
        );
        assert!(static_entries(MenuKind::Controls).contains(&"Standard"));
        assert_eq!(pane_width_for(MenuKind::Config), GRAPHICS_PANE_WIDTH);
    }

    // FFXiMain.dll retail-2026-09 RVA 0x13F750 answers mgcmenu selects and maps
    // the eight item ids {4..B} to internal school ids esi ∈ {1,2,5,4,3,6,7,8}
    // through jump table 0x13F7CC - display order is NOT numeric (vault main_menu.md §21).
    #[test]
    fn mgcmenu_picker_rows_follow_the_jump_table_order() {
        let cats: Vec<u8> = (0..MAGIC_CATEGORY_ROWS.len())
            .map(|i| magic_category_at(i).unwrap())
            .collect();
        assert_eq!(cats, [1, 2, 5, 4, 3, 6, 7, 8]);
        assert_eq!(static_entries(MenuKind::MagicGroups), MAGIC_CATEGORY_LABELS);
        let labels: Vec<&str> = MAGIC_CATEGORY_ROWS.iter().map(|(_, l)| *l).collect();
        assert_eq!(labels.to_vec(), MAGIC_CATEGORY_LABELS.to_vec());
        assert_eq!(magic_category_for_label("Songs"), Some(5));
        assert_eq!(magic_category_at(MAGIC_CATEGORY_ROWS.len()), None);
    }

    #[test]
    fn magic_windows_keep_retail_titles() {
        // The picker carries the Magic row's own caption; the spell list it opens is
        // titled from the ROM/165/76.DAT catalog index 76 (§10).
        assert_eq!(menu_title(MenuKind::MagicGroups), "Magic");
        assert_eq!(menu_title(MenuKind::MagicGroup(1)), "Magic List");
    }

    #[test]
    fn magic_group_lists_only_its_school_and_casts_like_magic() {
        // No install loaded, so this exercises the LSB `group`-column fallback.
        let table = MagicSchoolTable::default();
        assert_eq!(table.school_of(1), Some(1)); // cure -> white
        assert_eq!(table.school_of(144), Some(2)); // fire -> black
        assert_eq!(table.school_of(603), Some(6)); // wild oats -> blue
        let rows = magic_school_rows(&[1, 144, 603], 2, |id| table.school_of(id));
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::CastSpell { spell_id: 144 }
        );
        let unknown_school = magic_school_rows(&[], 8, |id| table.school_of(id));
        assert!(unknown_school.is_empty());
        assert!(is_dynamic(MenuKind::MagicGroup(2)));
    }

    /// One real spell per school bucket (ids are the first row of each
    /// `spell_list.group` value in vendor/server/sql/spell_list.sql). The client's
    /// school numbering is a shuffled bijection of `xi.magic.spellGroup`
    /// (vault main_menu.md §21), so pinning all eight keeps the picker → list link
    /// from silently drifting when either enum changes.
    #[test]
    fn every_school_bucket_maps_real_spells_to_the_client_number() {
        let table = MagicSchoolTable::default();
        const PINNED: &[(u16, u8, &str)] = &[
            (1, MAGIC_CAT_WHITE, "cure (lsb group 6)"),
            (144, MAGIC_CAT_BLACK, "fire (2)"),
            (368, MAGIC_CAT_SONGS, "foe requiem (1)"),
            (318, MAGIC_CAT_NINJUTSU, "monomi ichi (4)"),
            (288, MAGIC_CAT_SUMMONING, "fire spirit (5)"),
            (513, MAGIC_CAT_BLUE, "venom shell (3)"),
            (768, MAGIC_CAT_GEOMANCY, "indi-regen (7)"),
            (896, MAGIC_CAT_TRUST, "shantotto (8)"),
        ];
        for (id, school, what) in PINNED {
            assert_eq!(table.school_of(*id), Some(*school), "{what}");
        }
        let all: Vec<u16> = PINNED.iter().map(|(id, _, _)| *id).collect();
        for (school, label) in MAGIC_CATEGORY_ROWS {
            let rows = magic_school_rows(&all, *school, |id| table.school_of(id));
            assert_eq!(
                rows.len(),
                1,
                "the {label} list must hold exactly one of the pinned spells"
            );
        }
    }

    /// The flat spell list (kuluu's Retail+ view) names every known spell from the
    /// scraped table and casts it by id; unknown ids drop out. Neither magic kind
    /// carries a static placeholder, so an empty one shows its grounded hint.
    #[test]
    fn magic_lists_build_from_data_and_keep_no_placeholder() {
        let rows = spell_rows(&[1, 144]);
        assert_eq!(
            rows.iter()
                .map(|r| (r.label.as_str(), r.action))
                .collect::<Vec<_>>(),
            vec![
                ("Cure", DynamicMenuAction::CastSpell { spell_id: 1 }),
                ("Fire", DynamicMenuAction::CastSpell { spell_id: 144 }),
            ]
        );
        assert!(spell_rows(&[0]).is_empty(), "unknown spell ids drop out");

        let none = DynamicMenu::default();
        assert!(static_entries(MenuKind::Magic).is_empty());
        assert!(static_entries(MenuKind::MagicGroup(2)).is_empty());
        assert_eq!(
            entry_label(MenuKind::Magic, 0, &none),
            "(no spells learned yet)"
        );
        assert_eq!(
            entry_label(MenuKind::MagicGroup(2), 0, &none),
            "(no spells learned in this school)"
        );
    }

    #[test]
    fn root_magic_row_drills_into_the_picker_unless_flat_is_on() {
        assert_eq!(
            root_child_kind("Magic", false, false),
            Some(MenuKind::MagicGroups)
        );
        // The Abilities row mirrors Magic: picker by default, flat list behind
        // the Retail+ toggle (vault main_menu.md §23).
        assert_eq!(
            root_child_kind("Abilities", false, false),
            Some(MenuKind::AbilitiesGroups)
        );
        assert_eq!(
            root_child_kind("Abilities", true, false),
            Some(MenuKind::AbilitiesGroups)
        );
        assert_eq!(
            root_child_kind("Abilities", false, true),
            Some(MenuKind::Abilities)
        );
        assert_eq!(root_child_kind("Magic", true, false), Some(MenuKind::Magic));
    }

    /// s2c COMMAND_DATA ships three bitmaps in three id spaces, so a
    /// weapon-skill row must be named from `weapon_skills` and a job/pet row from
    /// `abilities`. Ability id 32 is Warcry while weapon skill 32 is Fast Blade —
    /// naming one through the other table is the bug this pins (vault §52).
    #[test]
    fn ability_rows_name_each_id_space_from_its_own_table() {
        use crate::hud::action_model::AbilityGroup;
        let snap = kuluu_snapshot::SceneSnapshot {
            weaponskills_known: vec![1, 32],
            job_abilities_known: vec![16, 32],
            pet_abilities_known: vec![80],
            ..Default::default()
        };
        let ws: Vec<String> = ability_group_rows(&snap, AbilityGroup::WeaponSkill)
            .iter()
            .map(|r| r.label.clone())
            .collect();
        assert_eq!(ws, ["Combo", "Fast Blade"], "weapon skills sort by name");

        let job: Vec<String> = ability_group_rows(&snap, AbilityGroup::JobAbilities)
            .iter()
            .map(|r| r.label.clone())
            .collect();
        assert_eq!(job, ["Mighty Strikes", "Warcry"]);

        let pet: Vec<String> = ability_group_rows(&snap, AbilityGroup::PetCommand)
            .iter()
            .map(|r| r.label.clone())
            .collect();
        assert_eq!(pet, ["Spirit Link"]);
    }

    /// The flat list keeps the three spaces apart by suffix and drops nothing that
    /// a bitmap reports; neither ability kind carries a static placeholder, so an
    /// empty one falls back to its grounded hint.
    #[test]
    fn flat_ability_list_separates_the_spaces_and_keeps_no_placeholder() {
        let snap = kuluu_snapshot::SceneSnapshot {
            weaponskills_known: vec![32],
            job_abilities_known: vec![16],
            pet_abilities_known: vec![80],
            ..Default::default()
        };
        assert_eq!(
            ability_flat_rows(&snap)
                .iter()
                .map(|r| (r.label.as_str(), r.action))
                .collect::<Vec<_>>(),
            vec![
                (
                    "Mighty Strikes",
                    DynamicMenuAction::JobAbility { ability_id: 16 }
                ),
                (
                    "Fast Blade (WS)",
                    DynamicMenuAction::Weaponskill { skill_id: 32 }
                ),
                (
                    "Spirit Link (Pet)",
                    DynamicMenuAction::PetAbility { ability_id: 80 }
                ),
            ]
        );

        let none = DynamicMenu::default();
        assert!(static_entries(MenuKind::Abilities).is_empty());
        assert!(static_entries(MenuKind::AbilitiesGroup(1)).is_empty());
        assert_eq!(
            entry_label(MenuKind::Abilities, 0, &none),
            "(no abilities available — wrong job?)"
        );
    }

    /// The abimenu picker rows pair positionally with the target-action groups
    /// (AbilityGroup::ALL) that back their lists, and keep the retail two-window
    /// titles like Magic's pair (vault main_menu.md §23).
    #[test]
    fn abilities_picker_rows_pair_with_the_groups_and_keep_titles() {
        use crate::hud::action_model::AbilityGroup;
        assert_eq!(ABILITIES_CATEGORY_LABELS.len(), AbilityGroup::ALL.len());
        for (idx, _group) in AbilityGroup::ALL.iter().enumerate() {
            let row = ABILITIES_CATEGORY_LABELS[idx];
            assert!(!row.is_empty(), "picker row {idx} needs a label");
            assert_eq!(abilities_category_index(row), Some(idx));
        }
        assert_eq!(
            static_entries(MenuKind::AbilitiesGroups),
            ABILITIES_CATEGORY_LABELS
        );
        assert!(is_dynamic(MenuKind::AbilitiesGroup(2)));
        assert_eq!(menu_title(MenuKind::AbilitiesGroups), "Abilities");
        assert_eq!(menu_title(MenuKind::AbilitiesGroup(1)), "Ability List");
    }

    /// The Party window's rows follow the helper's state dispatch (FFXiMain.dll
    /// retail-2026-09 RVA 0x2016E0; vault main_menu.md §24): a pending solicit
    /// yields prtyjoin's YES/NO answer rows, else the roster renders as display
    /// rows, and solo leaves no rows so the "(not in a party)" hint stands.
    #[test]
    fn party_window_rows_follow_the_helper_law() {
        let solo = kuluu_snapshot::SceneSnapshot::default();
        assert!(
            party_window_rows(&solo).is_empty(),
            "solo shows the empty hint"
        );

        let rostered = kuluu_snapshot::SceneSnapshot {
            party: vec![war_party_member(7)],
            ..Default::default()
        };
        let rows = party_window_rows(&rostered);
        assert_eq!(labels(&rows), ["Kupo"]);
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::PartyMemberRow { char_id: 7 }
        );

        let invited = kuluu_snapshot::SceneSnapshot {
            party_invite: Some(kuluu_snapshot::PartyInvite {
                char_id: 9,
                name: "Pruneo".into(),
                alliance: false,
            }),
            party: vec![war_party_member(7)],
            ..Default::default()
        };
        let rows = party_window_rows(&invited);
        assert_eq!(labels(&rows), ["Yes (join Pruneo)", "No"]);
        assert_eq!(rows[0].action, DynamicMenuAction::AcceptPartyInvite);
        assert_eq!(rows[1].action, DynamicMenuAction::DeclinePartyInvite);
    }

    #[test]
    fn party_root_row_drills_into_the_party_window() {
        assert_eq!(
            root_child_kind("Party", false, false),
            Some(MenuKind::Party)
        );
        assert_eq!(menu_title(MenuKind::Party), "Party");
        assert!(is_dynamic(MenuKind::Party));
    }

    #[test]
    fn settings_pages_are_disjoint_and_have_no_duplicate_rows() {
        let mut seen: Vec<GraphicsField> = Vec::new();
        for page in [
            crate::graphics_settings::graphics_fields(),
            crate::graphics_settings::config_fields(),
            crate::graphics_settings::dlss_config_fields(),
        ] {
            for f in page {
                assert!(!seen.contains(&f), "{f:?} appears on more than one page");
                seen.push(f);
            }
        }
        assert!(
            crate::graphics_settings::dlss_config_fields()
                .iter()
                .all(|f| f.is_dlss_config()),
            "every DLSS submenu row must be flagged is_dlss_config"
        );
        assert!(
            !crate::graphics_settings::graphics_fields()
                .iter()
                .any(|f| f.is_dlss_config()),
            "DLSS submenu rows must not also sit on the main Graphics page"
        );
    }

    #[test]
    fn field_rows_carry_their_field_label() {
        for kind in [MenuKind::Graphics, MenuKind::Config, MenuKind::GraphicsDlss] {
            for (slot, row) in settings_rows(kind, true).unwrap().iter().enumerate() {
                if let MenuRow::Field(field) = row {
                    assert_eq!(
                        settings_field_at(kind, slot, true),
                        Some(*field),
                        "{kind:?} slot {slot} resolves to the wrong field"
                    );
                    assert_eq!(row.label(), field.label());
                }
            }
        }
    }

    #[test]
    fn graphics_rows_drop_both_dlss_rows_when_unsupported() {
        use crate::graphics_settings::MenuAction;
        let bare = graphics_rows(false);
        assert!(!bare
            .iter()
            .any(|r| *r == MenuRow::Field(GraphicsField::Dlss)
                || *r == MenuRow::Action(MenuAction::DlssConfig)));
        assert_eq!(
            bare.last(),
            Some(&MenuRow::Action(MenuAction::ResetToMinimum))
        );

        let full = graphics_rows(true);
        let dlss = full
            .iter()
            .position(|r| *r == MenuRow::Field(GraphicsField::Dlss))
            .expect("DLSS row present when supported");
        assert_eq!(
            full[dlss + 1],
            MenuRow::Action(MenuAction::DlssConfig),
            "the config row must sit directly under the DLSS on/off row"
        );
        assert_eq!(bare.len(), full.len() - 2);
    }

    #[test]
    fn the_cursor_never_rests_on_a_header() {
        for kind in [MenuKind::Graphics, MenuKind::Config, MenuKind::GraphicsDlss] {
            for dlss in [true, false] {
                let rows = settings_rows(kind, dlss).unwrap();
                for (slot, _) in rows.iter().enumerate() {
                    for down in [true, false] {
                        let settled = settle_cursor(kind, dlss, slot, down);
                        assert!(
                            rows[settled].is_selectable(),
                            "{kind:?} slot {slot} settled onto header row {settled}"
                        );
                    }
                }
                assert!(rows[settle_cursor(kind, dlss, 0, true)].is_selectable());
            }
        }
    }

    #[test]
    fn enhanced_only_rows_sit_in_the_trailing_group() {
        use crate::graphics_settings::{ENHANCED_SECTION, GRAPHICS_SECTIONS};
        let last = GRAPHICS_SECTIONS.last().expect("sections");
        assert_eq!(
            last.header, ENHANCED_SECTION,
            "the no-Vanilla-equivalent group must be the last section on the page"
        );
        for section in GRAPHICS_SECTIONS
            .iter()
            .filter(|s| s.header != ENHANCED_SECTION)
        {
            assert!(
                !section.fields.contains(&GraphicsField::CameraSpring)
                    && !section.fields.contains(&GraphicsField::DepthOfField)
                    && !section.fields.contains(&GraphicsField::BloomIntensity),
                "{} must not hold a row the original client never had",
                section.header
            );
        }
    }

    /// One-slot snapshot builder for the LSB 0x037 usability gate tests
    /// (kuluu-268h). Item ids pinned against vendor/server/sql:
    /// 4112 = potion (consumable), 15840 = kupofried's ring (charged
    /// equipment), 13327 = silver earring (plain equipment, no activation).
    fn slot_snapshot(container: u8, item_no: u16, locked: bool) -> kuluu_snapshot::SceneSnapshot {
        kuluu_snapshot::SceneSnapshot {
            containers: vec![kuluu_snapshot::ContainerView {
                id: container,
                capacity: 30,
                items: vec![kuluu_snapshot::InventoryItem {
                    container,
                    index: 0,
                    item_no,
                    quantity: 1,
                    locked,
                    unselectable: false,
                    charges_remaining: None,
                    next_use_vana_ts: None,
                    use_delay_end_vana_ts: None,
                    ready: None,
                }],
            }],
            ..Default::default()
        }
    }

    fn only_slot(snap: &kuluu_snapshot::SceneSnapshot) -> &kuluu_snapshot::InventoryItem {
        &snap.containers[0].items[0]
    }

    /// Consumables fire from LOC_INVENTORY and LOC_TEMPITEMS only
    /// (0x037 → CState container check).
    #[test]
    fn consumable_usable_from_inventory_and_tempitems_only() {
        use ffxi_proto::map::container as c;
        for (container, expect) in [
            (c::LOC_INVENTORY, true),
            (c::LOC_TEMPITEMS, true),
            (c::LOC_MOGSAFE, false),
            (c::LOC_STORAGE, false),
            (c::LOC_WARDROBE, false),
        ] {
            let snap = slot_snapshot(container, 4112, false);
            assert_eq!(
                item_usable_now(&snap, only_slot(&snap)),
                expect,
                "potion in container {container}"
            );
        }
    }

    /// Locked (bazaar/linkshell-reserved) consumables are rejected by the
    /// server, so the menu must not offer them.
    #[test]
    fn locked_consumable_is_not_usable() {
        use ffxi_proto::map::container as c;
        let snap = slot_snapshot(c::LOC_INVENTORY, 4112, true);
        assert!(!item_usable_now(&snap, only_slot(&snap)));
    }

    /// Charged equipment (kupofried's ring) is usable only while equipped:
    /// locked slot + mirrored in SceneSnapshot::equipped.
    #[test]
    fn charged_equipment_usable_only_while_equipped() {
        use ffxi_proto::map::container as c;
        let mut snap = slot_snapshot(c::LOC_INVENTORY, 15840, true);
        snap.equipped[13] = Some(15840);
        assert!(item_usable_now(&snap, only_slot(&snap)));

        // Sitting unequipped in the bag: not usable.
        let unequipped = slot_snapshot(c::LOC_INVENTORY, 15840, false);
        assert!(!item_usable_now(&unequipped, only_slot(&unequipped)));

        // Equipped but stored via a non-wardrobe container: not usable.
        let mut stored = slot_snapshot(c::LOC_MOGSAFE, 15840, true);
        stored.equipped[13] = Some(15840);
        assert!(!item_usable_now(&stored, only_slot(&stored)));
    }

    /// Plain equipment (no item_usable entry) never appears, even equipped.
    #[test]
    fn plain_equipment_is_never_usable() {
        use ffxi_proto::map::container as c;
        let mut snap = slot_snapshot(c::LOC_INVENTORY, 13327, true);
        snap.equipped[11] = Some(13327);
        assert!(!item_usable_now(&snap, only_slot(&snap)));
    }

    /// `vendor/server/sql/item_equipment.sql` rows used by the equip pick-list
    /// tests, with their (`level`, `jobs`, `slot`) columns: bronze cap (1, WAR,
    /// head), Pummelers Mask +2 (99, WAR-only, head), white belt (1, MNK-only,
    /// waist), bronze dagger (1, WAR, main+sub).
    const BRONZE_CAP: u16 = 12448;
    const PM_MASK_PLUS_2: u16 = 23040;
    const WHITE_BELT: u16 = 13184;
    const BRONZE_DAGGER: u16 = 16448;

    /// Main-bag snapshot holding those four pieces, self rostered as `main_job`
    /// at level `main_job_lv` (the pair the server's gate reads).
    fn equip_bag_snapshot(main_job: u8, main_job_lv: u8) -> kuluu_snapshot::SceneSnapshot {
        let gear = |index: u8, item_no: u16| kuluu_snapshot::InventoryItem {
            container: 0,
            index,
            item_no,
            quantity: 1,
            locked: false,
            unselectable: false,
            charges_remaining: None,
            next_use_vana_ts: None,
            use_delay_end_vana_ts: None,
            ready: None,
        };
        let mut member = war_party_member(7);
        member.main_job = main_job;
        member.main_job_lv = main_job_lv;
        kuluu_snapshot::SceneSnapshot {
            self_char_id: Some(7),
            party: vec![member],
            containers: vec![kuluu_snapshot::ContainerView {
                id: 0,
                capacity: 30,
                items: vec![
                    gear(4, BRONZE_CAP),
                    gear(5, PM_MASK_PLUS_2),
                    gear(6, WHITE_BELT),
                    gear(7, BRONZE_DAGGER),
                ],
            }],
            ..Default::default()
        }
    }

    /// What the window actually shows: `equip_slot_rows` then the refresh pass's
    /// ordering step for this kind.
    fn equip_pick_labels(snap: &kuluu_snapshot::SceneSnapshot, equip_slot: u8) -> Vec<String> {
        let mut rows = equip_slot_rows(snap, equip_slot);
        order_dynamic_rows(MenuKind::EquipSlot(equip_slot), true, &mut rows);
        rows.iter().map(|r| r.label.clone()).collect()
    }

    /// The pick list is the bag filtered by the server's own acceptance gate
    /// (`EquipArmor`: equip-slot bit, main-job bit, required level) — vault
    /// main_menu.md §50. A wrong-slot item never shows; an over-level or
    /// wrong-job item never shows.
    #[test]
    fn equip_pick_list_filters_by_slot_job_and_level() {
        let war40 = equip_bag_snapshot(1, 40);
        assert_eq!(
            equip_pick_labels(&war40, 4),
            ["Bronze Cap (Lv1)"],
            "head: the level-99 mask is gated and the belt/dagger are wrong-slot"
        );
        assert_eq!(equip_pick_labels(&war40, 0), ["Bronze Dagger (Lv1)"]);
        assert!(
            equip_pick_labels(&war40, 10).is_empty(),
            "a white belt is MNK-only: WAR must not be offered it"
        );

        let mnk99 = equip_bag_snapshot(2, 99);
        assert_eq!(
            equip_pick_labels(&mnk99, 10),
            ["White Belt (Lv1)"],
            "MNK gets the belt it can wear"
        );
        assert_eq!(
            equip_pick_labels(&mnk99, 4),
            ["Bronze Cap (Lv1)"],
            "the WAR-only mask stays out even at level 99"
        );
    }

    /// Rows carry the exact c2s 0x050 field triple (bag, bag index, equip slot)
    /// so a select equips that very slot, and the empty state renders the
    /// grounded hint rather than any loading placeholder.
    #[test]
    fn equip_pick_rows_carry_the_equip_set_fields_and_hint_when_empty() {
        let snap = equip_bag_snapshot(1, 40);
        let rows = equip_slot_rows(&snap, 4);
        assert_eq!(
            rows.iter().map(|r| r.action).collect::<Vec<_>>(),
            vec![DynamicMenuAction::EquipItem {
                container: 0,
                container_index: 4,
                equip_slot: 4,
                item_no: BRONZE_CAP,
            }],
            "one row, and it must name the bag slot the packet carries"
        );

        let empty = DynamicMenu::default();
        assert!(static_entries(MenuKind::EquipSlot(4)).is_empty());
        assert_eq!(
            entry_label(MenuKind::EquipSlot(4), 0, &empty),
            "(no equippable items for this slot)"
        );
    }

    /// The quick-menu Items rows fire Use directly and skip unusable slots;
    /// any_usable_item gates the entry itself.
    #[test]
    fn usable_item_rows_fire_use_directly() {
        use ffxi_proto::map::container as c;
        let snap = slot_snapshot(c::LOC_INVENTORY, 4112, false);
        let rows = usable_item_rows(&snap);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(matches!(
            rows[0].action,
            DynamicMenuAction::UseItem {
                container: c::LOC_INVENTORY,
                index: 0,
                item_no: 4112,
            }
        ));
        assert!(any_usable_item(&snap));

        let safe = slot_snapshot(c::LOC_MOGSAFE, 4112, false);
        assert!(usable_item_rows(&safe).is_empty());
        assert!(!any_usable_item(&safe));
    }

    fn trade_offer(item_no: u16, qty: u32) -> kuluu_snapshot::TradeOffer {
        kuluu_snapshot::TradeOffer { item_no, qty }
    }

    /// The trade window's row law (FFXiMain.dll retail-2026-09 vtable 0x335060
    /// handler RVA 0x1515E0; vault main_menu.md §25): an unanswered s2c 0x021
    /// yields the answer rows; a live trade shows gil (trade slot 0 per LSB's
    /// `GilSlot = 0`), own item slots {1..8} (select byte table .rdata 0x371D60),
    /// then the button law {9=Confirm, 10=Exit}; finally the partner-side views.
    #[test]
    fn trade_window_rows_follow_the_helper_law() {
        use kuluu_snapshot::{TradeOffer, TradeWindow};

        // No transaction behind the window: the empty-hint display state.
        let solo = kuluu_snapshot::SceneSnapshot::default();
        assert!(trade_window_rows(&solo).is_empty());

        // Unanswered invite (no entity fixture, so the hex fallback names the
        // partner): YES/NO rows answer c2s TRADE_RES Start/Cancell.
        let invited = kuluu_snapshot::SceneSnapshot {
            trade: Some(Box::new(TradeWindow {
                partner_id: 42,
                incoming_pending: true,
                active: false,
                my_slots: Vec::new(),
                partner_slots: Vec::new(),
                my_confirmed: false,
                partner_confirmed: false,
            })),
            ..Default::default()
        };
        let rows = trade_window_rows(&invited);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[0].label, "Yes (accept 0000002A)");
        assert_eq!(rows[1].label, "No");
        assert_eq!(rows[0].action, DynamicMenuAction::TradeAcceptInvite);
        assert_eq!(rows[1].action, DynamicMenuAction::TradeDeclineInvite);

        // Live trade: gil staged in slot 0 plus an item stack in slot 3, the
        // partner offering a single unit in their slot 7.
        let mut my_slots: Vec<Option<TradeOffer>> = vec![None; 9];
        my_slots[0] = Some(trade_offer(ffxi_proto::decode::TRADE_GIL_ITEM_NO, 150));
        my_slots[3] = Some(trade_offer(4112, 2));
        let mut partner_slots: Vec<Option<TradeOffer>> = vec![None; 9];
        partner_slots[7] = Some(trade_offer(4112, 1));
        let live = kuluu_snapshot::SceneSnapshot {
            trade: Some(Box::new(TradeWindow {
                partner_id: 42,
                incoming_pending: false,
                active: true,
                my_slots,
                partner_slots,
                my_confirmed: false,
                partner_confirmed: true,
            })),
            ..Default::default()
        };
        let rows = trade_window_rows(&live);
        let potion = ffxi_vocab::item_names::lookup(4112).expect("4112 has a retail name");
        assert_eq!(
            rows.len(),
            1 + 8 + 2 + 9,
            "gil cell + own slots 1..=8 + Confirm/Exit + partner views: {rows:?}"
        );
        assert_eq!(&rows[0].label, &format!("Gil x{GIL}", GIL = 150));
        assert!(matches!(rows[0].action, DynamicMenuAction::TradeGilRow));
        // Row index == own trade-slot index (slot 0 sits above slots 1..8 in the
        // same column), so slot 3 is row 3.
        assert_eq!(&rows[3].label, &format!("{potion} x2"));
        assert_eq!(
            rows[3].action,
            DynamicMenuAction::TradeOwnSlot { trade_index: 3 }
        );
        assert_eq!(rows[1].label, "(empty)");
        assert_eq!(&rows[9].label, "Confirm");
        assert!(matches!(rows[9].action, DynamicMenuAction::TradeConfirmRow));
        assert_eq!(&rows[10].label, "Exit");
        assert!(matches!(rows[10].action, DynamicMenuAction::TradeCancelRow));
        // Partner views append after the buttons, their slot 7 at row 18.
        assert_eq!(&rows[18].label, potion);
        assert_eq!(
            rows[18].action,
            DynamicMenuAction::TradePartnerOfferRow { trade_index: 7 }
        );
    }

    /// The Add-Item picker skips locked and server-unselectable slots (LSB's
    /// setSlot refuses them) and stages ONE unit per pick while the qty keypad
    /// stays [I-pending] (vault main_menu.md §25).
    #[test]
    fn trade_pick_rows_skip_locked_slots() {
        let snap = kuluu_snapshot::SceneSnapshot {
            containers: vec![kuluu_snapshot::ContainerView {
                id: 0,
                capacity: 30,
                items: vec![
                    kuluu_snapshot::InventoryItem {
                        container: 0,
                        index: 4,
                        item_no: 4112,
                        quantity: 3,
                        locked: false,
                        unselectable: false,
                        charges_remaining: None,
                        next_use_vana_ts: None,
                        use_delay_end_vana_ts: None,
                        ready: None,
                    },
                    kuluu_snapshot::InventoryItem {
                        container: 0,
                        index: 5,
                        item_no: 4112,
                        quantity: 1,
                        locked: true,
                        unselectable: false,
                        charges_remaining: None,
                        next_use_vana_ts: None,
                        use_delay_end_vana_ts: None,
                        ready: None,
                    },
                ],
            }],
            ..Default::default()
        };
        let rows = trade_pick_rows(&snap, 5);
        assert_eq!(
            rows.len(),
            1,
            "the locked slot must not be offered: {rows:?}"
        );
        let potion = ffxi_vocab::item_names::lookup(4112).expect("4112 has a retail name");
        assert_eq!(&rows[0].label, &format!("{potion} x3"));
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::TradeStageFromBag {
                trade_index: 5,
                item_no: 4112,
                qty: 1,
                bag_slot: 4,
            }
        );
    }

    fn craft_bag(items: &[(u8, u16, u32)], key_items: &[u16]) -> kuluu_snapshot::SceneSnapshot {
        kuluu_snapshot::SceneSnapshot {
            containers: vec![kuluu_snapshot::ContainerView {
                id: 0,
                capacity: 30,
                items: items
                    .iter()
                    .map(|(index, item_no, quantity)| kuluu_snapshot::InventoryItem {
                        container: 0,
                        index: *index,
                        item_no: *item_no,
                        quantity: *quantity,
                        locked: false,
                        unselectable: false,
                        charges_remaining: None,
                        next_use_vana_ts: None,
                        use_delay_end_vana_ts: None,
                        ready: None,
                    })
                    .collect(),
            }],
            key_items: key_items.to_vec(),
            ..Default::default()
        }
    }

    fn plan_of(row: &DynamicMenuRow) -> Vec<ffxi_proto::synth::SynthIngredient> {
        match row.action {
            DynamicMenuAction::SynthesizeRecipe { ingredients, .. } => {
                ingredients.into_iter().flatten().collect()
            }
            other => panic!("expected a synth plan, got {other:?}"),
        }
    }

    fn crystal_of(row: &DynamicMenuRow) -> (u16, u8) {
        match row.action {
            DynamicMenuAction::SynthesizeRecipe {
                crystal_id,
                crystal_slot,
                ..
            } => (crystal_id, crystal_slot),
            other => panic!("expected a synth plan, got {other:?}"),
        }
    }

    /// Craft-list law (vault main_menu.md §28): a recipe earns a row when its crystal passes the
    /// element-tier gate (`crystalProps` folds tiers; `resolveRecipe` keys on the folded name) and
    /// every positional ingredient assigns to bag slots under LSB's per-stack use law. Rows carry
    /// the wire plan verbatim, in recipe-id order; key-item gates mirror `resolveRecipe`.
    #[test]
    fn craft_list_rows_are_the_recipes_the_bag_can_currently_satisfy() {
        assert!(craft_list_rows(&kuluu_snapshot::SceneSnapshot::default()).is_empty());

        // synth_recipes rows 1-3: Banquet Table (earth crystal + 3553/3554/3555), Arrowwood
        // Lumber (wind crystal + one 688), and the key-item-1986 synth (688 x3 + 1657). The seeded
        // crystals are each ONE TIER UP (CYCLONE for wind, TERRA for earth) to also prove the §28
        // element-tier substitution law on every row.
        let bag = craft_bag(
            &[
                (2, 4240, 1),
                (3, 4241, 1),
                (5, 3553, 1),
                (6, 3554, 1),
                (7, 3555, 1),
                (8, 688, 3),
                (9, 1657, 1),
            ],
            &[1986],
        );
        let rows = craft_list_rows(&bag);

        let table = rows
            .iter()
            .find(|r| r.label == "Banquet Table")
            .expect("row 1");
        assert_eq!(crystal_of(table), (4241, 3));
        assert_eq!(
            plan_of(table).iter().map(|e| e.item_id).collect::<Vec<_>>(),
            vec![3553, 3554, 3555],
            "entries keep the recipe row's own column order"
        );

        // Stack law on the KI synth: three copies of item 688 share ONE slot-8 stack (LSB counts
        // uses per slot against the quantity - slotQty in GP_CLI_COMMAND_COMBINE_ASK::process).
        let triple = rows
            .iter()
            .find(|r| plan_of(r).iter().any(|e| e.item_id == 1657))
            .expect("the KI synth");
        assert_eq!(triple.label, "Arrowwood Lumber");
        assert_eq!(
            plan_of(triple)
                .iter()
                .map(|e| (e.item_id, e.inv_slot))
                .collect::<Vec<_>>(),
            vec![(688, 8), (688, 8), (688, 8), (1657, 9)]
        );

        // Id order: Banquet Table (id 1) precedes both Arrowwood rows (ids 2 and 3), and the KI gate
        // drops id 3 when the key item is unknown (`resolveRecipe`).
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        let first_table = labels.iter().position(|l| *l == "Banquet Table").unwrap();
        let last_row = labels.len() - 1;
        assert!(labels[..last_row].contains(&"Arrowwood Lumber"));
        assert_eq!(
            rows.iter()
                .filter(|r| r.label == "Arrowwood Lumber")
                .count(),
            2,
            "id order keeps both Arrowwood rows after Banquet Table (pos {first_table})"
        );

        let no_ki = craft_bag(
            &[
                (2, 4240, 1),
                (3, 4241, 1),
                (5, 3553, 1),
                (6, 3554, 1),
                (7, 3555, 1),
                (8, 688, 3),
            ],
            &[],
        );
        assert_eq!(
            craft_list_rows(&no_ki)
                .iter()
                .filter(|r| r.label == "Arrowwood Lumber")
                .count(),
            1,
            "without KI 1986 the server cancels that recipe"
        );

        // A bag without any matching crystal earns nothing even when materials sit there.
        let no_crystal = craft_bag(&[(5, 3553, 1), (6, 3554, 1), (7, 3555, 1)], &[]);
        assert!(craft_list_rows(&no_crystal)
            .iter()
            .all(|r| r.label != "Banquet Table"));
    }

    /// The `is_dynamic` registry must list every kind whose rows come from the refresh arms;
    /// Search and Linkshell were missing it until the craft-list round (§26/§27 surfaces).
    #[test]
    fn server_state_windows_are_registered_as_dynamic() {
        assert!(is_dynamic(MenuKind::Search));
        assert!(is_dynamic(MenuKind::Linkshell));
        assert!(is_dynamic(MenuKind::CraftList));
    }

    /// The cmbmenu launcher law (vault main_menu.md §22/§28): two fixed rows in select-id order;
    /// the window keeps the Synthesis title, and the Root row drills it directly. Captions stay
    /// [I] from DATs (§22 caption law), so kuluu's labels are pinned here instead.
    #[test]
    fn synth_launcher_rows_and_title_track_the_cmbmenu_law() {
        assert_eq!(
            entries_for(MenuKind::SynthLauncher, false),
            &[SYNTH_ROW_PICK_MATERIAL, SYNTH_ROW_OPEN_LIST]
        );
        assert_eq!(menu_title(MenuKind::SynthLauncher), "Synthesis");
        assert_eq!(menu_title(MenuKind::CraftList), "Craft List");
        assert_eq!(
            root_child_kind("Synthesis", false, false),
            Some(MenuKind::SynthLauncher)
        );
    }

    /// The mcrmenu launcher law (vault main_menu.md §34): two rows in
    /// select-id order, the DAT-derived MacroPalette title (comwin token 80 =
    /// ROM/165/76.DAT catalog index 80), and the Root `Edit Macros` row
    /// drilling it directly. Row captions stay [I] (§34 chrome-only law).
    #[test]
    fn macros_palette_launcher_rows_and_title_track_the_mcrmenu_law() {
        assert_eq!(
            entries_for(MenuKind::MacrosPalette, false),
            &[MACRO_ROW_SELECT_MACRO, MACRO_ROW_MACRO_RESULT]
        );
        assert_eq!(menu_title(MenuKind::MacrosPalette), "MacroPalette");
        assert_eq!(
            root_child_kind("Edit Macros", false, false),
            Some(MenuKind::MacrosPalette)
        );
    }

    /// The configwi hub law (vault main_menu.md §35/§43): twelve page rows in chunk
    /// widget file order, each captioned by its `windowpsNNN` suffix id indexing catalog
    /// table 11 = ROM/165/75.DAT (§43; supersedes §35's chrome-only numbering), the
    /// DAT-grounded title Config (comwin token 54 = catalog string `Config`), and Root
    /// `Config` drilling it.
    #[test]
    fn retail_config_hub_rows_and_title_track_the_configwi_law() {
        let entries = entries_for(MenuKind::RetailConfig, false);
        assert_eq!(entries.len(), 12);
        assert_eq!(entries[0], "Adjust the keyboard settings.");
        assert_eq!(
            entries[6],
            "Remove onscreen battle effects to improve frame rate."
        );
        assert_eq!(
            entries[10],
            "Designate which messages appear in which window. Only active when multiple log windows are displayed."
        );
        assert_eq!(
            entries[11],
            "Adjust further miscellaneous settings for FINAL FANTASY XI."
        );
        assert_eq!(menu_title(MenuKind::RetailConfig), "Config");
        assert_eq!(
            root_child_kind("Config", false, false),
            Some(MenuKind::RetailConfig)
        );
    }

    #[test]
    fn retail_config_page_rows_track_each_window_dat_chunk() {
        // Page rows are the option widgets of each window's own DAT chunk (vault
        // main_menu.md §47, and §49 for the two windows that exist only in the newer
        // DAT generation). Counts pin which page is which; empty pages are `cfilter`
        // and `fxfilter`, whose rows still have to come from handler 0x197BE0.
        let counts: Vec<usize> = (0..12u8)
            .map(|page| entries_for(MenuKind::RetailConfigPage(page), false).len())
            .collect();
        assert_eq!(counts, vec![11, 0, 1, 2, 11, 14, 0, 12, 1, 1, 6, 6]);
        let conf6win = entries_for(MenuKind::RetailConfigPage(5), false);
        assert_eq!(
            conf6win[0],
            "Adjust the distance of the back clipping plane of the 3D view."
        );
        let conf13wi = entries_for(MenuKind::RetailConfigPage(11), false);
        assert_eq!(conf13wi[0], "Disable software keyboard.");
        for page in 0..12u8 {
            assert_eq!(menu_title(MenuKind::RetailConfigPage(page)), "Config");
        }
    }

    /// The faqsub panel law (vault main_menu.md §36): three destinations
    /// labelled from the window's own caption-builder strings (table 9 ids
    /// 0xC3/0xCF/0xDD), title `Help Desk` (ROM/165/76.DAT idx 60), and the
    /// Root Help Desk row drills it.
    #[test]
    fn helpdesk_panel_rows_and_title_track_the_faqsub_law() {
        assert_eq!(
            entries_for(MenuKind::HelpDeskPanel, false),
            &["Q&A", "Mentor Search", "Config"]
        );
        assert_eq!(menu_title(MenuKind::HelpDeskPanel), "Help Desk");
        assert_eq!(
            root_child_kind("Help Desk", false, false),
            Some(MenuKind::HelpDeskPanel)
        );
    }

    /// The Equipment-row variant gate order, fresh from FFXiMain.dll
    /// retail-2026-09 RVA 0x1404D5 (vault main_menu.md §39): Monstrosity beats
    /// puppetry, puppetry beats the ordinary equip window; data presence on the
    /// folded snapshot stands in for both client flags.
    #[test]
    fn equipment_row_variant_gate_order_follows_the_dll_branch_law() {
        let plain = kuluu_snapshot::SceneSnapshot::default();
        assert_eq!(equipment_drill_kind(&plain), MenuKind::Equipment);

        let automaton = kuluu_snapshot::AutomatonInfo {
            is_sub_job: false,
            head: 0,
            frame: 0,
            attachments: [0; 12],
            unlocked_heads: 0x3F,
            unlocked_frames: 0x0F,
        };
        let pup = kuluu_snapshot::SceneSnapshot {
            automaton: Some(Box::new(automaton.clone())),
            ..Default::default()
        };
        assert_eq!(equipment_drill_kind(&pup), MenuKind::AutomatonEquip);

        // Both records present — the DLL tests the Monstrosity predicate first
        // (call 0x97D50 before the flag/job compares), so mp wins.
        let both = kuluu_snapshot::SceneSnapshot {
            automaton: Some(Box::new(automaton)),
            monstrosity: Some(kuluu_snapshot::MonstrosityInfo {
                species: 1,
                equipped_instincts: [0; 12],
            }),
            ..Default::default()
        };
        assert_eq!(equipment_drill_kind(&both), MenuKind::MpCustomize);
    }

    /// The Map-row variant gate, fresh from FFXiMain.dll retail-2026-09 RVA
    /// 0x140653 (vault main_menu.md §40): the myroom name-open only when the
    /// server holds the player in a Mog House; data presence stands in.
    #[test]
    fn map_row_variant_gate_follows_mog_house_presence() {
        let plain = kuluu_snapshot::SceneSnapshot::default();
        assert_eq!(map_drill_kind(&plain), MenuKind::Map);

        let home = kuluu_snapshot::SceneSnapshot {
            myroom: Some(kuluu_snapshot::MyRoom {
                model: 615,
                sub_map: 1,
            }),
            ..Default::default()
        };
        assert_eq!(map_drill_kind(&home), MenuKind::HomeView);
    }

    /// The myroom panel rows and inherited title (DAT 'myro' caption tokens
    /// against ROM/165/76.DAT; vault main_menu.md §40 as corrected by §53 —
    /// fourteen named widgets, display order pinned twice over by the geometry
    /// y column 0x15..0xE5 and the records' prev/next chain ring).
    #[test]
    fn home_view_rows_and_title_track_the_myro_dat_law() {
        assert_eq!(
            entries_for(MenuKind::HomeView, false),
            &[
                "M.Ward.",
                "Mog Safe",
                "Storage",
                "Mog Locker",
                "Satchel",
                "Open Mog House",
                "Mog Case",
                "Mog Sack",
                "Delivery Box",
                "Change Jobs",
                "Gardening",
                "Layout",
                "Safe 2",
                "Remodel",
            ]
        );
        assert_eq!(menu_title(MenuKind::HomeView), "Map");
    }

    /// Container rows name their LSB bags (vendor/server/src/map/item_container.h)
    /// and the Gardening/Layout rows their literals — id 4 = "/garden", id 5 =
    /// "/layout" per the corrected arms of handler FFXiMain.dll retail-2026-09
    /// RVA 0x145460 (vault main_menu.md §53).
    #[test]
    fn home_view_rows_map_to_lsb_bags_and_text_command_literals() {
        use ffxi_proto::map::container as c;
        assert_eq!(home_view_row_container("Mog Safe"), Some(c::LOC_MOGSAFE));
        assert_eq!(home_view_row_container("Storage"), Some(c::LOC_STORAGE));
        assert_eq!(
            home_view_row_container("Mog Locker"),
            Some(c::LOC_MOGLOCKER)
        );
        assert_eq!(home_view_row_container("Satchel"), Some(c::LOC_MOGSATCHEL));
        assert_eq!(home_view_row_container("Mog Case"), Some(c::LOC_MOGCASE));
        assert_eq!(home_view_row_container("Mog Sack"), Some(c::LOC_MOGSACK));
        assert_eq!(home_view_row_container("M.Ward."), Some(c::LOC_WARDROBE));
        assert_eq!(home_view_row_container("Safe 2"), Some(c::LOC_MOGSAFE2));
        assert_eq!(home_view_text_command("Gardening"), Some("/garden"));
        assert_eq!(home_view_text_command("Layout"), Some("/layout"));
        // The rows whose retail arms are unbuilt stay un-mapped here and reach
        // the dispatcher's NotImplemented path.
        for label in ["Open Mog House", "Delivery Box", "Remodel", "Change Jobs"] {
            assert!(home_view_row_container(label).is_none());
            assert!(home_view_text_command(label).is_none());
        }
    }

    /// `jobchang` shows its two DAT rows; the pick lists build from the folded
    /// s2c 0x01B roster — locked jobs are not offered, and a sub-side list
    /// waits for the packet's grant (vault main_menu.md §53).
    #[test]
    fn job_change_drill_rows_and_roster_built_picks() {
        assert_eq!(
            entries_for(MenuKind::JobChange, false),
            &["Main Job", "Support Job"]
        );

        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        assert!(job_pick_rows(&snap, true).is_empty(), "roster unknown");
        snap.jobs = Some(kuluu_snapshot::JobList {
            unlocked: (1 << 1) | (1 << 3),
            sub_job_unlocked: false,
            job_levels: vec![0, 99, 0, 24],
        });
        let mains = job_pick_rows(&snap, true);
        assert_eq!(mains.len(), 2, "only unlocked jobs are offered");
        let warrior = ffxi_vocab::job_names::lookup(1).unwrap_or("???");
        assert_eq!(format!("{warrior} LV99"), mains[0].label);
        assert!(matches!(
            mains[0].action,
            DynamicMenuAction::HomeJobRow {
                job_id: 1,
                as_main: true
            }
        ));
        assert!(
            job_pick_rows(&snap, false).is_empty(),
            "sub side waits for the grant"
        );
        snap.jobs.as_mut().unwrap().sub_job_unlocked = true;
        let subs = job_pick_rows(&snap, false);
        assert_eq!(subs.len(), 2);
        assert!(matches!(
            subs[1].action,
            DynamicMenuAction::HomeJobRow {
                job_id: 3,
                as_main: false
            }
        ));
    }

    /// The pupequip rows (vault main_menu.md §39): fourteen in AutomatonSlot
    /// order — Head, Frame, Attachment 1..12 — with the LSB enum vocabulary for
    /// head/frame and "(empty)" cells; slot ids match c2s 0x102's AutomatonSlot.
    #[test]
    fn pupequip_rows_are_fourteen_slots_in_automatonslot_order() {
        let snap = kuluu_snapshot::SceneSnapshot {
            automaton: Some(Box::new(kuluu_snapshot::AutomatonInfo {
                is_sub_job: false,
                head: 2, // Valoredge (vendor/server/src/map/enums/automaton.h)
                frame: 0x21,
                attachments: [0; 12],
                unlocked_heads: 0x3F,
                unlocked_frames: 0x0F,
            })),
            ..Default::default()
        };
        let rows = automaton_equip_rows(&snap);
        assert_eq!(rows.len(), 14);
        assert_eq!(rows[0].label, "Head: Valoredge");
        assert_eq!(rows[1].label, "Frame: Valoredge");
        assert_eq!(rows[2].label, "Attachment 1: (empty)");
        assert_eq!(rows[13].label, "Attachment 12: (empty)");
        let slots: Vec<u8> = rows
            .iter()
            .map(|r| match r.action {
                DynamicMenuAction::AutomatonSlotRow { slot } => slot,
                ref a => panic!("unexpected action {a:?}"),
            })
            .collect();
        assert_eq!(slots, (0u8..14).collect::<Vec<u8>>());
        // The window with no automaton record (frame not arrived) shows nothing.
        assert!(automaton_equip_rows(&kuluu_snapshot::SceneSnapshot::default()).is_empty());
    }

    /// `mp_mncst` is the grounded-title empty display: catalog 76.DAT idx 165
    /// = Customization; its DAT record names no rows (vault main_menu.md §39).
    #[test]
    fn mp_customize_window_is_the_grounded_title_empty_display() {
        assert_eq!(menu_title(MenuKind::MpCustomize), "Customization");
        assert!(entries_for(MenuKind::MpCustomize, false).is_empty());
        assert_eq!(menu_title(MenuKind::AutomatonEquip), "Equipment");
    }

    /// The §41 macro law: DAT grids 'mcrs'/'mcr7' hold 20 keytops each under
    /// two palette windows, so capacity is 2 x 20; mcrselec lists filled store
    /// slots in (block, index) order with pinned labels, and both sub-windows
    /// inherit `Macros` because their comwin tokens read -1.
    #[test]
    fn macro_windows_track_the_dat_grid_capacity_law() {
        assert_eq!(MACRO_BLOCKS * MACRO_SLOTS_PER_BLOCK, 40);
        assert!(is_dynamic(MenuKind::MacroSelect));
        assert_eq!(menu_title(MenuKind::MacroSelect), "Macros");
        assert_eq!(menu_title(MenuKind::MacroResult), "Macros");
        // A fresh character: retail's keytop cells are blank, kuluu lists
        // nothing.
        let fresh = kuluu_snapshot::SceneSnapshot::default();
        assert!(macro_select_rows(&fresh).is_empty());

        let seeded = kuluu_snapshot::SceneSnapshot {
            macros: vec![
                kuluu_snapshot::MacroSlot {
                    block: 1,
                    index: 0,
                    text: "/lastshout".to_string(),
                },
                kuluu_snapshot::MacroSlot {
                    block: 0,
                    index: 4,
                    text: "/ra".to_string(),
                },
            ],
            ..Default::default()
        };
        let rows = macro_select_rows(&seeded);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "B1/5: /ra");
        assert_eq!(rows[1].label, "B2/1: /lastshout");
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::MacroSlotRow { block: 0, index: 4 }
        );
    }

    /// The commenu law (vault main_menu.md §37): two list widgets in DAT order —
    /// Friend List (catalog 61) then Emote List (catalog 180) — under the grounded
    /// title Communication (comwin token 179). The friend window's own row set is
    /// retail-empty on LSB servers (§38.5): no friends table, SearchType Friend
    /// 0x0C has no handler case there, so kuluu shows an empty grounded window.
    #[test]
    fn communication_menu_rows_track_the_commen_law() {
        assert_eq!(
            entries_for(MenuKind::Communication, false),
            &[COMM_FRIEND_LIST, COMM_EMOTE_LIST]
        );
        assert_eq!(menu_title(MenuKind::Communication), "Communication");
        assert_eq!(menu_title(MenuKind::FriendList), "Friend List");
        assert_eq!(entries_for(MenuKind::FriendList, false).len(), 0);
    }

    /// The Link Shell law (vault main_menu.md §27): the window always shows both shell slots the
    /// server tracks, labelled by whether s2c 0x0E0 reports a pearl and where it lives.
    #[test]
    fn linkshell_window_rows_track_the_two_shell_slots_the_server_reports() {
        // Both slots always exist (vault main_menu.md section 27.7: LSB pushes LinkshellNum 1 and 2
        // at login); only the s2c 0x0E0 payload changes whether a slot holds a pearl, and where.
        let empty = kuluu_snapshot::SceneSnapshot::default();
        let rows = linkshell_window_rows(&empty);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Link Shell 1   (no pearl equipped)");
        assert_eq!(rows[1].label, "Link Shell 2   (no pearl equipped)");
        assert_eq!(
            rows[1].action,
            DynamicMenuAction::LinkShellSlot { linkshell_slot: 2 }
        );

        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        snap.linkshells.two = kuluu_snapshot::LinkShellSlot {
            category: 0,
            item_index: 12,
        };
        let rows = linkshell_window_rows(&snap);
        assert_eq!(rows[0].label, "Link Shell 1   (no pearl equipped)");
        assert_eq!(
            rows[1].label,
            "Link Shell 2   pearl at container 0, index 12"
        );

        // Root row 10 and the right-pane preview share one mapping.
        assert_eq!(
            root_child_kind("Linkshell", false, false),
            Some(MenuKind::Linkshell)
        );
        assert_eq!(menu_title(MenuKind::Linkshell), "Link Shell");
    }

    /// The search surface law (vault main_menu.md §26): the Run row leads, then
    /// one hit row per server frame entry in order. Label = name, job abbrevs +
    /// levels from the AddPlayer entries — or `???` for a hidden player whose
    /// job/level/race/rank entries were dropped (flags1 & 0x4000), then zone.
    #[test]
    fn search_window_rows_lead_with_run_then_hits_in_server_order() {
        use kuluu_snapshot::{SearchPlayer, SearchResults};

        let empty = kuluu_snapshot::SceneSnapshot::default();
        assert_eq!(search_window_rows(&empty).len(), 1);

        let snap = kuluu_snapshot::SceneSnapshot {
            search: Some(Box::new(SearchResults {
                total: 2,
                players: vec![
                    SearchPlayer {
                        name: "bob".into(),
                        zone: 103,
                        nation: Some(1),
                        main_job: Some(3),
                        sub_job: Some(1),
                        main_level: Some(75),
                        sub_level: Some(37),
                        race: Some(2),
                        rank: Some(3),
                        flags1: 0x42,
                        id: 0x1_2345,
                        comment_type: None,
                        flags2: 0,
                        languages: 3,
                    },
                    SearchPlayer {
                        name: "ALongNameX".into(),
                        zone: 0,
                        nation: None,
                        main_job: None,
                        sub_job: None,
                        main_level: None,
                        sub_level: None,
                        race: None,
                        rank: None,
                        flags1: 0x4000,
                        id: 99,
                        comment_type: Some(7),
                        flags2: 0xDEAD,
                        languages: 1,
                    },
                ],
            })),
            ..Default::default()
        };
        let rows = search_window_rows(&snap);
        assert_eq!(rows.len(), 3, "{rows:?}");
        assert_eq!(rows[0].label, "Run Search");
        // Job ids 3/1 are WHM/WAR in the LSB job table scraped by ffxi-vocab.
        assert_eq!(&rows[1].label, &{
            let whm = ffxi_vocab::job_names::abbrev(3).expect("WHM has a name");
            let war = ffxi_vocab::job_names::abbrev(1).expect("WAR has a name");
            format!("bob  {whm}75/{war}37  Z103")
        });
        assert_eq!(rows[1].action, DynamicMenuAction::SearchHitRow { index: 0 });
        // The hidden shape carries no job/level entries — the row says so.
        assert_eq!(&rows[2].label, "ALongNameX  ???  Z0");
        assert_eq!(rows[2].action, DynamicMenuAction::SearchHitRow { index: 1 });
    }

    /// Region Info law (vault main_menu.md §29): the window answers items 1..4 as display modes
    /// 0..3; kuluu renders exactly four rows in that order with the marker on the installed mode.
    #[test]
    fn region_info_rows_are_the_four_modes_with_the_active_marker() {
        let known = Some(REGION_PRED_ZONES_PINNED[0]);
        let rows = region_info_rows(2, known);
        assert_eq!(rows.len(), 4, "items 1..4 only — always four rows");
        assert_eq!(
            rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            vec!["World", "Nation", "\u{bb} Region", "Zone"],
            "handler item order 1..4, marker on the installed mode"
        );
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(
                row.action,
                DynamicMenuAction::RegionInfoMode { mode: i as u8 }
            );
        }

        // The message-0x7B seam: with mode 0 installed over a zone outside FFXiMain's predicate
        // table (and with no zone resolved yet) the World row wears the notice; other rows never.
        let unknown = region_info_rows(0, Some(u16::MAX));
        assert_eq!(
            unknown[0].label,
            format!("\u{bb} World {}", REGION_UNAVAILABLE_NOTICE_PINNED),
            "the notice rides the active marker on the same row"
        );
        assert!(!unknown[1].label.contains("unavailable"));
        let no_zone = region_info_rows(0, None);
        assert!(no_zone[0].label.contains(REGION_UNAVAILABLE_NOTICE_PINNED));

        // Known zone or non-zero mode: the notice is gone.
        assert_eq!(region_info_rows(0, known)[0].label, "\u{bb} World");
        assert_eq!(region_info_rows(3, Some(u16::MAX))[0].label, "World");
    }

    #[test]
    fn region_info_window_is_registered_dynamic_and_titled() {
        assert!(is_dynamic(MenuKind::RegionInfo { mode: 0 }));
        assert_eq!(menu_title(MenuKind::RegionInfo { mode: 0 }), "Region Info");
        assert_eq!(
            root_child_kind("Region Info", false, false),
            Some(MenuKind::RegionInfo { mode: 0 })
        );
    }

    /// Mission law (vault main_menu.md §30): missionm lists exactly the logs the
    /// s2c 0x056 state holds and every row drills that port's miss00 detail.
    #[test]
    fn mission_list_rows_follow_the_folded_battery() {
        let empty = kuluu_snapshot::SceneSnapshot::default();
        let rows = mission_list_rows(&empty);
        assert_eq!(rows.len(), 1, "logged-out state keeps one hint row");
        assert!(rows[0].label.contains("no mission logs"));

        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        snap.missions.info = Some(kuluu_snapshot::MissionInfo {
            nation: 2,
            nation_mission: 3,
            rotz: 0,
            cop: 0,
            cop2: 0,
            addon_acp: 0,
            addon_amk: 0,
            addon_asa: 0,
            tales_beginning: 0,
            soa: 0,
            rov: 0,
        });
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: 0x00D0,
            data: [1, 0, 0, 0, 0, 0, 0, 0],
        });
        let rows = mission_list_rows(&snap);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "Windurst: mission #3");
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::MissionOpenLog {
                port: ffxi_proto::mission::PORT_MISSION_MAIN
            }
        );
        assert_eq!(rows[1].label, "Completed missions — Nations");

        // A quest port carries data but belongs to the /quest seam, not here.
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: 0x0058,
            data: [7; 8],
        });
        assert_eq!(mission_list_rows(&snap).len(), 2);

        // Detail rows are legible-only field dumps per the [I-pending] paging law.
        let detail = mission_detail_rows(&snap, ffxi_proto::mission::PORT_MISSION_MAIN);
        assert_eq!(detail[0].label, "Windurst: mission #3");
        for row in &detail {
            assert!(matches!(
                row.action,
                DynamicMenuAction::MissionLogRow { .. }
            ));
        }
    }

    #[test]
    fn mission_windows_are_dynamic_titled_and_rooted() {
        assert!(is_dynamic(MenuKind::MissionList));
        assert!(is_dynamic(MenuKind::MissionDetail { port: 0xD0 }));
        assert_eq!(menu_title(MenuKind::MissionList), "Missions");
        assert_eq!(
            root_child_kind("Missions", false, false),
            Some(MenuKind::MissionList)
        );
    }

    #[test]
    fn quest_list_rows_expand_the_quest_ports() {
        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        let empty = quest_list_rows(&snap);
        assert_eq!(empty.len(), 1, "logged-out state keeps one hint row");
        assert!(empty[0].label.contains("no quests"));

        // San d'Oria offer bits 0/2 and the id-18 overlap between that same
        // offer port and Sandoria's complete port: three rows, log-then-id order.
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: ffxi_proto::mission::QUEST_OFFER_PORTS[0],
            data: [1 | (1 << 2) | (1 << 18), 0, 0, 0, 0, 0, 0, 0],
        });
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: ffxi_proto::mission::QUEST_COMPLETE_PORTS[0],
            data: [1 << 18, 0, 0, 0, 0, 0, 0, 0],
        });
        // A complete-mission port must not leak into this seam either direction.
        snap.missions.logs.push(kuluu_snapshot::MissionLogEntry {
            port: 0x00D0,
            data: [7; 8],
        });

        let rows = quest_list_rows(&snap);
        assert_eq!(rows.len(), 3, "one row per (log, id) bit held");
        // scripts/globals/quests.lua San d'Oria block.
        assert_eq!(rows[0].label, "A Sentrys Peril");
        assert_eq!(rows[1].label, "Rosel the Armorer");
        assert_eq!(rows[2].label, "Gates to Paradise (complete)");
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::QuestOpenDetail { log: 0, id: 0 }
        );
        assert_eq!(
            rows[2].action,
            DynamicMenuAction::QuestOpenDetail { log: 0, id: 18 }
        );

        // The quest01 stand-in is a legible-only dump of the fields kuluu holds.
        let detail = quest_detail_rows(&snap, 0, 18);
        assert_eq!(detail[0].label, "Gates to Paradise");
        assert_eq!(detail[1].label, "Area: Sandoria");
        assert_eq!(detail[2].label, "Complete.");
        for row in &detail {
            assert!(matches!(
                row.action,
                DynamicMenuAction::QuestDetailRow { log: 0, id: 18 }
            ));
        }
    }

    #[test]
    fn quest_windows_are_dynamic_titled_and_never_root_drilled() {
        assert!(is_dynamic(MenuKind::QuestList));
        assert!(is_dynamic(MenuKind::QuestDetail { log: 0, id: 4 }));
        assert_eq!(menu_title(MenuKind::QuestList), "Quests");
        assert_eq!(
            menu_title(MenuKind::QuestDetail { log: 2, id: 9 }),
            "Quest Log"
        );
        // Retail opens the list through the "/quest" text command, never a row drill.
        assert_eq!(root_child_kind("Quests", false, false), None);
    }

    #[test]
    fn view_house_veto_follows_the_capital_zone_table() {
        // Gate law pinned from the FFXiMain.dll retail-2026-09 RVA 0xF9680 zone
        // map (vault main_menu.md §32): ten capitals need the in-house operand,
        // Chateau d'Oraguille and Metalworks never do, anything else is free.
        assert!(view_house_blocked(Some(230), false));
        assert!(view_house_blocked(Some(241), false));
        assert!(!view_house_blocked(Some(233), false));
        assert!(!view_house_blocked(Some(237), false));
        assert!(!view_house_blocked(Some(229), false));
        assert!(!view_house_blocked(Some(242), false));
        // In your own Mog House the predicate passes in a capital too.
        assert!(!view_house_blocked(Some(230), true));
        // Unknown zone (not yet reported) never vetoes.
        assert!(!view_house_blocked(None, false));
    }

    #[test]
    fn bazaar_stall_picker_is_dynamic_titled_and_root_drilled() {
        assert!(is_dynamic(MenuKind::BazaarStalls));
        assert_eq!(menu_title(MenuKind::BazaarStalls), "Bazaar");
        // The ungated page-2 arm (vault main_menu.md §33) drills it from Root.
        assert_eq!(
            root_child_kind("Bazaar", false, false),
            Some(MenuKind::BazaarStalls)
        );
    }

    #[test]
    fn bazaar_stall_rows_follow_the_entity_bazaar_flag() {
        use kuluu_snapshot::{Entity, EntityKind};
        let mut snap = kuluu_snapshot::SceneSnapshot::default();
        assert_eq!(bazaar_stall_rows(&snap).len(), 1);
        assert!(bazaar_stall_rows(&snap)[0]
            .label
            .contains("no player bazaars"));

        snap.entities.push(Entity {
            id: 7,
            act_index: 2,
            kind: EntityKind::Pc,
            name: Some("Kupipi".into()),
            pos: kuluu_snapshot::Vec3::default(),
            heading: 0,
            hp_pct: Some(100),
            bt_target_id: 0,
            face_target: 0,
            claim_id: 0,
            speed: 0,
            speed_base: 0,
            look: None,
            animation: 0,
            animationsub: 0,
            mount: None,
            status: 0,
            char_flags: Default::default(),
            monstrosity: false,
            name_vis: None,
        });
        assert!(
            bazaar_stall_rows(&snap)[0]
                .label
                .contains("no player bazaars"),
            "the flag is required"
        );
        snap.entities[0].char_flags.bazaar = true;
        let rows = bazaar_stall_rows(&snap);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Kupipi's Bazaar");
        assert_eq!(
            rows[0].action,
            DynamicMenuAction::BazaarStall { char_id: 7 }
        );
        snap.self_char_id = Some(7);
        assert!(bazaar_stall_rows(&snap)[0]
            .label
            .contains("no player bazaars"));
    }
}
