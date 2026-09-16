use anyhow::{bail, Context, Result};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use ffxi_proto::login::{
    expansion_display, feature_display, lobby_error, CLIENT_VER_ENV, IXFF_TERMINATOR,
    LSB_CLIENT_VER,
};

use crate::auth_client::{AuthRejected, AuthSession};

const DATA_CHARLIST_SIZE: usize = 0x148;

const IXFF_HEADER_SIZE: usize = 28;

const DATA_CMD_CHAR_LIST: u8 = 0xA1;
const DATA_CMD_HANDOFF: u8 = 0xA2;

// vendor/server/src/login/view_session.cpp view_session::read_func, the
// VIEW_CMD_SELECT branch's ack on the data socket.
const DATA_RESP_SELECT_ACK: u8 = 0x02;
// vendor/server/src/login/data_session.cpp data_session::read_func uList[0]
const DATA_RESP_CHAR_LIST: u8 = 0x03;

/// Capability marker + word in bytes [2..10) of the 0xA1 char-list response.
/// vendor/server/src/login/data_session.cpp zero-inits `uList[500]` and only
/// writes [0], [1] and entries at 16*(i+1), so vanilla LSB always sends zeros
/// here; a patched build stamps KULUU_CAP_KEY plus the caps it honors.
const CAP_KEY_OFFSET: usize = 2;
const CAP_WORD_OFFSET: usize = 6;

const KULUU_CAP_KEY: u32 = 867309;
pub const CAP_SKIP_INTRO_CS: u32 = 1 << 0;

// research/XiPackets/lobby/C2S_0x0026_RequestLobbyLogin.md: retail's first
// lobby packet; vendor/server/src/login/view_session.cpp view_session::read_func
// case 0x26 reads only versionCode from it. The header's identifer is an MD5
// of the packet on retail (research/XiPackets/lobby/Header.md) but the auth
// session hash on LSB (vendor/server/src/login/login_helpers.cpp
// getHashFromPacket); LSB is the connect target, so the hash goes there.
const VIEW_CMD_LOBBY_LOGIN: u32 = 0x26;
const LOBBY_LOGIN_PACKET_SIZE: usize = 0x98;
const LOBBY_LOGIN_CLIENT_CODE_OFFSET: usize = 0x2C;
const LOBBY_LOGIN_VERSION_OFFSET: usize = 0x74;
const LOBBY_LOGIN_VERSION_LEN: usize = 16;
const LOBBY_LOGIN_EXCODE_OFFSET: usize = 0x84;
/// client_code[0] as the XiPackets capture of an English client shows it; the
/// doc describes it as language plus a first-launch render flag, 3 or 5.
const LOBBY_LOGIN_CLIENT_CODE_ENGLISH: u8 = 5;

// research/XiPackets/lobby/S2C_0x0005_ResponseKey.md
const VIEW_RESP_KEY: u32 = 0x05;
const KEY_PACKET_SIZE: usize = 0x28;
const KEY_OFFSET: usize = 0x1C;
const KEY_EXCODE_SERVER_OFFSET: usize = 0x20;
const KEY_EXCODE_SERVER2_OFFSET: usize = 0x24;
// vendor/server/src/login/login_helpers.cpp generateErrorMessage
const VIEW_ERROR_CODE_OFFSET: usize = 0x20;

const VIEW_CMD_SELECT: u32 = 0x07;

const VIEW_CMD_DELETE_CHAR: u32 = 0x14;

const VIEW_CMD_REGISTER_CHAR: u32 = 0x21;

const VIEW_CMD_NAME_CHECK: u32 = 0x22;

const VIEW_RESP_NEXT_LOGIN: u32 = 0x0B;

const NEXT_LOGIN_PACKET_SIZE: u32 = 0x48;

const LOBBY_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

async fn lobby_io<T>(
    step: &'static str,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(LOBBY_IO_TIMEOUT, fut).await {
        Ok(inner) => inner,
        Err(_elapsed) => {
            tracing::warn!(
                step,
                secs = LOBBY_IO_TIMEOUT.as_secs(),
                "lobby step timed out waiting for server"
            );
            bail!(
                "lobby {step}: server did not respond within {}s",
                LOBBY_IO_TIMEOUT.as_secs()
            )
        }
    }
}

#[derive(Debug, Clone)]
pub struct CharListEntry {
    pub content_id: u32,
    pub char_id_main: u16,
    pub world_id: u8,
    pub char_id_extra: u8,
}

#[derive(Debug, Clone)]
pub struct CharList {
    pub characters: Vec<CharListEntry>,
}

#[derive(Debug, Clone)]
pub struct CharSlot {
    pub char_id: u32,
    pub name: String,
    pub status: u16,

    pub race: u8,

    pub face: u8,

    pub head: u16,
    pub body: u16,
    pub hands: u16,
    pub legs: u16,
    pub feet: u16,
    pub main: u16,
    pub sub: u16,
    pub ranged: u16,

    pub zone_id: u16,
}

#[derive(Debug, Clone)]
pub struct MapHandoff {
    pub char_id: u32,
    pub character_name: String,
    pub server_ip: u32,
    pub server_port: u16,

    pub session_key_seed: [u8; 20],
}

pub struct LobbyClient {
    pub host: String,
    pub data_port: u16,
    pub view_port: u16,
    /// versionCode for the 0x26; `None` resolves [`client_version_code`] at
    /// open time.
    pub version_code: Option<String>,
}

/// S2C 0x05: the lobby admitted the client version; the bitmasks say which
/// expansions the server serves and which account features are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LobbyKey {
    pub key: u32,
    /// uint32 on the wire; LSB fills the low half from a uint16 bitmask.
    pub excode_server: u32,
    pub excode_server2: u32,
}

impl LobbyKey {
    pub fn expansion_names(&self) -> Vec<&'static str> {
        expansion_display::NAMES
            .iter()
            .filter(|(bit, name)| {
                self.excode_server & u32::from(*bit) != 0 && !name.starts_with("UNUSED_")
            })
            .map(|(_, name)| *name)
            .collect()
    }

    pub fn feature_names(&self) -> Vec<&'static str> {
        feature_display::NAMES
            .iter()
            .filter(|(bit, name)| {
                self.excode_server2 & u32::from(*bit) != 0 && !name.starts_with("UNUSED_")
            })
            .map(|(_, name)| *name)
            .collect()
    }
}

/// The patch stamp the 0x26 versionCode carries: [`CLIENT_VER_ENV`], else the
/// install every `DatRoot::from_env_or_default` in this process loads, else
/// the vendored pin (a session with no install is not a client the lobby can
/// judge, and the pin keeps a DAT-less agent session reachable).
pub fn client_version_code() -> String {
    if let Some(v) = std::env::var(CLIENT_VER_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        if v.len() >= ffxi_proto::login::CLIENT_VER_ERA_LEN {
            return v;
        }
        tracing::warn!(
            value = %v,
            "{CLIENT_VER_ENV} is shorter than the {} bytes the lobby compares; ignoring it",
            ffxi_proto::login::CLIENT_VER_ERA_LEN
        );
    }
    let root = std::env::var_os(ffxi_dat::archive::DAT_PATH_ENV)
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var(ffxi_dat::archive::CLIENT_TARGET_ENV)
                .ok()
                .and_then(|name| ffxi_dat::archive::workspace_target(&name))
        })
        .or_else(ffxi_dat::archive::workspace_default);
    match root.and_then(|r| ffxi_dat::client_profile::patch_version_at(&r)) {
        Some(stamp) => stamp,
        None => {
            tracing::warn!(
                pin = LSB_CLIENT_VER,
                "no FFXI install to read a patch stamp from; the lobby login carries the LSB pin"
            );
            LSB_CLIENT_VER.to_string()
        }
    }
}

