//! The Mog House layout-mode machine: which modes exist, what swaps in and out when one is entered or
//! left, and which requests have to be refused before anything happens.
//!
//! Everything here comes from the mode switch FFXiMain.dll retail-2026-09 `.text 0x1D3380(mode, slot,
//! container)` (vault main_menu.md §59 addendum 31). Its arguments are read off its own two slash call
//! sites: `/layout` pushes `(1, -1, LOC_MOGSAFE)` at `.text 0x7E78B` and `/garden` pushes
//! `(2, -1, LOC_MOGSAFE)` at `.text 0x7E999`, so the first argument is the mode, a slot of `-1` means
//! "pick one for me", and the third carries the Mog House container (the switch itself reduces it with
//! `cmp esi,9; sete al`, i.e. to "which floor half of the object array"). The current mode lives in the
//! myroom manager's own word at +0x1C — read back at `.text 0x1D34FA` (to decide whether the closing
//! beat is owed) and `.text 0x1D350F`.
//!
//! Modes are validated before dispatch: a negative one bails at `.text 0x1D33A2` and anything `>= 5` at
//! `.text 0x1D33AB`, which is why [`FIRST_INVALID_MODE`] exists as a constant rather than an inline 5.

use crate::myroom_furniture::{
    self as furniture, furniture_index, is_selectable_slot, PlacedFurniture, CONTAINER_MOGSAFE,
};

/// Normal play: nothing is being arranged.
pub const MODE_NORMAL: u8 = 0;
/// Layout mode — furniture can be picked up, moved and stored (`/layout`).
pub const MODE_LAYOUT: u8 = 1;
/// Gardening mode (`/garden`); retail may hand this to a confirmation step first (`.text 0x1D36BC`).
pub const MODE_GARDEN: u8 = 2;
/// Flowerpot mode: the next thing the player picks is a seed or feed item, so the switch opens the
/// item window instead of touching furniture (`.text 0x1D36D9`, which name-opens `inventor` through
/// `.text 0x1F23A0`).
pub const MODE_FLOWERPOT: u8 = 3;
/// Select-one-piece mode: the switch only re-selects an object by index (`.text 0x1D372E`).
pub const MODE_PICK_ONE: u8 = 4;

/// The first rejected mode. The dispatch is a five-entry jump table at `.rdata 0x1D37CC` and every
/// caller funnels through the two range checks, so a mode here or above never has an effect (see the
/// module docs for the two checked RVAs).
pub const FIRST_INVALID_MODE: u8 = MODE_PICK_ONE + 1;

/// The key-action context of normal play. Restoring it is one of the last things leaving layout mode
/// does: `push 0xffff; call .text 0x1236E0` at `.text 0x1D34A6`.
pub const INPUT_CONTEXT_NORMAL: u16 = 0xFFFF;

/// The context every non-normal mode installs — `push 0x42; call .text 0x1236E0` appears once per mode
/// body (`.text 0x1D359A`, `.text 0x1D3705`, `.text 0x1D3739`), and never the reverse while a mode is on.
pub const INPUT_CONTEXT_ARRANGING: u16 = 0x42;

/// Which key-action context a mode runs under. Mode 0 is the only one that gets normal input back.
pub fn input_context(mode: u8) -> u16 {
    if mode == MODE_NORMAL {
        INPUT_CONTEXT_NORMAL
    } else {
        INPUT_CONTEXT_ARRANGING
    }
}

/// The DAT record names of the windows layout entry closes, in the order the client closes them — the
/// five string operands of the `menu    playermo`/…/`menu    mogpost` chain at `.text 0x1D35B8`..`.text
/// 0x1D3602`, read from their `.data` addresses (`.data 0x36F5B4`, `.data 0x35BAD8`, `.data 0x371B4C`,
/// `.data 0x36FFF0`, `.data 0x37160C`). They are record roots, not display names, so a caller that has
/// to match them against its own window table must use the same form.
pub const WINDOWS_CLOSED_BY_LAYOUT: [&str; 5] = [
    "menu    playermo",
    "menu    targetwi",
    "menu    menuwind",
    "menu    myroom  ",
    "menu    mogpost ",
];

/// The windows the other three modes close instead (`.text 0x1D3778`..`.text 0x1D37AF`: `.data
/// 0x35BAD8`, `.data 0x371B4C`, `.data 0x37EC80`, `.data 0x37160C`). Note it drops `playermo`/`myroom`
/// and adds the room menu — gardening does not need your own name plate hidden.
pub const WINDOWS_CLOSED_BY_OTHER_MODES: [&str; 4] = [
    "menu    targetwi",
    "menu    menuwind",
    "menu    roommenu",
    "menu    mogpost ",
];

