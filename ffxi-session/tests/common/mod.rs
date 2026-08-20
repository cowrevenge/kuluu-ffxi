#![allow(dead_code)]

pub mod mcp_client;

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use mysql_async::prelude::*;
use mysql_async::{Conn, Pool};

use ffxi_session::auth_client::AuthClient;

pub const DEFAULT_DB_URL: &str = "mysql://xiadmin:password@127.0.0.1:3306/xidb";

// A half-up stack (something accepts on the port but mysqld never completes
// the handshake) must self-skip like an absent one, not hang the test binary.
const XIDB_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

const FIXTURE_PASSWORD: &str = "TestPass!1234";

// vendor/server/sql/triggers.sql `char_insert` (BEFORE INSERT ON chars).
// A leftover row in any of these makes the next COALESCE(MAX(charid),…)+1
// insert fail 1062 from inside the trigger, reported against `chars`.
const TRIGGER_CHILD_TABLES: &[&str] = &[
    "char_equip",
    "char_exp",
    "char_history",
    "char_inventory",
    "char_jobs",
    "char_pet",
    "char_points",
    "char_profile",
    "char_storage",
    "char_unlocks",
];

// Rows this fixture inserts on top of the trigger's. `char_flags` appears in
// neither trigger in vendor/server/sql/triggers.sql, so only this list frees it.
const FIXTURE_CHILD_TABLES: &[&str] = &["char_flags", "char_look", "char_stats"];

fn char_child_tables() -> impl Iterator<Item = &'static str> {
    TRIGGER_CHILD_TABLES
        .iter()
        .chain(FIXTURE_CHILD_TABLES)
        .copied()
}

// Fixture-owned name shape, emitted by `create` and matched by the tombstone
// sweep. Nothing else in xidb may look like this.
const FIXTURE_ACCOUNT_PREFIX: &str = "it_";
const FIXTURE_CHARNAME_PREFIX: &str = "It";
const FIXTURE_SUFFIX_HEX_DIGITS: usize = 6;

// vendor/server/settings/default/map.lua:18 `MAX_TIME_LASTUPDATE = 60`: a map
// session — and the charid it pins — outlives the client's last packet by this
// many seconds (vendor/server/src/map/map_session_container.cpp:222). While it
// is resident, LSB answers the lobby's CharZone by refreshing that session
// instead of creating the pending session a fresh login needs
// (vendor/server/src/map/ipc_client.cpp:196), so the new client's 0x00A is
// dropped (map_networking.cpp:270) and it never zones in. The fixture therefore
// parks its account as a tombstone rather than deleting it, so neither
// MAX(accounts.id)+1 nor COALESCE(MAX(chars.charid),…)+1 can hand the same ids
// to the next test while its session may still be resident.
const LSB_MAX_TIME_LASTUPDATE_SECS: u32 = 60;
// The tombstone has to outlive the session's *last packet*, not the account's
// creation, so it carries a budget for the longest a live test runs.
const FIXTURE_SESSION_BUDGET_SECS: u32 = 300;
const TOMBSTONE_TTL_SECS: u32 = LSB_MAX_TIME_LASTUPDATE_SECS + FIXTURE_SESSION_BUDGET_SECS;

fn fixture_name_suffix(nanos: u128) -> String {
    let mask = (1u128 << (4 * FIXTURE_SUFFIX_HEX_DIGITS)) - 1;
    format!(
        "{:0width$x}",
        nanos & mask,
        width = FIXTURE_SUFFIX_HEX_DIGITS
    )
}

fn fixture_login_pattern() -> String {
    format!("^{FIXTURE_ACCOUNT_PREFIX}[0-9a-f]{{{FIXTURE_SUFFIX_HEX_DIGITS}}}$")
}

pub struct EphemeralChar {
    pub username: String,
    pub password: String,
    pub accid: u32,
    pub charid: u32,
    pub charname: String,
    pool: Pool,
}

// Ok(None) = xidb is effectively unreachable (timed out mid-handshake, or the
// accept-then-drop / refused IO class) and the caller should self-skip; any
// other failure is a real provisioning error and still propagates.
async fn xidb_conn(db_url: &str, connect_timeout: Duration) -> Result<Option<(Pool, Conn)>> {
    let pool = Pool::new(db_url);
    match tokio::time::timeout(connect_timeout, pool.get_conn()).await {
        Ok(Ok(conn)) => Ok(Some((pool, conn))),
        Ok(Err(mysql_async::Error::Io(err))) => {
            eprintln!("xidb at {db_url}: handshake failed ({err}); treating as unreachable");
            let _ = pool.disconnect().await;
            Ok(None)
        }
        Ok(Err(err)) => {
            let _ = pool.disconnect().await;
            Err(err).with_context(|| format!("connecting to xidb at {db_url}"))
        }
        Err(_) => {
            eprintln!(
                "xidb at {db_url}: no handshake within {connect_timeout:?}; \
                 treating as unreachable"
            );
            let _ = pool.disconnect().await;
            Ok(None)
        }
    }
}

