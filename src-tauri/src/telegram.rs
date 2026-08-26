use std::{
    collections::{HashMap, HashSet},
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use grammers_client::{
    peer::Peer,
    session::{
        types::{ChannelKind, DcOption, PeerId, PeerInfo, UpdateState, UpdatesState},
        BoxFuture, Session, SessionData,
    },
    Client, SenderPool, SignInError,
};
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::persistence::{
    now_ms, PersistenceBridge, TelegramEvidenceItem, TelegramMessageRevision, TelegramSourceRecord,
};

const KEYRING_SERVICE: &str = "com.bumniverse.investa.telegram";
const API_ID_ACCOUNT: &str = "api-id";
const API_HASH_ACCOUNT: &str = "api-hash";
const SESSION_CORE_ACCOUNT: &str = "session-core";
const MAX_CHANNELS: usize = 50;
const MAX_DIALOGS: usize = 500;
const MAX_MESSAGES_PER_CHANNEL: usize = 200;
const MAX_MESSAGE_CHARS: usize = 20_000;
const MAX_EVIDENCE_CHARS: usize = 8_000;
const DEFAULT_EVIDENCE_WINDOW_MS: u64 = 7 * 24 * 60 * 60 * 1_000;

#[derive(Default)]
pub struct TelegramBridge {
    pending: Mutex<Option<PendingAuthorization>>,
}

enum PendingAuthorization {
    Code {
        client: Client,
        session: Arc<SecureSession>,
        token: grammers_client::client::LoginToken,
    },
    Password {
        client: Client,
        session: Arc<SecureSession>,
        token: grammers_client::client::PasswordToken,
    },
}

#[derive(Debug)]
struct SecureSessionError;

impl fmt::Display for SecureSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("secure session lock is unavailable")
    }
}

impl std::error::Error for SecureSessionError {}

struct SecureSession(Mutex<SessionData>);

impl SecureSession {
    fn new(data: SessionData) -> Self {
        Self(Mutex::new(data))
    }

    fn data(&self) -> Result<MutexGuard<'_, SessionData>, SecureSessionError> {
        self.0.lock().map_err(|_| SecureSessionError)
    }

    fn snapshot(&self) -> Result<SessionSnapshot, String> {
        let data = self
            .data()
            .map_err(|_| "텔레그램 세션 상태를 읽지 못했습니다.".to_owned())?;
        Ok(SessionSnapshot {
            home_dc: data.home_dc,
            dc_options: data.dc_options.values().cloned().collect(),
            updates_state: UpdatesState {
                pts: data.updates_state.pts,
                qts: data.updates_state.qts,
                date: data.updates_state.date,
                seq: data.updates_state.seq,
                channels: Vec::new(),
            },
        })
    }
}

impl Session for SecureSession {
    type Error = SecureSessionError;

