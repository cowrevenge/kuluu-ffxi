use super::*;

// s2c 0x0C9 GP_SERV_COMMAND_EQUIP_INSPECT — the /check answer for a PC target.
// LSB pushes up to three EQUIPMENT batches (OptionFlag 0x03, 8 checkitems each)
// followed by one GENERAL packet (OptionFlag 0x01), all carrying the target's
// UniqNo/ActIndex (vendor/server/src/map/packets/c2s/0x0dd_equip_inspect.cpp:135-136).
// Offsets are into the subpacket body (4-byte GP_SERV_HEADER stripped).

// vendor/server/src/map/packets/s2c/0x0c9_equip_inspect_general.cpp:33
pub(crate) const EQUIP_INSPECT_OPTION_GENERAL: u8 = 0x01;
// vendor/server/src/map/packets/s2c/0x0c9_equip_inspect_equipment.cpp:37
pub(crate) const EQUIP_INSPECT_OPTION_EQUIPMENT: u8 = 0x03;

const EQUIP_INSPECT_OPTION_FLAG_OFFSET: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EquipInspect {
    General(EquipInspectGeneral),
    Equipment(EquipInspectEquipment),
}

impl EquipInspect {
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let flag = *body
            .get(EQUIP_INSPECT_OPTION_FLAG_OFFSET)
            .ok_or(DecodeError::Truncated(
                EQUIP_INSPECT_OPTION_FLAG_OFFSET + 1,
                body.len(),
            ))?;
        match flag {
            EQUIP_INSPECT_OPTION_GENERAL => EquipInspectGeneral::decode(body).map(Self::General),
            EQUIP_INSPECT_OPTION_EQUIPMENT => {
                EquipInspectEquipment::decode(body).map(Self::Equipment)
            }
            other => Err(DecodeError::UnknownDiscriminant(other)),
        }
    }
}

/// Mode 0x01: jobs/levels (zeroed while the target is /anon) and linkshell.
/// vendor/server/src/map/packets/s2c/0x0c9_equip_inspect_general.h:38-60.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipInspectGeneral {
    pub unique_no: u32,
    pub act_index: u16,
    pub linkshell_item_no: u16,
    pub linkshell_name_raw: [u8; Self::LS_NAME_LEN],
    pub linkshell_color_raw: u16,
    pub main_job: u8,
    pub sub_job: u8,
    pub main_job_lv: u8,
    pub sub_job_lv: u8,
    pub master_job: u8,
    pub master_lv: u8,
    pub master_flags: u8,
}

impl EquipInspectGeneral {
    pub(crate) const LS_NAME_LEN: usize = 16;
    const UNIQUE_NO_OFFSET: usize = 0;
    const ACT_INDEX_OFFSET: usize = 4;
    const LS_ITEM_NO_OFFSET: usize = 10;
    const LS_NAME_OFFSET: usize = 12;
    const LS_COLOR_OFFSET: usize = 28;
    const JOB_OFFSET: usize = 30;
    const LVL_OFFSET: usize = 32;
    const MJOB_OFFSET: usize = 34;
    const MLVL_OFFSET: usize = 35;
    const MFLAGS_OFFSET: usize = 36;
    pub(crate) const SIZE: usize = Self::MFLAGS_OFFSET + 1;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SIZE {
            return Err(DecodeError::Truncated(Self::SIZE, body.len()));
        }
        let rd16 = |o: usize| u16::from_le_bytes([body[o], body[o + 1]]);
        Ok(Self {
            unique_no: u32::from_le_bytes(
                body[Self::UNIQUE_NO_OFFSET..Self::UNIQUE_NO_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ),
            act_index: rd16(Self::ACT_INDEX_OFFSET),
            linkshell_item_no: rd16(Self::LS_ITEM_NO_OFFSET),
            linkshell_name_raw: body
                [Self::LS_NAME_OFFSET..Self::LS_NAME_OFFSET + Self::LS_NAME_LEN]
                .try_into()
                .unwrap(),
            linkshell_color_raw: rd16(Self::LS_COLOR_OFFSET),
            main_job: body[Self::JOB_OFFSET],
            sub_job: body[Self::JOB_OFFSET + 1],
            main_job_lv: body[Self::LVL_OFFSET],
            sub_job_lv: body[Self::LVL_OFFSET + 1],
            master_job: body[Self::MJOB_OFFSET],
            master_lv: body[Self::MLVL_OFFSET],
            master_flags: body[Self::MFLAGS_OFFSET],
        })
    }

    /// The equipped linkshell's name, or empty when the target wears no pearl
    /// (LSB leaves `sComLinkName` zeroed — 0x0c9_equip_inspect_general.cpp:36-42).
    pub fn linkshell_name(&self) -> String {
        crate::linkshell_name::decode(&self.linkshell_name_raw)
    }
}

