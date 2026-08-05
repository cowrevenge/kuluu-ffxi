//! Auction House queries against the LSB search server: one TCP connection per
//! request (vendor/server/src/search/tcp_server.cpp accepts, handles, closes),
//! frames encoded/decrypted by [`ffxi_proto::search::SearchCrypto`].

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use ffxi_proto::search::{self, AhHistory, AhListing, SearchCrypto};

/// vendor/server/settings/default/network.lua SEARCH_PORT.
pub const SEARCH_PORT: u16 = 54002;

pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(5);

// The server only echoes the client key back (SearchHandler::encrypt); any
// nonzero value works, and a process-wide counter keeps concurrent
// connections' key splices distinct without any time-derived entropy.
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
    let mut crypto = SearchCrypto::new();
    let request = crypto.encode_ah_list_request(category, sorts, next_client_key());
    let mut stream = connect(host).await?;
    stream
        .write_all(&request)
        .await
        .context("sending AH list request")?;

    let mut listings = Vec::new();
    let total = loop {
        let frame = read_frame(&mut stream).await?;
        let body = crypto.decrypt_response(&frame)?;
        let page = search::parse_ah_list(&body)?;
        listings.extend(page.listings);
        if page.is_final {
            break page.total_count;
        }
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
