//! Auction House queries against the LSB search server: one TCP connection per
//! request (vendor/server/src/search/tcp_server.cpp accepts, handles, closes),
//! frames encoded/decrypted by [`ffxi_proto::search::SearchCrypto`].

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use ffxi_proto::search::{self, AhHistory, AhListing, SearchCrypto};

/// vendor/server/settings/default/network.lua SEARCH_PORT.
pub const SEARCH_PORT: u16 = 54002;

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(5);

// The server only echoes the client key back (SearchHandler::encrypt); any
// nonzero value works, and a process-wide counter keeps concurrent
// connections' key splices distinct without any time-derived entropy. The key
// is per CONNECTION, not per request: SearchHandler::key evolves in place, and
// a second request that splices a different key makes the server's response
// undecryptable (observed vs HorizonXI 2026-08-05,
// .agents/skills/retail-observe/references/auction-house.md).
static NEXT_CLIENT_KEY: AtomicU32 = AtomicU32::new(1);

fn next_client_key() -> u32 {
    loop {
        let key = NEXT_CLIENT_KEY.fetch_add(1, Ordering::Relaxed);
        if key != 0 {
            return key;
        }
    }
}

/// A full category catalog: every [`search::AhListPage`] of one TCP_AH_REQUEST
/// merged in server order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AhCatalog {
    pub total: u16,
    pub listings: Vec<AhListing>,
}

pub async fn ah_list(host: &str, category: u8, sorts: &[u32]) -> Result<AhCatalog> {
    tokio::time::timeout(SEARCH_TIMEOUT, ah_list_inner(host, category, sorts))
        .await
        .context("AH list request timed out")?
}

async fn ah_list_inner(host: &str, category: u8, sorts: &[u32]) -> Result<AhCatalog> {
    let mut stream = connect(host).await?;
    drain_ah_list(&mut stream, category, sorts, next_client_key()).await
}

/// Drive one catalog request to its final page over an already-connected
/// search socket.
async fn drain_ah_list<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    category: u8,
    sorts: &[u32],
    client_key: u32,
) -> Result<AhCatalog> {
    let mut crypto = SearchCrypto::new();
    stream
        .write_all(&crypto.encode_ah_list_request(category, sorts, client_key))
        .await
        .context("sending AH list request")?;

    let mut listings = Vec::new();
    let total = loop {
        let frame = read_frame(stream).await?;
        let body = crypto.decrypt_response(&frame)?;
        let page = search::parse_ah_list(&body)?;
        listings.extend(page.listings);
        if page.is_final {
            break page.total_count;
        }
        // Retail pulls the catalog a page at a time. LSB queues every page up
        // front (search_handler.cpp HandleAuctionHouseRequest), so its surplus
        // re-dump lands behind the pages we are still reading and is dropped
        // when we close; a server that pages on demand sends nothing at all
        // until this goes out.
        stream
            .write_all(&crypto.encode_ah_list_more_request(category, sorts, client_key))
            .await
            .context("requesting the next AH list page")?;
    };
    Ok(AhCatalog { total, listings })
}

pub async fn ah_history(host: &str, item_id: u16, stack: bool) -> Result<AhHistory> {
    tokio::time::timeout(SEARCH_TIMEOUT, ah_history_inner(host, item_id, stack))
        .await
        .context("AH history request timed out")?
}

async fn ah_history_inner(host: &str, item_id: u16, stack: bool) -> Result<AhHistory> {
    let mut crypto = SearchCrypto::new();
    let request = crypto.encode_ah_history_request(item_id, stack, next_client_key());
    let mut stream = connect(host).await?;
    stream
        .write_all(&request)
        .await
        .context("sending AH history request")?;

    let frame = read_frame(&mut stream).await?;
    let body = crypto.decrypt_response(&frame)?;
    Ok(search::parse_ah_history(&body)?)
}

async fn connect(host: &str) -> Result<TcpStream> {
    TcpStream::connect((host, SEARCH_PORT))
        .await
        .with_context(|| format!("connecting to search server {host}:{SEARCH_PORT}"))
}