/// One `checkitem_t`; `equip_kind` is SAVE_EQUIP_KIND, whose 0..=15 order equals
/// LSB SLOTTYPE (the EQUIPMENT constructor casts the slot loop index directly).
/// vendor/server/src/map/packets/s2c/0x0c9_equip_inspect_equipment.h:28-56.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquipInspectItem {
    pub item_no: u16,
    pub equip_kind: u8,
}

/// Mode 0x03: one batch of up to 8 equipped items.
/// vendor/server/src/map/packets/s2c/0x0c9_equip_inspect_equipment.h:65-78.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquipInspectEquipment {
    pub unique_no: u32,
    pub act_index: u16,
    pub items: Vec<EquipInspectItem>,
}

impl EquipInspectEquipment {
    pub(crate) const MAX_ITEMS: usize = 8;
    // SAVE_EQUIP_KIND_END, 0x0c9_equip_inspect_equipment.h:46
    pub const SLOT_COUNT: usize = 16;
    const UNIQUE_NO_OFFSET: usize = 0;
    const ACT_INDEX_OFFSET: usize = 4;
    const EQUIP_COUNT_OFFSET: usize = 7;
    const ITEMS_OFFSET: usize = 8;
    // sizeof checkitem_t: ItemNo u16 + EquipKind u8 + padding03 u8 + Data[24]
    pub(crate) const ITEM_STRIDE: usize = 28;
    const ITEM_NO_OFFSET: usize = 0;
    const ITEM_EQUIP_KIND_OFFSET: usize = 2;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::ITEMS_OFFSET {
            return Err(DecodeError::Truncated(Self::ITEMS_OFFSET, body.len()));
        }
        let count = (body[Self::EQUIP_COUNT_OFFSET] as usize).min(Self::MAX_ITEMS);
        let need = Self::ITEMS_OFFSET + count * Self::ITEM_STRIDE;
        if body.len() < need {
            return Err(DecodeError::Truncated(need, body.len()));
        }
        let items = (0..count)
            .map(|i| {
                let base = Self::ITEMS_OFFSET + i * Self::ITEM_STRIDE;
                EquipInspectItem {
                    item_no: u16::from_le_bytes([
                        body[base + Self::ITEM_NO_OFFSET],
                        body[base + Self::ITEM_NO_OFFSET + 1],
                    ]),
                    equip_kind: body[base + Self::ITEM_EQUIP_KIND_OFFSET],
                }
            })
            .collect();
        Ok(Self {
            unique_no: u32::from_le_bytes(
                body[Self::UNIQUE_NO_OFFSET..Self::UNIQUE_NO_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            ),
            act_index: u16::from_le_bytes([
                body[Self::ACT_INDEX_OFFSET],
                body[Self::ACT_INDEX_OFFSET + 1],
            ]),
            items,
        })
    }
}

#[cfg(test)]
mod equip_inspect_tests {
    use super::*;

    // GENERAL body is sizeof(PacketData) = 80 bytes
    // (0x0c9_equip_inspect_general.h:38-60; no setSize override).
    fn equip_inspect_general_body() -> Vec<u8> {
        let mut buf = vec![0u8; 80];
        buf[0..4].copy_from_slice(&0x0104_00D2u32.to_le_bytes()); // UniqNo
        buf[4..6].copy_from_slice(&0x01A2u16.to_le_bytes()); // ActIndex
        buf[6] = 0x01; // OptionFlag = GENERAL
        buf[10..12].copy_from_slice(&513u16.to_le_bytes()); // linkshell ItemNo
        buf[12..28].copy_from_slice(&crate::linkshell_name::encode("Kuluu")); // sComLinkName
        buf[28..30].copy_from_slice(&0xABCDu16.to_le_bytes()); // sComColor
        buf[30] = 1; // job[0] = WAR
        buf[31] = 13; // job[1] = NIN
        buf[32] = 75; // lvl[0]
        buf[33] = 37; // lvl[1]
        buf[34] = 1; // mjob
        buf[35] = 20; // mlvl
        buf[36] = 0x01; // mflags = Leveling
        buf
    }

    #[test]
    fn equip_inspect_general_reads_job_and_linkshell_offsets() {
        let buf = equip_inspect_general_body();
        let EquipInspect::General(g) = EquipInspect::decode(&buf).expect("decode") else {
            panic!("OptionFlag 0x01 must decode as GENERAL");
        };
        assert_eq!(g.unique_no, 0x0104_00D2);
        assert_eq!(g.act_index, 0x01A2);
        assert_eq!(g.linkshell_item_no, 513);
        assert_eq!(g.linkshell_name(), "Kuluu");
        assert_eq!(g.linkshell_color_raw, 0xABCD);
        assert_eq!(g.main_job, 1);
        assert_eq!(g.sub_job, 13);
        assert_eq!(g.main_job_lv, 75);
        assert_eq!(g.sub_job_lv, 37);
        assert_eq!(g.master_job, 1);
        assert_eq!(g.master_lv, 20);
        assert_eq!(g.master_flags, 0x01);
    }

