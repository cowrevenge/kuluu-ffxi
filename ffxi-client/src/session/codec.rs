use super::*;

// GP_CLI_COMMAND_BUFFCANCEL, vendor/server/src/map/packets/c2s/0x0f1_buffcancel.h:
// BuffNo u16 (the status icon id), padding u16. The server runs
// DelStatusEffectsByIcon(BuffNo) and blocks the packet only while InEvent
// (0x0f1_buffcancel.cpp); it does NOT re-check cancelability, so the caller
// gates on ffxi_proto::status_effects::is_cancelable.
pub fn build_subpacket_buffcancel(sync: u16, buff_no: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x0F1, 2, sync));
    buf[4..6].copy_from_slice(&buff_no.to_le_bytes());
    buf
}

pub fn build_subpacket_shop_buy(sync: u16, qty: u32, shop_no: u16, shop_index: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::SHOP_BUY,
        4,
        sync,
    ));
    buf[4..8].copy_from_slice(&qty.to_le_bytes());
    buf[8..10].copy_from_slice(&shop_no.to_le_bytes());
    buf[10..12].copy_from_slice(&(shop_index as u16).to_le_bytes());
    buf[12] = 0;
    buf
}

// GP_CLI_COMMAND_SHOP_SELL_REQ, vendor/server/src/map/packets/c2s/0x084_shop_sell_req.h:
// ItemNum u32, ItemNo u16, ItemIndex u8, padding u8. The server appraises the item in
// that LOC_INVENTORY slot, clamps ItemNum to the held quantity, parks it in the shop
// trade container, and answers with s2c 0x03D SHOP_SELL (0x084_shop_sell_req.cpp).
pub fn build_subpacket_shop_sell_req(sync: u16, qty: u32, item_no: u16, item_index: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 12];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::SHOP_SELL_REQ,
        3,
        sync,
    ));
    buf[4..8].copy_from_slice(&qty.to_le_bytes());
    buf[8..10].copy_from_slice(&item_no.to_le_bytes());
    buf[10] = item_index;
    buf
}

// GP_CLI_COMMAND_SHOP_SELL_SET, vendor/server/src/map/packets/c2s/0x085_shop_sell_set.h:
// SellFlag u16, padding u16. The server validator rejects the packet unless SellFlag
// equals 1 and a SHOP_SELL_REQ preceded it (0x085_shop_sell_set.cpp validate).
pub fn build_subpacket_shop_sell_set(sync: u16) -> Vec<u8> {
    const SELL_FLAG_CONFIRM: u16 = 1;
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::SHOP_SELL_SET,
        2,
        sync,
    ));
    buf[4..6].copy_from_slice(&SELL_FLAG_CONFIRM.to_le_bytes());
    buf
}

pub fn build_subpacket_action(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    kind: &crate::state::ActionKind,
) -> Vec<u8> {
    let mut buf = vec![0u8; 28];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::ACTION,
        7,
        sync,
    ));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());
    buf[8..10].copy_from_slice(&act_index.to_le_bytes());
    buf[10..12].copy_from_slice(&kind.action_id().to_le_bytes());
    let mut action_buf = [0u8; 16];
    kind.fill_action_buf(&mut action_buf);
    buf[12..28].copy_from_slice(&action_buf);
    buf
}

// c2s 0x05D GP_CLI_COMMAND_MOTION: UniqueNo u32 @4, ActIndex u16 @8, Number u8
// @10 (emote id), Mode u8 @11, Param u16 @12, pad u16 @14
// (vendor/server/src/map/packets/c2s/0x05d_motion.h:28-35). Note the c2s Mode
// byte precedes Param, unlike the s2c 0x05A layout.
pub fn build_subpacket_motion(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    number: u8,
    mode: u8,
    param: u16,
) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::MOTION,
        4,
        sync,
    ));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());
    buf[8..10].copy_from_slice(&act_index.to_le_bytes());
    buf[10] = number;
    buf[11] = mode;
    buf[12..14].copy_from_slice(&param.to_le_bytes());
    buf
}