pub struct LobbyHandle {
    view: TcpStream,
    data: TcpStream,

    chars: Vec<CharSlot>,
    session_hash: [u8; 16],
    server_caps: u32,
    key: LobbyKey,
}

impl LobbyHandle {
    pub fn chars(&self) -> &[CharSlot] {
        &self.chars
    }

    pub fn key(&self) -> LobbyKey {
        self.key
    }

    /// Caps word echoed by this lobby connection's char-list reply. Zero until
    /// (and unless) the server stamps KULUU_CAP_KEY — never carried across
    /// connections, so a server switch can't serve stale caps.
    pub fn server_caps(&self) -> u32 {
        self.server_caps
    }

    pub fn supports_skip_intro_cs(&self) -> bool {
        self.server_caps & CAP_SKIP_INTRO_CS != 0
    }

    pub async fn create_character(
        mut self,
        auth: &AuthSession,
        spec: &CharCreateSpec,
    ) -> Result<Self> {
        let name_check = build_view_name_check(&spec.name, &self.session_hash);
        self.view.write_all(&name_check).await?;
        self.view.flush().await?;
        tracing::debug!(name = %spec.name, "lobby handle: 0x22 name check sent");
        read_create_reply(&mut self.view, "name check")
            .await
            .context("0x22 name check response")?;

        let skip_intro_cs = spec.skip_intro_cs & u8::from(self.supports_skip_intro_cs());
        if spec.skip_intro_cs != 0 && !self.supports_skip_intro_cs() {
            tracing::warn!(
                "server did not advertise CAP_SKIP_INTRO_CS; register sent without the skip flag"
            );
        }
        let register_char = build_view_register_char(
            spec.race,
            spec.job,
            spec.nation,
            spec.size,
            spec.face,
            skip_intro_cs,
            &self.session_hash,
        );
        self.view.write_all(&register_char).await?;
        self.view.flush().await?;
        tracing::debug!(
            race = spec.race,
            job = spec.job,
            nation = spec.nation,
            size = spec.size,
            face = spec.face,
            skip_intro_cs = skip_intro_cs != 0,
            "lobby handle: 0x21 register sent"
        );
        read_create_reply(&mut self.view, "register character")
            .await
            .context("0x21 register character response")?;

        let req_a1 = build_data_a1(auth.account_id, 0, &self.session_hash);
        self.data.write_all(&req_a1).await?;
        self.data.flush().await?;

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (_, caps) = read_data_charlist(&mut self.data)
            .await
            .context("reading 0xA1 char-list refresh after create")?;
        self.server_caps = caps;
        let slots = parse_view_chr_info2(&mut self.view)
            .await
            .context("reading chr_info2 refresh after create")?;
        self.chars = slots
            .into_iter()
            .filter(|s| s.char_id != 0 && !s.name.starts_with(' '))
            .collect();
        tracing::info!(
            new_name = %spec.name,
            total = self.chars.len(),
            "lobby handle: char list refreshed after create"
        );
        Ok(self)
    }

    pub async fn delete_character(mut self, auth: &AuthSession, char_id: u32) -> Result<Self> {
        let pkt = build_delete_char(char_id, 0, [0u8; 16], &self.session_hash);
        self.view.write_all(&pkt).await?;
        self.view.flush().await?;
        tracing::debug!(char_id, "lobby handle: 0x14 delete-char sent");

        let mut ack = [0u8; 0x20];
        self.view
            .read_exact(&mut ack)
            .await
            .context("reading 0x14 delete-char ack on view socket")?;

        let req_a1 = build_data_a1(auth.account_id, 0, &self.session_hash);
        self.data.write_all(&req_a1).await?;
        self.data.flush().await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (_, caps) = read_data_charlist(&mut self.data)
            .await
            .context("reading 0xA1 char-list refresh after delete")?;
        self.server_caps = caps;
        let slots = parse_view_chr_info2(&mut self.view)
            .await
            .context("reading chr_info2 refresh after delete")?;
        self.chars = slots
            .into_iter()
            .filter(|s| s.char_id != 0 && !s.name.starts_with(' '))
            .collect();
        tracing::info!(
            char_id,
            total = self.chars.len(),
            "lobby handle: char list refreshed after delete"
        );
        Ok(self)
    }

    pub async fn select(self, char_id: u32, char_name: &str, key3: [u8; 20]) -> Result<MapHandoff> {
        let req_select = build_view_select(char_id, char_name, &self.session_hash);
        self.send_view_select(char_id, req_select, key3).await
    }

    /// Select by id alone. The 0x07's name is resolved from this handle's own
    /// chr_info2 slots, so no caller can put a name on the wire that LSB will
    /// reject (see [`build_view_select_by_id`]).
    pub async fn select_by_id(self, char_id: u32, key3: [u8; 20]) -> Result<MapHandoff> {
        let req_select = build_view_select_by_id(&self.chars, char_id, &self.session_hash)?;
        self.send_view_select(char_id, req_select, key3).await
    }

    async fn send_view_select(
        mut self,
        char_id: u32,
        req_select: Vec<u8>,
        key3: [u8; 20],
    ) -> Result<MapHandoff> {
        self.view.write_all(&req_select).await?;
        self.view.flush().await?;
        tracing::info!(char_id, "lobby: 0x07 select sent");

        let mut ack = [0u8; 5];
        lobby_io("0x02 ack (data)", async {
            self.data
                .read_exact(&mut ack)
                .await
                .context("reading 0x07 ack on data port")?;
            Ok(())
        })
        .await?;
        if ack[0] != DATA_RESP_SELECT_ACK {
            bail!("expected 0x02 ack after view select, got {ack:?}");
        }
        tracing::info!("lobby: 0x02 ack received");

        let req_a2 = build_data_a2(&key3);
        self.data.write_all(&req_a2).await?;
        self.data.flush().await?;
        tracing::info!("lobby: 0xA2 handoff sent");

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let handoff = lobby_io(
            "lpkt_next_login (view)",
            read_lpkt_next_login(&mut self.view, char_id, &key3),
        )
        .await?;
        tracing::info!(
            server_port = handoff.server_port,
            "lobby: lpkt_next_login received"
        );
        Ok(handoff)
    }
}