/// The window leaving layout mode re-opens (`.text 0x1D34EB`, name operand `.data 0x36FFF0`). Only the
/// Mog House panel is reopened; the other closed windows stay shut until something asks for them.
pub const WINDOW_REOPENED_ON_EXIT: &str = "menu    myroom  ";

/// The window opened when layout mode starts with **nothing placed in the room** (`.text 0x1D36B2`, name
/// operand `.data 0x37EC94`) — there is no object to select, so retail shows this instead.
pub const WINDOW_NO_FURNITURE: &str = "menu    rmlo1   ";

/// What a caller has to know about the room before it may change mode. Every field names one of the DLL's
/// own refusal predicates; kuluu fills them from state it already keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrangeGates {
    /// The mode currently in force (retail: myroom manager +0x1C).
    pub current_mode: u8,
    /// A Mog House operation is still outstanding — retail's `call .text 0x1D5FE0` on the myroom
    /// manager, which answers while a layout/bankin request has not been acknowledged yet.
    pub manager_busy: bool,
    /// The house door is open to visitors — `.text 0xF96E0`, whose law already drives the Mog House menu
    /// (vault main_menu.md §59 addendum 24). `/layout` refuses this one with the error bell.
    pub door_open: bool,
    /// Already arranging something (`/layout` tests its own layout-window state byte at `+0x16`, `.text
    /// 0x7E73B`) — also the error-bell case.
    pub already_arranging: bool,
    /// The player is in their own Mog House and able to arrange it (`.text 0x97B40`, the same predicate
    /// that gates the Mog House menu's Layout/Remodel rows).
    pub home_view_available: bool,
}

/// Why a mode request produced nothing at all. Each variant records the signal retail emits, because two
/// of these are audible refusals (the error bell) and two are chat messages with an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `mode >= FIRST_INVALID_MODE`: rejected before dispatch, no signal at all.
    ModeOutOfRange,
    /// `home_view_available` was false — retail posts message code 6 through `.text 0x129D90`.
    NotInOwnMogHouse,
    /// `manager_busy` — retail posts message code 5 through `.text 0x129D90`.
    ManagerBusy,
    /// `door_open` — retail rings the error bell (`.text 0x37C50`) and says nothing.
    DoorOpen,
    /// `already_arranging` — also the error bell, from `.text 0x7E754`.
    AlreadyArranging,
}

impl Refusal {
    /// The audible/text signal a refusal carries: `Some(true)` for the error bell (`.text FFXiMain.dll
    /// retail-2026-09 0x37C50`), `Some(false)` with the message code for the two chat refusals, and None
    /// where retail fails silently.
    pub fn signal(self) -> RefusalSignal {
        match self {
            Refusal::DoorOpen | Refusal::AlreadyArranging => RefusalSignal::Buzzer,
            Refusal::ManagerBusy => RefusalSignal::Message(5),
            Refusal::NotInOwnMogHouse => RefusalSignal::Message(6),
            Refusal::ModeOutOfRange => RefusalSignal::Silent,
        }
    }
}

/// What the client does when a request is refused. The two message codes are the operands of
/// `.text FFXiMain.dll retail-2026-09 0x129D90`, which both slash handlers tail-call before returning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalSignal {
    /// The error bell — the same sound a disabled menu row makes (`.text FFXiMain.dll retail-2026-09 0x37C50`).
    Buzzer,
    /// A short status message carrying this code.
    Message(u16),
    /// Nothing at all is said or played.
    Silent,
}

/// One mode request: the mode plus what to select when entering it (`None` = "no particular piece",
/// which retail spells as a slot of -1). `container` selects which Mog House shelf half the explicit
/// slot belongs to and defaults to [`CONTAINER_MOGSAFE`] exactly as both slash call sites pass it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrangeRequest {
    pub mode: u8,
    /// The requested slot, already validated by the caller into 0..=80 (`None` for retail's -1).
    pub slot: Option<u8>,
    pub container: Option<u8>,
}

impl ArrangeRequest {
    /// A slash-style request: no particular piece, first shelf.
    pub fn from_slash(mode: u8) -> Self {
        Self {
            mode,
            slot: None,
            container: Some(CONTAINER_MOGSAFE),
        }
    }
}