// c2s 0x119 GP_CLI_COMMAND_EMOTE_LIST — header only
// (vendor/server/src/map/packets/c2s/0x119_emote_list.h declares no payload).
pub fn build_subpacket_emote_list_req(sync: u16) -> Vec<u8> {
    build_subpacket_header(ffxi_proto::map::c2s::EMOTE_LIST, 1, sync).to_vec()
}

// c2s 0x0F4 GP_CLI_COMMAND_TRACKING_LIST: uint32 SendFlg (must be 1). Requests
// the wide-scan list; the server frames the reply with 0x0F6 ListStart/ListEnd.
// vendor/server/src/map/packets/c2s/0x0f4_tracking_list.h.
pub fn build_subpacket_tracking_list(sync: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::TRACKING_LIST,
        2,
        sync,
    ));
    buf[4..8].copy_from_slice(&ffxi_proto::map::tracking::SEND_FLG_REQUEST.to_le_bytes());
    buf
}

// c2s 0x0F5 GP_CLI_COMMAND_TRACKING_START: uint32 ActIndex. Begins tracking one
// entity; the server then streams 0x0F5 position updates.
// vendor/server/src/map/packets/c2s/0x0f5_tracking_start.h.
pub fn build_subpacket_tracking_start(sync: u16, act_index: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::TRACKING_START,
        2,
        sync,
    ));
    buf[4..8].copy_from_slice(&u32::from(act_index).to_le_bytes());
    buf
}

// c2s 0x0F6 GP_CLI_COMMAND_TRACKING_END: uint32 padding (Dammy). Stops tracking.
// vendor/server/src/map/packets/c2s/0x0f6_tracking_end.h.
pub fn build_subpacket_tracking_end(sync: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::TRACKING_END,
        2,
        sync,
    ));
    buf
}

/// c2s 0x110 GP_CLI_COMMAND_FISHING_2 — the mini-game request the client streams while
/// fishing (check-hook, end-game, release, timeout). `mode`/`para`/`para2` follow the
/// LSB validator: vendor/server/src/map/packets/c2s/0x110_fishing_2.{h,cpp}.
pub fn build_subpacket_fishing(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    mode: crate::state::FishingMode,
    para: i32,
    para2: i32,
) -> Vec<u8> {
    let mut buf = vec![0u8; 20];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::FISHING_2,
        5,
        sync,
    ));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());
    buf[8..12].copy_from_slice(&para.to_le_bytes());
    buf[12..14].copy_from_slice(&act_index.to_le_bytes());
    buf[14] = mode as u8;
    buf[16..20].copy_from_slice(&para2.to_le_bytes());
    buf
}

pub fn build_subpacket_equip_inspect(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    kind: u8,
) -> Vec<u8> {
    let mut buf = vec![0u8; 16];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x0DD, 4, sync));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());

    buf[8..12].copy_from_slice(&(act_index as u32).to_le_bytes());
    buf[12] = kind;

    buf
}

// GP_CLI_COMMAND_BAZAAR_LIST, vendor/server/src/map/packets/c2s/0x105_bazaar_list.h:27-31:
// UniqueNo u32, ActIndex u16, padding u16. The server rejects it while we still
// hold a BazaarID, so leave the previous bazaar first (0x105_bazaar_list.cpp validate).
pub fn build_subpacket_bazaar_list(sync: u16, unique_no: u32, act_index: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 12];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::BAZAAR_LIST,
        3,
        sync,
    ));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());
    buf[8..10].copy_from_slice(&act_index.to_le_bytes());
    buf
}

// GP_CLI_COMMAND_BAZAAR_BUY, vendor/server/src/map/packets/c2s/0x106_bazaar_buy.h:27-31:
// BazaarItemIndex u8, padding u8[3], BuyNum u32. `index` is the seller-side
// LOC_INVENTORY slot from the s2c 0x105 row.
pub fn build_subpacket_bazaar_buy(sync: u16, index: u8, quantity: u32) -> Vec<u8> {
    let mut buf = vec![0u8; 12];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::BAZAAR_BUY,
        3,
        sync,
    ));
    buf[4] = index;
    buf[8..12].copy_from_slice(&quantity.to_le_bytes());
    buf
}

