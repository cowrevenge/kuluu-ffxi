use crate::blowfish;

pub const FFXI_HEADER_SIZE: usize = 28;
pub const MD5_TRAILER_SIZE: usize = 16;

pub const SUBPACKET_OPCODE_MASK: u16 = 0x1FF;
pub const SUBPACKET_SIZE_WORDS_SHIFT: u32 = 9;
pub const SUBPACKET_SIZE_WORDS_MASK: u16 = 0x7F;
pub const SUBPACKET_WORD_BYTES: usize = 4;
pub const SUBPACKET_HEADER_SIZE: usize = 4;

pub const MIN_FRAME_SIZE: usize = FFXI_HEADER_SIZE + SUBPACKET_HEADER_SIZE + MD5_TRAILER_SIZE;

// vendor/server/src/map/packets/basic.h:106-111 setType and basic.h:113-119
// setSize: the id is masked to 9 bits, and the halved length lands in byte 1
// whose lowest bit belongs to the id -- so the length field is 7 bits wide and a
// wider value truncates there exactly as `& 0x7F` does here. basic.h:91-99
// getType/getSize and vendor/server/src/map/map_networking.cpp:419-423 are the
// matching decode.
pub const fn subpacket_header_word(opcode: u16, size_words: u16) -> u16 {
    (opcode & SUBPACKET_OPCODE_MASK)
        | ((size_words & SUBPACKET_SIZE_WORDS_MASK) << SUBPACKET_SIZE_WORDS_SHIFT)
}

// vendor/server/src/map/packets/basic.h:118 setSize rounds the byte length up to
// a 4-byte multiple before halving it. The saturation caps at the widest value
// the 7-bit header field can carry, so an oversized body reports a truncated
// length instead of a wrapped-around small one.
pub const fn subpacket_size_words(size_bytes: usize) -> u16 {
    let words = size_bytes.div_ceil(SUBPACKET_WORD_BYTES);
    if words > SUBPACKET_SIZE_WORDS_MASK as usize {
        SUBPACKET_SIZE_WORDS_MASK
    } else {
        words as u16
    }
}

pub const fn subpacket_opcode(header_word: u16) -> u16 {
    header_word & SUBPACKET_OPCODE_MASK
}

pub const fn subpacket_size_bytes(header_word: u16) -> usize {
    (header_word >> SUBPACKET_SIZE_WORDS_SHIFT) as usize * SUBPACKET_WORD_BYTES
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Header {
    pub id_and_size: u16,

    pub sync_in: u16,

    pub sync_out: u16,

    pub reserved0: u16,

    pub timestamp: u32,

    pub size_or_reserved: u32,
}

impl Header {
    pub fn read(buf: &[u8]) -> Self {
        debug_assert!(buf.len() >= FFXI_HEADER_SIZE);
        Self {
            id_and_size: u16::from_le_bytes(buf[0..2].try_into().unwrap()),
            sync_in: u16::from_le_bytes(buf[2..4].try_into().unwrap()),
            sync_out: u16::from_le_bytes(buf[4..6].try_into().unwrap()),
            reserved0: u16::from_le_bytes(buf[6..8].try_into().unwrap()),
            timestamp: u32::from_le_bytes(buf[8..12].try_into().unwrap()),
            size_or_reserved: u32::from_le_bytes(buf[12..16].try_into().unwrap()),
        }
    }

    pub fn write(&self, buf: &mut [u8]) {
        debug_assert!(buf.len() >= FFXI_HEADER_SIZE);
        buf[0..2].copy_from_slice(&self.id_and_size.to_le_bytes());
        buf[2..4].copy_from_slice(&self.sync_in.to_le_bytes());
        buf[4..6].copy_from_slice(&self.sync_out.to_le_bytes());
        buf[6..8].copy_from_slice(&self.reserved0.to_le_bytes());
        buf[8..12].copy_from_slice(&self.timestamp.to_le_bytes());
        buf[12..16].copy_from_slice(&self.size_or_reserved.to_le_bytes());

        for b in &mut buf[16..FFXI_HEADER_SIZE] {
            *b = 0;
        }
    }

    pub fn opcode(&self) -> u16 {
        subpacket_opcode(self.id_and_size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubPacket<'a> {
    pub opcode: u16,

    pub sequence: u16,

    pub data: &'a [u8],
}

pub fn walk_sub_packets(payload: &[u8]) -> SubPacketWalker<'_> {
    SubPacketWalker { rest: payload }
}

#[derive(Debug, Clone)]
pub struct SubPacketWalker<'a> {
    rest: &'a [u8],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WalkError {
    #[error("truncated sub-packet: have {have} bytes, header claims {want}")]
    Truncated { have: usize, want: usize },
    #[error("zero-length sub-packet — would loop forever")]
    ZeroSize,
}

impl<'a> Iterator for SubPacketWalker<'a> {
    type Item = Result<SubPacket<'a>, WalkError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.rest.len() < SUBPACKET_HEADER_SIZE {
            return if self.rest.is_empty() {
                None
            } else {
                Some(Err(WalkError::Truncated {
                    have: self.rest.len(),
                    want: SUBPACKET_HEADER_SIZE,
                }))
            };
        }

        let header_word = u16::from_le_bytes([self.rest[0], self.rest[1]]);
        let opcode = subpacket_opcode(header_word);
        let size_bytes = subpacket_size_bytes(header_word);
        if size_bytes == 0 {
            return Some(Err(WalkError::ZeroSize));
        }
        if size_bytes > self.rest.len() {
            return Some(Err(WalkError::Truncated {
                have: self.rest.len(),
                want: size_bytes,
            }));
        }
        let sequence = u16::from_le_bytes([self.rest[2], self.rest[3]]);
        let sub = SubPacket {
            opcode,
            sequence,
            data: &self.rest[SUBPACKET_HEADER_SIZE..size_bytes],
        };
        self.rest = &self.rest[size_bytes..];
        Some(Ok(sub))
    }
}