/// What entering/leaving a mode does to the rest of the client. Every field is one of the mode switch's
/// own observable side effects; a consumer that has no equivalent simply ignores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrangeTransition {
    pub previous: u8,
    pub next: u8,
    /// The key-action context to install ([`INPUT_CONTEXT_ARRANGING`] / [`INPUT_CONTEXT_NORMAL`]).
    pub input_context: u16,
    /// Window record roots to close — [`WINDOWS_CLOSED_BY_LAYOUT`] when entering layout mode and
    /// [`WINDOWS_CLOSED_BY_OTHER_MODES`] for the other three. Empty when returning to normal play.
    pub closes: &'static [&'static str],
    /// Whether to re-open [`WINDOW_REOPENED_ON_EXIT`].
    pub reopens_myroom: bool,
    /// Send the all-zero c2s 0x0FA (`.text FFXiMain.dll retail-2026-09 0xF0C80`). Retail owes this beat
    /// only when it is leaving *layout* mode for normal play and its own mode word really was layout:
    /// `.text 0x1D34FA` tests it, then tail-calls the zeroed sender.
    pub announces_layout_end: bool,
    /// The object the mode switch selects, when the request asked for one and the room could answer
    /// (see [`select_furniture`]).
    pub selected: Option<SelectedFurniture>,
    /// Open [`WINDOW_NO_FURNITURE`] because layout started with an empty object array. Only ever true
    /// when `placed` was known to be empty — no install means no answer either way, and retail says
    /// nothing in that case either (`.text 0x1D369B` tests the object count word).
    pub opens_no_furniture_window: bool,
}

/// Run the refusal chain in retail's order and, if it survives, describe the transition. The chain order
/// is `/layout`'s (`.text FFXiMain.dll retail-2026-09 0x7E630`, gate tests in instruction order): busy
/// manager first (message 5), then door/already-arranging (both the bell), then home-view availability
/// (message 6) — followed by the mode switch's own range checks, which are silent.
/// `placed` is the caller's view of what stands in the room (the derived furniture table), or `None`
/// when it cannot be known — which happens without an item DAT to ask about kinds.
pub fn plan_arrange(
    request: ArrangeRequest,
    gates: &ArrangeGates,
    placed: Option<&[PlacedFurniture]>,
) -> Result<ArrangeTransition, Refusal> {
    if request.mode >= FIRST_INVALID_MODE {
        return Err(Refusal::ModeOutOfRange);
    }
    // `/layout` reaches the manager-busy predicate before either bell (`.text 0x7E714`, whose refusal is
    // the message-5 tail-call at `.text 0x7E723`).
    if gates.manager_busy {
        return Err(Refusal::ManagerBusy);
    }
    // The two audible refusals, tested back to back at `.text 0x7E73B`..`.text 0x7E75F`.
    if gates.already_arranging {
        return Err(Refusal::AlreadyArranging);
    }
    if gates.door_open {
        return Err(Refusal::DoorOpen);
    }
    if !gates.home_view_available {
        return Err(Refusal::NotInOwnMogHouse);
    }
    // A request for the mode already in force changes nothing; retail's own re-dispatch collapses an
    // unusable request to mode 0 (`.text 0x1D3423`..`.text 0x1D3433`) rather than re-entering it.
    let entering = request.mode != MODE_NORMAL;
    // Only the two modes that act on a piece select one: layout (`.text 0x1D3652`) and
    // select-one-piece (`.text 0x1D3743`).
    let selects = matches!(request.mode, MODE_LAYOUT | MODE_PICK_ONE);
    let selected = if selects {
        placed.and_then(|p| select_furniture(p, request.slot, request.container))
    } else {
        None
    };
    let opens_no_furniture_window =
        selects && request.mode == MODE_LAYOUT && placed.is_some_and(|p| p.is_empty());
    Ok(ArrangeTransition {
        previous: gates.current_mode,
        next: request.mode,
        input_context: input_context(request.mode),
        closes: if !entering {
            &[]
        } else if request.mode == MODE_LAYOUT {
            &WINDOWS_CLOSED_BY_LAYOUT
        } else {
            &WINDOWS_CLOSED_BY_OTHER_MODES
        },
        reopens_myroom: !entering,
        announces_layout_end: gates.current_mode == MODE_LAYOUT && request.mode == MODE_NORMAL,
        selected,
        opens_no_furniture_window,
    })
}

/// The object a mode switch ends up selecting. `index` is the flat object-array seat (see
/// [`crate::myroom_furniture::furniture_index`]), which for kuluu is also the ordering key of the
/// derived table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SelectedFurniture {
    pub index: usize,
    pub container: u8,
    pub slot: u8,
}

