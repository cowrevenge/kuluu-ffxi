//! Anchors the build-time `offsetof` walker against offsets verified by hand
//! against vendor/server/src/map/packets/s2c/*.h. The decoders const-assert
//! themselves against `s2c_layout`, so those asserts only mean anything if the
//! walker itself is right; these are the hand-checked numbers that say it is.

use ffxi_proto::s2c_layout::{abil_recast, clistatus, grap_list, group_list, login, weather};

/// vendor/server/src/map/packets/s2c/0x00a_login.h GP_SERV_COMMAND_LOGIN,
/// counting `GP_SERV_POS_HEAD` (0x2C bytes) from the body start.
#[test]
fn login_body_offsets_match_the_hand_walked_header() {
    assert_eq!(login::POS_HEAD, 0x00);
    assert_eq!(login::POS_HEAD_HP_MAX, 0x1A);
    assert_eq!(login::POS_HEAD_BT_TARGET_ID, 0x28);
    assert_eq!(login::ZONE_NO, 0x2C);
    assert_eq!(login::GAME_TIME, 0x38);
    assert_eq!(login::GRAP_ID_TBL, 0x40);
    assert_eq!(login::GRAP_ID_TBL_LEN, 18);
    assert_eq!(login::MUSIC_NUM, 0x52);
    assert_eq!(login::SUB_MAP_NUMBER, 0x5C);
    assert_eq!(login::EVENT_NUM, 0x5E);
    assert_eq!(login::WEATHER_NUMBER, 0x64);
    assert_eq!(login::SHIP_START, 0x74);
    assert_eq!(login::SHIP_END, 0x78);
    assert_eq!(login::LOGIN_STATE, 0x7C);
    assert_eq!(login::DEAD_COUNTER, 0xA0);
    assert_eq!(login::MYROOM_SUB_MAP_NUMBER, 0xA4);
    assert_eq!(login::MYROOM_MAP_NUMBER, 0xA6);
    assert_eq!(login::MY_ROOM_EXIT_BIT, 0xAA);
    assert_eq!(login::MOG_ZONE_FLAG, 0xAB);
}

/// vendor/server/src/map/packets/s2c/0x061_clistatus.h CLISTATUS.
#[test]
fn clistatus_body_offsets_match_the_hand_walked_header() {
    assert_eq!(clistatus::STATUSDATA_HPMAX, 0);
    assert_eq!(clistatus::STATUSDATA_MJOB_NO, 8);
    assert_eq!(clistatus::STATUSDATA_BP_BASE, 16);
    assert_eq!(clistatus::STATUSDATA_BP_ADJ, 30);
    assert_eq!(clistatus::STATUSDATA_ATK, 44);
    assert_eq!(clistatus::STATUSDATA_DEF_ELEM, 48);
    // `padding4F` and `ilvl_mhand` carry their own offsets in their LSB names.
    assert_eq!(clistatus::STATUSDATA_PADDING4F, 0x4F);
    assert_eq!(clistatus::STATUSDATA_ILVL, 0x51);
    assert_eq!(clistatus::STATUSDATA_ILVL_MHAND, 0x52);
}

/// vendor/server/src/map/packets/s2c/0x0dd_group_list.h GP_SERV_COMMAND_GROUP_LIST.
/// `GAttr` is a `uint32_t` bit-field run that fills exactly one word.
#[test]
fn group_list_body_offsets_and_gattr_bits_match_the_hand_walked_header() {
    assert_eq!(group_list::G_ATTR, 16);
    assert_eq!(group_list::G_ATTR_PARTY_NO_SHIFT, 0);
    assert_eq!(group_list::G_ATTR_PARTY_NO_MASK, 0b11);
    assert_eq!(group_list::G_ATTR_PARTY_LEADER_FLG_SHIFT, 2);
    assert_eq!(group_list::G_ATTR_ALLIANCE_LEADER_FLG_SHIFT, 3);
    assert_eq!(group_list::G_ATTR_LEVEL_SYNC_FLG_SHIFT, 8);
    assert_eq!(group_list::ACT_INDEX, 20);
    assert_eq!(group_list::PADDING1F, 27);
    assert_eq!(group_list::ZONE_NO, 28);
    assert_eq!(group_list::NAME, 36);
    assert_eq!(group_list::SIZE, 52);
}

/// vendor/server/src/map/packets/s2c/0x057_weather.h GP_SERV_COMMAND_WEATHER —
/// `xi::Weather` is `uint16_t` per vendor/server/data/enums/weather.yaml.
#[test]
fn weather_body_is_the_three_fields_the_header_declares() {
    assert_eq!(weather::START_TIME, 0);
    assert_eq!(weather::WEATHER_NUMBER, 4);
    assert_eq!(weather::WEATHER_OFFSET_TIME, 6);
    assert_eq!(weather::SIZE, 8);
}

/// vendor/server/src/map/packets/s2c/0x051_grap_list.h GP_SERV_COMMAND_GRAP_LIST.
#[test]
fn grap_list_body_opens_with_the_look_table() {
    assert_eq!(grap_list::GRAP_ID_TBL, 0);
    assert_eq!(grap_list::GRAP_ID_TBL_COUNT, 9);
    assert_eq!(grap_list::GRAP_ID_TBL_LEN, 18);
}

/// vendor/server/src/map/packets/s2c/0x119_abil_recast.h GP_SERV_COMMAND_ABIL_RECAST.
#[test]
fn abil_recast_entries_are_eight_byte_recasttimer_t() {
    assert_eq!(abil_recast::TIMERS, 0);
    assert_eq!(abil_recast::TIMERS_COUNT, 31);
    assert_eq!(abil_recast::TIMERS_STRIDE, 8);
    assert_eq!(abil_recast::TIMERS_TIMER, 0);
    assert_eq!(abil_recast::TIMERS_TIMER_ID, 3);
    assert_eq!(abil_recast::MOUNT_RECAST, 0xF8);
    assert_eq!(abil_recast::MOUNT_RECAST_ID, 0xFC);
}