/// Build the 0x07 view-select for `char_id`, taking the name from the account's
/// own chr_info2 slots. LSB looks the selection up with
/// `WHERE charid = ? AND charname = ?` and closes the socket on a mismatch
/// (vendor/server/src/login/view_session.cpp view_session::read_func), so an id the account does
/// not own fails here rather than on the wire — there is no caller-supplied
/// name to fall back to.
fn build_view_select_by_id(
    chars: &[CharSlot],
    char_id: u32,
    session_hash: &[u8; 16],
) -> Result<Vec<u8>> {
    let Some(slot) = chars.iter().find(|c| c.char_id == char_id) else {
        bail!(
            "char id {char_id} not present on account (have: {:?})",
            chars
                .iter()
                .map(|c| (c.char_id, c.name.as_str()))
                .collect::<Vec<_>>()
        );
    };
    Ok(build_view_select(char_id, &slot.name, session_hash))
}

impl LobbyClient {
    pub fn new(host: impl Into<String>, data_port: u16, view_port: u16) -> Self {
        Self {
            host: host.into(),
            data_port,
            view_port,
            version_code: None,
        }
    }

    pub fn with_version_code(mut self, version_code: Option<String>) -> Self {
        self.version_code = version_code;
        self
    }

    pub async fn open(&self, auth: &AuthSession) -> Result<LobbyHandle> {
        let mut view = self.connect(self.view_port).await?;
        tracing::info!("lobby: view socket connected");
        let mut data = self.connect(self.data_port).await?;
        tracing::info!("lobby: data socket connected");

        let version_code = self
            .version_code
            .clone()
            .unwrap_or_else(client_version_code);
        let login = build_lobby_login(
            &auth.session_hash,
            &version_code,
            expansion_display::ALL_KNOWN,
        );
        view.write_all(&login).await?;
        view.flush().await?;
        tracing::info!(version_code, "lobby: 0x26 lobby login sent");
        let key = lobby_io("0x05 key (view)", read_key_reply(&mut view)).await?;
        tracing::info!(
            key = format_args!("{:#010x}", key.key),
            expansions = ?key.expansion_names(),
            features = ?key.feature_names(),
            "lobby: 0x05 key received"
        );

        let req_a1 = build_data_a1(auth.account_id, 0, &auth.session_hash);
        data.write_all(&req_a1).await?;
        data.flush().await?;
        tracing::info!(
            account_id = auth.account_id,
            "lobby: 0xA1 char-list request sent"
        );

        let (charlist, server_caps) =
            lobby_io("0xA1 char-list (data)", read_data_charlist(&mut data)).await?;
        tracing::info!(
            count = charlist.characters.len(),
            "lobby: 0xA1 char-list received"
        );
        let slots = lobby_io("chr_info2 (view)", parse_view_chr_info2(&mut view)).await?;

        let chars: Vec<CharSlot> = slots
            .into_iter()
            .filter(|s| s.char_id != 0 && !s.name.starts_with(' '))
            .collect();
        tracing::info!(populated = chars.len(), "lobby: chr_info2 parsed");

        Ok(LobbyHandle {
            view,
            data,
            chars,
            session_hash: auth.session_hash,
            server_caps,
            key,
        })
    }

    pub async fn handshake(
        &self,
        auth: &AuthSession,
        char_id: u32,
        _search_server_ip: u32,
        key3: [u8; 20],
    ) -> Result<MapHandoff> {
        let handle = self.open(auth).await?;
        if handle.chars.is_empty() {
            bail!("no characters found for account");
        }
        handle.select_by_id(char_id, key3).await
    }

    pub async fn handshake_by_name(
        &self,
        auth: &AuthSession,
        char_name: &str,
        key3: [u8; 20],
    ) -> Result<(u32, MapHandoff)> {
        let handle = self.open(auth).await?;
        if handle.chars.is_empty() {
            bail!("no characters found for account");
        }
        let slot = handle
            .chars()
            .iter()
            .find(|c| c.name == char_name)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no character named '{char_name}' on account (have: {:?})",
                    handle
                        .chars()
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                )
            })?;
        let handoff = handle.select(slot.char_id, &slot.name, key3).await?;
        Ok((slot.char_id, handoff))
    }

    pub async fn create_character(&self, auth: &AuthSession, spec: &CharCreateSpec) -> Result<()> {
        let handle = self.open(auth).await?;
        let mut view = handle.view;
        tracing::debug!("create_character: lobby open complete");

        let name_check = build_view_name_check(&spec.name, &auth.session_hash);
        view.write_all(&name_check).await?;
        view.flush().await?;
        tracing::debug!(name = %spec.name, "create_character: 0x22 name check sent");
        read_create_reply(&mut view, "name check")
            .await
            .context("0x22 name check response")?;

        let register_char = build_view_register_char(
            spec.race,
            spec.job,
            spec.nation,
            spec.size,
            spec.face,
            spec.skip_intro_cs,
            &auth.session_hash,
        );
        view.write_all(&register_char).await?;
        view.flush().await?;
        tracing::debug!(
            race = spec.race,
            job = spec.job,
            nation = spec.nation,
            size = spec.size,
            face = spec.face,
            skip_intro_cs = spec.skip_intro_cs != 0,
            "create_character: 0x21 register sent"
        );
        read_create_reply(&mut view, "register character")
            .await
            .context("0x21 register character response")?;

        Ok(())
    }

    async fn connect(&self, port: u16) -> Result<TcpStream> {
        lobby_io("TCP connect", async {
            TcpStream::connect((self.host.as_str(), port))
                .await
                .with_context(|| format!("TCP connect to {}:{}", self.host, port))
        })
        .await
    }
}

fn build_data_a1(account_id: u32, search_server_ip: u32, session_hash: &[u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; 28];
    buf[0] = DATA_CMD_CHAR_LIST;
    buf[1..5].copy_from_slice(&account_id.to_le_bytes());
    buf[5..9].copy_from_slice(&search_server_ip.to_le_bytes());
    buf[12..28].copy_from_slice(session_hash);
    buf
}

fn build_lobby_login(session_hash: &[u8; 16], version_code: &str, excode_client: u16) -> Vec<u8> {
    let mut buf = vec![0u8; LOBBY_LOGIN_PACKET_SIZE];
    buf[0..4].copy_from_slice(&(LOBBY_LOGIN_PACKET_SIZE as u32).to_le_bytes());
    buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
    buf[8..12].copy_from_slice(&VIEW_CMD_LOBBY_LOGIN.to_le_bytes());
    buf[12..28].copy_from_slice(session_hash);
    buf[LOBBY_LOGIN_CLIENT_CODE_OFFSET] = LOBBY_LOGIN_CLIENT_CODE_ENGLISH;
    let version = version_code.as_bytes();
    let n = version.len().min(LOBBY_LOGIN_VERSION_LEN - 1);
    buf[LOBBY_LOGIN_VERSION_OFFSET..LOBBY_LOGIN_VERSION_OFFSET + n].copy_from_slice(&version[..n]);
    buf[LOBBY_LOGIN_EXCODE_OFFSET..LOBBY_LOGIN_EXCODE_OFFSET + 4]
        .copy_from_slice(&u32::from(excode_client).to_le_bytes());
    buf
}