impl SelectedFurniture {
    fn from_object(object: &PlacedFurniture) -> Self {
        Self {
            index: object.index,
            container: object.container,
            slot: object.slot,
        }
    }
}

/// The selection law of `.text FFXiMain.dll retail-2026-09 0x1D3380` in layout mode (`.text 0x1D3652`..
/// `.text 0x1D36B7`) and select-one-piece mode (`.text 0x1D3743`).
///
/// An explicit slot is honoured only when it lies inside the shelf (`slot < 0x51`, tested at
/// `.text 0x1D3656`/`.text 0x1D374A`) **and** that seat in the object array is occupied, where the seat
/// is `slot + (container == LOC_MOGSAFE2 ? 81 : 0)` — the same arithmetic as
/// [`crate::myroom_furniture::furniture_index`] (the switch writes it with `lea eax,[eax+eax*8];`
/// `lea edx,[ebx+eax*8]; add eax,edx`). An out-of-range or empty seat falls through to the first-object
/// scan; so does retail's own "no particular piece" (-1).
pub fn select_furniture(
    objects: &[PlacedFurniture],
    slot: Option<u8>,
    container: Option<u8>,
) -> Option<SelectedFurniture> {
    if let (Some(slot), Some(container)) = (slot, container) {
        if is_selectable_slot(slot) {
            let index = furniture_index(container, slot);
            if let Some(object) = objects.iter().find(|o| o.index == index) {
                return Some(SelectedFurniture::from_object(object));
            }
        }
    }
    first_furniture(objects)
}

/// Retail's first-object scan (`.text FFXiMain.dll retail-2026-09 0x1D3E70`, reached whenever the
/// requested seat is empty or no seat was asked for): it walks the object array's
/// first shelf half slot-by-slot, then the second half, and returns the first occupied seat. The derived
/// table is already sorted by that same index, so this is its first element. `None` means nothing is
/// placed — which is when retail opens [`WINDOW_NO_FURNITURE`] instead of selecting anything.
pub fn first_furniture(objects: &[PlacedFurniture]) -> Option<SelectedFurniture> {
    objects.first().map(SelectedFurniture::from_object)
}

/// Record a Mog House shelf seat with a furniture request outstanding, which is what holds the
/// [`ArrangeGates::manager_busy`] gate. Retail's busy predicate (`.text FFXiMain.dll retail-2026-09
/// 0x1D5FE0`) counts exactly these unacknowledged operations; LSB answers a layout or bankin request with
/// the ITEM_ATTR of that very shelf slot (vendor/server/src/map/packets/c2s/0x0fa_myroom_layout.cpp), and
/// that echo is what [`clear_pending_edit`] removes.
pub fn note_pending_edit(pending: &mut Vec<(u8, u8)>, container: u8, slot: u8) {
    if furniture::is_mog_house_shelf(container) && !pending.contains(&(container, slot)) {
        pending.push((container, slot));
    }
}

