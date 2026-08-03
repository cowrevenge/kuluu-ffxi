//! Per-opcode metadata for the event VM, re-expressed (as our own committed
//! table) from atom0s/XiEvents `OpCodes/*.md` — a studied `research/` reference,
//! not a build input.
//!
//! - `size`: byte width to advance `ExecPointer` past an opcode the VM does not
//!   handle explicitly (only valid when `!jumps`).
//! - `jumps`: the opcode sets `ExecPointer` non-linearly (a real jump/branch);
//!   the VM must not skip an unimplemented one by size — it would desync.
//! - `sets_ret`: the opcode yields (breaks the exec loop this tick).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpMeta {
    pub size: u8,
    pub jumps: bool,
    pub sets_ret: bool,
    pub valid: bool,
}

/// Indexed by opcode value (0x00..=0xD9); every opcode in that range is defined.
pub const OPCODE_META: &[OpMeta] = &[
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0000
    OpMeta {
        size: 3,
        jumps: true,
        sets_ret: false,
        valid: true,
    }, // 0x0001
    OpMeta {
        size: 8,
        jumps: true,
        sets_ret: false,
        valid: true,
    }, // 0x0002
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0003
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0004
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0005
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0006
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0007
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0008
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0009
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x000A
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x000B
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x000C
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x000D
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x000E
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x000F
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0010
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0011
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0012
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0013
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0014
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0015
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0016
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0017
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0018
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0019
    OpMeta {
        size: 3,
        jumps: true,
        sets_ret: true,
        valid: true,
    }, // 0x001A
    OpMeta {
        size: 1,
        jumps: true,
        sets_ret: true,
        valid: true,
    }, // 0x001B
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x001C
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x001D
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x001E
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x001F
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0020
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0021
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0022
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0023
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0024
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0025
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0026
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0027
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0028
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0029
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x002A
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x002B
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x002C
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x002D
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x002E
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x002F
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0030
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0031
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0032
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0033
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0034
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0035
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0036
    OpMeta {
        size: 9,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0037
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0038
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0039
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x003A
    OpMeta {
        size: 11,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x003B
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x003C
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x003D
    OpMeta {
        size: 7,
        jumps: true,
        sets_ret: false,
        valid: true,
    }, // 0x003E
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x003F
    OpMeta {
        size: 9,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0040
    OpMeta {
        size: 9,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0041
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0042
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0043
    OpMeta {
        size: 5,
        jumps: true,
        sets_ret: false,
        valid: true,
    }, // 0x0044
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0045
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0046
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0047
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0048
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0049
    OpMeta {
        size: 9,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x004A
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x004B
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x004C
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x004D
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x004E
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x004F
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0050
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0051
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0052
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0053
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0054
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0055
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0056
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0057
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0058
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0059
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x005A
    OpMeta {
        // research/XiEvents/OpCodes/0x005B.md:33,144 — dispatched with param3=0,
        // so every ExecPointer path advances 15 (the +2 is param3-gated and unused
        // by 0x5B/0x66). atom0s's size table ambiguously lists "15, 17"; the
        // param3=0 call site is authoritative. Confirm against a captured event
        // stream if one containing 0x5B/0x66 becomes available.
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x005B
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x005C
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x005D
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x005E
    OpMeta {
        size: 18,
        jumps: true,
        sets_ret: true,
        valid: true,
    }, // 0x005F
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0060
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0061
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0062
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0063
    OpMeta {
        size: 11,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0064
    OpMeta {
        size: 11,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0065
    OpMeta {
        // See 0x005B: 0x0066 dispatches to the same helper with param3=0, so it
        // advances 15 too (research/XiEvents/OpCodes/0x0066.md:22).
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0066
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0067
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0068
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0069
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x006A
    OpMeta {
        size: 9,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x006B
    OpMeta {
        size: 9,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x006C
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x006D
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x006E
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x006F
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0070
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0071
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0072
    OpMeta {
        size: 11,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0073
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0074
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0075
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0076
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0077
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0078
    OpMeta {
        size: 12,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0079
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x007A
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x007B
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x007C
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x007D
    OpMeta {
        size: 18,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x007E
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x007F
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0080
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0081
    OpMeta {
        size: 7,
        jumps: true,
        sets_ret: false,
        valid: true,
    }, // 0x0082
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0083
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0084
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0085
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0086
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0087
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0088
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0089
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x008A
    OpMeta {
        size: 25,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x008B
    OpMeta {
        size: 14,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x008C
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x008D
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x008E
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x008F
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0090
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0091
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0092
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0093
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0094
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0095
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0096
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x0097
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0098
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x0099
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x009A
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x009B
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x009C
    OpMeta {
        size: 0,
        jumps: true,
        sets_ret: false,
        valid: true,
    }, // 0x009D
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x009E
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x009F
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00A0
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A1
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A2
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A3
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A4
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A5
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00A6
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00A7
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A8
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00A9
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00AA
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00AB
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00AC
    OpMeta {
        size: 12,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00AD
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00AE
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00AF
    OpMeta {
        size: 12,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00B0
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00B1
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00B2
    OpMeta {
        size: 18,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00B3
    OpMeta {
        size: 20,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00B4
    OpMeta {
        size: 4,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00B5
    OpMeta {
        size: 20,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00B6
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00B7
    OpMeta {
        size: 27,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00B8
    OpMeta {
        size: 8,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00B9
    OpMeta {
        size: 13,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00BA
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00BB
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00BC
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00BD
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00BE
    OpMeta {
        size: 10,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00BF
    OpMeta {
        size: 3,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C0
    OpMeta {
        size: 5,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00C1
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C2
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C3
    OpMeta {
        size: 11,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C4
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C5
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00C6
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C7
    OpMeta {
        size: 7,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C8
    OpMeta {
        size: 1,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00C9
    OpMeta {
        size: 0,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00CA
    OpMeta {
        size: 0,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00CB
    OpMeta {
        size: 14,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00CC
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00CD
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00CE
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00CF
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D0
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00D1
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D2
    OpMeta {
        size: 6,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D3
    OpMeta {
        size: 12,
        jumps: true,
        sets_ret: true,
        valid: true,
    }, // 0x00D4
    OpMeta {
        size: 17,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D5
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: true,
        valid: true,
    }, // 0x00D6
    OpMeta {
        size: 15,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D7
    OpMeta {
        size: 12,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D8
    OpMeta {
        size: 2,
        jumps: false,
        sets_ret: false,
        valid: true,
    }, // 0x00D9
];

/// Width of a *variable*-width opcode, selected by its sub byte
/// (`EventData[ExecPointer + 1]`).
///
/// [`OpMeta::size`] carries one number per opcode, so for these it holds the
/// widest case. Advancing by that where the data encodes a narrower one walks
/// `ExecPointer` into the middle of the next instruction and the VM then
/// executes operand bytes as opcodes — silently, because the skip looks
/// successful. `None` means the opcode is fixed-width, or that its width is not
/// modelled yet and the caller should fall back to `OpMeta::size`.
///
/// Rules transcribed from the retail pseudo-code in atom0s/XiEvents
/// `OpCodes/*.md`, which lists each of these with a multi-valued `OpCode Size`.
/// Only opcodes whose dispatch is decided by the sub byte alone appear here:
/// 0xAE, 0xB7 and 0xD8 also vary, but their width is entangled with a runtime
/// actor lookup, so they are left to the wider VM work rather than guessed at.
pub fn sub_size(op: u8, sub: u8) -> Option<u8> {
    match op {
        // 0x0046.md: sub 2 reads a work offset (4); every other path, including
        // the render-flag-gated fall-through, advances 2.
        OP_DEFCAMERA => Some(if sub == 2 { 4 } else { 2 }),
        // 0x0079.md: sub 1 is lookatone with a trailing work offset (12); sub 2
        // and the zero path are both 10.
        OP_LOOKAT => Some(if sub == 1 { 12 } else { 10 }),
        // 0x005C.md: the low band 0x00-0x07 sets a song id (4); the 0x80-0x87
        // band adds a volume (6), as do 0xA0/0xA1. Any other sub returns without
        // touching ExecPointer at all — a hang, so authored data cannot contain
        // one; fall back rather than encode a width retail never advances by.
        OP_MUSIC => match sub {
            0x00..=0x07 => Some(4),
            0x80..=0x87 | 0xA0 | 0xA1 => Some(6),
            _ => None,
        },
        // 0x00C2.md: sub 1 writes a party mask (4), sub 2 a validity flag (6),
        // anything else advances 2.
        OP_MOGHOUSE_VISIT => Some(match sub {
            1 => 4,
            2 => 6,
            _ => 2,
        }),
        _ => None,
    }
}

const OP_DEFCAMERA: u8 = 0x46;
const OP_LOOKAT: u8 = 0x79;
const OP_MUSIC: u8 = 0x5C;
const OP_MOGHOUSE_VISIT: u8 = 0xC2;

#[cfg(test)]
mod tests {
    use super::*;

    // Widths transcribed from the retail pseudo-code in atom0s/XiEvents. Each of
    // these opcodes has a single `OpMeta::size` holding its WIDEST case, so a
    // regression that drops sub_size would silently advance too far and start
    // executing operand bytes as opcodes.
    #[test]
    fn variable_width_opcodes_pick_their_width_from_the_sub_byte() {
        // 0x0046.md — only sub 2 is the 4-byte work-offset form.
        assert_eq!(sub_size(0x46, 2), Some(4));
        for sub in [0u8, 1, 3, 0xFF] {
            assert_eq!(sub_size(0x46, sub), Some(2), "0x46 sub {sub}");
        }
        // 0x0079.md — only sub 1 carries the trailing work offset.
        assert_eq!(sub_size(0x79, 1), Some(12));
        for sub in [0u8, 2, 9, 0xFF] {
            assert_eq!(sub_size(0x79, sub), Some(10), "0x79 sub {sub}");
        }
        // 0x005C.md — two documented bands, and a hang everywhere else.
        assert_eq!(sub_size(0x5C, 0x00), Some(4));
        assert_eq!(sub_size(0x5C, 0x07), Some(4));
        assert_eq!(sub_size(0x5C, 0x80), Some(6));
        assert_eq!(sub_size(0x5C, 0x87), Some(6));
        assert_eq!(sub_size(0x5C, 0xA1), Some(6));
        assert_eq!(sub_size(0x5C, 0x40), None, "undocumented sub falls back");
        // 0x00C2.md
        assert_eq!(sub_size(0xC2, 1), Some(4));
        assert_eq!(sub_size(0xC2, 2), Some(6));
        assert_eq!(sub_size(0xC2, 7), Some(2));
    }

    // Every width sub_size can return must be <= the table's fixed size, which is
    // the widest documented case. A value above it would mean the table is wrong,
    // not the dispatch.
    #[test]
    fn sub_widths_never_exceed_the_tables_widest_case() {
        for op in [0x46u8, 0x79, 0x5C, 0xC2] {
            let fixed = OPCODE_META[op as usize].size;
            for sub in 0..=u8::MAX {
                if let Some(w) = sub_size(op, sub) {
                    assert!(w <= fixed, "op {op:#04X} sub {sub}: {w} > {fixed}");
                    assert!(w >= 2, "op {op:#04X} sub {sub}: {w} cannot advance");
                }
            }
        }
    }
}