    fn home_dc_id(&self) -> Result<i32, Self::Error> {
        Ok(self.data()?.home_dc)
    }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            self.data()?.home_dc = dc_id;
            Ok(())
        })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, Self::Error> {
        Ok(self.data()?.dc_options.get(&dc_id).cloned())
    }

    fn set_dc_option(&self, option: &DcOption) -> BoxFuture<'_, Result<(), Self::Error>> {
        let option = option.clone();
        Box::pin(async move {
            self.data()?.dc_options.insert(option.id, option);
            Ok(())
        })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, Self::Error>> {
        Box::pin(async move { Ok(self.data()?.peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), Self::Error>> {
        let peer = peer.clone();
        Box::pin(async move {
            self.data()?.peer_infos.insert(peer.id(), peer);
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, Self::Error>> {
        Box::pin(async move { Ok(self.data()?.updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), Self::Error>> {
        Box::pin(async move {
            let mut data = self.data()?;
            match update {
                UpdateState::All(state) => data.updates_state = state,
                UpdateState::Primary { pts, date, seq } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => data.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    data.updates_state.channels.retain(|state| state.id != id);
                    data.updates_state
                        .channels
                        .push(grammers_client::session::types::ChannelState { id, pts });
                }
            }
            Ok(())
        })
    }
}

#[derive(Serialize, Deserialize)]
struct SessionCore {
    home_dc: i32,
    updates_state: UpdatesState,
    dc_ids: Vec<i32>,
}

struct SessionSnapshot {
    home_dc: i32,
    dc_options: Vec<DcOption>,
    updates_state: UpdatesState,
}

#[derive(Clone)]
struct TelegramCredentials {
    api_id: i32,
    api_hash: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramCredentialsRequest {
    api_id: String,
    api_hash: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginStartRequest {
    phone: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginCodeRequest {
    code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginPasswordRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramSourceSelectionRequest {
    peer_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramEvidenceRequest {
    as_of_ms: Option<u64>,
    since_ms: Option<u64>,
    limit: Option<u16>,
    query: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramConnectionStatus {
    configured: bool,
    session_stored: bool,
    authorized: bool,
    selected_channel_count: usize,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginState {
    stage: &'static str,
    password_hint: Option<String>,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChannel {
    peer_id: i64,
    title: String,
    username: Option<String>,
    selected: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramSyncResult {
    selected_channel_count: usize,
    fetched_message_count: u64,
    inserted_revision_count: u64,
    synced_at_ms: u64,
    message: String,
    channels: Vec<TelegramChannelSyncResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramChannelSyncResult {
    peer_id: i64,
    title: String,
    status: &'static str,
    fetched_message_count: u64,
    inserted_revision_count: u64,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramEvidenceSnapshot {
    provider: &'static str,
    as_of_ms: u64,
    since_ms: u64,
    point_in_time: bool,
    query_terms: Vec<String>,
    total_available_count: usize,
    selected_source_count: usize,
    truncated: bool,
    items: Vec<TelegramEvidenceItem>,
    message: String,
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|_| "Windows 자격 증명 저장소를 열지 못했습니다.".to_owned())
}

fn optional_password(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("Windows 자격 증명 저장소를 읽지 못했습니다.".to_owned()),
    }
}

fn delete_entry(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => {
            Err("Windows 자격 증명 저장소에서 텔레그램 정보를 삭제하지 못했습니다.".to_owned())
        }
    }
}

fn validate_credentials(
    request: TelegramCredentialsRequest,
) -> Result<TelegramCredentials, String> {
    let api_id = request
        .api_id
        .trim()
        .parse::<i32>()
        .map_err(|_| "Telegram API ID는 양의 정수여야 합니다.".to_owned())?;
    let api_hash = request.api_hash.trim().to_ascii_lowercase();
    if api_id <= 0 || api_hash.len() != 32 || !api_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Telegram API ID와 32자리 API Hash 형식을 확인해 주세요.".to_owned());
    }
    Ok(TelegramCredentials { api_id, api_hash })
}

fn load_credentials() -> Result<Option<TelegramCredentials>, String> {
    let api_id = optional_password(API_ID_ACCOUNT)?;
    let api_hash = optional_password(API_HASH_ACCOUNT)?;
    match (api_id, api_hash) {
        (None, None) => Ok(None),
        (Some(api_id), Some(api_hash)) => {
            validate_credentials(TelegramCredentialsRequest { api_id, api_hash }).map(Some)
        }
        _ => Err("텔레그램 API 자격정보가 불완전합니다. 삭제 후 다시 등록해 주세요.".to_owned()),
    }
}

fn save_credentials(credentials: &TelegramCredentials) -> Result<(), String> {
    entry(API_ID_ACCOUNT)?
        .set_password(&credentials.api_id.to_string())
        .map_err(|_| "Telegram API ID를 저장하지 못했습니다.".to_owned())?;
    if entry(API_HASH_ACCOUNT)?
        .set_password(&credentials.api_hash)
        .is_err()
    {
        let _ = delete_entry(API_ID_ACCOUNT);
        return Err("Telegram API Hash를 저장하지 못했습니다.".to_owned());
    }
    Ok(())
}

fn session_dc_account(dc_id: i32) -> String {
    format!("session-dc-{dc_id}")
}

fn save_session(session: &SecureSession) -> Result<(), String> {
    let snapshot = session.snapshot()?;
    let dc_ids = snapshot.dc_options.iter().map(|option| option.id).collect();
    let core = SessionCore {
        home_dc: snapshot.home_dc,
        updates_state: snapshot.updates_state,
        dc_ids,
    };
    let core_json = serde_json::to_string(&core)
        .map_err(|_| "텔레그램 세션 메타데이터를 직렬화하지 못했습니다.".to_owned())?;
    for option in &snapshot.dc_options {
        let json = serde_json::to_string(option)
            .map_err(|_| "텔레그램 데이터센터 세션을 직렬화하지 못했습니다.".to_owned())?;
        entry(&session_dc_account(option.id))?
            .set_password(&json)
            .map_err(|_| "텔레그램 데이터센터 세션을 저장하지 못했습니다.".to_owned())?;
    }
    entry(SESSION_CORE_ACCOUNT)?
        .set_password(&core_json)
        .map_err(|_| "텔레그램 세션을 저장하지 못했습니다.".to_owned())
}

fn load_session() -> Result<Option<SessionData>, String> {
    let Some(core_json) = optional_password(SESSION_CORE_ACCOUNT)? else {
        return Ok(None);
    };
    let core: SessionCore = serde_json::from_str(&core_json)
        .map_err(|_| "저장된 텔레그램 세션 메타데이터가 손상되었습니다.".to_owned())?;
    let mut data = SessionData::default();
    data.home_dc = core.home_dc;
    data.updates_state = core.updates_state;
    for dc_id in core.dc_ids {
        let json = optional_password(&session_dc_account(dc_id))?
            .ok_or_else(|| "저장된 텔레그램 데이터센터 세션이 불완전합니다.".to_owned())?;
        let option: DcOption = serde_json::from_str(&json)
            .map_err(|_| "저장된 텔레그램 데이터센터 세션이 손상되었습니다.".to_owned())?;
        data.dc_options.insert(option.id, option);
    }
    Ok(Some(data))
}

fn delete_session() -> Result<(), String> {
    if let Some(core_json) = optional_password(SESSION_CORE_ACCOUNT)? {
        if let Ok(core) = serde_json::from_str::<SessionCore>(&core_json) {
            for dc_id in core.dc_ids {
                delete_entry(&session_dc_account(dc_id))?;
            }
        }
    }
    // 코어 메타데이터가 손상돼도 기본 Telegram DC 자격정보가 남지 않게 정리한다.
    for dc_id in 1..=5 {
        delete_entry(&session_dc_account(dc_id))?;
    }
    delete_entry(SESSION_CORE_ACCOUNT)
}

fn query_terms(query: Option<&str>) -> Vec<String> {
    let mut terms = query
        .unwrap_or_default()
        .split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|value| value.chars().count() >= 2)
        .map(|value| value.to_uppercase())
        .filter(|value| {
            !matches!(
                value.as_str(),
                "분석" | "주가" | "종목" | "투자" | "매수" | "매도" | "해줘" | "해주세요"
            ) && !value.ends_with("해줘")
                && !value.ends_with("해주세요")
        })
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    terms.truncate(12);
    terms
}

fn select_relevant_evidence(
    mut items: Vec<TelegramEvidenceItem>,
    terms: &[String],
    limit: usize,
) -> (Vec<TelegramEvidenceItem>, bool) {
    let total = items.len();
    if !terms.is_empty() {
        items.sort_by(|left, right| {
            let score = |item: &TelegramEvidenceItem| {
                let haystack = format!("{} {}", item.source_title, item.text).to_uppercase();
                terms
                    .iter()
                    .map(|term| haystack.matches(term).count())
                    .sum::<usize>()
            };
            score(right)
                .cmp(&score(left))
                .then_with(|| right.posted_at_ms.cmp(&left.posted_at_ms))
                .then_with(|| right.message_id.cmp(&left.message_id))
        });
    }

    let mut selected = Vec::new();
    let mut used_chars = 0_usize;
    for mut item in items.into_iter().take(limit) {
        let remaining = MAX_EVIDENCE_CHARS.saturating_sub(used_chars);
        if remaining == 0 {
            break;
        }
        let original_chars = item.text.chars().count();
        if original_chars > remaining {
            item.text = item.text.chars().take(remaining).collect();
        }
        used_chars += item.text.chars().count();
        selected.push(item);
    }
    let truncated = selected.len() < total
        || selected
            .iter()
            .map(|item| item.text.chars().count())
            .sum::<usize>()
            >= MAX_EVIDENCE_CHARS;
    (selected, truncated)
}

fn validate_phone(phone: &str) -> Result<String, String> {
    let phone = phone.trim().replace([' ', '-'], "");
    if !(8..=16).contains(&phone.len())
        || !phone.starts_with('+')
        || !phone[1..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("전화번호는 국가번호를 포함한 +8210… 형식으로 입력해 주세요.".to_owned());
    }
    Ok(phone)
}

fn pending_lock(
    bridge: &TelegramBridge,
) -> Result<MutexGuard<'_, Option<PendingAuthorization>>, String> {
    bridge
        .pending
        .lock()
        .map_err(|_| "텔레그램 인증 상태 잠금을 획득하지 못했습니다.".to_owned())
}

fn start_client(
    credentials: &TelegramCredentials,
    data: SessionData,
) -> (Client, Arc<SecureSession>) {
    let session = Arc::new(SecureSession::new(data));
    let pool = SenderPool::new(Arc::clone(&session), credentials.api_id);
    let client = Client::new(pool.handle);
    tauri::async_runtime::spawn(async move {
        let _ = pool.runner.run().await;
    });
    (client, session)
}

async fn authorized_client() -> Result<(Client, Arc<SecureSession>), String> {
    let credentials = load_credentials()?
        .ok_or_else(|| "먼저 Telegram API ID와 API Hash를 저장해 주세요.".to_owned())?;
    let data = load_session()?.ok_or_else(|| "텔레그램 계정 인증이 필요합니다.".to_owned())?;
    let (client, session) = start_client(&credentials, data);
    if !client
        .is_authorized()
        .await
        .map_err(|_| "저장된 텔레그램 세션을 확인하지 못했습니다.".to_owned())?
    {
        return Err("텔레그램 세션이 만료되었습니다. 다시 인증해 주세요.".to_owned());
    }
    Ok((client, session))
}

async fn broadcast_channels(
    selected_ids: &[i64],
) -> Result<
    (
        Vec<TelegramChannel>,
        HashMap<i64, grammers_client::session::types::PeerRef>,
    ),
    String,
> {
    let (client, session) = authorized_client().await?;
    let mut dialogs = client.iter_dialogs();
    let mut channels = Vec::new();
    let mut refs = HashMap::new();
    while channels.len() < MAX_DIALOGS {
        let Some(dialog) = dialogs
            .next()
            .await
            .map_err(|_| "텔레그램 채널 목록을 불러오지 못했습니다.".to_owned())?
        else {
            break;
        };
        let Peer::Channel(channel) = dialog.peer() else {
            continue;
        };
        if channel.kind() != Some(ChannelKind::Broadcast) {
            continue;
        }
        let peer_id = dialog
            .peer_id()
            .bot_api_dialog_id()
            .ok_or_else(|| "텔레그램 채널 식별자를 확인하지 못했습니다.".to_owned())?;
        refs.insert(peer_id, dialog.peer_ref());
        channels.push(TelegramChannel {
            peer_id,
            title: channel.title().chars().take(256).collect(),
            username: channel
                .username()
                .map(|value| value.chars().take(64).collect()),
            selected: selected_ids.contains(&peer_id),
        });
    }
    save_session(&session)?;
    Ok((channels, refs))
}

#[tauri::command]
pub async fn telegram_connection_status(
    persistence: State<'_, PersistenceBridge>,
) -> Result<TelegramConnectionStatus, String> {
    let configured = load_credentials()?.is_some();
    let session_stored = load_session()?.is_some();
    let selected_channel_count = persistence.telegram_sources()?.len();
    let authorized = if session_stored {
        authorized_client().await.is_ok()
    } else {
        false
    };
    Ok(TelegramConnectionStatus {
        configured,
        session_stored,
        authorized,
        selected_channel_count,
        message: if authorized {
            format!("읽기 전용 세션 저장됨 · 채널 {selected_channel_count}개 선택")
        } else if session_stored {
            "저장된 세션을 확인하지 못했습니다. Telegram 연결 상태를 확인하거나 다시 인증해 주세요."
                .to_owned()
        } else if configured {
            "API 자격정보 저장됨 · 계정 인증 필요".to_owned()
        } else {
            "Telegram API ID와 API Hash가 필요합니다.".to_owned()
        },
    })
}

#[tauri::command]
pub fn telegram_save_credentials(
    request: TelegramCredentialsRequest,
) -> Result<TelegramConnectionStatus, String> {
    let credentials = validate_credentials(request)?;
    save_credentials(&credentials)?;
    Ok(TelegramConnectionStatus {
        configured: true,
        session_stored: load_session()?.is_some(),
        authorized: false,
        selected_channel_count: 0,
        message: "API 자격정보를 Windows 자격 증명 관리자에 저장했습니다.".to_owned(),
    })
}

#[tauri::command]
pub async fn telegram_login_start(
    bridge: State<'_, TelegramBridge>,
    request: TelegramLoginStartRequest,
) -> Result<TelegramLoginState, String> {
    let credentials = load_credentials()?
        .ok_or_else(|| "먼저 Telegram API ID와 API Hash를 저장해 주세요.".to_owned())?;
    let phone = validate_phone(&request.phone)?;
    let data = load_session()?.unwrap_or_default();
    let (client, session) = start_client(&credentials, data);
    if client
        .is_authorized()
        .await
        .map_err(|_| "텔레그램 인증 상태를 확인하지 못했습니다.".to_owned())?
    {
        save_session(&session)?;
        return Ok(TelegramLoginState {
            stage: "authorized",
            password_hint: None,
            message: "이미 인증된 텔레그램 세션입니다.".to_owned(),
        });
    }
    let token = client
        .request_login_code(&phone, &credentials.api_hash)
        .await
        .map_err(|_| {
            "텔레그램 인증 코드를 요청하지 못했습니다. 잠시 후 다시 시도해 주세요.".to_owned()
        })?;
    *pending_lock(&bridge)? = Some(PendingAuthorization::Code {
        client,
        session,
        token,
    });
    Ok(TelegramLoginState {
        stage: "code",
        password_hint: None,
        message: "Telegram 앱으로 전송된 인증 코드를 입력해 주세요.".to_owned(),
    })
}

#[tauri::command]
pub async fn telegram_login_code(
    bridge: State<'_, TelegramBridge>,
    request: TelegramLoginCodeRequest,
) -> Result<TelegramLoginState, String> {
    let code = request.code.trim().to_owned();
    if !(4..=8).contains(&code.len()) || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("텔레그램 인증 코드 형식을 확인해 주세요.".to_owned());
    }
    let pending = pending_lock(&bridge)?
        .take()
        .ok_or_else(|| "먼저 인증 코드를 요청해 주세요.".to_owned())?;
    let PendingAuthorization::Code {
        client,
        session,
        token,
    } = pending
    else {
        return Err("현재는 2단계 인증 비밀번호 입력 단계입니다.".to_owned());
    };
    match client.sign_in(&token, &code).await {
        Ok(_) => {
            save_session(&session)?;
            Ok(TelegramLoginState {
                stage: "authorized",
                password_hint: None,
                message: "텔레그램 읽기 전용 세션을 안전하게 저장했습니다.".to_owned(),
            })
        }
        Err(SignInError::PasswordRequired(token)) => {
            let hint = token.hint().map(str::to_owned);
            *pending_lock(&bridge)? = Some(PendingAuthorization::Password {
                client,
                session,
                token,
            });
            Ok(TelegramLoginState {
                stage: "password",
                password_hint: hint,
                message: "Telegram 2단계 인증 비밀번호를 입력해 주세요.".to_owned(),
            })
        }
        Err(SignInError::InvalidCode) => {
            Err("텔레그램 인증 코드가 올바르지 않습니다. 새 코드를 요청해 주세요.".to_owned())
        }
        Err(_) => Err("텔레그램 계정 인증을 완료하지 못했습니다.".to_owned()),
    }
}

#[tauri::command]
pub async fn telegram_login_password(
    bridge: State<'_, TelegramBridge>,
    request: TelegramLoginPasswordRequest,
) -> Result<TelegramLoginState, String> {
    if request.password.is_empty() || request.password.len() > 256 {
        return Err("텔레그램 2단계 인증 비밀번호를 확인해 주세요.".to_owned());
    }
    let pending = pending_lock(&bridge)?
        .take()
        .ok_or_else(|| "먼저 인증 코드 확인을 완료해 주세요.".to_owned())?;
    let PendingAuthorization::Password {
        client,
        session,
        token,
    } = pending
    else {
        return Err("현재는 인증 코드 입력 단계입니다.".to_owned());
    };
    match client.check_password(token, request.password).await {
        Ok(_) => {
            save_session(&session)?;
            Ok(TelegramLoginState {
                stage: "authorized",
                password_hint: None,
                message: "텔레그램 읽기 전용 세션을 안전하게 저장했습니다.".to_owned(),
            })
        }
        Err(SignInError::InvalidPassword(token)) => {
            let hint = token.hint().map(str::to_owned);
            *pending_lock(&bridge)? = Some(PendingAuthorization::Password {
                client,
                session,
                token,
            });
            Err(format!(
                "2단계 인증 비밀번호가 올바르지 않습니다.{}",
                hint.map(|value| format!(" 힌트: {value}"))
                    .unwrap_or_default()
            ))
        }
        Err(_) => Err("텔레그램 2단계 인증을 완료하지 못했습니다.".to_owned()),
    }
}

#[tauri::command]
pub async fn telegram_channels(
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<TelegramChannel>, String> {
    let selected_ids = persistence
        .telegram_sources()?
        .into_iter()
        .map(|source| source.peer_id)
        .collect::<Vec<_>>();
    Ok(broadcast_channels(&selected_ids).await?.0)
}

#[tauri::command]
pub async fn telegram_select_channels(
    persistence: State<'_, PersistenceBridge>,
    request: TelegramSourceSelectionRequest,
) -> Result<Vec<TelegramChannel>, String> {
    if request.peer_ids.len() > MAX_CHANNELS {
        return Err(format!(
            "텔레그램 뉴스 채널은 최대 {MAX_CHANNELS}개까지 선택할 수 있습니다."
        ));
    }
    let mut requested = request.peer_ids;
    requested.sort_unstable();
    requested.dedup();
    let (channels, _) = broadcast_channels(&requested).await?;
    let available = channels
        .iter()
        .map(|channel| channel.peer_id)
        .collect::<Vec<_>>();
    if requested.iter().any(|peer_id| !available.contains(peer_id)) {
        return Err("현재 계정의 방송 채널만 선택할 수 있습니다.".to_owned());
    }
    let updated_at_ms = now_ms()?;
    let selected = channels
        .iter()
        .filter(|channel| requested.contains(&channel.peer_id))
        .map(|channel| TelegramSourceRecord {
            peer_id: channel.peer_id,
            title: channel.title.clone(),
            username: channel.username.clone(),
            enabled: true,
            last_message_id: None,
            updated_at_ms,
        })
        .collect::<Vec<_>>();
    persistence.replace_telegram_sources(&selected)?;
    Ok(channels
        .into_iter()
        .map(|mut channel| {
            channel.selected = requested.contains(&channel.peer_id);
            channel
        })
        .collect())
}

#[tauri::command]
pub async fn telegram_sync_selected(
    persistence: State<'_, PersistenceBridge>,
) -> Result<TelegramSyncResult, String> {
    let sources = persistence.telegram_sources()?;
    if sources.is_empty() {
        return Err("먼저 수집할 텔레그램 뉴스 채널을 선택해 주세요.".to_owned());
    }
    let selected_ids = sources
        .iter()
        .map(|source| source.peer_id)
        .collect::<Vec<_>>();
    let (_, refs) = broadcast_channels(&selected_ids).await?;
    let (client, session) = authorized_client().await?;
    let synced_at_ms = now_ms()?;
    let mut fetched_message_count = 0_u64;
    let mut inserted_revision_count = 0_u64;
    let mut channel_results = Vec::with_capacity(sources.len());
    for source in &sources {
        let Some(peer) = refs.get(&source.peer_id).copied() else {
            channel_results.push(TelegramChannelSyncResult {
                peer_id: source.peer_id,
                title: source.title.clone(),
                status: "failed",
                fetched_message_count: 0,
                inserted_revision_count: 0,
                error: Some("현재 Telegram 계정에서 채널을 찾지 못했습니다.".to_owned()),
            });
            continue;
        };
        let mut iterator = client.iter_messages(peer).limit(MAX_MESSAGES_PER_CHANNEL);
        let mut revisions = Vec::new();
        let mut highest_message_id = source.last_message_id;
        let mut channel_error = None;
        loop {
            let message = match iterator.next().await {
                Ok(Some(message)) => message,
                Ok(None) => break,
                Err(_) => {
                    channel_error = Some("채널 메시지를 불러오지 못했습니다.".to_owned());
                    break;
                }
            };
            if source
                .last_message_id
                .is_some_and(|last| message.id() < last)
            {
                break;
            }
            let text = message.text().trim();
            if text.is_empty() {
                continue;
            }
            let text = text.chars().take(MAX_MESSAGE_CHARS).collect::<String>();
            let posted_at_ms = message.date().timestamp_millis().max(0) as u64;
            let edited_at_ms = message
                .edit_date()
                .map(|date| date.timestamp_millis().max(0) as u64);
            let content_hash = format!("{:x}", Sha256::digest(text.as_bytes()));
            highest_message_id =
                Some(highest_message_id.map_or(message.id(), |last| last.max(message.id())));
            revisions.push((message.id(), posted_at_ms, edited_at_ms, content_hash, text));
        }
        if let Some(error) = channel_error {
            channel_results.push(TelegramChannelSyncResult {
                peer_id: source.peer_id,
                title: source.title.clone(),
                status: "failed",
                fetched_message_count: revisions.len() as u64,
                inserted_revision_count: 0,
                error: Some(error),
            });
            continue;
        }
        let channel_fetched = revisions.len() as u64;
        fetched_message_count += channel_fetched;
        let records = revisions
            .iter()
            .map(
                |(message_id, posted_at_ms, edited_at_ms, content_hash, text)| {
                    TelegramMessageRevision {
                        peer_id: source.peer_id,
                        message_id: *message_id,
                        posted_at_ms: *posted_at_ms,
                        edited_at_ms: *edited_at_ms,
                        ingested_at_ms: synced_at_ms,
                        content_hash,
                        text,
                    }
                },
            )
            .collect::<Vec<_>>();
        let channel_inserted = persistence.persist_telegram_revisions(
            source.peer_id,
            &records,
            highest_message_id,
            synced_at_ms,
        )?;
        inserted_revision_count += channel_inserted;
        channel_results.push(TelegramChannelSyncResult {
            peer_id: source.peer_id,
            title: source.title.clone(),
            status: "synced",
            fetched_message_count: channel_fetched,
            inserted_revision_count: channel_inserted,
            error: None,
        });
    }
    save_session(&session)?;
    let failed_count = channel_results
        .iter()
        .filter(|result| result.status == "failed")
        .count();
    let synced_count = channel_results.len().saturating_sub(failed_count);
    Ok(TelegramSyncResult {
        selected_channel_count: sources.len(),
        fetched_message_count,
        inserted_revision_count,
        synced_at_ms,
        message: format!(
            "채널 {}개 동기화, {}개 실패 · 메시지 {}건 확인 · 새 리비전 {}건 저장",
            synced_count, failed_count, fetched_message_count, inserted_revision_count
        ),
        channels: channel_results,
    })
}

#[tauri::command]
pub fn telegram_evidence_snapshot(
    persistence: State<'_, PersistenceBridge>,
    request: TelegramEvidenceRequest,
) -> Result<TelegramEvidenceSnapshot, String> {
    let as_of_ms = request.as_of_ms.unwrap_or(now_ms()?);
    let since_ms = request
        .since_ms
        .unwrap_or(as_of_ms.saturating_sub(DEFAULT_EVIDENCE_WINDOW_MS));
    let limit = request.limit.unwrap_or(30).clamp(1, 50) as usize;
    let terms = query_terms(request.query.as_deref());
    let available = persistence.telegram_evidence(as_of_ms, since_ms, 100)?;
    let total_available_count = available.len();
    let (items, truncated) = select_relevant_evidence(available, &terms, limit);
    let selected_source_count = items
        .iter()
        .map(|item| item.peer_id)
        .collect::<HashSet<_>>()
        .len();
    Ok(TelegramEvidenceSnapshot {
        provider: "TELEGRAM_USER_SELECTED_CHANNELS",
        as_of_ms,
        since_ms,
        point_in_time: true,
        query_terms: terms,
        total_available_count,
        selected_source_count,
        truncated,
        message: if items.is_empty() {
            "기준 시각 범위에 저장된 텔레그램 뉴스가 없습니다.".to_owned()
        } else {
            format!("선택 채널의 시점 기준 뉴스 {}건", items.len())
        },
        items,
    })
}

#[tauri::command]
pub fn telegram_delete_connection(
    bridge: State<'_, TelegramBridge>,
) -> Result<TelegramConnectionStatus, String> {
    *pending_lock(&bridge)? = None;
    delete_session()?;
    delete_entry(API_ID_ACCOUNT)?;
    delete_entry(API_HASH_ACCOUNT)?;
    Ok(TelegramConnectionStatus {
        configured: false,
        session_stored: false,
        authorized: false,
        selected_channel_count: 0,
        message: "텔레그램 API 자격정보와 로그인 세션을 삭제했습니다.".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_api_credentials_without_accepting_whitespace_or_non_hex_hash() {
        assert!(validate_credentials(TelegramCredentialsRequest {
            api_id: "123456".to_owned(),
            api_hash: "0123456789abcdef0123456789abcdef".to_owned(),
        })
        .is_ok());
        assert!(validate_credentials(TelegramCredentialsRequest {
            api_id: "0".to_owned(),
            api_hash: "0123456789abcdef0123456789abcdef".to_owned(),
        })
        .is_err());
        assert!(validate_credentials(TelegramCredentialsRequest {
            api_id: "123456".to_owned(),
            api_hash: "not-a-valid-api-hash".to_owned(),
        })
        .is_err());
    }

    #[test]
    fn accepts_only_international_phone_numbers() {
        assert_eq!(validate_phone("+82 10-1234-5678").unwrap(), "+821012345678");
        assert!(validate_phone("01012345678").is_err());
        assert!(validate_phone("+82abc").is_err());
    }

    fn evidence(message_id: i32, posted_at_ms: u64, text: &str) -> TelegramEvidenceItem {
        TelegramEvidenceItem {
            peer_id: i64::from(message_id),
            source_title: "테스트 채널".to_owned(),
            source_username: None,
            message_id,
            posted_at_ms,
            edited_at_ms: None,
            ingested_at_ms: posted_at_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn extracts_specific_query_terms_and_removes_generic_instructions() {
        assert_eq!(
            query_terms(Some("한화 한화 주가 분석해줘 NASDAQ")),
            vec!["NASDAQ".to_owned(), "한화".to_owned()]
        );
    }

    #[test]
    fn prioritizes_relevant_evidence_before_recent_unrelated_messages() {
        let (selected, truncated) = select_relevant_evidence(
            vec![
                evidence(1, 300, "금리 전망"),
                evidence(2, 100, "한화 실적 발표"),
                evidence(3, 200, "환율 동향"),
            ],
            &["한화".to_owned()],
            2,
        );
        assert_eq!(selected[0].message_id, 2);
        assert_eq!(selected[1].message_id, 1);
        assert!(truncated);
    }
}