// GP_CLI_COMMAND_BAZAAR_EXIT, vendor/server/src/map/packets/c2s/0x104_bazaar_exit.h:
// header only. Clears our BazaarID server-side and notifies the seller.
pub fn build_subpacket_bazaar_exit(sync: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 4];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::BAZAAR_EXIT,
        1,
        sync,
    ));
    buf
}

pub fn build_subpacket_reqlogout(sync: u16, mode: u16, kind: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::REQ_LOGOUT,
        2,
        sync,
    ));
    buf[4..6].copy_from_slice(&mode.to_le_bytes());
    buf[6..8].copy_from_slice(&kind.to_le_bytes());
    buf
}

pub fn build_subpacket_camp(sync: u16, mode: HealMode) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x0E8, 2, sync));
    buf[4..8].copy_from_slice(&mode.as_u32().to_le_bytes());
    buf
}

pub fn build_subpacket_item_use(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    category: u8,
    slot: u8,
) -> Vec<u8> {
    let mut buf = vec![0u8; 20];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x037, 5, sync));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());

    buf[12..14].copy_from_slice(&act_index.to_le_bytes());
    buf[14] = slot;

    buf[16..20].copy_from_slice(&(category as u32).to_le_bytes());
    buf
}

pub fn build_subpacket_equip_set(
    sync: u16,
    container_index: u8,
    equip_slot: u8,
    container: u8,
) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::EQUIP_SET,
        2,
        sync,
    ));
    buf[4] = container_index;
    buf[5] = equip_slot;
    buf[6] = container;

    buf
}

// GP_CLI_COMMAND_ITEM_STACK, vendor/server/src/map/packets/c2s/0x03a_item_stack.h:
// `uint32_t Category` (container id) after the 4-byte subpacket header, so 8 bytes
// total (size_words = 2). The server consolidates same-id partial stacks.
pub fn build_subpacket_item_stack(sync: u16, container: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::ITEM_STACK,
        2,
        sync,
    ));
    buf[4..8].copy_from_slice(&(container as u32).to_le_bytes());

    buf
}

// GP_CLI_COMMAND_TROPHY_ENTRY / GP_CLI_COMMAND_TROPHY_ABSENCE,
// vendor/server/src/map/packets/c2s/0x041_trophy_entry.h and
// 0x042_trophy_absence.h: a single TrophyItemIndex after the header, the rest
// padding, so 8 bytes total (size_words = 2). The server rolls the lot value
// itself and ignores repeat lots/passes on a slot the player already acted on
// (0x041_trophy_entry.cpp / 0x042_trophy_absence.cpp process).
pub fn build_subpacket_trophy_lot(sync: u16, slot: u8) -> Vec<u8> {
    build_subpacket_trophy(ffxi_proto::map::c2s::TROPHY_ENTRY, sync, slot)
}

pub fn build_subpacket_trophy_pass(sync: u16, slot: u8) -> Vec<u8> {
    build_subpacket_trophy(ffxi_proto::map::c2s::TROPHY_ABSENCE, sync, slot)
}

fn build_subpacket_trophy(opcode: u16, sync: u16, slot: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(opcode, 2, sync));
    buf[4] = slot;
    buf
}

pub fn build_subpacket_item_move(
    sync: u16,
    quantity: u32,
    from_container: u8,
    to_container: u8,
    from_slot: u8,
    to_slot: Option<u8>,
) -> Vec<u8> {
    let mut buf = vec![0u8; 12];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::ITEM_MOVE,
        3,
        sync,
    ));
    buf[4..8].copy_from_slice(&quantity.to_le_bytes());
    buf[8] = from_container;
    buf[9] = to_container;
    buf[10] = from_slot;
    buf[11] = to_slot.unwrap_or(ITEM_MOVE_AUTO_SLOT);
    buf
}

