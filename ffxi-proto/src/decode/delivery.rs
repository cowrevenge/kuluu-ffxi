use super::*;

/// GP_POST_BOX_STATE item payload of the full-form s2c 0x04B
/// (vendor/server/src/map/packets/s2c/0x04b_pbx_result.h:57-67). `counterpart`
/// is the GC_PBOX name field: sender (Incoming box) or recipient (Outgoing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbxBoxState {
    pub stat: u32,
    pub counterpart: Option<String>,
    pub item_sub_no: i32,
    pub item_no: u16,
    pub kind: i32,
    pub stack: u32,
    pub extra: [u8; 28],
}

/// GP_SERV_COMMAND_PBX_RESULT (vendor/server/src/map/packets/s2c/
/// 0x04b_pbx_result.h:71-94). `state` is present only in the full 0x58 form;
/// the short 0x14 form carries just the header fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbxResult {
    pub command: u8,
    pub box_no: i8,
    pub post_work_no: i8,
    pub item_work_no: i8,
    pub item_stacks: i32,
    pub result: u8,
    pub res_param1: i8,
    pub res_param2: i8,
    pub res_param3: i8,
    pub state: Option<PbxBoxState>,
}

impl PbxResult {
    /// setSize(0x14) minus the 4-byte subpacket header (0x04b_pbx_result.cpp:31).
    pub const SHORT_SIZE: usize = 16;
    /// setSize(0x58) minus the header (0x04b_pbx_result.cpp:67).
    pub const FULL_SIZE: usize = 84;

    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() < Self::SHORT_SIZE {
            return Err(DecodeError::Truncated(Self::SHORT_SIZE, body.len()));
        }
        let state = (body.len() >= Self::FULL_SIZE).then(|| {
            let mut extra = [0u8; 28];
            extra.copy_from_slice(&body[56..84]);
            PbxBoxState {
                stat: u32::from_le_bytes(body[12..16].try_into().unwrap()),
                // Not read_name_slot: its 3-char minimum would drop short
                // auction-house senders ("AH…"), which retail special-cases.
                counterpart: {
                    let raw = &body[16..32];
                    let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                    (n > 0).then(|| String::from_utf8_lossy(&raw[..n]).into_owned())
                },
                item_sub_no: i32::from_le_bytes(body[40..44].try_into().unwrap()),
                item_no: u16::from_le_bytes(body[44..46].try_into().unwrap()),
                kind: i32::from_le_bytes(body[48..52].try_into().unwrap()),
                stack: u32::from_le_bytes(body[52..56].try_into().unwrap()),
                extra,
            }
        });
        Ok(Self {
            command: body[0],
            box_no: body[1] as i8,
            post_work_no: body[2] as i8,
            item_work_no: body[3] as i8,
            item_stacks: i32::from_le_bytes(body[4..8].try_into().unwrap()),
            result: body[8],
            res_param1: body[9] as i8,
            res_param2: body[10] as i8,
            res_param3: body[11] as i8,
            state,
        })
    }
}

#[cfg(test)]
mod pbx_result_tests {
    use super::*;

    /// Pins the GP_SERV_COMMAND_PBX_RESULT layout to LSB's PacketData struct
    /// (vendor/server/src/map/packets/s2c/0x04b_pbx_result.h): header fields at
    /// 0-11, GP_POST_BOX_STATE at 12 (Stat, name[16], request id/time, sub id,
    /// ItemNo, Kind, Stack, Data[28]) in the 0x58 full form.
    #[test]
    fn pbx_result_reads_lsb_offsets_full_form() {
        let mut buf = vec![0u8; PbxResult::FULL_SIZE];
        buf[0] = 0x06; // Command = Recv
        buf[1] = 1; // BoxNo = Incoming
        buf[2] = 3; // PostWorkNo
        buf[3] = 0xFF; // ItemWorkNo = -1
        buf[4..8].copy_from_slice(&(-1i32).to_le_bytes()); // ItemStacks
        buf[8] = 0x01; // Result = OK
        buf[9] = 1; // ResParam1 = count
        buf[10] = 0xFF; // ResParam2 = -1
        buf[11] = 0xFF; // ResParam3 = -1
        buf[12..16].copy_from_slice(&7u32.to_le_bytes()); // Stat = incoming
        buf[16..20].copy_from_slice(b"Atti"); // From (NUL-padded)
        buf[40..44].copy_from_slice(&0i32.to_le_bytes()); // sub id
        buf[44..46].copy_from_slice(&5075u16.to_le_bytes()); // ItemNo
        buf[52..56].copy_from_slice(&2u32.to_le_bytes()); // Stack
        buf[56] = 0xAB; // Data[0] (m_extra)

        let r = PbxResult::decode(&buf).expect("decodes");
        assert_eq!(r.command, 0x06);
        assert_eq!(r.box_no, 1);
        assert_eq!(r.post_work_no, 3);
        assert_eq!(r.item_work_no, -1);
        assert_eq!(r.item_stacks, -1);
        assert_eq!(r.result, 0x01);
        assert_eq!(r.res_param1, 1);
        assert_eq!(r.res_param2, -1);
        let s = r.state.expect("full form carries GP_POST_BOX_STATE");
        assert_eq!(s.stat, 7);
        assert_eq!(s.counterpart.as_deref(), Some("Atti"));
        assert_eq!(s.item_no, 5075);
        assert_eq!(s.stack, 2);
        assert_eq!(s.extra[0], 0xAB);
    }

    /// The short 0x14 form (4-arg LSB ctor) has no box state; a Check response
    /// carries the new-item count in ResParam2 (Incoming) / ResParam3 (Outgoing)
    /// (0x04b_pbx_result.cpp:44-54).
    #[test]
    fn pbx_result_short_form_check_counts() {
        let mut buf = vec![0u8; PbxResult::SHORT_SIZE];
        buf[0] = 0x05; // Check
        buf[1] = 1; // Incoming
        buf[2] = 0xFF;
        buf[3] = 0xFF;
        buf[4..8].copy_from_slice(&(-1i32).to_le_bytes());
        buf[8] = 0x01; // Result = OK
        buf[9] = 0xFF;
        buf[10] = 2; // ResParam2 = 2 new items
        buf[11] = 0xFF;

        let r = PbxResult::decode(&buf).expect("decodes");
        assert_eq!(r.command, 0x05);
        assert_eq!(r.res_param2, 2);
        assert!(r.state.is_none(), "short form has no state");

        assert!(PbxResult::decode(&buf[..12]).is_err(), "truncated rejects");
    }
}
