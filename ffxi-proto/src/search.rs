//! FFXI search-server TCP protocol: frame crypto plus the Auction House
//! list/history request and response layouts, per LSB
//! `vendor/server/src/search/search_handler.{h,cpp}` and
//! `vendor/server/src/search/packets/auction_{list,history}.{h,cpp}`.

use crate::{blowfish, md5};

include!(concat!(env!("OUT_DIR"), "/search_handler_table.rs"));

pub const FRAME_LEN_OFFSET: usize = 0x00;
pub const MAGIC_OFFSET: usize = 0x04;
// vendor/server/src/search/search_handler.cpp:162 — "IXFF"
pub const MAGIC_IXFF: u32 = 0x4646_5849;
pub const REQUEST_TYPE_OFFSET: usize = 0x0B;

pub const FRAME_HEADER_SIZE: usize = 8;
pub const INTEGRITY_HASH_SIZE: usize = 16;
pub const TRAILER_KEY_SIZE: usize = 4;
// vendor/server/src/search/search_handler.cpp:235 — `length < 28`
pub const MIN_FRAME_LEN: usize = 28;

// vendor/server/src/search/search_handler.cpp:198 — hash at `length - 0x14`
pub const HASH_FROM_END: usize = 0x14;
// vendor/server/src/search/search_handler.cpp:154 — response-key word at `length - 0x18`
pub const SERVER_KEY_FROM_END: usize = 0x18;

// vendor/server/src/search/search_handler.cpp:139/154/164 — key splice offsets
pub const CLIENT_KEY_SPLICE: usize = 16;
pub const SERVER_KEY_SPLICE: usize = 20;
pub const REQUEST_KEY_HASH_LEN: usize = 20;
pub const FULL_KEY_LEN: usize = 24;

// vendor/server/src/search/search_handler.cpp:146/170 — `(length - 12) / 4` words,
// rounded down to an even count, enciphered from word offset 2
const CIPHER_SKIP: usize = FRAME_HEADER_SIZE + TRAILER_KEY_SIZE;

// vendor/server/src/search/search_handler.cpp:456-470
pub const AH_LIST_SORT_COUNT_OFFSET: usize = 0x12;
pub const AH_LIST_CATEGORY_OFFSET: usize = 0x16;
pub const AH_LIST_PARAMS_OFFSET: usize = 0x18;
pub const AH_LIST_PARAM_STRIDE: usize = 8;

// vendor/server/src/search/search_handler.cpp:472-486 — the only sort ids the
// server acts on (3=race, 4=job, 7=defense, 8=resistance are listed but ignored)
pub const SORT_LEVEL_DESC: u32 = 2;
pub const SORT_DAMAGE_DESC: u32 = 5;
pub const SORT_DELAY_DESC: u32 = 6;
pub const SORT_NAME: u32 = 9;

// vendor/server/src/search/search_handler.cpp:518-519
pub const AH_HISTORY_ITEM_ID_OFFSET: usize = 0x12;
pub const AH_HISTORY_STACK_OFFSET: usize = 0x15;
const AH_HISTORY_REQUEST_PAYLOAD_END: usize = 0x18;

pub const RESPONSE_BODY_SIZE_OFFSET: usize = 0x08;
pub const RESPONSE_FLAGS_OFFSET: usize = 0x0A;
// vendor/server/src/search/packets/auction_list.cpp:74 — final-packet marker
pub const RESPONSE_FINAL_FLAG: u8 = 0x80;
pub const RESPONSE_TYPE_OFFSET: usize = 0x0B;

// vendor/server/src/search/packets/auction_list.cpp:42
pub const AH_LIST_RESPONSE_TYPE: u8 = 0x95;
pub const AH_LIST_TOTAL_COUNT_OFFSET: usize = 0x0E;
pub const AH_LIST_ITEMS_OFFSET: usize = 0x18;
pub const AH_LIST_ITEM_SIZE: usize = 0x0A;
pub const AH_LIST_ITEMS_PER_PACKET: usize = 20;