fn parse_key_reply(buf: &[u8; KEY_PACKET_SIZE]) -> Result<LobbyKey> {
    let term = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if term != IXFF_TERMINATOR {
        bail!("0x05 key: bad terminator {term:#x}");
    }
    let cmd = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    if cmd != VIEW_RESP_KEY {
        bail!("0x05 key: unexpected command {cmd:#x}");
    }
    let word = |at: usize| u32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
    Ok(LobbyKey {
        key: word(KEY_OFFSET),
        excode_server: word(KEY_EXCODE_SERVER_OFFSET),
        excode_server2: word(KEY_EXCODE_SERVER2_OFFSET),
    })
}

/// The 0x26 is answered with the 0x28-byte key on success or LSB's 0x24-byte
/// error frame when the version lock rejects the client; the latter is a
/// verdict on this install, so it is an [`AuthRejected`], not a retry.
async fn read_key_reply(stream: &mut TcpStream) -> Result<LobbyKey> {
    let mut size_bytes = [0u8; 4];
    stream
        .read_exact(&mut size_bytes)
        .await
        .context("reading 0x05 key reply size (server may have closed socket)")?;
    let size = u32::from_le_bytes(size_bytes) as usize;
    match size {
        KEY_PACKET_SIZE => {
            let mut buf = [0u8; KEY_PACKET_SIZE];
            buf[0..4].copy_from_slice(&size_bytes);
            stream
                .read_exact(&mut buf[4..])
                .await
                .context("reading 0x05 key reply body")?;
            parse_key_reply(&buf)
        }
        VIEW_REPLY_ERROR_SIZE => {
            let mut rest = vec![0u8; size - 4];
            stream
                .read_exact(&mut rest)
                .await
                .context("reading 0x26 error reply body")?;
            let term = u32::from_le_bytes(rest[0..4].try_into().unwrap());
            if term != IXFF_TERMINATOR {
                bail!("0x26 error reply: bad terminator {term:#x}");
            }
            if rest[4] != VIEW_REPLY_ERROR_RESULT {
                bail!("0x26 error reply: unexpected result {:#x}", rest[4]);
            }
            let at = VIEW_ERROR_CODE_OFFSET - 4;
            let code = u16::from_le_bytes(rest[at..at + 2].try_into().unwrap());
            Err(AuthRejected(format!(
                "lobby login: server rejected the client version with loginErrors code {code} ({})",
                lobby_error::name(code).unwrap_or("unknown")
            ))
            .into())
        }
        other => bail!("0x26 reply: implausible size {other:#x} (want 0x28 or 0x24)"),
    }
}

fn build_data_a2(key3: &[u8; 20]) -> Vec<u8> {
    let mut buf = vec![0u8; 28];
    buf[0] = DATA_CMD_HANDOFF;
    buf[1..21].copy_from_slice(key3);
    buf
}

/// vendor/server/src/login/view_session.cpp view_session::read_func requestedCharacterID reads the selected char id at
/// buffer offset 28 and copies `PacketNameLength - 1` name bytes from offset 36
/// (`PacketNameLength = 16`, vendor/server/src/common/utils.h).
const VIEW_SELECT_PACKET_SIZE: u32 = 0x44;
const VIEW_SELECT_CHAR_ID_OFFSET: usize = 28;
const VIEW_SELECT_WORLD_CHAR_ID_OFFSET: usize = 32;
const VIEW_SELECT_NAME_OFFSET: usize = 36;
const VIEW_SELECT_NAME_LEN: usize = 15;
// vendor/server/src/login/data_session.cpp data_session::read_func ffxi_id_world
const WORLD_CHAR_ID_MASK: u32 = 0xFFFF;

fn build_view_select(char_id: u32, char_name: &str, session_hash: &[u8; 16]) -> Vec<u8> {
    let mut buf = vec![0u8; VIEW_SELECT_PACKET_SIZE as usize];
    buf[0..4].copy_from_slice(&VIEW_SELECT_PACKET_SIZE.to_le_bytes());
    buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
    buf[8..12].copy_from_slice(&VIEW_CMD_SELECT.to_le_bytes());
    buf[12..28].copy_from_slice(session_hash);
    buf[VIEW_SELECT_CHAR_ID_OFFSET..VIEW_SELECT_CHAR_ID_OFFSET + 4]
        .copy_from_slice(&char_id.to_le_bytes());

    buf[VIEW_SELECT_WORLD_CHAR_ID_OFFSET..VIEW_SELECT_WORLD_CHAR_ID_OFFSET + 4]
        .copy_from_slice(&(char_id & WORLD_CHAR_ID_MASK).to_le_bytes());
    let name_bytes = char_name.as_bytes();
    let n = name_bytes.len().min(VIEW_SELECT_NAME_LEN);
    buf[VIEW_SELECT_NAME_OFFSET..VIEW_SELECT_NAME_OFFSET + n].copy_from_slice(&name_bytes[..n]);
    buf
}

pub fn build_delete_char(
    content_id: u32,
    world_id: u32,
    passwd: [u8; 16],
    session_hash: &[u8; 16],
) -> Vec<u8> {
    let packet_size = 0x34u32;
    let mut buf = vec![0u8; packet_size as usize];
    buf[0..4].copy_from_slice(&packet_size.to_le_bytes());
    buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
    buf[8..12].copy_from_slice(&VIEW_CMD_DELETE_CHAR.to_le_bytes());
    buf[12..28].copy_from_slice(session_hash);
    buf[28..32].copy_from_slice(&content_id.to_le_bytes());
    buf[32..36].copy_from_slice(&world_id.to_le_bytes());
    buf[36..52].copy_from_slice(&passwd);
    buf
}

#[derive(Debug, Clone)]
pub struct CharCreateSpec {
    pub name: String,

    pub race: u8,

    pub job: u8,

    pub nation: u8,

    pub size: u8,

    pub face: u8,

    /// 1 = skip the opening (new-character) cutscene. Carried in spare byte 58
    /// of the C2L 0x21 register packet; retail leaves that byte zero, which
    /// the server reads as "play it".
    pub skip_intro_cs: u8,
}

fn build_view_name_check(name: &str, session_hash: &[u8; 16]) -> Vec<u8> {
    let packet_size = 0x40u32;
    let mut buf = vec![0u8; packet_size as usize];
    buf[0..4].copy_from_slice(&packet_size.to_le_bytes());
    buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
    buf[8..12].copy_from_slice(&VIEW_CMD_NAME_CHECK.to_le_bytes());
    buf[12..28].copy_from_slice(session_hash);

    let name_bytes = name.as_bytes();
    let n = name_bytes.len().min(15);
    buf[32..32 + n].copy_from_slice(&name_bytes[..n]);
    buf
}

fn build_view_register_char(
    race: u8,
    job: u8,
    nation: u8,
    size: u8,
    face: u8,
    skip_intro_cs: u8,
    session_hash: &[u8; 16],
) -> Vec<u8> {
    let packet_size = 0x40u32;
    let mut buf = vec![0u8; packet_size as usize];
    buf[0..4].copy_from_slice(&packet_size.to_le_bytes());
    buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
    buf[8..12].copy_from_slice(&VIEW_CMD_REGISTER_CHAR.to_le_bytes());
    buf[12..28].copy_from_slice(session_hash);
    buf[48] = race;
    buf[50] = job;
    buf[54] = nation;
    buf[57] = size;
    // Byte 58 is unused by retail and ignored by the server's field reads —
    // our kuluu<->LSB extension: 1 = skip the intro cutscene.
    buf[58] = skip_intro_cs;
    buf[60] = face;
    buf
}

