use super::*;

#[derive(Debug, Clone, Copy)]
pub struct ItemMax {
    pub capacities: [u16; Self::CONTAINER_COUNT],
}

impl ItemMax {
    /// One capacity per LSB CONTAINER_ID (LOC_INVENTORY..=LOC_RECYCLEBIN),
    /// vendor/server/src/map/item_container.h:32-49.
    pub const CONTAINER_COUNT: usize = 18;
    pub const SIZE: usize = 96;
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let mut capacities = [0u16; Self::CONTAINER_COUNT];
        // Fall back to the legacy u8 array only when the whole wide array is
        // absent (pre-widening servers). A per-slot fallback would erase LSB's
        // "container disabled" sentinel — ItemNum2 = 0 while the legacy byte
        // stays sized, e.g. a lapsed Mog Locker lease
        // (vendor/server/src/map/packets/s2c/0x01c_item_max.cpp:52-57).
        let wide_at = |i: usize| {
            let off = 18 + 14 + i * 2;
            u16::from_le_bytes(body[off..off + 2].try_into().unwrap())
        };
        let wide_present = (0..Self::CONTAINER_COUNT).any(|i| wide_at(i) != 0);
        for (i, cap) in capacities.iter_mut().enumerate() {
            let raw = if wide_present {
                wide_at(i)
            } else {
                body[i] as u16
            };
            *cap = raw.saturating_sub(1);
        }
        Ok(Self { capacities })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemSameState {
    StillLoading,
    AllLoaded,
}

#[derive(Debug, Clone, Copy)]
pub struct ItemSame {
    pub state: ItemSameState,
    pub flags: u32,
}

impl ItemSame {
    pub const SIZE: usize = 8;
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let state = match body[0] {
            0 => ItemSameState::StillLoading,

            _ => ItemSameState::AllLoaded,
        };
        let flags = u32::from_le_bytes(body[4..8].try_into().unwrap());
        Ok(Self { state, flags })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ItemNum {
    pub quantity: u32,

    pub category: u8,

    pub index: u8,

    pub lock_flg: u8,
}

impl ItemNum {
    pub const SIZE: usize = 8;
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            quantity: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            category: body[4],
            index: body[5],
            lock_flg: body[6],
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ItemList {
    pub quantity: u32,

    pub item_no: u16,
    pub category: u8,
    pub index: u8,
    pub lock_flg: u8,
}

impl ItemList {
    pub const SIZE: usize = 12;
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            quantity: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            item_no: u16::from_le_bytes(body[4..6].try_into().unwrap()),
            category: body[6],
            index: body[7],
            lock_flg: body[8],
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ItemAttr {
    pub quantity: u32,
    pub price: u32,
    pub item_no: u16,
    pub category: u8,
    pub index: u8,
    pub lock_flg: u8,
    pub extdata: [u8; 24],
}

/// Charges + live recast decoded from the 24-byte item extdata of a charged
/// (usable/enchanted) item. `next_use_vana_ts` is an absolute Vana'diel
/// timestamp (Earth seconds since [`crate::vana_time::VANA_EPOCH_UNIX`]).
/// Readiness is signaled by `ready` (extdata flags-hi bit 0x40), NOT by a zero
/// timestamp: LSB only writes Attr[4..8] on the cooldown path and leaves stale
/// m_extra bytes there when ready (0x020_item_attr.cpp:57-68), so consumers
/// must gate on `ready` / `ts > now` rather than `ts == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeInfo {
    pub charges: u8,
    pub next_use_vana_ts: u32,
    pub ready: bool,
}

/// Item extdata byte layout for charged items.
/// vendor/server/src/map/items/exdata/timer_info.h:29-41 and
/// vendor/server/src/map/packets/s2c/0x020_item_attr.cpp:47-82.
mod extdata {
    use core::ops::Range;
    pub const HEADER_CHARGED: u8 = 0x01;
    pub const OFF_HEADER: usize = 0;
    pub const OFF_CHARGES: usize = 1;
    pub const OFF_FLAGS_HI: usize = 3;
    pub const NEXT_USE: Range<usize> = 4..8;
    pub const FLAG_READY: u8 = 0x40;
}

impl ItemAttr {
    pub const SIZE: usize = 37;
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let mut extdata = [0u8; 24];
        extdata.copy_from_slice(&body[13..37]);
        Ok(Self {
            quantity: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            price: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            item_no: u16::from_le_bytes(body[8..10].try_into().unwrap()),
            category: body[10],
            index: body[11],
            lock_flg: body[12],
            extdata,
        })
    }

    /// Charge/recast info for a charged (usable/enchanted) item, or `None` when
    /// the extdata header marks it as non-charged.
    pub fn charge_info(&self) -> Option<ChargeInfo> {
        if self.extdata[extdata::OFF_HEADER] != extdata::HEADER_CHARGED {
            return None;
        }
        Some(ChargeInfo {
            charges: self.extdata[extdata::OFF_CHARGES],
            next_use_vana_ts: u32::from_le_bytes(
                self.extdata[extdata::NEXT_USE].try_into().unwrap(),
            ),
            ready: self.extdata[extdata::OFF_FLAGS_HI] & extdata::FLAG_READY != 0,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EquipList {
    pub container_index: u8,

    pub equip_slot: u8,

    pub container: u8,
}

impl EquipList {
    pub const SIZE: usize = 4;
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        Ok(Self {
            container_index: body[0],
            equip_slot: body[1],
            container: body[2],
        })
    }
}