// GP_CLI_COMMAND_PBX, vendor/server/src/map/packets/c2s/0x04d_pbx.h: Command u8
// @4, BoxNo i8 @5, PostWorkNo i8 @6, ItemWorkNo i8 @7, ItemStacks i32 @8,
// Result/ResParam1-3 i8 @12-15 (the validator requires all four zero c2s),
// TargetName[16] @16 — 32 bytes (size_words = 8). Per-command field defaults
// mirror the LSB PacketValidator (0x04d_pbx.cpp validate): unused numeric
// fields are -1; Recv's ItemWorkNo must be 1; Set/Send/Cancel are pinned to the
// Outgoing box, Recv/Accept/Reject to Incoming, Query/Confirm/*Open/Close to
// BoxNo None.
pub fn build_subpacket_pbx(sync: u16, op: &crate::state::DeliveryBoxOp) -> Vec<u8> {
    use crate::state::DeliveryBoxOp as Op;
    use ffxi_proto::map::pbx::{boxno, command};
    type Fields<'a> = (u8, i8, i8, i8, i32, Option<&'a str>);
    let (cmd, box_no, post_work_no, item_work_no, item_stacks, name): Fields = match op {
        Op::Work { box_no } => (command::WORK, box_no.wire(), -1, -1, -1, None),
        Op::Set {
            slot,
            inventory_slot,
            quantity,
            recipient,
        } => (
            command::SET,
            boxno::OUTGOING,
            *slot as i8,
            *inventory_slot as i8,
            *quantity as i32,
            Some(recipient),
        ),
        Op::Send { slot } => (command::SEND, boxno::OUTGOING, *slot as i8, -1, -1, None),
        Op::Cancel { slot } => (command::CANCEL, boxno::OUTGOING, *slot as i8, -1, -1, None),
        Op::Check { box_no } => (command::CHECK, box_no.wire(), -1, -1, -1, None),
        Op::Recv { slot } => (command::RECV, boxno::INCOMING, *slot as i8, 1, -1, None),
        Op::Confirm => (command::CONFIRM, boxno::NONE, -1, -1, -1, None),
        Op::Accept { slot } => (command::ACCEPT, boxno::INCOMING, *slot as i8, -1, -1, None),
        Op::Reject { slot } => (command::REJECT, boxno::INCOMING, *slot as i8, -1, -1, None),
        Op::Get { box_no, slot } => (command::GET, box_no.wire(), *slot as i8, -1, -1, None),
        Op::Clear { box_no, slot } => (command::CLEAR, box_no.wire(), *slot as i8, -1, -1, None),
        Op::Query { recipient } => (command::QUERY, boxno::NONE, -1, -1, -1, Some(recipient)),
        Op::DeliOpen => (command::DELI_OPEN, boxno::NONE, -1, -1, -1, None),
        Op::PostOpen => (command::POST_OPEN, boxno::NONE, -1, -1, -1, None),
        Op::PostClose { .. } => (command::POST_CLOSE, boxno::NONE, -1, -1, -1, None),
    };
    let mut buf = vec![0u8; 32];
    buf[0..4].copy_from_slice(&build_subpacket_header(ffxi_proto::map::c2s::PBX, 8, sync));
    buf[4] = cmd;
    buf[5] = box_no as u8;
    buf[6] = post_work_no as u8;
    buf[7] = item_work_no as u8;
    buf[8..12].copy_from_slice(&item_stacks.to_le_bytes());
    if let Some(name) = name {
        let bytes = name.as_bytes();
        let n = bytes.len().min(15); // NUL terminator stays inside TargetName[16]
        buf[16..16 + n].copy_from_slice(&bytes[..n]);
    }
    buf
}

// c2s 0x100 GP_CLI_COMMAND_MYROOM_JOB: MainJobIndex u8 @4, SupportJobIndex u8 @5,
// u16 pad; 0 = keep the current job
// (vendor/server/src/map/packets/c2s/0x100_myroom_job.h:27-31).
pub fn build_subpacket_myroom_job(sync: u16, main_job: Option<u8>, sub_job: Option<u8>) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::MYROOM_JOB,
        2,
        sync,
    ));
    buf[4] = main_job.unwrap_or(0);
    buf[5] = sub_job.unwrap_or(0);
    buf
}