// vendor/server/src/login/view_session.cpp view_session::read_func: the ixff
// result reply on success, loginHelpers::generateErrorMessage otherwise.
const VIEW_REPLY_RESULT_SIZE: usize = 0x20;
const VIEW_REPLY_ERROR_SIZE: usize = 0x24;
const VIEW_REPLY_ERROR_RESULT: u8 = 0x04;

async fn read_create_reply(stream: &mut TcpStream, stage: &str) -> Result<()> {
    let mut size_bytes = [0u8; 4];
    stream
        .read_exact(&mut size_bytes)
        .await
        .with_context(|| format!("reading {stage} reply size (server may have closed socket)"))?;
    let size = u32::from_le_bytes(size_bytes) as usize;
    if size != VIEW_REPLY_RESULT_SIZE && size != VIEW_REPLY_ERROR_SIZE {
        bail!("{stage}: implausible reply size {size:#x} (want 0x20 or 0x24)");
    }
    let mut rest = vec![0u8; size - 4];
    stream
        .read_exact(&mut rest)
        .await
        .with_context(|| format!("reading {stage} reply body"))?;

    let term = u32::from_le_bytes(rest[0..4].try_into().unwrap());
    if term != IXFF_TERMINATOR {
        bail!("{stage}: bad terminator {term:#x}");
    }
    let result = rest[4];
    match (size, result) {
        (0x20, 0x03) => Ok(()),
        (0x24, 0x04) => {
            let err = u16::from_le_bytes(rest[28..30].try_into().unwrap());
            bail!(
                "{stage}: server rejected with loginErrors code {err} ({})",
                lobby_error::name(err).unwrap_or("unknown")
            );
        }
        _ => bail!("{stage}: unexpected reply size={size:#x} result={result:#x}"),
    }
}

async fn read_data_charlist(stream: &mut TcpStream) -> Result<(CharList, u32)> {
    let mut buf = vec![0u8; DATA_CHARLIST_SIZE];
    stream
        .read_exact(&mut buf)
        .await
        .context("reading 0xA1 char list")?;
    if buf[0] != DATA_RESP_CHAR_LIST {
        bail!("expected 0x03 char-list response code, got {:#x}", buf[0]);
    }
    let count = buf[1] as usize;
    let mut chars = Vec::with_capacity(count);
    for i in 0..count.min(16) {
        let off = 16 * (i + 1);
        let entry = CharListEntry {
            content_id: u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()),
            char_id_main: u16::from_le_bytes(buf[off + 4..off + 6].try_into().unwrap()),
            world_id: buf[off + 6],
            char_id_extra: buf[off + 7],
        };
        chars.push(entry);
    }
    Ok((CharList { characters: chars }, parse_server_caps(&buf)))
}

fn parse_server_caps(buf: &[u8]) -> u32 {
    let key = u32::from_le_bytes(buf[CAP_KEY_OFFSET..CAP_WORD_OFFSET].try_into().unwrap());
    if key == KULUU_CAP_KEY {
        u32::from_le_bytes(
            buf[CAP_WORD_OFFSET..CAP_WORD_OFFSET + 4]
                .try_into()
                .unwrap(),
        )
    } else {
        0
    }
}

/// Offsets into the lpkt_chr_info2 body (the packet past its 4-byte size
/// field): the slot count sits at the end of the ixff header, then one
/// `chr_info2_sub2` record per populated slot.
const CHR_INFO2_COUNT_OFFSET: usize = 24;
const CHR_INFO2_SLOTS_OFFSET: usize = 28;
const CHR_INFO2_SLOT_SIZE: usize = 140;
const CHR_INFO2_SLOT_NAME_OFFSET: usize = 12;
const CHR_INFO2_SLOT_NAME_LEN: usize = 16;
// vendor/server/src/login/login_packets.h TC_OPERATION_MAKE (mon_no, face_no,
// GrapIDTbl, zone_no2 field widths).
const CHR_INFO2_RACE_MASK: u16 = 0x00FF;
const CHR_INFO2_FACE_MASK: u16 = 0x00FF;
const GRAP_ID_MODEL_MASK: u16 = 0x0FFF;
const GRAP_ID_SLOT_SHIFT: u32 = 12;
const ZONE_NO2_HIGH_BIT: u8 = 0x01;

async fn parse_view_chr_info2(stream: &mut TcpStream) -> Result<Vec<CharSlot>> {
    tracing::debug!("lobby: waiting for chr_info2 packet on view socket");
    let mut size_bytes = [0u8; 4];
    stream
        .read_exact(&mut size_bytes)
        .await
        .context("reading lpkt_chr_info2 size header — server may not be sending chr_info2")?;
    let size = u32::from_le_bytes(size_bytes) as usize;
    if !(IXFF_HEADER_SIZE..=64 * 1024).contains(&size) {
        bail!("implausible lpkt_chr_info2 size {size}");
    }
    let mut rest = vec![0u8; size - 4];
    stream
        .read_exact(&mut rest)
        .await
        .context("reading lpkt_chr_info2 body")?;

    if rest.len() < CHR_INFO2_SLOTS_OFFSET {
        bail!("chr_info2 body too short ({})", rest.len());
    }
    let count = u32::from_le_bytes(
        rest[CHR_INFO2_COUNT_OFFSET..CHR_INFO2_COUNT_OFFSET + 4]
            .try_into()
            .unwrap(),
    ) as usize;

    let needed = CHR_INFO2_SLOTS_OFFSET + count * CHR_INFO2_SLOT_SIZE;
    if rest.len() < needed {
        bail!(
            "chr_info2 body short: have {} bytes, need {} for {} slot(s)",
            rest.len(),
            needed,
            count
        );
    }

    if tracing::enabled!(tracing::Level::TRACE) {
        let total = needed;
        let hex: String = rest[..rest.len().min(total)]
            .chunks(16)
            .enumerate()
            .map(|(i, chunk)| {
                let off = i * 16;
                let bytes: String = chunk.iter().map(|b| format!("{b:02x} ")).collect();
                format!("  {off:04x}: {bytes}")
            })
            .collect::<Vec<_>>()
            .join("\n");
        tracing::trace!(count, total_bytes = rest.len(), "chr_info2 raw:\n{hex}");
    }
    let mut slots = Vec::with_capacity(count);
    for i in 0..count {
        let off = CHR_INFO2_SLOTS_OFFSET + i * CHR_INFO2_SLOT_SIZE;
        let char_id = u32::from_le_bytes(rest[off..off + 4].try_into().unwrap());
        let status = u16::from_le_bytes(rest[off + 8..off + 10].try_into().unwrap());
        let name_bytes = &rest[off + CHR_INFO2_SLOT_NAME_OFFSET
            ..off + CHR_INFO2_SLOT_NAME_OFFSET + CHR_INFO2_SLOT_NAME_LEN];
        let nul = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(CHR_INFO2_SLOT_NAME_LEN);
        let name = String::from_utf8_lossy(&name_bytes[..nul]).into_owned();

        let tc = off + 44;
        let mon_no = u16::from_le_bytes(rest[tc..tc + 2].try_into().unwrap());
        let race = (mon_no & CHR_INFO2_RACE_MASK) as u8;
        let face_u16 = u16::from_le_bytes(rest[tc + 4..tc + 6].try_into().unwrap());
        let face = (face_u16 & CHR_INFO2_FACE_MASK) as u8;
        let grap = |i: usize| -> u16 {
            let o = tc + 12 + i * 2;
            u16::from_le_bytes(rest[o..o + 2].try_into().unwrap())
        };

        let tag = |slot_idx: u16, raw: u16| -> u16 {
            (slot_idx << GRAP_ID_SLOT_SHIFT) | (raw & GRAP_ID_MODEL_MASK)
        };

        let head = tag(1, grap(1));
        let body = tag(2, grap(2));
        let hands = tag(3, grap(3));
        let legs = tag(4, grap(4));
        let feet = tag(5, grap(5));
        let main = tag(6, grap(6));
        let sub = tag(7, grap(7));

        let ranged = 0u16;

        let zone_no = rest[tc + 28];
        let zone_no2 = rest[tc + 34];
        let zone_id = (zone_no as u16) | (((zone_no2 & ZONE_NO2_HIGH_BIT) as u16) << 8);

        slots.push(CharSlot {
            char_id,
            name,
            status,
            race,
            face,
            head,
            body,
            hands,
            legs,
            feet,
            main,
            sub,
            ranged,
            zone_id,
        });
    }
    Ok(slots)
}