/// The echo beat: an s2c 0x020 record for this seat releases the busy gate it was holding.
pub fn clear_pending_edit(pending: &mut Vec<(u8, u8)>, container: u8, slot: u8) {
    pending
        .retain(|(edit_container, edit_slot)| *edit_container != container || *edit_slot != slot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::myroom_furniture::{CONTAINER_MOGSAFE2, FURNITURE_SLOTS_PER_CONTAINER};

    fn object(container: u8, slot: u8) -> PlacedFurniture {
        let extdata = [0u8; 24];
        crate::myroom_furniture::describe_object(container, slot, 100 + slot as u16, &extdata)
    }

    fn gates(current_mode: u8) -> ArrangeGates {
        ArrangeGates {
            current_mode,
            manager_busy: false,
            door_open: false,
            already_arranging: false,
            home_view_available: true,
        }
    }

    #[test]
    fn only_normal_play_gets_normal_input() {
        assert_eq!(input_context(MODE_NORMAL), INPUT_CONTEXT_NORMAL);
        for mode in [MODE_LAYOUT, MODE_GARDEN, MODE_FLOWERPOT, MODE_PICK_ONE] {
            assert_eq!(
                input_context(mode),
                INPUT_CONTEXT_ARRANGING,
                "mode {mode} installs the arranging context"
            );
        }
        assert_eq!(FIRST_INVALID_MODE, 5);
    }

    #[test]
    fn a_mode_outside_the_jump_table_is_refused_silently() {
        let err = plan_arrange(
            ArrangeRequest::from_slash(FIRST_INVALID_MODE),
            &gates(MODE_NORMAL),
            None,
        )
        .unwrap_err();
        assert_eq!(err, Refusal::ModeOutOfRange);
        assert_eq!(err.signal(), RefusalSignal::Silent);
    }

    #[test]
    fn the_refusal_chain_is_busy_then_bells_then_availability() {
        // Manager busy wins over every other gate and carries message 5.
        let mut g = gates(MODE_NORMAL);
        g.manager_busy = true;
        g.door_open = true;
        g.home_view_available = false;
        assert_eq!(
            plan_arrange(ArrangeRequest::from_slash(MODE_LAYOUT), &g, None).unwrap_err(),
            Refusal::ManagerBusy
        );
        // Between the two bells, the already-arranging test comes first (`.text 0x7E73B` before `.text
        // 0x7E742`) and both are audible.
        let mut g = gates(MODE_NORMAL);
        g.door_open = true;
        g.already_arranging = true;
        assert_eq!(
            plan_arrange(ArrangeRequest::from_slash(MODE_LAYOUT), &g, None).unwrap_err(),
            Refusal::AlreadyArranging
        );
        let mut g = gates(MODE_NORMAL);
        g.door_open = true;
        let err = plan_arrange(ArrangeRequest::from_slash(MODE_LAYOUT), &g, None).unwrap_err();
        assert_eq!(err, Refusal::DoorOpen);
        assert_eq!(err.signal(), RefusalSignal::Buzzer);
        // Availability is the last gate, with message 6.
        let mut g = gates(MODE_NORMAL);
        g.home_view_available = false;
        let err = plan_arrange(ArrangeRequest::from_slash(MODE_LAYOUT), &g, None).unwrap_err();
        assert_eq!(err, Refusal::NotInOwnMogHouse);
        assert_eq!(err.signal(), RefusalSignal::Message(6));
    }

    #[test]
    fn entering_layout_mode_closes_the_five_named_windows() {
        let t = plan_arrange(
            ArrangeRequest::from_slash(MODE_LAYOUT),
            &gates(MODE_NORMAL),
            None,
        )
        .unwrap();
        assert_eq!(t.next, MODE_LAYOUT);
        assert_eq!(t.input_context, INPUT_CONTEXT_ARRANGING);
        assert_eq!(t.closes.len(), 5);
        assert_eq!(t.closes[2], "menu    menuwind");
        assert_eq!(t.closes[3], WINDOW_REOPENED_ON_EXIT);
        assert!(!t.reopens_myroom);
        // Entering is not leaving: nothing is announced on the wire yet.
        assert!(!t.announces_layout_end);
    }

    #[test]
    fn gardening_and_flowerpot_modes_close_the_other_window_set() {
        for mode in [MODE_GARDEN, MODE_FLOWERPOT, MODE_PICK_ONE] {
            let t =
                plan_arrange(ArrangeRequest::from_slash(mode), &gates(MODE_NORMAL), None).unwrap();
            assert_eq!(t.closes, &WINDOWS_CLOSED_BY_OTHER_MODES[..], "mode {mode}");
        }
    }

    #[test]
    fn leaving_layout_mode_reopens_the_mog_house_and_announces_the_end() {
        let exit = ArrangeRequest::from_slash(MODE_NORMAL);
        let from_layout = plan_arrange(exit, &gates(MODE_LAYOUT), None).unwrap();
        assert!(from_layout.reopens_myroom);
        assert_eq!(from_layout.closes.len(), 0, "leaving closes nothing");
        assert_eq!(from_layout.input_context, INPUT_CONTEXT_NORMAL);
        assert!(from_layout.announces_layout_end);

        // Leaving gardening does not owe the zeroed layout packet — retail tests its mode word for 1.
        let from_garden = plan_arrange(exit, &gates(MODE_GARDEN), None).unwrap();
        assert!(!from_garden.announces_layout_end);
        assert!(from_garden.reopens_myroom);
    }

    #[test]
    fn an_explicit_slot_is_honoured_through_the_object_array_seat() {
        let objects = [object(CONTAINER_MOGSAFE, 4), object(CONTAINER_MOGSAFE2, 6)];
        assert_eq!(
            select_furniture(&objects, Some(6), Some(CONTAINER_MOGSAFE2)),
            Some(SelectedFurniture {
                index: FURNITURE_SLOTS_PER_CONTAINER + 6,
                container: CONTAINER_MOGSAFE2,
                slot: 6,
            }),
            "the second shelf lives in the back half of the array"
        );
        assert_eq!(
            select_furniture(&objects, Some(4), Some(CONTAINER_MOGSAFE))
                .unwrap()
                .index,
            4
        );
    }

    #[test]
    fn a_slot_that_is_not_occupied_falls_back_to_the_first_object() {
        let objects = [object(CONTAINER_MOGSAFE, 4), object(CONTAINER_MOGSAFE, 9)];
        // Asked for an empty seat: retail's `je` on the array lookup lands it on the scan instead.
        assert_eq!(
            select_furniture(&objects, Some(5), Some(CONTAINER_MOGSAFE))
                .unwrap()
                .slot,
            4
        );
        // A slot outside the shelf is ignored the same way (`.text 0x1D3656`).
        for slot in [80u8 + 1, 255] {
            assert_eq!(
                select_furniture(&objects, Some(slot), Some(CONTAINER_MOGSAFE))
                    .unwrap()
                    .slot,
                4,
                "slot {slot} is out of range"
            );
        }
        // No request at all (-1 → None) also lands on the scan.
        assert_eq!(
            select_furniture(&objects, None, Some(CONTAINER_MOGSAFE))
                .unwrap()
                .index,
            4
        );
    }

    #[test]
    fn layout_mode_with_an_empty_room_opens_the_no_furniture_window() {
        // Retail tests its object count word before it will name a piece (`.text 0x1D3689`), and opens
        // `menu    rmlo1   ` when the table is empty.
        let placed: [PlacedFurniture; 0] = [];
        let t = plan_arrange(
            ArrangeRequest::from_slash(MODE_LAYOUT),
            &gates(MODE_NORMAL),
            Some(&placed),
        )
        .unwrap();
        assert!(t.opens_no_furniture_window);
        assert_eq!(t.selected, None);

        // With something standing in the room it selects instead and says nothing about windows.
        let placed = [object(CONTAINER_MOGSAFE2, 3)];
        let t = plan_arrange(
            ArrangeRequest::from_slash(MODE_LAYOUT),
            &gates(MODE_NORMAL),
            Some(&placed),
        )
        .unwrap();
        assert!(!t.opens_no_furniture_window);
        assert_eq!(
            t.selected.map(|s| (s.container, s.slot)),
            Some((CONTAINER_MOGSAFE2, 3))
        );

        // Gardening selects nothing at all.
        let t = plan_arrange(
            ArrangeRequest::from_slash(MODE_GARDEN),
            &gates(MODE_NORMAL),
            Some(&placed),
        )
        .unwrap();
        assert_eq!(t.selected, None);
        assert!(!t.opens_no_furniture_window);
    }

    #[test]
    fn pending_edits_hold_the_busy_gate_until_their_seat_echoes() {
        let mut pending = Vec::new();
        note_pending_edit(&mut pending, CONTAINER_MOGSAFE, 7);
        note_pending_edit(&mut pending, CONTAINER_MOGSAFE2, 9);
        // A second request for the same seat does not double-count the outstanding operation.
        note_pending_edit(&mut pending, CONTAINER_MOGSAFE, 7);
        assert_eq!(pending.len(), 2);
        // Shelves outside the two Mog House containers are not furniture edits at all.
        note_pending_edit(&mut pending, 0, 1);
        assert_eq!(pending.len(), 2);

        clear_pending_edit(&mut pending, CONTAINER_MOGSAFE2, 9);
        let busy = !pending.is_empty();
        let gates = ArrangeGates {
            current_mode: MODE_NORMAL,
            manager_busy: busy,
            door_open: false,
            already_arranging: false,
            home_view_available: true,
        };
        assert!(plan_arrange(ArrangeRequest::from_slash(MODE_LAYOUT), &gates, None).is_err());

        clear_pending_edit(&mut pending, CONTAINER_MOGSAFE, 7);
        let gates = ArrangeGates {
            manager_busy: !pending.is_empty(),
            ..gates
        };
        assert!(plan_arrange(ArrangeRequest::from_slash(MODE_LAYOUT), &gates, None).is_ok());
    }

    #[test]
    fn nothing_placed_selects_nothing() {
        let objects: [PlacedFurniture; 0] = [];
        assert_eq!(first_furniture(&objects), None);
        assert_eq!(
            select_furniture(&objects, Some(3), Some(CONTAINER_MOGSAFE)),
            None
        );
    }
}
