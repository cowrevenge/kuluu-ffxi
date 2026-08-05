//! Live AH search-server smoke test against a running LSB stack: browse a
//! category and fetch an item's sale history over TCP SEARCH_PORT. No map
//! session needed — the search server answers standalone. Skips (passes) when
//! no stack is reachable, matching delivery_box_live.rs.

use std::time::Duration;

use ffxi_client::search_client::{self, SEARCH_PORT};

// vendor/server/documentation/Auction Categories.txt (item_basic.aH).
const AH_CATEGORY_CRYSTALS: u8 = 35;

// vendor/server/sql/item_basic.sql: itemid 4096 = fire_crystal, a stackable
// Crystals-category item present in every LSB item DB.
const FIRE_CRYSTAL: u16 = 4096;

async fn is_reachable(host: &str, port: u16) -> bool {
    tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

// The search server closes any new socket while >5 sessions from one IP are
// live (vendor/server/src/search/search_handler.cpp SearchHandler ctor), and
// a colima/lima port-forward can hold just-closed sockets open until the
// server's 10s read timeout reaps them (SearchHandler::run withTimeout) — so
// transient failures get retries spaced past that reap window.
const LIVE_ATTEMPTS: usize = 3;
const LIVE_RETRY_DELAY: Duration = Duration::from_secs(11);

async fn with_retry<T, F, Fut>(mut call: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    let mut last = None;
    for attempt in 0..LIVE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(LIVE_RETRY_DELAY).await;
        }
        match call().await {
            Ok(v) => return Ok(v),
            Err(e) => last = Some(e),
        }
    }
    Err(last.expect("at least one attempt ran"))
}

#[tokio::test]
async fn auction_browse_and_history_against_live_lsb() {
    let server_host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    if !is_reachable(&server_host, SEARCH_PORT).await {
        eprintln!(
            "skipping: no LSB search server reachable at {server_host}:{SEARCH_PORT}. \
             Start the stack to run this test."
        );
        return;
    }

    let catalog = with_retry(|| search_client::ah_list(&server_host, AH_CATEGORY_CRYSTALS, &[]))
        .await
        .expect("AH category browse against live search server");
    assert_eq!(
        catalog.listings.len(),
        catalog.total as usize,
        "merged pages must cover the advertised item count"
    );
    for listing in &catalog.listings {
        assert_ne!(listing.item_id, 0, "catalog rows carry real item ids");
    }

    let history = with_retry(|| search_client::ah_history(&server_host, FIRE_CRYSTAL, true))
        .await
        .expect("AH history against live search server");
    assert_eq!(history.item_id, FIRE_CRYSTAL);
    assert_eq!(history.category, AH_CATEGORY_CRYSTALS as u16);
    assert!(
        history.sales.len() <= ffxi_proto::search::AH_HISTORY_MAX_ROWS,
        "server sends at most the last 10 sales"
    );
}