/// Field offsets of the 0x0B lpkt_next_login the view socket answers the select
/// with: a 28-byte packet_t header then ffxi_id, ffxi_id_world,
/// character_name[16], server_id, server_ip, server_port
/// (vendor/server/src/login/login_packets.h lpkt_next_login).
const NEXT_LOGIN_CHAR_ID_OFFSET: usize = 28;
const NEXT_LOGIN_NAME_OFFSET: usize = 36;
const NEXT_LOGIN_NAME_LEN: usize = 16;
const NEXT_LOGIN_SERVER_IP_OFFSET: usize = 56;
const NEXT_LOGIN_SERVER_PORT_OFFSET: usize = 60;

async fn read_lpkt_next_login(
    stream: &mut TcpStream,
    char_id: u32,
    key3: &[u8; 20],
) -> Result<MapHandoff> {
    let mut buf = vec![0u8; NEXT_LOGIN_PACKET_SIZE as usize];
    stream
        .read_exact(&mut buf)
        .await
        .context("reading lpkt_next_login")?;

    let packet_size = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    if packet_size != NEXT_LOGIN_PACKET_SIZE {
        bail!(
            "lpkt_next_login: unexpected packet_size {:#x} (want {:#x})",
            packet_size,
            NEXT_LOGIN_PACKET_SIZE
        );
    }
    let term = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    if term != IXFF_TERMINATOR {
        bail!("lpkt_next_login: bad terminator {term:#x}");
    }
    let cmd = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    if cmd != VIEW_RESP_NEXT_LOGIN {
        bail!("lpkt_next_login: unexpected command {cmd:#x}");
    }

    let resp_char_id = u32::from_le_bytes(
        buf[NEXT_LOGIN_CHAR_ID_OFFSET..NEXT_LOGIN_CHAR_ID_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    if resp_char_id != char_id {
        bail!("lpkt_next_login char_id {resp_char_id:#x} != requested {char_id:#x}");
    }

    let name_bytes = &buf[NEXT_LOGIN_NAME_OFFSET..NEXT_LOGIN_NAME_OFFSET + NEXT_LOGIN_NAME_LEN];
    let nul = name_bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(NEXT_LOGIN_NAME_LEN);
    let character_name = String::from_utf8_lossy(&name_bytes[..nul]).into_owned();

    let server_ip = u32::from_le_bytes(
        buf[NEXT_LOGIN_SERVER_IP_OFFSET..NEXT_LOGIN_SERVER_IP_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    let server_port = u32::from_le_bytes(
        buf[NEXT_LOGIN_SERVER_PORT_OFFSET..NEXT_LOGIN_SERVER_PORT_OFFSET + 4]
            .try_into()
            .unwrap(),
    ) as u16;

    Ok(MapHandoff {
        char_id,
        character_name,
        server_ip,
        server_port,
        session_key_seed: *key3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const ROSTER_CHAR_ID: u32 = 0x0100_1234;
    const CHAR_ID_FLIP: u32 = 0xFF;
    const UNOWNED_CHAR_ID: u32 = ROSTER_CHAR_ID ^ CHAR_ID_FLIP;
    const ROSTER_CHAR_NAME: &str = "Bravo";
    const SESSION_HASH: [u8; 16] = [0x5A; 16];
    const KEY3: [u8; 20] = [0x11; 20];
    const HANDOFF_SERVER_IP: u32 = 0x0100_007F;
    const HANDOFF_SERVER_PORT: u16 = ffxi_proto::map::MAP_PORT;
    const DATA_ACK_SELECT: [u8; 5] = [DATA_RESP_SELECT_ACK, 0, 0, 0, 0];

    fn slot(char_id: u32, name: &str) -> CharSlot {
        CharSlot {
            char_id,
            name: name.to_owned(),
            status: 0,
            race: 0,
            face: 0,
            head: 0,
            body: 0,
            hands: 0,
            legs: 0,
            feet: 0,
            main: 0,
            sub: 0,
            ranged: 0,
            zone_id: 0,
        }
    }

    fn select_name(packet: &[u8]) -> String {
        let field =
            &packet[VIEW_SELECT_NAME_OFFSET..VIEW_SELECT_NAME_OFFSET + VIEW_SELECT_NAME_LEN];
        let nul = field
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(VIEW_SELECT_NAME_LEN);
        String::from_utf8_lossy(&field[..nul]).into_owned()
    }

    fn select_char_id(packet: &[u8]) -> u32 {
        u32::from_le_bytes(
            packet[VIEW_SELECT_CHAR_ID_OFFSET..VIEW_SELECT_CHAR_ID_OFFSET + 4]
                .try_into()
                .unwrap(),
        )
    }

    fn chr_info2_packet(slots: &[CharSlot]) -> Vec<u8> {
        let body_len = CHR_INFO2_SLOTS_OFFSET + slots.len() * CHR_INFO2_SLOT_SIZE;
        let mut buf = vec![0u8; 4 + body_len];
        let size = buf.len() as u32;
        buf[0..4].copy_from_slice(&size.to_le_bytes());
        let body = &mut buf[4..];
        body[CHR_INFO2_COUNT_OFFSET..CHR_INFO2_COUNT_OFFSET + 4]
            .copy_from_slice(&(slots.len() as u32).to_le_bytes());
        for (i, s) in slots.iter().enumerate() {
            let off = CHR_INFO2_SLOTS_OFFSET + i * CHR_INFO2_SLOT_SIZE;
            body[off..off + 4].copy_from_slice(&s.char_id.to_le_bytes());
            let name = s.name.as_bytes();
            let n = name.len().min(CHR_INFO2_SLOT_NAME_LEN - 1);
            let name_off = off + CHR_INFO2_SLOT_NAME_OFFSET;
            body[name_off..name_off + n].copy_from_slice(&name[..n]);
        }
        buf
    }

    fn charlist_packet(count: u8) -> Vec<u8> {
        let mut buf = vec![0u8; DATA_CHARLIST_SIZE];
        buf[0] = 0x03;
        buf[1] = count;
        buf
    }

    #[test]
    fn vanilla_charlist_advertises_no_caps() {
        // vendor/server/src/login/data_session.cpp zero-inits uList and only
        // writes [0], [1] and the 16-byte entries — [2..10) stay zero.
        assert_eq!(parse_server_caps(&charlist_packet(2)), 0);
    }

    #[test]
    fn patched_charlist_echoes_key_and_caps() {
        let mut buf = charlist_packet(2);
        buf[CAP_KEY_OFFSET..CAP_WORD_OFFSET].copy_from_slice(&KULUU_CAP_KEY.to_le_bytes());
        let caps = CAP_SKIP_INTRO_CS | (1 << 3);
        buf[CAP_WORD_OFFSET..CAP_WORD_OFFSET + 4].copy_from_slice(&caps.to_le_bytes());
        assert_eq!(parse_server_caps(&buf), caps);
    }

    #[test]
    fn foreign_key_in_cap_slot_is_not_our_echo() {
        let mut buf = charlist_packet(0);
        buf[CAP_KEY_OFFSET..CAP_WORD_OFFSET].copy_from_slice(&999u32.to_le_bytes());
        buf[CAP_WORD_OFFSET..CAP_WORD_OFFSET + 4].copy_from_slice(&7u32.to_le_bytes());
        assert_eq!(parse_server_caps(&buf), 0);
    }

    const FAKE_EXCODE_SERVER: u32 = (expansion_display::RISE_OF_ZILART
        | expansion_display::CHAINS_OF_PROMATHIA
        | expansion_display::BASE_GAME) as u32;

    fn key_packet(excode_server: u32) -> Vec<u8> {
        let mut buf = vec![0u8; KEY_PACKET_SIZE];
        buf[0..4].copy_from_slice(&(KEY_PACKET_SIZE as u32).to_le_bytes());
        buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
        buf[8..12].copy_from_slice(&VIEW_RESP_KEY.to_le_bytes());
        buf[KEY_OFFSET..KEY_OFFSET + 4].copy_from_slice(&0xAD5D_E04Fu32.to_le_bytes());
        buf[KEY_EXCODE_SERVER_OFFSET..KEY_EXCODE_SERVER_OFFSET + 4]
            .copy_from_slice(&excode_server.to_le_bytes());
        buf
    }

    /// research/XiPackets/lobby/C2S_0x0026_RequestLobbyLogin.md example packet
    /// (session-specific bytes replaced): the fields the lobby reads sit where
    /// retail puts them.
    #[test]
    fn lobby_login_matches_the_retail_layout() {
        let hash = [0x48u8; 16];
        let buf = build_lobby_login(&hash, "30220329_2", 0x0FFF);
        assert_eq!(buf.len(), 0x98);
        assert_eq!(&buf[0..4], &[0x98, 0, 0, 0]);
        assert_eq!(&buf[4..8], b"IXFF");
        assert_eq!(&buf[8..12], &[0x26, 0, 0, 0]);
        assert_eq!(&buf[12..28], &hash);
        assert_eq!(&buf[0x1C..0x2C], &[0u8; 16], "pol_account is nulled");
        assert_eq!(&buf[0x2C..0x34], &[5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&buf[0x74..0x7F], b"30220329_2\0");
        assert_eq!(&buf[0x84..0x88], &[0xFF, 0x0F, 0, 0]);
        assert_eq!(&buf[0x88..0x98], &[0u8; 16]);
        // LSB reads six bytes at 0x74 and pads; an overlong stamp cannot spill
        // past the 16-byte field.
        let long = build_lobby_login(&hash, "30220329_2_this_is_too_long", 0);
        assert_eq!(long[0x74 + LOBBY_LOGIN_VERSION_LEN - 1], 0);
        assert_eq!(&long[0x84..0x88], &[0, 0, 0, 0]);
    }

    /// research/XiPackets/lobby/S2C_0x0005_ResponseKey.md example packet.
    #[test]
    fn key_reply_parses_the_retail_example() {
        let mut buf = [0u8; KEY_PACKET_SIZE];
        buf.copy_from_slice(&key_packet(0x0FFF));
        buf[KEY_OFFSET..KEY_OFFSET + 4].copy_from_slice(&[0xBB, 0x87, 0x75, 0xCF]);
        buf[KEY_EXCODE_SERVER2_OFFSET..KEY_EXCODE_SERVER2_OFFSET + 4]
            .copy_from_slice(&[1, 0, 0, 0]);
        let key = parse_key_reply(&buf).unwrap();
        assert_eq!(key.key, 0xCF75_87BB);
        assert_eq!(key.excode_server, 0x0FFF);
        assert_eq!(key.excode_server2, 0x0001);
        assert_eq!(key.expansion_names().len(), 12);
        assert_eq!(key.feature_names(), vec!["SECURE_TOKEN"]);
        buf[KEY_EXCODE_SERVER_OFFSET + 2] = 0x01;
        assert_eq!(
            parse_key_reply(&buf).unwrap().excode_server,
            0x0001_0FFF,
            "the wire field is 32 bits wide; the high half is kept"
        );
        buf[8] = 0x04;
        assert!(parse_key_reply(&buf).is_err());
    }

    /// vendor/server/src/login/view_session.cpp view_session::read_func case
    /// 0x26 under a fatal version lock: generateErrorMessage(GAMES_DATA_HAS_BEEN_UPDATED).
    #[tokio::test]
    async fn version_lock_rejection_is_a_typed_non_retryable_error() {
        use crate::auth_client::is_auth_rejected;
        let view = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let data = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let view_port = view.local_addr().unwrap().port();
        let data_port = data.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut view, _) = view.accept().await.unwrap();
            let (_data, _) = data.accept().await.unwrap();
            let mut login = [0u8; LOBBY_LOGIN_PACKET_SIZE];
            view.read_exact(&mut login).await.unwrap();
            assert_eq!(&login[0x74..0x7E], b"30230905_0");
            let mut err = vec![0u8; VIEW_REPLY_ERROR_SIZE];
            err[0] = VIEW_REPLY_ERROR_SIZE as u8;
            err[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
            err[8] = 0x04;
            err[VIEW_ERROR_CODE_OFFSET..VIEW_ERROR_CODE_OFFSET + 2]
                .copy_from_slice(&lobby_error::GAMES_DATA_HAS_BEEN_UPDATED.to_le_bytes());
            view.write_all(&err).await.unwrap();
        });
        let auth = AuthSession {
            account_id: 1,
            session_hash: [7u8; 16],
        };
        let err = match LobbyClient::new("127.0.0.1", data_port, view_port)
            .with_version_code(Some("30230905_0".into()))
            .open(&auth)
            .await
        {
            Ok(_) => panic!("a version-lock refusal must fail the open"),
            Err(e) => e,
        };
        assert!(is_auth_rejected(&err), "{err:#}");
        assert!(
            err.to_string().contains("GAMES_DATA_HAS_BEEN_UPDATED"),
            "{err:#}"
        );
    }

    fn next_login_packet(char_id: u32, name: &str) -> Vec<u8> {
        let mut buf = vec![0u8; NEXT_LOGIN_PACKET_SIZE as usize];
        buf[0..4].copy_from_slice(&NEXT_LOGIN_PACKET_SIZE.to_le_bytes());
        buf[4..8].copy_from_slice(&IXFF_TERMINATOR.to_le_bytes());
        buf[8..12].copy_from_slice(&VIEW_RESP_NEXT_LOGIN.to_le_bytes());
        buf[NEXT_LOGIN_CHAR_ID_OFFSET..NEXT_LOGIN_CHAR_ID_OFFSET + 4]
            .copy_from_slice(&char_id.to_le_bytes());
        let name_bytes = name.as_bytes();
        let n = name_bytes.len().min(NEXT_LOGIN_NAME_LEN - 1);
        buf[NEXT_LOGIN_NAME_OFFSET..NEXT_LOGIN_NAME_OFFSET + n].copy_from_slice(&name_bytes[..n]);
        buf[NEXT_LOGIN_SERVER_IP_OFFSET..NEXT_LOGIN_SERVER_IP_OFFSET + 4]
            .copy_from_slice(&HANDOFF_SERVER_IP.to_le_bytes());
        buf[NEXT_LOGIN_SERVER_PORT_OFFSET..NEXT_LOGIN_SERVER_PORT_OFFSET + 4]
            .copy_from_slice(&(HANDOFF_SERVER_PORT as u32).to_le_bytes());
        buf
    }

    /// The lobby exchange `handshake` drives, down to the 0x07 select it hands
    /// back for inspection. `None` when the client hung up before selecting.
    async fn fake_lobby(
        view: TcpListener,
        data: TcpListener,
        roster: Vec<CharSlot>,
    ) -> Option<Vec<u8>> {
        let (mut view, _) = view.accept().await.ok()?;
        let (mut data, _) = data.accept().await.ok()?;

        let mut login = [0u8; LOBBY_LOGIN_PACKET_SIZE];
        view.read_exact(&mut login).await.ok()?;
        assert_eq!(&login[8..12], &VIEW_CMD_LOBBY_LOGIN.to_le_bytes());
        view.write_all(&key_packet(FAKE_EXCODE_SERVER)).await.ok()?;
        let mut req_charlist = [0u8; IXFF_HEADER_SIZE];
        data.read_exact(&mut req_charlist).await.ok()?;

        data.write_all(&charlist_packet(roster.len() as u8))
            .await
            .ok()?;
        view.write_all(&chr_info2_packet(&roster)).await.ok()?;

        let mut select = vec![0u8; VIEW_SELECT_PACKET_SIZE as usize];
        view.read_exact(&mut select).await.ok()?;

        data.write_all(&DATA_ACK_SELECT).await.ok()?;
        let mut req_handoff = [0u8; IXFF_HEADER_SIZE];
        data.read_exact(&mut req_handoff).await.ok()?;
        view.write_all(&next_login_packet(
            select_char_id(&select),
            ROSTER_CHAR_NAME,
        ))
        .await
        .ok()?;

        Some(select)
    }

    /// The regression kuluu-3nd2 fixed: `CharSelection::Id` has no name to pass
    /// (session.rs calls `handshake` with the id alone), and LSB closes the
    /// socket when the 0x07's name doesn't match the id
    /// (vendor/server/src/login/view_session.cpp view_session::read_func).
    #[tokio::test]
    async fn handshake_by_id_puts_the_roster_name_on_the_wire() {
        let view = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let data = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let view_port = view.local_addr().unwrap().port();
        let data_port = data.local_addr().unwrap().port();

        let roster = vec![
            slot(UNOWNED_CHAR_ID, "Alpha"),
            slot(ROSTER_CHAR_ID, ROSTER_CHAR_NAME),
        ];
        let server = tokio::spawn(fake_lobby(view, data, roster));

        let auth = AuthSession {
            account_id: 42,
            session_hash: SESSION_HASH,
        };
        let handoff = LobbyClient::new("127.0.0.1", data_port, view_port)
            .handshake(&auth, ROSTER_CHAR_ID, 0, KEY3)
            .await
            .expect("handshake");
        assert_eq!(handoff.char_id, ROSTER_CHAR_ID);
        assert_eq!(handoff.server_port, HANDOFF_SERVER_PORT);

        let select = server
            .await
            .unwrap()
            .expect("the 0x07 select the client sent");
        assert_eq!(select_char_id(&select), ROSTER_CHAR_ID);
        assert_eq!(select_name(&select), ROSTER_CHAR_NAME);
    }

    #[tokio::test]
    async fn handshake_fails_before_the_wire_when_the_account_lacks_the_id() {
        let view = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let data = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let view_port = view.local_addr().unwrap().port();
        let data_port = data.local_addr().unwrap().port();

        let roster = vec![slot(ROSTER_CHAR_ID, ROSTER_CHAR_NAME)];
        let server = tokio::spawn(fake_lobby(view, data, roster));

        let auth = AuthSession {
            account_id: 42,
            session_hash: SESSION_HASH,
        };
        let err = LobbyClient::new("127.0.0.1", data_port, view_port)
            .handshake(&auth, UNOWNED_CHAR_ID, 0, KEY3)
            .await
            .expect_err("an id the account does not own must not reach the 0x07");
        let err = err.to_string();
        assert!(err.contains(&UNOWNED_CHAR_ID.to_string()), "{err}");
        assert!(err.contains(ROSTER_CHAR_NAME), "{err}");
        assert!(
            server.await.unwrap().is_none(),
            "no 0x07 may reach the server for an id the account does not own"
        );
    }

    #[test]
    fn view_select_carries_the_roster_name_where_lsb_reads_it() {
        let chars = [slot(1, "Alpha"), slot(ROSTER_CHAR_ID, ROSTER_CHAR_NAME)];
        let packet = build_view_select_by_id(&chars, ROSTER_CHAR_ID, &SESSION_HASH).unwrap();

        assert_eq!(packet.len(), VIEW_SELECT_PACKET_SIZE as usize);
        assert_eq!(select_char_id(&packet), ROSTER_CHAR_ID);
        assert_eq!(select_name(&packet), ROSTER_CHAR_NAME);
    }

    #[test]
    fn an_unknown_id_reports_the_account_roster() {
        let chars = [slot(1, "Alpha"), slot(2, "Bravo")];
        let err = build_view_select_by_id(&chars, 9, &SESSION_HASH)
            .unwrap_err()
            .to_string();
        assert!(err.contains('9'), "{err}");
        for c in &chars {
            assert!(err.contains(&c.char_id.to_string()), "{err}");
            assert!(err.contains(&c.name), "{err}");
        }
        assert!(build_view_select_by_id(&[], 9, &SESSION_HASH).is_err());
    }
}