pub(crate) fn build_subpacket_header(opcode: u16, size_words: u16, sync: u16) -> [u8; 4] {
    let id_and_size = framing::subpacket_header_word(opcode, size_words);
    let mut h = [0u8; 4];
    h[0..2].copy_from_slice(&id_and_size.to_le_bytes());
    h[2..4].copy_from_slice(&sync.to_le_bytes());
    h
}

pub(crate) fn build_subpacket_gameok(sync: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 12];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::GAMEOK,
        3,
        sync,
    ));
    buf
}

pub(crate) fn build_subpacket_zone_transition(sync: u16) -> Vec<u8> {
    let mut buf = vec![0u8; 8];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::ZONE_TRANSITION,
        2,
        sync,
    ));

    buf[4] = 2;
    buf
}

pub(crate) fn build_subpacket_chat(sync: u16, kind: u8, text: &str) -> Vec<u8> {
    let str_bytes = text.as_bytes();
    let str_len = str_bytes.len().min(127);
    let body_unpadded = 2 + str_len + 1;
    let body_padded = (body_unpadded + 3) & !3;
    let total = 4 + body_padded;
    let size_words = (total / 4) as u16;

    let mut buf = vec![0u8; total];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::CHAT,
        size_words,
        sync,
    ));
    buf[4] = kind;

    buf[6..6 + str_len].copy_from_slice(&str_bytes[..str_len]);

    buf
}

pub(crate) fn build_subpacket_tell(sync: u16, recipient: &str, text: &str) -> Vec<u8> {
    let r_bytes = recipient.as_bytes();
    let r_len = r_bytes.len().min(14);
    let t_bytes = text.as_bytes();
    let t_len = t_bytes.len().min(127);

    let body_unpadded = 1 + 1 + 15 + t_len + 1;
    let body_padded = (body_unpadded + 3) & !3;
    let total = 4 + body_padded;
    let size_words = (total / 4) as u16;

    let mut buf = vec![0u8; total];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x0B6, size_words, sync));
    // GP_CLI_COMMAND_CHAT_NAME.unknown00 must be 3 or the server rejects the
    // packet (PacketValidator .mustEqual(unknown00, 3), 0x0b6_chat_name.cpp:60);
    // the retail client always sends 3. Without it every tell — and the
    // customMenu reply — is silently dropped.
    buf[4] = CHAT_NAME_UNKNOWN00;

    buf[6..6 + r_len].copy_from_slice(&r_bytes[..r_len]);

    buf[21..21 + t_len].copy_from_slice(&t_bytes[..t_len]);
    buf
}

pub(crate) const CHAT_NAME_UNKNOWN00: u8 = 3;

// c2s 0x05B GP_CLI_COMMAND_EVENTEND (vendor/server/src/map/packets/c2s/
// 0x05b_eventend.h:34-41): UniqueNo u32, EndPara u32, ActIndex u16, Mode u16
// (0 = End), EventNum u16 (zone id — retail echoes GP_SERV LOGIN EventNum,
// 0x00a_login.cpp:187), EventPara u16 (the event id the validator matches
// against currentEvent->eventId, validation.cpp:71-76).
pub(crate) fn build_subpacket_event_end(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    event_zone: u16,
    event_id: u16,
    choice: u32,
) -> Vec<u8> {
    let mut buf = vec![0u8; 20];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::EVENT_END,
        5,
        sync,
    ));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());
    buf[8..12].copy_from_slice(&choice.to_le_bytes());
    buf[12..14].copy_from_slice(&act_index.to_le_bytes());

    buf[16..18].copy_from_slice(&event_zone.to_le_bytes());
    buf[18..20].copy_from_slice(&event_id.to_le_bytes());
    buf
}