    #[test]
    fn equip_inspect_equipment_reads_checkitem_batch() {
        // Final batch sized 8 + 28*count (0x0c9_equip_inspect_equipment.cpp:100).
        let items: [(u16, u8); 3] = [(17440, 0), (12511, 4), (13465, 9)];
        let mut buf = vec![0u8; 8 + EquipInspectEquipment::ITEM_STRIDE * items.len()];
        buf[0..4].copy_from_slice(&0x0104_00D2u32.to_le_bytes());
        buf[4..6].copy_from_slice(&0x01A2u16.to_le_bytes());
        buf[6] = 0x03; // OptionFlag = EQUIPMENT
        buf[7] = items.len() as u8; // EquipCount
        for (i, (item_no, kind)) in items.iter().enumerate() {
            let base = 8 + i * EquipInspectEquipment::ITEM_STRIDE;
            buf[base..base + 2].copy_from_slice(&item_no.to_le_bytes());
            buf[base + 2] = *kind;
        }
        let EquipInspect::Equipment(eq) = EquipInspect::decode(&buf).expect("decode") else {
            panic!("OptionFlag 0x03 must decode as EQUIPMENT");
        };
        assert_eq!(eq.unique_no, 0x0104_00D2);
        assert_eq!(eq.act_index, 0x01A2);
        assert_eq!(
            eq.items,
            items
                .iter()
                .map(|&(item_no, equip_kind)| EquipInspectItem {
                    item_no,
                    equip_kind
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn equip_inspect_equipment_full_batch_is_232_bytes() {
        // 8 + sizeof(checkitem_t)*8 (0x0c9_equip_inspect_equipment.cpp:89).
        let mut buf = vec![0u8; 232];
        buf[6] = 0x03;
        buf[7] = EquipInspectEquipment::MAX_ITEMS as u8;
        for i in 0..EquipInspectEquipment::MAX_ITEMS {
            let base = 8 + i * EquipInspectEquipment::ITEM_STRIDE;
            buf[base..base + 2].copy_from_slice(&(100 + i as u16).to_le_bytes());
            buf[base + 2] = i as u8;
        }
        let EquipInspect::Equipment(eq) = EquipInspect::decode(&buf).expect("decode") else {
            panic!("EQUIPMENT expected");
        };
        assert_eq!(eq.items.len(), 8);
        assert_eq!(eq.items[7].item_no, 107);
        assert_eq!(eq.items[7].equip_kind, 7);
    }

    #[test]
    fn equip_inspect_empty_final_batch_decodes_zero_items() {
        // EquipCount can be 0 in the final packet; size stays >= 8 + one stride
        // (std::max<uint8>(count, 1), 0x0c9_equip_inspect_equipment.cpp:100).
        let mut buf = vec![0u8; 8 + EquipInspectEquipment::ITEM_STRIDE];
        buf[6] = 0x03;
        let EquipInspect::Equipment(eq) = EquipInspect::decode(&buf).expect("decode") else {
            panic!("EQUIPMENT expected");
        };
        assert!(eq.items.is_empty());
    }

    #[test]
    fn equip_inspect_mode_is_the_option_flag_byte() {
        let mut buf = equip_inspect_general_body();
        assert!(matches!(
            EquipInspect::decode(&buf),
            Ok(EquipInspect::General(_))
        ));
        buf[6] = 0x03;
        buf[7] = 0;
        assert!(matches!(
            EquipInspect::decode(&buf),
            Ok(EquipInspect::Equipment(_))
        ));
        buf[6] = 0x02;
        assert!(matches!(
            EquipInspect::decode(&buf),
            Err(DecodeError::UnknownDiscriminant(0x02))
        ));
    }

    #[test]
    fn equip_inspect_truncated_bodies_error() {
        assert!(matches!(
            EquipInspect::decode(&[0u8; 6]),
            Err(DecodeError::Truncated(7, 6))
        ));
        let mut short_general = vec![0u8; EquipInspectGeneral::SIZE - 1];
        short_general[6] = 0x01;
        assert!(matches!(
            EquipInspect::decode(&short_general),
            Err(DecodeError::Truncated(n, _)) if n == EquipInspectGeneral::SIZE
        ));
        let mut short_equipment = vec![0u8; 8 + EquipInspectEquipment::ITEM_STRIDE - 1];
        short_equipment[6] = 0x03;
        short_equipment[7] = 1;
        assert!(matches!(
            EquipInspect::decode(&short_equipment),
            Err(DecodeError::Truncated(n, _)) if n == 8 + EquipInspectEquipment::ITEM_STRIDE
        ));
    }
}