async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; size_of::<u16>()];
    stream
        .read_exact(&mut len_bytes)
        .await
        .context("reading search frame length")?;
    let len = u16::from_le_bytes(len_bytes) as usize;
    if len < search::MIN_FRAME_LEN {
        bail!(
            "search frame length {len} below minimum {}",
            search::MIN_FRAME_LEN
        );
    }
    let mut frame = vec![0u8; len];
    frame[..len_bytes.len()].copy_from_slice(&len_bytes);
    stream
        .read_exact(&mut frame[len_bytes.len()..])
        .await
        .context("reading search frame body")?;
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    use ffxi_proto::search::{
        AH_LIST_ITEMS_OFFSET, AH_LIST_ITEMS_PER_PACKET, AH_LIST_ITEM_SIZE, AH_LIST_RESPONSE_TYPE,
        AH_LIST_TOTAL_COUNT_OFFSET, RESPONSE_BODY_SIZE_OFFSET, RESPONSE_FINAL_FLAG,
        RESPONSE_FLAGS_OFFSET, RESPONSE_TYPE_OFFSET, SEARCH_BASE_KEY, TCP_AH_REQUEST,
        TCP_AH_REQUEST_MORE,
    };
    use ffxi_proto::{blowfish, md5};

    /// Independent reimplementation of SearchHandler::decrypt/encrypt
    /// (vendor/server/src/search/search_handler.cpp:134-178) plus
    /// CAHItemsListPacket (packets/auction_list.cpp), so the paging loop is
    /// checked against the server algorithm rather than against our own codec.
    struct FakeSearchServer {
        key: [u8; 24],
    }

    impl FakeSearchServer {
        fn new() -> Self {
            Self {
                key: SEARCH_BASE_KEY,
            }
        }

        fn cipher(&self, frame: &mut [u8], key_bytes: usize, decrypt: bool) {
            let state = blowfish::State::new(&md5::md5(&self.key[..key_bytes]));
            let mut words = (frame.len() - 12) / 4;
            words -= words % 2;
            for i in (0..words).step_by(2) {
                let (a, b) = (4 * (i + 2), 4 * (i + 3));
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

        /// Returns the decrypted request's type byte, panicking if the MD5
        /// integrity check the server enforces (validatePacket) fails.
        fn accept(&mut self, frame: &mut [u8]) -> u8 {
            let len = frame.len();
            self.key[16..20].copy_from_slice(&frame[len - 4..]);
            self.cipher(frame, 20, true);
            self.key[20..24].copy_from_slice(&frame[len - 0x18..len - 0x18 + 4]);
            assert_eq!(
                md5::md5(&frame[8..len - 0x14]),
                frame[len - 0x14..len - 0x14 + 16],
                "server-side request hash"
            );
            frame[0x0B]
        }

        fn page(&self, total: u16, offset: usize) -> Vec<u8> {
            let rows = (total as usize - offset).min(AH_LIST_ITEMS_PER_PACKET);
            let body_size = AH_LIST_ITEMS_OFFSET + AH_LIST_ITEM_SIZE * rows;
            let mut frame = vec![0u8; body_size + 28];
            frame[RESPONSE_TYPE_OFFSET] = AH_LIST_RESPONSE_TYPE;
            frame[RESPONSE_BODY_SIZE_OFFSET..][..2]
                .copy_from_slice(&(body_size as u16).to_le_bytes());
            frame[AH_LIST_TOTAL_COUNT_OFFSET..][..2].copy_from_slice(&total.to_le_bytes());
            if total as usize - offset <= AH_LIST_ITEMS_PER_PACKET {
                frame[RESPONSE_FLAGS_OFFSET] = RESPONSE_FINAL_FLAG;
            }
            for row in 0..rows {
                let off = AH_LIST_ITEMS_OFFSET + AH_LIST_ITEM_SIZE * row;
                let item_id = (offset + row) as u16 + 1;
                frame[off..off + 2].copy_from_slice(&item_id.to_le_bytes());
            }
            let len = frame.len();
            frame[0..2].copy_from_slice(&(len as u16).to_le_bytes());
            frame[4..8].copy_from_slice(&ffxi_proto::search::MAGIC_IXFF.to_le_bytes());
            let hash = md5::md5(&frame[8..len - 0x18 - 0x04 + 8]);
            frame[len - 0x14..len - 0x14 + 16].copy_from_slice(&hash);
            self.cipher(&mut frame, 24, false);
            frame[len - 4..].copy_from_slice(&self.key[16..20]);
            frame
        }
    }

    async fn read_request<R: AsyncRead + Unpin>(stream: &mut R) -> Vec<u8> {
        read_frame(stream).await.expect("request frame")
    }

    /// The bug behind "AH browse: AH list request timed out": a server that
    /// pages on demand sends page 0 and then waits. Every later page must be
    /// pulled with TCP_AH_REQUEST_MORE under the connection's original key.
    #[tokio::test]
    async fn multi_page_catalog_pulls_each_page_with_a_more_request() {
        const TOTAL: u16 = 45;
        const CLIENT_KEY: u32 = 0xDEAD_BEEF;
        let (client, mut server) = tokio::io::duplex(4096);

        let serve = tokio::spawn(async move {
            let mut lsb = FakeSearchServer::new();
            let mut types = Vec::new();
            let mut offset = 0;
            loop {
                let mut req = read_request(&mut server).await;
                types.push(lsb.accept(&mut req));
                server.write_all(&lsb.page(TOTAL, offset)).await.unwrap();
                offset += AH_LIST_ITEMS_PER_PACKET;
                if offset >= TOTAL as usize {
                    break;
                }
            }
            types
        });

        let mut client = client;
        let catalog = drain_ah_list(&mut client, 18, &[], CLIENT_KEY)
            .await
            .expect("multi-page catalog");
        let types = serve.await.unwrap();

        assert_eq!(catalog.total, TOTAL);
        assert_eq!(
            catalog.listings.len(),
            TOTAL as usize,
            "every page merged, not just the first"
        );
        assert_eq!(catalog.listings[0].item_id, 1);
        assert_eq!(catalog.listings[TOTAL as usize - 1].item_id, TOTAL);
        assert_eq!(
            types,
            vec![TCP_AH_REQUEST, TCP_AH_REQUEST_MORE, TCP_AH_REQUEST_MORE],
            "one initial request then a MORE per follow-up page"
        );
    }

    /// A single-page catalog must not ask for more — the final flag ends it.
    #[tokio::test]
    async fn final_first_page_sends_no_more_request() {
        let (mut client, mut server) = tokio::io::duplex(4096);

        let serve = tokio::spawn(async move {
            let mut lsb = FakeSearchServer::new();
            let mut req = read_request(&mut server).await;
            let first = lsb.accept(&mut req);
            server.write_all(&lsb.page(3, 0)).await.unwrap();
            // Anything further would be an unwanted MORE; the client should
            // close instead, leaving this read at EOF.
            let mut trailing = Vec::new();
            server.read_to_end(&mut trailing).await.unwrap();
            (first, trailing)
        });

        let catalog = drain_ah_list(&mut client, 35, &[], 7)
            .await
            .expect("catalog");
        drop(client);
        let (first, trailing) = serve.await.unwrap();

        assert_eq!(catalog.listings.len(), 3);
        assert_eq!(first, TCP_AH_REQUEST);
        assert!(trailing.is_empty(), "no MORE after a final page");
    }

    #[tokio::test]
    async fn read_frame_splits_coalesced_frames_on_the_length_prefix() {
        let (mut client, mut server) = tokio::io::duplex(256);
        let mut two_frames = Vec::new();
        for fill in [0xAAu8, 0xBB] {
            let len = search::MIN_FRAME_LEN as u16;
            let mut frame = vec![fill; len as usize];
            frame[..2].copy_from_slice(&len.to_le_bytes());
            two_frames.extend_from_slice(&frame);
        }
        client.write_all(&two_frames).await.unwrap();

        let first = read_frame(&mut server).await.unwrap();
        assert_eq!(first.len(), search::MIN_FRAME_LEN);
        assert_eq!(first[2], 0xAA);
        let second = read_frame(&mut server).await.unwrap();
        assert_eq!(second[2], 0xBB);
    }

    #[tokio::test]
    async fn read_frame_rejects_undersized_length_field() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let bad_len = (search::MIN_FRAME_LEN as u16 - 1).to_le_bytes();
        client.write_all(&bad_len).await.unwrap();
        assert!(read_frame(&mut server).await.is_err());
    }

    #[test]
    fn client_keys_are_nonzero_and_distinct() {
        let a = next_client_key();
        let b = next_client_key();
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
    }
}