// vendor/server/src/search/packets/auction_history.cpp:30-55
pub const AH_HISTORY_RESPONSE_TYPE: u8 = 0x85;
pub const AH_HISTORY_ITEM_OFFSET: usize = 0x18;
pub const AH_HISTORY_OPEN_LISTINGS_OFFSET: usize = 0x1A;
pub const AH_HISTORY_CATEGORY_OFFSET: usize = 0x1E;
pub const AH_HISTORY_ROWS_OFFSET: usize = 0x20;
pub const AH_HISTORY_ROW_SIZE: usize = 40;
pub const AH_HISTORY_ROW_SELLER_OFFSET: usize = 0x08;
pub const AH_HISTORY_ROW_BUYER_OFFSET: usize = 0x18;
pub const AH_HISTORY_NAME_FIELD_SIZE: usize = 16;
pub const AH_HISTORY_MAX_ROWS: usize = 10;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SearchError {
    #[error("frame is {0} bytes, minimum {MIN_FRAME_LEN}")]
    TooShort(usize),
    #[error("length field {field} does not match frame size {actual}")]
    LengthMismatch { field: u16, actual: usize },
    #[error("integrity hash mismatch")]
    BadHash,
    #[error("magic {0:#010x} is not IXFF")]
    BadMagic(u32),
    #[error("trailer key {got:#010x} does not echo client key {want:#010x}")]
    KeyEchoMismatch { got: u32, want: u32 },
    #[error("response type {got:#04x}, expected {want:#04x}")]
    UnexpectedType { got: u8, want: u8 },
    #[error("body size field {field} is inconsistent with a {actual}-byte frame")]
    BadBodySize { field: u16, actual: usize },
}

fn rd_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes(buf[off..off + 2].try_into().unwrap())
}

fn rd_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(buf[off..off + 4].try_into().unwrap())
}

fn cipher_words(frame: &mut [u8], state: &blowfish::State, decrypt: bool) {
    let mut words = (frame.len() - CIPHER_SKIP) / 4;
    words -= words % 2;
    for w in (0..words).step_by(2) {
        let off = FRAME_HEADER_SIZE + w * 4;
        let mut xl = rd_u32(frame, off);
        let mut xr = rd_u32(frame, off + 4);
        if decrypt {
            blowfish::decipher(&mut xl, &mut xr, &state.p, &state.s);
        } else {
            blowfish::encipher(&mut xl, &mut xr, &state.p, &state.s);
        }
        frame[off..off + 4].copy_from_slice(&xl.to_le_bytes());
        frame[off + 4..off + 8].copy_from_slice(&xr.to_le_bytes());
    }
}

/// Per-connection key state. Mirrors the server's `SearchHandler::key` evolution
/// (vendor/server/src/search/search_handler.cpp:139/154): each request splices the
/// client trailer key at [16..20) and the request's `len-0x18` word at [20..24),
/// and the response is enciphered under MD5 of the full 24 bytes.
pub struct SearchCrypto {
    key: [u8; FULL_KEY_LEN],
}

impl Default for SearchCrypto {
    fn default() -> Self {
        Self {
            key: SEARCH_BASE_KEY,
        }
    }
}

impl SearchCrypto {
    pub fn new() -> Self {
        Self::default()
    }