impl EphemeralChar {
    pub async fn create(server_host: &str, auth_port: u16) -> Result<Option<Self>> {
        let db_url = std::env::var("TEST_DB_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());

        let suffix = fixture_name_suffix(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );

        let username = format!("{FIXTURE_ACCOUNT_PREFIX}{suffix}");
        let charname = format!("{FIXTURE_CHARNAME_PREFIX}{suffix}");

        let password = FIXTURE_PASSWORD.to_string();

        let Some((pool, mut conn)) = xidb_conn(&db_url, XIDB_CONNECT_TIMEOUT).await? else {
            return Ok(None);
        };

        let auth = AuthClient::new(server_host.to_string(), auth_port);
        auth.ensure_account(&username, &password)
            .await
            .context("LOGIN_CREATE for ephemeral account")?;

        let accid: u32 = "SELECT id FROM accounts WHERE login = ?"
            .with((&username,))
            .first(&mut conn)
            .await
            .context("looking up accid for new ephemeral account")?
            .ok_or_else(|| anyhow!("ensure_account succeeded but accid {username:?} not found"))?;

        const POS_ZONE: u32 = 230;
        const NATION: u8 = 0;
        const GMLEVEL: u8 = 5;

        const FACE: u8 = 0;
        const RACE: u8 = 1;
        const SIZE: u8 = 0;

        const MJOB: u8 = 1;

        sweep_expired_tombstones(&mut conn)
            .await
            .context("sweeping expired fixture accounts before provisioning")?;
        sweep_orphaned_child_rows(&mut conn)
            .await
            .context("sweeping orphaned char_* rows before provisioning")?;

        let charid = run_inserts(
            &mut conn, accid, &charname, POS_ZONE, NATION, GMLEVEL, FACE, RACE, SIZE, MJOB,
        )
        .await
        .context("running LSB char-creation INSERT chain")?;

        drop(conn);

        Ok(Some(Self {
            username,
            password,
            accid,
            charid,
            charname,
            pool,
        }))
    }

    // Frees only the session row; the account (and the char + child rows it
    // cascades to) is left as the id-reuse tombstone that
    // `sweep_expired_tombstones` retires once TOMBSTONE_TTL_SECS have passed.
    pub async fn cleanup(&self) -> Result<()> {
        let mut conn = self.pool.get_conn().await.context("DB conn for cleanup")?;

        // Must go: LSB refuses the next login for an accid that still has a
        // session row (vendor/server/src/login/data_session.cpp:427).
        "DELETE FROM accounts_sessions WHERE accid = ?"
            .with((self.accid,))
            .ignore(&mut conn)
            .await
            .context("DELETE FROM accounts_sessions")?;

        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_inserts(
    conn: &mut Conn,
    accid: u32,
    charname: &str,
    pos_zone: u32,
    nation: u8,
    gmlevel: u8,
    face: u8,
    race: u8,
    size: u8,
    mjob: u8,
) -> Result<u32> {
    // Mirror LSB's own char creation (MAX(charid)+1) inside a single
    // INSERT ... SELECT, then read the actual created row back. Precomputing
    // an id from a sentinel scheme is unsound: nothing guarantees the next id,
    // and a charid that doesn't match the created row makes the lobby reject
    // char select with "mismatched character name".
    "INSERT INTO chars(charid, accid, charname, pos_zone, nation, gmlevel) \
     SELECT COALESCE(MAX(c.charid), 1000000) + 1, ?, ?, ?, ?, ? FROM chars AS c"
        .with((accid, charname, pos_zone, nation, gmlevel))
        .ignore(&mut *conn)
        .await
        .context("INSERT INTO chars")?;

    let charid: u32 = "SELECT charid FROM chars WHERE accid = ? AND charname = ? \
                       ORDER BY charid DESC LIMIT 1"
        .with((accid, charname))
        .first(&mut *conn)
        .await
        .context("reading back created charid")?
        .ok_or_else(|| {
            anyhow!("chars row for accid {accid} / charname {charname:?} not found after insert")
        })?;

    "INSERT INTO char_look(charid, face, race, size) VALUES (?, ?, ?, ?)"
        .with((charid, face, race, size))
        .ignore(&mut *conn)
        .await
        .context("INSERT INTO char_look")?;

    "INSERT INTO char_stats(charid, mjob) VALUES (?, ?)"
        .with((charid, mjob))
        .ignore(&mut *conn)
        .await
        .context("INSERT INTO char_stats")?;

    for table in [
        "char_exp",
        "char_flags",
        "char_jobs",
        "char_points",
        "char_unlocks",
        "char_profile",
        "char_storage",
    ] {
        let stmt = format!(
            "INSERT INTO {table}(charid) VALUES (?) ON DUPLICATE KEY UPDATE charid = charid"
        );
        stmt.with((charid,))
            .ignore(&mut *conn)
            .await
            .with_context(|| format!("INSERT INTO {table}"))?;
    }

    Ok(charid)
}

// Retires tombstones whose map session cannot still be resident. Scoped by the
// login shape this fixture emits, so a real account is never a candidate. One
// DELETE frees the whole identity: LSB's `account_delete` trigger cascades to
// `chars`, whose `char_delete` trigger cascades to the child tables
// (vendor/server/sql/triggers.sql).
async fn sweep_expired_tombstones(conn: &mut Conn) -> Result<()> {
    "DELETE FROM accounts \
     WHERE login REGEXP ? \
       AND UNIX_TIMESTAMP(timecreate) < UNIX_TIMESTAMP() - ?"
        .with((fixture_login_pattern(), TOMBSTONE_TTL_SECS))
        .ignore(&mut *conn)
        .await
        .context("tombstone sweep on accounts")?;

    let swept = conn.affected_rows();
    if swept > 0 {
        eprintln!("fixture: swept {swept} expired fixture account tombstone(s)");
    }
    Ok(())
}

// LSB's map server REPLACEs into char_history on save well after the client
// drops (vendor/server/src/map/utils/charutils.cpp:7727), and a panicking test
// never reaches cleanup() at all, so a previous run can leave child rows whose
// `chars` row is gone. Sweeping them here is what keeps the `char_insert`
// trigger from colliding when their charid comes back around.
async fn sweep_orphaned_child_rows(conn: &mut Conn) -> Result<()> {
    for table in char_child_tables() {
        let stmt = format!(
            "DELETE t FROM {table} AS t \
             LEFT JOIN chars AS c ON c.charid = t.charid \
             WHERE c.charid IS NULL"
        );
        conn.query_drop(&stmt)
            .await
            .with_context(|| format!("orphan sweep on {table}"))?;
        let swept = conn.affected_rows();
        if swept > 0 {
            eprintln!("fixture: swept {swept} orphaned {table} row(s) with no chars row");
        }
    }
    Ok(())
}

#[cfg(test)]
mod xidb_conn_tests {
    use super::*;

    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);

    fn db_url(port: u16) -> String {
        format!("mysql://user:pass@127.0.0.1:{port}/xidb")
    }

    #[tokio::test]
    async fn accept_then_drop_self_skips() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let _ = listener.accept().await;
            }
        });

        let got = xidb_conn(&db_url(port), HANDSHAKE_TIMEOUT)
            .await
            .expect("accept-then-drop must self-skip, not error");
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn refused_port_self_skips() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let got = xidb_conn(&db_url(port), HANDSHAKE_TIMEOUT)
            .await
            .expect("connect-refused must self-skip, not error");
        assert!(got.is_none());
    }
}

#[cfg(test)]
mod fixture_name_tests {
    use super::*;

    // The sweep matches names with a SQL REGEXP built from these same consts;
    // this pins the emitter to the character class that pattern accepts.
    #[test]
    fn emitted_names_match_the_sweep_pattern() {
        for nanos in [0u128, 1, 0x0f_ff_ff, u128::MAX] {
            let suffix = fixture_name_suffix(nanos);
            assert_eq!(suffix.len(), FIXTURE_SUFFIX_HEX_DIGITS);
            assert!(suffix
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));

            let login = format!("{FIXTURE_ACCOUNT_PREFIX}{suffix}");
            assert!(login.starts_with(FIXTURE_ACCOUNT_PREFIX));
            assert_eq!(
                login.len(),
                FIXTURE_ACCOUNT_PREFIX.len() + FIXTURE_SUFFIX_HEX_DIGITS
            );
        }
    }
}