// Inverse of the s2c 0x055 id decode — LSB reads the bits back as
// keyItemId = TableIndex*512 + word*32 + bit (vendor/server/src/map/packets/
// c2s/0x064_scenarioitem.cpp:44). Ids outside `table_index`'s range are
// ignored; returns whether any bit changed.
pub(crate) fn fold_seen_ids_into_look_flags(
    table_index: u16,
    ids: &[u16],
    look_flags: &mut [u32; decode::ScenarioItem::WORDS],
) -> bool {
    let word_bits = u32::BITS as usize;
    let mut changed = false;
    for id in ids {
        let global = *id as usize;
        if global / decode::ScenarioItem::BITS_PER_TABLE != table_index as usize {
            continue;
        }
        let local = global % decode::ScenarioItem::BITS_PER_TABLE;
        let mask = 1u32 << (local % word_bits);
        if look_flags[local / word_bits] & mask == 0 {
            look_flags[local / word_bits] |= mask;
            changed = true;
        }
    }
    changed
}

// LSB gates c2s 0x064 with blockedBy(InEvent) and silently drops it unless
// UniqueNo == char id and ActIndex == self targid (vendor/server/src/map/
// packets/c2s/0x064_scenarioitem.cpp:31-33), so an unseeded targid must skip
// the send; a table whose s2c 0x055 never arrived has only default-zeroed
// local flags, so marking against it would report the table empty. Ok carries
// the validated ActIndex.
pub(crate) fn mark_seen_send_block_reason(
    in_event: bool,
    self_act_index: Option<u16>,
    table_received: bool,
) -> Result<u16, &'static str> {
    if in_event {
        return Err("InEvent blocks 0x064");
    }
    let act_index = self_act_index.ok_or("self act_index not yet seeded")?;
    if !table_received {
        return Err("table's 0x055 not received this session");
    }
    Ok(act_index)
}

// c2s 0x064 GP_CLI_COMMAND_SCENARIOITEM (vendor/server/src/map/packets/c2s/
// 0x064_scenarioitem.h): UniqueNo u32, LookItemFlag u32[16], ActIndex u16,
// TableIndex u16. The server ORs every set LookItemFlag bit into the table's
// seen list and validates UniqueNo == char id, ActIndex == self targid,
// TableIndex < tables.size() (0x064_scenarioitem.cpp); blocked while InEvent.
pub(crate) fn build_subpacket_scenario_item(
    sync: u16,
    unique_no: u32,
    act_index: u16,
    table_index: u16,
    look_flags: &[u32; decode::ScenarioItem::WORDS],
) -> Vec<u8> {
    const TOTAL: usize = 4 + 4 + decode::ScenarioItem::WORDS * 4 + 2 + 2;
    let mut buf = vec![0u8; TOTAL];
    buf[0..4].copy_from_slice(&build_subpacket_header(
        ffxi_proto::map::c2s::SCENARIO_ITEM,
        (TOTAL / 4) as u16,
        sync,
    ));
    buf[4..8].copy_from_slice(&unique_no.to_le_bytes());
    for (i, w) in look_flags.iter().enumerate() {
        let o = 8 + i * 4;
        buf[o..o + 4].copy_from_slice(&w.to_le_bytes());
    }
    buf[72..74].copy_from_slice(&act_index.to_le_bytes());
    buf[74..76].copy_from_slice(&table_index.to_le_bytes());
    buf
}

// GP_CLI_COMMAND_ITEM_MOVE, vendor/server/src/map/packets/c2s/0x029_item_move.h:
// ItemNum u32 @4, Category1 u8 @8, Category2 u8 @9, ItemIndex1 u8 @10,
// ItemIndex2 u8 @11 — 12 bytes (size_words = 3). An ItemIndex2 < 82 asks for a
// same-id stack merge into that slot; anything larger lets the server pick a free
// slot (0x029_item_move.cpp process), which retail requests with 0xFF.
pub(crate) const ITEM_MOVE_AUTO_SLOT: u8 = 0xFF;

