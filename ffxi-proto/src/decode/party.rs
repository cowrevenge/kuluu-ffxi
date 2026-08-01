use super::*;

#[derive(Debug, Clone)]
pub struct PartyAttrs {
    pub unique_no: u32,
    pub act_index: u16,
    pub hp: u32,
    pub mp: u32,
    pub tp: u32,
    pub hpp: u8,
    pub mpp: u8,
    pub kind: u8,

    pub moghouse_flg: u8,
    pub zone_no: u16,
    pub mjob_no: u8,
    pub mjob_lv: u8,
    pub sjob_no: u8,
    pub sjob_lv: u8,
}

#[derive(Debug, Clone)]
pub struct PartyListExtra {
    pub member_number: u8,
    pub is_party_leader: bool,
    pub is_alliance_leader: bool,

    pub name: Option<String>,
}

impl PartyAttrs {
    pub fn decode_group_attr(body: &[u8]) -> Result<Self, DecodeError> {
        const NEEDED: usize = 32;
        if body.len() < NEEDED {
            return Err(DecodeError::Truncated(NEEDED, body.len()));
        }
        Ok(Self {
            unique_no: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            hp: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            mp: u32::from_le_bytes(body[8..12].try_into().unwrap()),
            tp: u32::from_le_bytes(body[12..16].try_into().unwrap()),
            act_index: u16::from_le_bytes(body[16..18].try_into().unwrap()),
            hpp: body[18],
            mpp: body[19],
            kind: body[20],
            moghouse_flg: body[21],
            zone_no: u16::from_le_bytes(body[22..24].try_into().unwrap()),
            mjob_no: body[28],
            mjob_lv: body[29],
            sjob_no: body[30],
            sjob_lv: body[31],
        })
    }

    pub fn decode_group_list(body: &[u8]) -> Result<(Self, PartyListExtra), DecodeError> {
        const NEEDED: usize = 52;
        if body.len() < NEEDED {
            return Err(DecodeError::Truncated(NEEDED, body.len()));
        }
        let attrs = Self {
            unique_no: u32::from_le_bytes(body[0..4].try_into().unwrap()),
            hp: u32::from_le_bytes(body[4..8].try_into().unwrap()),
            mp: u32::from_le_bytes(body[8..12].try_into().unwrap()),
            tp: u32::from_le_bytes(body[12..16].try_into().unwrap()),
            act_index: u16::from_le_bytes(body[20..22].try_into().unwrap()),
            kind: body[24],
            hpp: body[25],
            mpp: body[26],
            moghouse_flg: body[23],
            zone_no: u16::from_le_bytes(body[28..30].try_into().unwrap()),
            mjob_no: body[30],
            mjob_lv: body[31],
            sjob_no: body[32],
            sjob_lv: body[33],
        };
        let gattr = u32::from_le_bytes(body[16..20].try_into().unwrap());

        let is_party_leader = (gattr >> 2) & 1 == 1;
        let is_alliance_leader = (gattr >> 3) & 1 == 1;
        let name_bytes = &body[36..52];
        let n = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        let name = if n > 0 && name_bytes[..n].iter().all(|&b| (0x20..=0x7E).contains(&b)) {
            Some(String::from_utf8_lossy(&name_bytes[..n]).into_owned())
        } else {
            None
        };
        let extra = PartyListExtra {
            member_number: body[22],
            is_party_leader,
            is_alliance_leader,
            name,
        };
        Ok((attrs, extra))
    }
}