pub fn decrypt_in_place(frame: &mut [u8], state: &blowfish::State) {
    if frame.len() <= FFXI_HEADER_SIZE {
        return;
    }
    let payload_words = (frame.len() - FFXI_HEADER_SIZE) / 4;
    let pair_count = payload_words & !1;
    for j in (0..pair_count).step_by(2) {
        let off = FFXI_HEADER_SIZE + j * 4;
        let (l, r) = read_u32_pair(&frame[off..off + 8]);
        let mut xl = l;
        let mut xr = r;
        blowfish::decipher(&mut xl, &mut xr, &state.p, &state.s);
        write_u32_pair(&mut frame[off..off + 8], xl, xr);
    }
}

pub fn encrypt_in_place(frame: &mut [u8], state: &blowfish::State) {
    if frame.len() <= FFXI_HEADER_SIZE {
        return;
    }
    let payload_words = (frame.len() - FFXI_HEADER_SIZE) / 4;
    let pair_count = payload_words & !1;
    for j in (0..pair_count).step_by(2) {
        let off = FFXI_HEADER_SIZE + j * 4;
        let (l, r) = read_u32_pair(&frame[off..off + 8]);
        let mut xl = l;
        let mut xr = r;
        blowfish::encipher(&mut xl, &mut xr, &state.p, &state.s);
        write_u32_pair(&mut frame[off..off + 8], xl, xr);
    }
}

#[inline]
fn read_u32_pair(buf: &[u8]) -> (u32, u32) {
    (
        u32::from_le_bytes(buf[0..4].try_into().unwrap()),
        u32::from_le_bytes(buf[4..8].try_into().unwrap()),
    )
}