pub(crate) fn build_subpacket_maprect(
    sync: u16,
    rect_id: u32,
    x: f32,
    y: f32,
    z: f32,
    act_index: u16,
) -> Vec<u8> {
    let mut buf = vec![0u8; 24];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x05E, 6, sync));
    buf[4..8].copy_from_slice(&rect_id.to_le_bytes());
    buf[8..12].copy_from_slice(&x.to_le_bytes());
    buf[12..16].copy_from_slice(&y.to_le_bytes());
    buf[16..20].copy_from_slice(&z.to_le_bytes());
    buf[20..22].copy_from_slice(&act_index.to_le_bytes());

    buf
}

/// The RectID fourcc LSB matches for the universal MH exit
/// (vendor/server/src/map/packets/c2s/0x05e_maprect.cpp:72). Emitted by
/// [`build_subpacket_maprect_mh_exit`]; also the `pending_maprect` line id.
pub(crate) const ZMRQ_LE: u32 = u32::from_le_bytes(*b"zmrq");

pub(crate) fn build_subpacket_maprect_mh_exit(
    sync: u16,
    exit_bit: u8,
    exit_mode: u8,
    x: f32,
    y: f32,
    z: f32,
    act_index: u16,
) -> Vec<u8> {
    let mut buf = vec![0u8; 24];
    buf[0..4].copy_from_slice(&build_subpacket_header(0x05E, 6, sync));

    buf[4..8].copy_from_slice(&ZMRQ_LE.to_le_bytes());
    buf[8..12].copy_from_slice(&x.to_le_bytes());
    buf[12..16].copy_from_slice(&y.to_le_bytes());
    buf[16..20].copy_from_slice(&z.to_le_bytes());
    buf[20..22].copy_from_slice(&act_index.to_le_bytes());
    buf[22] = exit_bit;
    buf[23] = exit_mode;
    buf
}

/// Client-side mirror of the LSB 0x05D validator: `blockedBy InEvent`,
/// `oneOf<EmoteMode>`, `range Number Point..=Aim` (0x05d_motion.cpp:43-49) and
/// the bell note range (:82). `None` = OK to send. The bell-equip and
/// job-unlock checks stay server-side (the client lacks lockstyle state).
pub(crate) fn emote_send_block_reason(
    emote_id: u8,
    mode: u8,
    param: u16,
    in_event: bool,
) -> Option<String> {
    use ffxi_proto::map::emote;
    if in_event {
        return Some("busy with an event".into());
    }
    if mode > emote::mode::MOTION {
        return Some(format!("invalid mode {mode}"));
    }
    if ffxi_proto::emote_names::lookup(emote_id).is_none() {
        return Some(format!("unknown emote id {emote_id}"));
    }
    if emote_id == emote::BELL && !(emote::BELL_NOTE_MIN..=emote::BELL_NOTE_MAX).contains(&param) {
        return Some(format!(
            "bell note {param} out of range {}..={}",
            emote::BELL_NOTE_MIN,
            emote::BELL_NOTE_MAX
        ));
    }
    None
}

pub(crate) fn build_subpacket_pos(
    sync: u16,
    x: f32,
    y: f32,
    z: f32,
    heading: u8,
    face_target: u16,
) -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    buf[0..4].copy_from_slice(&build_subpacket_header(ffxi_proto::map::c2s::POS, 8, sync));
    buf[4..8].copy_from_slice(&x.to_le_bytes());
    buf[8..12].copy_from_slice(&z.to_le_bytes());
    buf[12..16].copy_from_slice(&y.to_le_bytes());
    buf[20] = heading;
    // GP_CLI_COMMAND_POS.facetarget (vendor/server/.../c2s/0x015_pos.h): the targid
    // we're looking at, relayed by the server so other clients turn our head. +21
    // is the TargetMode/RunMode/GroundMode bitfield, left 0.
    buf[22..24].copy_from_slice(&face_target.to_le_bytes());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    buf[24..28].copy_from_slice(&now.to_le_bytes());
    buf
}