    fn encode_request(
        &mut self,
        len: usize,
        request_type: u8,
        client_key: u32,
        write_fields: impl FnOnce(&mut [u8]),
    ) -> Vec<u8> {
        debug_assert!(len >= MIN_FRAME_LEN);
        let mut frame = vec![0u8; len];
        frame[FRAME_LEN_OFFSET..FRAME_LEN_OFFSET + 2].copy_from_slice(&(len as u16).to_le_bytes());
        frame[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(&MAGIC_IXFF.to_le_bytes());
        frame[REQUEST_TYPE_OFFSET] = request_type;
        write_fields(&mut frame);
        // LSB only requires the plaintext `len-0x18` word to agree with what we
        // splice at key[20..24) — reuse the client key as that entropy word.
        let seed = client_key;
        frame[len - SERVER_KEY_FROM_END..len - SERVER_KEY_FROM_END + 4]
            .copy_from_slice(&seed.to_le_bytes());
        let hash = md5::md5(&frame[FRAME_HEADER_SIZE..len - HASH_FROM_END]);
        frame[len - HASH_FROM_END..len - HASH_FROM_END + INTEGRITY_HASH_SIZE]
            .copy_from_slice(&hash);

        self.key[CLIENT_KEY_SPLICE..CLIENT_KEY_SPLICE + 4]
            .copy_from_slice(&client_key.to_le_bytes());
        self.key[SERVER_KEY_SPLICE..SERVER_KEY_SPLICE + 4].copy_from_slice(&seed.to_le_bytes());

        let bf_key = md5::md5(&self.key[..REQUEST_KEY_HASH_LEN]);
        let state = blowfish::State::new(&bf_key);
        cipher_words(&mut frame, &state, false);
        frame[len - TRAILER_KEY_SIZE..].copy_from_slice(&client_key.to_le_bytes());
        frame
    }

    pub fn encode_ah_list_request(
        &mut self,
        category: u8,
        sorts: &[u32],
        client_key: u32,
    ) -> Vec<u8> {
        let payload_end = AH_LIST_PARAMS_OFFSET + AH_LIST_PARAM_STRIDE * sorts.len();
        let len = payload_end + SERVER_KEY_FROM_END;
        self.encode_request(len, TCP_AH_REQUEST, client_key, |frame| {
            frame[AH_LIST_SORT_COUNT_OFFSET] = sorts.len() as u8;
            frame[AH_LIST_CATEGORY_OFFSET] = category;
            for (i, sort) in sorts.iter().enumerate() {
                let off = AH_LIST_PARAMS_OFFSET + AH_LIST_PARAM_STRIDE * i;
                frame[off..off + 4].copy_from_slice(&sort.to_le_bytes());
            }
        })
    }

    pub fn encode_ah_history_request(
        &mut self,
        item_id: u16,
        stack: bool,
        client_key: u32,
    ) -> Vec<u8> {
        let request_type = if stack {
            TCP_AH_HISTORY_STACK
        } else {
            TCP_AH_HISTORY_SINGLE
        };
        let len = AH_HISTORY_REQUEST_PAYLOAD_END + SERVER_KEY_FROM_END;
        self.encode_request(len, request_type, client_key, |frame| {
            frame[AH_HISTORY_ITEM_ID_OFFSET..AH_HISTORY_ITEM_ID_OFFSET + 2]
                .copy_from_slice(&item_id.to_le_bytes());
            frame[AH_HISTORY_STACK_OFFSET] = stack as u8;
        })
    }

    pub fn decrypt_response(&self, frame: &[u8]) -> Result<Vec<u8>, SearchError> {
        let len = frame.len();
        if len < MIN_FRAME_LEN {
            return Err(SearchError::TooShort(len));
        }
        let field = rd_u16(frame, FRAME_LEN_OFFSET);
        if field as usize != len {
            return Err(SearchError::LengthMismatch { field, actual: len });
        }

        let mut body = frame.to_vec();
        let bf_key = md5::md5(&self.key);
        let state = blowfish::State::new(&bf_key);
        cipher_words(&mut body, &state, true);

        let hash = md5::md5(&body[FRAME_HEADER_SIZE..len - HASH_FROM_END]);
        if hash != body[len - HASH_FROM_END..len - HASH_FROM_END + INTEGRITY_HASH_SIZE] {
            return Err(SearchError::BadHash);
        }
        let magic = rd_u32(&body, MAGIC_OFFSET);
        if magic != MAGIC_IXFF {
            return Err(SearchError::BadMagic(magic));
        }
        let echoed = rd_u32(&body, len - TRAILER_KEY_SIZE);
        let client_key = rd_u32(&self.key, CLIENT_KEY_SPLICE);
        if echoed != client_key {
            return Err(SearchError::KeyEchoMismatch {
                got: echoed,
                want: client_key,
            });
        }
        Ok(body)
    }
}

/// The item ctor overwrites StackAmount for `stackSize == 1` items
/// (vendor/server/src/search/data_loader.cpp GetAHItemsToCategory).
pub const AH_NOT_STACKABLE: u32 = u32::MAX;

/// Open-listing COUNTS, not prices — the retail catalog's bracketed `[N]`
/// stock numbers (data_loader.cpp GetAHItemsToCategory:
/// SingleAmount = COUNT(*)-SUM(stack), StackAmount = SUM(stack) over
/// unsold rows). Prices appear only in sale history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhListing {
    pub item_id: u16,
    /// Singles currently up for sale; 0 = none listed
    pub singles_for_sale: u32,
    /// Stacks currently up for sale; `None` = item is not stackable
    pub stacks_for_sale: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhListPage {
    pub total_count: u16,
    pub listings: Vec<AhListing>,
    pub is_final: bool,
}

pub fn parse_ah_list(body: &[u8]) -> Result<AhListPage, SearchError> {
    if body.len() < AH_LIST_ITEMS_OFFSET {
        return Err(SearchError::TooShort(body.len()));
    }
    let got = body[RESPONSE_TYPE_OFFSET];
    if got != AH_LIST_RESPONSE_TYPE {
        return Err(SearchError::UnexpectedType {
            got,
            want: AH_LIST_RESPONSE_TYPE,
        });
    }
    let body_size = rd_u16(body, RESPONSE_BODY_SIZE_OFFSET);
    let items_bytes = (body_size as usize)
        .checked_sub(AH_LIST_ITEMS_OFFSET)
        .ok_or(SearchError::BadBodySize {
            field: body_size,
            actual: body.len(),
        })?;
    let count = items_bytes / AH_LIST_ITEM_SIZE;
    if !items_bytes.is_multiple_of(AH_LIST_ITEM_SIZE)
        || AH_LIST_ITEMS_OFFSET + count * AH_LIST_ITEM_SIZE > body.len()
    {
        return Err(SearchError::BadBodySize {
            field: body_size,
            actual: body.len(),
        });
    }
    let listings = (0..count)
        .map(|i| {
            let off = AH_LIST_ITEMS_OFFSET + AH_LIST_ITEM_SIZE * i;
            let stacks = rd_u32(body, off + 6);
            AhListing {
                item_id: rd_u16(body, off),
                singles_for_sale: rd_u32(body, off + 2),
                stacks_for_sale: (stacks != AH_NOT_STACKABLE).then_some(stacks),
            }
        })
        .collect();
    Ok(AhListPage {
        total_count: rd_u16(body, AH_LIST_TOTAL_COUNT_OFFSET),
        listings,
        is_final: body[RESPONSE_FLAGS_OFFSET] & RESPONSE_FINAL_FLAG != 0,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhSale {
    pub price: u32,
    /// Unix timestamp (vendor/server/src/search/data_loader.cpp `sell_date` → `ahHistory::Data`)
    pub sell_date: u32,
    pub seller: String,
    pub buyer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhHistory {
    pub item_id: u16,
    /// Count of open listings of the requested form (single vs stack), not a
    /// price (auction_history.cpp ctor takes GetAHItemFromItemID's
    /// Single/StackAmount; the not-stackable override does not apply here)
    pub open_listings: u32,
    pub category: u16,
    pub sales: Vec<AhSale>,
}

fn name_field(body: &[u8], off: usize) -> String {
    let raw = &body[off..off + AH_HISTORY_NAME_FIELD_SIZE];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

pub fn parse_ah_history(body: &[u8]) -> Result<AhHistory, SearchError> {
    if body.len() < AH_HISTORY_ROWS_OFFSET {
        return Err(SearchError::TooShort(body.len()));
    }
    let got = body[RESPONSE_TYPE_OFFSET];
    if got != AH_HISTORY_RESPONSE_TYPE {
        return Err(SearchError::UnexpectedType {
            got,
            want: AH_HISTORY_RESPONSE_TYPE,
        });
    }
    let body_size = rd_u16(body, RESPONSE_BODY_SIZE_OFFSET);
    // vendor/server/src/search/packets/auction_history.cpp:55 — the size field is
    // only written once a row is added; a zero-sale response leaves it 0.
    let rows_bytes = (body_size as usize).saturating_sub(AH_HISTORY_ROWS_OFFSET);
    let count = rows_bytes / AH_HISTORY_ROW_SIZE;
    if !rows_bytes.is_multiple_of(AH_HISTORY_ROW_SIZE)
        || count > AH_HISTORY_MAX_ROWS
        || AH_HISTORY_ROWS_OFFSET + count * AH_HISTORY_ROW_SIZE > body.len()
    {
        return Err(SearchError::BadBodySize {
            field: body_size,
            actual: body.len(),
        });
    }
    let sales = (0..count)
        .map(|i| {
            let off = AH_HISTORY_ROWS_OFFSET + AH_HISTORY_ROW_SIZE * i;
            AhSale {
                price: rd_u32(body, off),
                sell_date: rd_u32(body, off + 4),
                seller: name_field(body, off + AH_HISTORY_ROW_SELLER_OFFSET),
                buyer: name_field(body, off + AH_HISTORY_ROW_BUYER_OFFSET),
            }
        })
        .collect();
    Ok(AhHistory {
        item_id: rd_u16(body, AH_HISTORY_ITEM_OFFSET),
        open_listings: rd_u32(body, AH_HISTORY_OPEN_LISTINGS_OFFSET),
        category: rd_u16(body, AH_HISTORY_CATEGORY_OFFSET),
        sales,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independent reimplementation of SearchHandler::decrypt/encrypt/validatePacket
    // (vendor/server/src/search/search_handler.cpp:134-206) so the client-side
    // codec is checked against the server algorithm, not against itself.
    struct LsbServer {
        key: [u8; 24],
    }

    impl LsbServer {
        fn new() -> Self {
            Self {
                key: SEARCH_BASE_KEY,
            }
        }

        fn cipher(&self, frame: &mut [u8], key_bytes: usize, decrypt: bool) {
            let hash = md5::md5(&self.key[..key_bytes]);
            let state = blowfish::State::new(&hash);
            let length = frame.len();
            let mut tmp = (length - 12) / 4;
            tmp -= tmp % 2;
            for i in (0..tmp).step_by(2) {
                let a = 4 * (i + 2);
                let b = 4 * (i + 3);
                let mut xl = u32::from_le_bytes(frame[a..a + 4].try_into().unwrap());
                let mut xr = u32::from_le_bytes(frame[b..b + 4].try_into().unwrap());
                if decrypt {
                    blowfish::decipher(&mut xl, &mut xr, &state.p, &state.s);
                } else {
                    blowfish::encipher(&mut xl, &mut xr, &state.p, &state.s);
                }
                frame[a..a + 4].copy_from_slice(&xl.to_le_bytes());
                frame[b..b + 4].copy_from_slice(&xr.to_le_bytes());
            }
        }

        fn decrypt(&mut self, frame: &mut [u8]) {
            let length = frame.len();
            let trailer: [u8; 4] = frame[length - 4..].try_into().unwrap();
            self.key[16..20].copy_from_slice(&trailer);
            self.cipher(frame, 20, true);
            let word: [u8; 4] = frame[length - 0x18..length - 0x18 + 4].try_into().unwrap();
            self.key[20..24].copy_from_slice(&word);
        }

        fn validate(&self, frame: &[u8]) -> bool {
            let length = frame.len();
            let to_hash = length - 0x08 - 0x10 - 0x04;
            let hash = md5::md5(&frame[8..8 + to_hash]);
            frame[length - 0x14..length - 0x14 + 16] == hash
        }

        fn encrypt(&self, frame: &mut [u8]) {
            let length = frame.len();
            frame[0..2].copy_from_slice(&(length as u16).to_le_bytes());
            frame[4..8].copy_from_slice(&0x46465849u32.to_le_bytes());
            let hash_off = length - 0x18 + 0x04;
            let body_hash = md5::md5(&frame[8..8 + (length - 0x18 - 0x04)]);
            frame[hash_off..hash_off + 16].copy_from_slice(&body_hash);
            self.cipher(frame, 24, false);
            let echo: [u8; 4] = self.key[16..20].try_into().unwrap();
            frame[length - 4..].copy_from_slice(&echo);
        }
    }

    const CLIENT_KEY: u32 = 0xA1B2_C3D4;

    #[test]
    fn ah_list_request_roundtrips_through_lsb_decrypt() {
        let mut client = SearchCrypto::new();
        let mut frame = client.encode_ah_list_request(7, &[SORT_LEVEL_DESC, SORT_NAME], CLIENT_KEY);
        assert_eq!(
            rd_u16(&frame, FRAME_LEN_OFFSET) as usize,
            frame.len(),
            "length field"
        );
        assert!(frame.len() >= MIN_FRAME_LEN);

        let mut server = LsbServer::new();
        server.decrypt(&mut frame);
        assert!(server.validate(&frame), "server-side MD5 validation");

        assert_eq!(frame[REQUEST_TYPE_OFFSET], TCP_AH_REQUEST);
        assert_eq!(frame[AH_LIST_SORT_COUNT_OFFSET], 2);
        assert_eq!(frame[AH_LIST_CATEGORY_OFFSET], 7);
        assert_eq!(rd_u32(&frame, AH_LIST_PARAMS_OFFSET), SORT_LEVEL_DESC);
        assert_eq!(
            rd_u32(&frame, AH_LIST_PARAMS_OFFSET + AH_LIST_PARAM_STRIDE),
            SORT_NAME
        );
        assert_eq!(rd_u32(&frame, MAGIC_OFFSET), MAGIC_IXFF);
        assert_eq!(
            server.key, client.key,
            "key evolution must stay in lockstep"
        );
    }

    #[test]
    fn ah_history_request_roundtrips_through_lsb_decrypt() {
        for (stack, want_type) in [(false, TCP_AH_HISTORY_SINGLE), (true, TCP_AH_HISTORY_STACK)] {
            let mut client = SearchCrypto::new();
            let mut frame = client.encode_ah_history_request(4096, stack, CLIENT_KEY);

            let mut server = LsbServer::new();
            server.decrypt(&mut frame);
            assert!(server.validate(&frame));

            assert_eq!(frame[REQUEST_TYPE_OFFSET], want_type);
            assert_eq!(rd_u16(&frame, AH_HISTORY_ITEM_ID_OFFSET), 4096);
            assert_eq!(frame[AH_HISTORY_STACK_OFFSET], stack as u8);
            assert_eq!(server.key, client.key);
        }
    }

    /// The `len-0x18` seed word sits inside the MD5-hashed span — the subtlest
    /// part of the framing (a hash starting after it would still roundtrip).
    #[test]
    fn seed_word_is_inside_hash_coverage() {
        let mut client = SearchCrypto::new();
        let mut frame = client.encode_ah_list_request(1, &[], CLIENT_KEY);
        let seed_off = frame.len() - SERVER_KEY_FROM_END;
        let mut server = LsbServer::new();
        server.decrypt(&mut frame);
        assert!(server.validate(&frame));
        frame[seed_off] ^= 0xFF;
        assert!(!server.validate(&frame));
    }

    #[test]
    fn tampered_request_fails_lsb_validation() {
        let mut client = SearchCrypto::new();
        let mut frame = client.encode_ah_list_request(1, &[], CLIENT_KEY);
        frame[AH_LIST_CATEGORY_OFFSET] ^= 0xFF;
        let mut server = LsbServer::new();
        server.decrypt(&mut frame);
        assert!(!server.validate(&frame));
    }

    // Response body per auction_list.cpp: total size = body-size field + 28
    // (CAHItemsListPacket::GetSize, auction_list.cpp:102).
    fn build_list_response(
        total: u16,
        offset: usize,
        items: &[(u16, u32, u32)],
        is_final: bool,
    ) -> Vec<u8> {
        assert!(items.len() <= AH_LIST_ITEMS_PER_PACKET);
        let body_size = AH_LIST_ITEMS_OFFSET + AH_LIST_ITEM_SIZE * items.len();
        let mut frame = vec![0u8; body_size + 28];
        frame[RESPONSE_TYPE_OFFSET] = AH_LIST_RESPONSE_TYPE;
        frame[AH_LIST_TOTAL_COUNT_OFFSET..AH_LIST_TOTAL_COUNT_OFFSET + 2]
            .copy_from_slice(&total.to_le_bytes());
        if is_final {
            assert_eq!(total as usize - offset, items.len());
            frame[RESPONSE_FLAGS_OFFSET] = RESPONSE_FINAL_FLAG;
        }
        frame[RESPONSE_BODY_SIZE_OFFSET..RESPONSE_BODY_SIZE_OFFSET + 2]
            .copy_from_slice(&(body_size as u16).to_le_bytes());
        for (i, &(id, single, stack)) in items.iter().enumerate() {
            let off = AH_LIST_ITEMS_OFFSET + AH_LIST_ITEM_SIZE * i;
            frame[off..off + 2].copy_from_slice(&id.to_le_bytes());
            frame[off + 2..off + 6].copy_from_slice(&single.to_le_bytes());
            frame[off + 6..off + 10].copy_from_slice(&stack.to_le_bytes());
        }
        frame
    }

    #[test]
    fn encrypted_list_response_roundtrips_and_parses() {
        let mut client = SearchCrypto::new();
        let mut request = client.encode_ah_list_request(2, &[SORT_NAME], CLIENT_KEY);
        let mut server = LsbServer::new();
        server.decrypt(&mut request);
        assert!(server.validate(&request));

        let items = [(4096u16, 120u32, 1100u32), (4097, 0, 900)];
        let mut response = build_list_response(2, 0, &items, true);
        server.encrypt(&mut response);

        let body = client.decrypt_response(&response).expect("decrypt");
        let page = parse_ah_list(&body).expect("parse");
        assert_eq!(page.total_count, 2);
        assert!(page.is_final);
        assert_eq!(
            page.listings,
            vec![
                AhListing {
                    item_id: 4096,
                    singles_for_sale: 120,
                    stacks_for_sale: Some(1100)
                },
                AhListing {
                    item_id: 4097,
                    singles_for_sale: 0,
                    stacks_for_sale: Some(900)
                },
            ]
        );
    }

    #[test]
    fn tampered_response_fails_integrity_check() {
        let mut client = SearchCrypto::new();
        let mut request = client.encode_ah_history_request(640, false, CLIENT_KEY);
        let mut server = LsbServer::new();
        server.decrypt(&mut request);

        let mut response = build_list_response(1, 0, &[(640, 10, 0)], true);
        server.encrypt(&mut response);
        response[AH_LIST_ITEMS_OFFSET] ^= 0x01;
        assert_eq!(
            client.decrypt_response(&response),
            Err(SearchError::BadHash)
        );
    }

    #[test]
    fn stale_key_state_cannot_decrypt_response() {
        let mut client = SearchCrypto::new();
        let mut request = client.encode_ah_list_request(2, &[], CLIENT_KEY);
        let mut server = LsbServer::new();
        server.decrypt(&mut request);

        let mut response = build_list_response(0, 0, &[], true);
        server.encrypt(&mut response);

        let fresh = SearchCrypto::new();
        assert!(fresh.decrypt_response(&response).is_err());
        assert!(client.decrypt_response(&response).is_ok());
    }

    #[test]
    fn multi_packet_list_final_marker() {
        let total: u16 = 25;
        let page1_items: Vec<(u16, u32, u32)> =
            (0..20).map(|i| (100 + i as u16, i * 10, i * 11)).collect();
        let page1 = build_list_response(total, 0, &page1_items, false);
        let parsed1 = parse_ah_list(&page1).unwrap();
        assert_eq!(parsed1.total_count, 25);
        assert!(!parsed1.is_final);
        assert_eq!(parsed1.listings.len(), AH_LIST_ITEMS_PER_PACKET);
        assert_eq!(
            rd_u16(&page1, RESPONSE_BODY_SIZE_OFFSET) as usize,
            AH_LIST_ITEMS_OFFSET + AH_LIST_ITEM_SIZE * AH_LIST_ITEMS_PER_PACKET
        );

        let page2_items: Vec<(u16, u32, u32)> =
            (20..25).map(|i| (100 + i as u16, i * 10, i * 11)).collect();
        let page2 = build_list_response(total, 20, &page2_items, true);
        let parsed2 = parse_ah_list(&page2).unwrap();
        assert!(parsed2.is_final);
        assert_eq!(parsed2.listings.len(), 5);
        assert_eq!(parsed2.listings[4].item_id, 124);

        let empty = build_list_response(0, 0, &[], true);
        let parsed = parse_ah_list(&empty).unwrap();
        assert_eq!(parsed.total_count, 0);
        assert!(parsed.is_final);
        assert!(parsed.listings.is_empty());
    }

    fn build_history_response(
        item_id: u16,
        price: u32,
        category: u16,
        rows: &[(u32, u32, &str, &str)],
    ) -> Vec<u8> {
        assert!(rows.len() <= AH_HISTORY_MAX_ROWS);
        let body_size = AH_HISTORY_ROWS_OFFSET + AH_HISTORY_ROW_SIZE * rows.len();
        let mut frame = vec![0u8; body_size + 28];
        frame[RESPONSE_FLAGS_OFFSET] = RESPONSE_FINAL_FLAG;
        frame[RESPONSE_TYPE_OFFSET] = AH_HISTORY_RESPONSE_TYPE;
        frame[0x10..0x12].copy_from_slice(&item_id.to_le_bytes());
        frame[AH_HISTORY_ITEM_OFFSET..AH_HISTORY_ITEM_OFFSET + 2]
            .copy_from_slice(&item_id.to_le_bytes());
        frame[AH_HISTORY_OPEN_LISTINGS_OFFSET..AH_HISTORY_OPEN_LISTINGS_OFFSET + 4]
            .copy_from_slice(&price.to_le_bytes());
        frame[AH_HISTORY_CATEGORY_OFFSET..AH_HISTORY_CATEGORY_OFFSET + 2]
            .copy_from_slice(&category.to_le_bytes());
        if !rows.is_empty() {
            frame[RESPONSE_BODY_SIZE_OFFSET..RESPONSE_BODY_SIZE_OFFSET + 2]
                .copy_from_slice(&(body_size as u16).to_le_bytes());
        }
        for (i, &(sale, date, seller, buyer)) in rows.iter().enumerate() {
            let off = AH_HISTORY_ROWS_OFFSET + AH_HISTORY_ROW_SIZE * i;
            frame[off..off + 4].copy_from_slice(&sale.to_le_bytes());
            frame[off + 4..off + 8].copy_from_slice(&date.to_le_bytes());
            frame[off + AH_HISTORY_ROW_SELLER_OFFSET..][..seller.len()]
                .copy_from_slice(seller.as_bytes());
            frame[off + AH_HISTORY_ROW_BUYER_OFFSET..][..buyer.len()]
                .copy_from_slice(buyer.as_bytes());
        }
        frame
    }

    #[test]
    fn history_response_parses() {
        let rows = [
            (1180u32, 1_754_000_000u32, "Sellera", "Buyerb"),
            (2670, 1_754_100_000, "Atti", "Verilight"),
        ];
        let frame = build_history_response(4096, 120, 12, &rows);
        let history = parse_ah_history(&frame).unwrap();
        assert_eq!(history.item_id, 4096);
        assert_eq!(history.open_listings, 120);
        assert_eq!(history.category, 12);
        assert_eq!(history.sales.len(), 2);
        assert_eq!(
            history.sales[0],
            AhSale {
                price: 1180,
                sell_date: 1_754_000_000,
                seller: "Sellera".into(),
                buyer: "Buyerb".into(),
            }
        );
        assert_eq!(history.sales[1].buyer, "Verilight");
    }

    #[test]
    fn history_response_with_no_sales_parses_empty() {
        let frame = build_history_response(4096, 0, 12, &[]);
        let history = parse_ah_history(&frame).unwrap();
        assert!(history.sales.is_empty());
        assert_eq!(history.open_listings, 0);
    }

    #[test]
    fn encrypted_history_response_roundtrips() {
        let mut client = SearchCrypto::new();
        let mut request = client.encode_ah_history_request(4096, true, CLIENT_KEY);
        let mut server = LsbServer::new();
        server.decrypt(&mut request);
        assert!(server.validate(&request));

        let mut response =
            build_history_response(4096, 1100, 12, &[(1100, 1_754_000_000, "Atti", "Buyerb")]);
        server.encrypt(&mut response);

        let body = client.decrypt_response(&response).expect("decrypt");
        let history = parse_ah_history(&body).expect("parse");
        assert_eq!(history.item_id, 4096);
        assert_eq!(history.sales.len(), 1);
        assert_eq!(history.sales[0].seller, "Atti");
    }

    #[test]
    fn scraped_key_and_request_types_have_expected_shape() {
        assert_eq!(SEARCH_BASE_KEY.len(), FULL_KEY_LEN);
        assert_eq!(&SEARCH_BASE_KEY[CLIENT_KEY_SPLICE..], &[0u8; 8]);
        assert_ne!(SEARCH_BASE_KEY[0], 0);
        assert_ne!(TCP_AH_REQUEST, TCP_AH_REQUEST_MORE);
        assert_ne!(TCP_AH_HISTORY_SINGLE, TCP_AH_HISTORY_STACK);
        // Retail's response type is the request type with the top bit set
        // (auction_list.cpp:42 vs TCP_AH_REQUEST; auction_history.cpp:34 vs
        // TCP_AH_HISTORY_SINGLE, "masked as val & 0x1F" per atom0s).
        assert_eq!(AH_LIST_RESPONSE_TYPE & 0x1F, TCP_AH_REQUEST);
        assert_eq!(AH_HISTORY_RESPONSE_TYPE & 0x1F, TCP_AH_HISTORY_SINGLE);
    }
}