#[inline]
fn write_u32_pair(buf: &mut [u8], l: u32, r: u32) {
    buf[0..4].copy_from_slice(&l.to_le_bytes());
    buf[4..8].copy_from_slice(&r.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trip() {
        let h = Header {
            id_and_size: 0x012A,
            sync_in: 0x1234,
            sync_out: 0x5678,
            reserved0: 0,
            timestamp: 0xDEAD_BEEF,
            size_or_reserved: 0xCAFE_BABE,
        };
        let mut buf = [0u8; FFXI_HEADER_SIZE];
        h.write(&mut buf);
        let parsed = Header::read(&buf);
        assert_eq!(parsed, h);
        assert_eq!(parsed.opcode(), 0x12A);
    }

    #[test]
    fn datagram_header_field_offsets_match_lsb_layout() {
        // LSB preparePacket (vendor/server/src/map/map_networking.cpp:653-654) writes a
        // server->client datagram header as byte[0..2]=server seq, byte[2..4]=the server's
        // ack of the client (MapSession::client_packet_id). Pin those offsets so the
        // network-health metric's reading of `sync_in` as "server ack of us" can't drift.
        let mut buf = [0u8; FFXI_HEADER_SIZE];
        buf[0..2].copy_from_slice(&0xAABBu16.to_le_bytes());
        buf[2..4].copy_from_slice(&0xCCDDu16.to_le_bytes());
        let h = Header::read(&buf);
        assert_eq!(h.id_and_size, 0xAABB, "byte[0..2] is the server sequence");
        assert_eq!(
            h.sync_in, 0xCCDD,
            "byte[2..4] is the server's ack of the client"
        );
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = b"ffxi-test-key";
        let st = blowfish::State::new(key);

        let mut frame = vec![0u8; FFXI_HEADER_SIZE + 32 + 16];

        frame[0..2].copy_from_slice(&0x012Au16.to_le_bytes());

        for (i, b) in frame[FFXI_HEADER_SIZE..FFXI_HEADER_SIZE + 32 + 16]
            .iter_mut()
            .enumerate()
        {
            *b = (i as u8).wrapping_mul(7);
        }
        let original = frame.clone();

        encrypt_in_place(&mut frame, &st);
        assert_ne!(frame, original, "encrypt should change the buffer");

        assert_eq!(&frame[..FFXI_HEADER_SIZE], &original[..FFXI_HEADER_SIZE]);

        decrypt_in_place(&mut frame, &st);
        assert_eq!(frame, original, "round-trip should restore original");
    }

    #[test]
    fn sub_packet_walker_basic() {
        let mut payload = vec![];

        payload.extend_from_slice(&[0x15, 0x04, 0x42, 0x00, 0xAA, 0xBB, 0xCC, 0xDD]);

        payload.extend_from_slice(&[0x0A, 0x04, 0x43, 0x00, 0x11, 0x22, 0x33, 0x44]);

        let subs: Vec<_> = walk_sub_packets(&payload)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].opcode, 0x015);
        assert_eq!(subs[0].sequence, 0x0042);
        assert_eq!(subs[0].data, &[0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(subs[1].opcode, 0x00A);
        assert_eq!(subs[1].sequence, 0x0043);
        assert_eq!(subs[1].data, &[0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn sub_packet_walker_zero_size_is_error() {
        let payload = [0u8; 4];
        let mut walker = walk_sub_packets(&payload);
        let first = walker.next().unwrap();
        assert_eq!(first, Err(WalkError::ZeroSize));
    }

    #[test]
    fn sub_packet_walker_truncated() {
        let payload = [0x15, 0x08, 0x42, 0x00, 0, 0, 0, 0];
        let mut walker = walk_sub_packets(&payload);
        let first = walker.next().unwrap();
        assert!(matches!(first, Err(WalkError::Truncated { .. })));
    }

    #[test]
    fn subpacket_header_word_round_trips_through_header_opcode() {
        for (opcode, size_words) in [(0x00Au16, 19u16), (0x1FFu16, 0x7Fu16), (0x000u16, 0u16)] {
            let word = subpacket_header_word(opcode, size_words);
            assert_eq!(word & SUBPACKET_OPCODE_MASK, opcode);
            assert_eq!(word >> SUBPACKET_SIZE_WORDS_SHIFT, size_words);
            assert_eq!(subpacket_opcode(word), opcode);
            assert_eq!(
                subpacket_size_bytes(word),
                size_words as usize * SUBPACKET_WORD_BYTES
            );
            let h = Header {
                id_and_size: word,
                ..Header::default()
            };
            assert_eq!(h.opcode(), opcode);
        }
    }

    // Independent transcription of vendor/server/src/map/packets/basic.h:105-119
    // (setType/setSize) and basic.h:91-99 (getType/getSize) as the oracle for the
    // helpers above.
    fn lsb_set_type_and_size(opcode: u16, size_bytes: usize) -> [u8; 2] {
        let mut buf = [0u8; 2];
        let id = (u16::from_le_bytes(buf) & !0x1FF) | (opcode & 0x1FF);
        buf = id.to_le_bytes();
        buf[1] = (buf[1] & 1) | ((((size_bytes + 3) & !3) / 2) as u8);
        buf
    }

    fn lsb_get_type_and_size(word: [u8; 2]) -> (u16, usize) {
        let id_and_size = u16::from_le_bytes(word);
        (
            id_and_size & 0x1FF,
            (2 * (word[1] & !1) as usize).min(0x1FF),
        )
    }

    #[test]
    fn subpacket_header_word_matches_lsb_set_type_set_size() {
        for opcode in [0x000u16, 0x00A, 0x0FF, 0x100, 0x15D, 0x1FF] {
            for size_words in [0u16, 1, 2, 19, 23, 38, 0x7F] {
                let size_bytes = size_words as usize * SUBPACKET_WORD_BYTES;
                let ours = subpacket_header_word(opcode, size_words).to_le_bytes();
                assert_eq!(
                    ours,
                    lsb_set_type_and_size(opcode, size_bytes),
                    "opcode {opcode:#05X} size_words {size_words}"
                );
                assert_eq!(
                    lsb_get_type_and_size(ours),
                    (opcode, size_bytes),
                    "opcode {opcode:#05X} size_words {size_words}"
                );
            }
        }
    }

    #[test]
    fn subpacket_header_word_truncates_oversized_fields_like_lsb() {
        for (opcode, size_words) in [(0x2FFu16, 0x80u16), (0xFFFFu16, 0x81u16)] {
            let ours = subpacket_header_word(opcode, size_words).to_le_bytes();
            let size_bytes = size_words as usize * SUBPACKET_WORD_BYTES;
            assert_eq!(ours, lsb_set_type_and_size(opcode, size_bytes));
            assert_eq!(
                lsb_get_type_and_size(ours),
                (
                    opcode & SUBPACKET_OPCODE_MASK,
                    (size_words & SUBPACKET_SIZE_WORDS_MASK) as usize * SUBPACKET_WORD_BYTES
                )
            );
        }
    }

    #[test]
    fn subpacket_size_words_rounds_up_to_word_multiple() {
        for (size_bytes, want) in [
            (0usize, 0u16),
            (1, 1),
            (4, 1),
            (5, 2),
            (8, 2),
            (90, 23),
            (92, 23),
        ] {
            assert_eq!(subpacket_size_words(size_bytes), want, "{size_bytes} bytes");
            assert!(subpacket_size_words(size_bytes) as usize * SUBPACKET_WORD_BYTES >= size_bytes);
        }
    }

    #[test]
    fn subpacket_size_words_saturates_at_the_header_field_width() {
        let widest = SUBPACKET_SIZE_WORDS_MASK as usize * SUBPACKET_WORD_BYTES;
        assert_eq!(subpacket_size_words(widest), SUBPACKET_SIZE_WORDS_MASK);
        for size_bytes in [widest + 1, 0x4_0000, usize::MAX] {
            assert_eq!(
                subpacket_size_words(size_bytes),
                SUBPACKET_SIZE_WORDS_MASK,
                "{size_bytes} bytes"
            );
        }
    }

    #[test]
    fn sub_packet_walker_decodes_what_the_emitter_writes() {
        let bodies: [&[u8]; 3] = [&[], &[0xAA, 0xBB, 0xCC, 0xDD], &[0x01; 12]];
        let opcodes = [0x00Au16, 0x100, 0x1FF];
        let mut payload = Vec::new();
        for (opcode, body) in opcodes.iter().zip(bodies.iter()) {
            let size_words = subpacket_size_words(SUBPACKET_HEADER_SIZE + body.len());
            payload.extend_from_slice(&subpacket_header_word(*opcode, size_words).to_le_bytes());
            payload.extend_from_slice(&(0x4321u16).to_le_bytes());
            payload.extend_from_slice(body);
        }

        let got: Vec<SubPacket<'_>> = walk_sub_packets(&payload).map(|s| s.unwrap()).collect();
        assert_eq!(got.len(), opcodes.len());
        for (i, sub) in got.iter().enumerate() {
            assert_eq!(sub.opcode, opcodes[i]);
            assert_eq!(sub.sequence, 0x4321);
            assert_eq!(sub.data, bodies[i]);
        }
    }
}
