use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::persistence::{now_ms, PersistenceBridge};

const MAX_ALLOWED_USERS: usize = 10;
const MAX_INSTRUCTION_CHARS: usize = 4_000;
const MAX_JOBS_LIMIT: u16 = 200;
const MAX_INSTRUCTION_AGE_MS: u64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlPolicyRequest {
    pub enabled: bool,
    pub allowed_user_ids: Vec<String>,
    pub analysis_enabled: bool,
    pub meeting_enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlPolicy {
    pub enabled: bool,
    pub allowed_user_ids: Vec<String>,
    pub analysis_enabled: bool,
    pub meeting_enabled: bool,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInstructionRequest {
    pub source: String,
    pub source_request_id: String,
    pub source_user_id: String,
    pub source_chat_id: String,
    pub instruction: String,
    pub received_at_ms: u64,
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCommandKind {
    Status,
    Analysis,
    Meeting,
    PaperOrderProposal,
    ShadowControl,
    SystemControl,
}

impl RemoteCommandKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Analysis => "analysis",
            Self::Meeting => "meeting",
            Self::PaperOrderProposal => "paper_order_proposal",
            Self::ShadowControl => "shadow_control",
            Self::SystemControl => "system_control",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "status" => Ok(Self::Status),
            "analysis" => Ok(Self::Analysis),
            "meeting" => Ok(Self::Meeting),
            "paper_order_proposal" => Ok(Self::PaperOrderProposal),
            "shadow_control" => Ok(Self::ShadowControl),
            "system_control" => Ok(Self::SystemControl),
            _ => Err("저장된 원격 명령 종류가 올바르지 않습니다.".to_owned()),
        }
    }

    fn requires_local_approval(&self) -> bool {
        matches!(
            self,
            Self::PaperOrderProposal | Self::ShadowControl | Self::SystemControl
        )
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlJob {
    pub job_id: String,
    pub source: String,
    pub source_request_id: String,
    pub source_user_id: String,
    pub source_chat_id: String,
    pub command_kind: RemoteCommandKind,
    pub instruction: String,
    pub status: String,
    pub provider_id: Option<String>,
    pub approval_reason: Option<String>,
    pub received_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlStatus {
    pub engine_ready: bool,
    pub transport_configured: bool,
    pub ai_provider_configured: bool,
    pub live_order_enabled: bool,
    pub policy: RemoteControlPolicy,
    pub queued_count: u64,
    pub approval_count: u64,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteJobActionRequest {
    pub job_id: String,
    pub reason: String,
}

fn valid_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn valid_telegram_id(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && value.len() <= 32 && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn contains_secret_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "api_secret",
        "client_secret",
        "access_token",
        "authorization:",
        "password=",
        "private_key",
        "bot_token",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn normalize_instruction(value: &str) -> Result<String, String> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0
        || count > MAX_INSTRUCTION_CHARS
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        || contains_secret_marker(value)
    {
        return Err(
            "원격 지시는 1~4,000자의 일반 텍스트여야 하며 비밀정보를 포함할 수 없습니다."
                .to_owned(),
        );
    }
    Ok(value.to_owned())
}

fn classify_instruction(value: &str) -> RemoteCommandKind {
    let compact = value.to_ascii_lowercase().replace(char::is_whitespace, "");
    let contains_any = |terms: &[&str]| terms.iter().any(|term| compact.contains(term));

    if contains_any(&[
        "킬스위치",
        "프로그램종료",
        "서버재시작",
        "시스템중지",
        "shutdown",
        "restartserver",
    ]) {
        RemoteCommandKind::SystemControl
    } else if contains_any(&["자동매매", "섀도우", "shadow", "감시시작", "감시중지"])
    {
        RemoteCommandKind::ShadowControl
    } else if contains_any(&[
        "매수",
        "매도",
        "주문",
        "체결",
        "포지션진입",
        "투자해",
        "거래해",
    ]) {
        RemoteCommandKind::PaperOrderProposal
    } else if contains_any(&["회의", "소집", "부서장불러", "보고회"]) {
        RemoteCommandKind::Meeting
    } else if contains_any(&["상태", "현황", "헬스체크", "status", "health"]) {
        RemoteCommandKind::Status
    } else {
        RemoteCommandKind::Analysis
    }
}

fn request_hash(request: &RemoteInstructionRequest, instruction: &str) -> String {
    let mut digest = Sha256::new();
    for value in [
        request.source.as_str(),
        request.source_request_id.as_str(),
        request.source_user_id.as_str(),
        request.source_chat_id.as_str(),
        instruction,
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn load_policy(bridge: &PersistenceBridge) -> Result<RemoteControlPolicy, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let row = connection
        .query_row(
            "SELECT enabled, allowed_user_ids_json, analysis_enabled, meeting_enabled, updated_at_ms FROM remote_control_policy WHERE id = 1",
            [],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?, row.get::<_, bool>(3)?, row.get::<_, u64>(4)?)),
        )
        .map_err(|error| format!("원격운영 정책을 읽지 못했습니다: {error}"))?;
    let allowed_user_ids = serde_json::from_str(&row.1)
        .map_err(|_| "저장된 원격운영 사용자 목록이 손상되었습니다.".to_owned())?;
    Ok(RemoteControlPolicy {
        enabled: row.0,
        allowed_user_ids,
        analysis_enabled: row.2,
        meeting_enabled: row.3,
        updated_at_ms: row.4,
    })
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteControlJob> {
    let command_kind = row.get::<_, String>(5)?;
    Ok(RemoteControlJob {
        job_id: row.get(0)?,
        source: row.get(1)?,
        source_request_id: row.get(2)?,
        source_user_id: row.get(3)?,
        source_chat_id: row.get(4)?,
        command_kind: RemoteCommandKind::parse(&command_kind).map_err(|message| {
            rusqlite::Error::FromSqlConversionFailure(
                5,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                )),
            )
        })?,
        instruction: row.get(6)?,
        status: row.get(7)?,
        provider_id: row.get(8)?,
        approval_reason: row.get(9)?,
        received_at_ms: row.get(10)?,
        updated_at_ms: row.get(11)?,
    })
}

fn find_job(bridge: &PersistenceBridge, job_id: &str) -> Result<RemoteControlJob, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .query_row(
            "SELECT job_id, source, source_request_id, source_user_id, source_chat_id, command_kind, instruction, status, provider_id, approval_reason, received_at_ms, updated_at_ms FROM remote_control_jobs WHERE job_id = ?1",
            params![job_id],
            job_from_row,
        )
        .optional()
        .map_err(|error| format!("원격 작업을 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "원격 작업을 찾지 못했습니다.".to_owned())
}

fn transition_job(
    bridge: &PersistenceBridge,
    job_id: &str,
    from_statuses: &[&str],
    to_status: &str,
    event_type: &str,
    actor: &str,
    reason: &str,
) -> Result<RemoteControlJob, String> {
    if !valid_identifier(job_id, 128) || reason.trim().is_empty() || reason.chars().count() > 1_000
    {
        return Err("작업 식별자와 처리 사유를 확인해 주세요.".to_owned());
    }
    let now = now_ms()?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("원격 작업 상태 변경을 시작하지 못했습니다: {error}"))?;
    let current: Option<String> = transaction
        .query_row(
            "SELECT status FROM remote_control_jobs WHERE job_id = ?1",
            params![job_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("원격 작업 상태를 확인하지 못했습니다: {error}"))?;
    let current = current.ok_or_else(|| "원격 작업을 찾지 못했습니다.".to_owned())?;
    if !from_statuses.contains(&current.as_str()) {
        return Err(format!(
            "현재 {current} 상태에서는 요청한 변경을 수행할 수 없습니다."
        ));
    }
    transaction
        .execute(
            "UPDATE remote_control_jobs SET status = ?2, approval_reason = ?3, updated_at_ms = ?4 WHERE job_id = ?1",
            params![job_id, to_status, reason.trim(), now],
        )
        .map_err(|error| format!("원격 작업 상태를 저장하지 못했습니다: {error}"))?;
    let event_index: u64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM remote_control_job_events WHERE job_id = ?1",
            params![job_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("원격 작업 사건 순서를 확인하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "INSERT INTO remote_control_job_events(job_id, event_index, event_type, actor, detail, occurred_at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![job_id, event_index, event_type, actor, reason.trim(), now],
        )
        .map_err(|error| format!("원격 작업 사건을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("원격 작업 상태를 확정하지 못했습니다: {error}"))?;
    drop(connection);
    find_job(bridge, job_id)
}

#[tauri::command]
pub fn remote_control_policy_save(
    request: RemoteControlPolicyRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RemoteControlPolicy, String> {
    if request.allowed_user_ids.len() > MAX_ALLOWED_USERS
        || request
            .allowed_user_ids
            .iter()
            .any(|value| !valid_telegram_id(value))
        || (request.enabled && request.allowed_user_ids.is_empty())
    {
        return Err("허용할 Telegram 숫자 사용자 ID를 1~10개 입력해 주세요.".to_owned());
    }
    let mut allowed_user_ids = request.allowed_user_ids;
    allowed_user_ids.sort();
    allowed_user_ids.dedup();
    let serialized = serde_json::to_string(&allowed_user_ids)
        .map_err(|_| "원격운영 사용자 목록을 저장할 수 없습니다.".to_owned())?;
    let now = now_ms()?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "UPDATE remote_control_policy SET enabled = ?1, allowed_user_ids_json = ?2, analysis_enabled = ?3, meeting_enabled = ?4, updated_at_ms = ?5 WHERE id = 1",
            params![request.enabled, serialized, request.analysis_enabled, request.meeting_enabled, now],
        )
        .map_err(|error| format!("원격운영 정책을 저장하지 못했습니다: {error}"))?;
    drop(connection);
    load_policy(&bridge)
}

#[tauri::command]
pub fn remote_control_instruction_ingest(
    request: RemoteInstructionRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RemoteControlJob, String> {
    ingest_instruction(&request, &bridge)
}

fn ingest_instruction(
    request: &RemoteInstructionRequest,
    bridge: &PersistenceBridge,
) -> Result<RemoteControlJob, String> {
    if !matches!(
        request.source.as_str(),
        "telegram" | "cloud_relay" | "local_test"
    ) || !valid_identifier(&request.source_request_id, 128)
        || !valid_telegram_id(&request.source_user_id)
        || !valid_telegram_id(&request.source_chat_id)
        || request.received_at_ms == 0
    {
        return Err("원격 지시의 출처와 식별자를 확인해 주세요.".to_owned());
    }
    let current_ms = now_ms()?;
    if request.received_at_ms > current_ms.saturating_add(5 * 60 * 1_000)
        || current_ms.saturating_sub(request.received_at_ms) > MAX_INSTRUCTION_AGE_MS
    {
        return Err("수신 시각이 24시간 범위를 벗어난 원격 지시는 받을 수 없습니다.".to_owned());
    }
    if request
        .provider_id
        .as_ref()
        .is_some_and(|value| !valid_identifier(value, 64))
    {
        return Err("AI 공급자 식별자 형식이 올바르지 않습니다.".to_owned());
    }
    let instruction = normalize_instruction(&request.instruction)?;
    let policy = load_policy(bridge)?;
    if !policy.enabled {
        return Err("원격운영이 로컬 설정에서 비활성화되어 있습니다.".to_owned());
    }
    if !policy.allowed_user_ids.contains(&request.source_user_id) {
        return Err("허용되지 않은 Telegram 사용자입니다.".to_owned());
    }
    let command_kind = classify_instruction(&instruction);
    if command_kind == RemoteCommandKind::Analysis && !policy.analysis_enabled {
        return Err("원격 분석 지시가 비활성화되어 있습니다.".to_owned());
    }
    if command_kind == RemoteCommandKind::Meeting && !policy.meeting_enabled {
        return Err("원격 회의 지시가 비활성화되어 있습니다.".to_owned());
    }
    let hash = request_hash(request, &instruction);
    {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let existing: Option<(String, String)> = connection
            .query_row(
                "SELECT job_id, request_hash FROM remote_control_jobs WHERE source = ?1 AND source_request_id = ?2",
                params![request.source, request.source_request_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("원격 지시 중복 여부를 확인하지 못했습니다: {error}"))?;
        if let Some((job_id, existing_hash)) = existing {
            drop(connection);
            if existing_hash != hash {
                return Err("같은 원격 요청 ID에 다른 내용이 재전송되었습니다.".to_owned());
            }
            return find_job(bridge, &job_id);
        }
    }

    let job_id = format!("remote:{}", Uuid::new_v4().simple());
    let requires_approval = command_kind.requires_local_approval();
    let status = if requires_approval {
        "awaiting_local_approval"
    } else {
        "queued"
    };
    let approval_reason = requires_approval
        .then(|| "투자·자동매매·시스템 제어 지시는 이 PC에서 사용자가 승인해야 합니다.".to_owned());
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("원격 작업 저장을 시작하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "INSERT INTO remote_control_jobs(job_id, source, source_request_id, source_user_id, source_chat_id, request_hash, command_kind, instruction, status, provider_id, approval_reason, received_at_ms, updated_at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![job_id, request.source, request.source_request_id, request.source_user_id, request.source_chat_id, hash, command_kind.as_str(), instruction, status, request.provider_id, approval_reason, request.received_at_ms],
        )
        .map_err(|error| format!("원격 작업을 저장하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "INSERT INTO remote_control_job_events(job_id, event_index, event_type, actor, detail, occurred_at_ms) VALUES(?1, 0, 'received', 'remote_user', '허용 사용자 원격 지시 수신', ?2)",
            params![job_id, request.received_at_ms],
        )
        .map_err(|error| format!("원격 수신 사건을 저장하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "INSERT INTO remote_control_job_events(job_id, event_index, event_type, actor, detail, occurred_at_ms) VALUES(?1, 1, ?2, 'investa_policy', ?3, ?4)",
            params![job_id, if requires_approval { "approval_required" } else { "queued" }, approval_reason.as_deref().unwrap_or("읽기·분석 작업 큐에 등록"), request.received_at_ms],
        )
        .map_err(|error| format!("원격 정책 사건을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("원격 작업을 확정하지 못했습니다: {error}"))?;
    drop(connection);
    find_job(bridge, &job_id)
}

pub(crate) fn ingest_cloud_instruction(
    request: RemoteInstructionRequest,
    bridge: &PersistenceBridge,
) -> Result<RemoteControlJob, String> {
    if request.source != "cloud_relay" {
        return Err("Cloud relay 어댑터는 cloud_relay 출처만 전달할 수 있습니다.".to_owned());
    }
    ingest_instruction(&request, bridge)
}

#[tauri::command]
pub fn remote_control_jobs(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<RemoteControlJob>, String> {
    if limit == 0 || limit > MAX_JOBS_LIMIT {
        return Err("원격 작업 조회 개수는 1~200개여야 합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare("SELECT job_id, source, source_request_id, source_user_id, source_chat_id, command_kind, instruction, status, provider_id, approval_reason, received_at_ms, updated_at_ms FROM remote_control_jobs ORDER BY updated_at_ms DESC, job_id DESC LIMIT ?1")
        .map_err(|error| format!("원격 작업 목록을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit], job_from_row)
        .map_err(|error| format!("원격 작업 목록을 읽지 못했습니다: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("저장된 원격 작업이 손상되었습니다: {error}"))
}

#[tauri::command]
pub fn remote_control_job_approve(
    request: RemoteJobActionRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RemoteControlJob, String> {
    transition_job(
        &bridge,
        &request.job_id,
        &["awaiting_local_approval"],
        "approved",
        "approved",
        "local_user",
        &request.reason,
    )
}

#[tauri::command]
pub fn remote_control_job_reject(
    request: RemoteJobActionRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RemoteControlJob, String> {
    transition_job(
        &bridge,
        &request.job_id,
        &["awaiting_local_approval"],
        "rejected",
        "rejected",
        "local_user",
        &request.reason,
    )
}

#[tauri::command]
pub fn remote_control_job_cancel(
    request: RemoteJobActionRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RemoteControlJob, String> {
    transition_job(
        &bridge,
        &request.job_id,
        &["queued", "awaiting_local_approval", "approved"],
        "cancelled",
        "cancelled",
        "local_user",
        &request.reason,
    )
}

#[tauri::command]
pub fn remote_control_status(
    bridge: State<'_, PersistenceBridge>,
) -> Result<RemoteControlStatus, String> {
    let policy = load_policy(&bridge)?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let queued_count = connection
        .query_row(
            "SELECT COUNT(*) FROM remote_control_jobs WHERE status IN ('queued', 'approved')",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("원격 대기 작업 수를 확인하지 못했습니다: {error}"))?;
    let approval_count = connection
        .query_row(
            "SELECT COUNT(*) FROM remote_control_jobs WHERE status = 'awaiting_local_approval'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("원격 승인 작업 수를 확인하지 못했습니다: {error}"))?;
    let transport_configured = crate::cloud_relay::is_configured();
    Ok(RemoteControlStatus {
        engine_ready: true,
        transport_configured,
        ai_provider_configured: false,
        live_order_enabled: false,
        policy,
        queued_count,
        approval_count,
        message: if transport_configured {
            "원격운영 로컬 엔진·Cloud relay 설정 준비 · 결과 처리 공급자 연결 대기 · 실전 주문 잠금"
                .to_owned()
        } else {
            "원격운영 로컬 엔진 준비 완료 · Telegram Bot/Google Cloud 연결 대기 · 실전 주문 잠금"
                .to_owned()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn bridge() -> (PersistenceBridge, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "investa-remote-control-{}.sqlite3",
            Uuid::new_v4().simple()
        ));
        (PersistenceBridge::open(&path).expect("database"), path)
    }

    fn request(id: &str, text: &str) -> RemoteInstructionRequest {
        RemoteInstructionRequest {
            source: "telegram".to_owned(),
            source_request_id: id.to_owned(),
            source_user_id: "123456789".to_owned(),
            source_chat_id: "123456789".to_owned(),
            instruction: text.to_owned(),
            received_at_ms: now_ms().expect("time"),
            provider_id: None,
        }
    }

    fn enable(bridge: &PersistenceBridge) {
        let now = now_ms().expect("time");
        bridge.connection.lock().expect("lock").execute(
            "UPDATE remote_control_policy SET enabled = 1, allowed_user_ids_json = '[\"123456789\"]', updated_at_ms = ?1 WHERE id = 1",
            params![now],
        ).expect("policy");
    }

    #[test]
    fn classifies_operational_commands_before_general_analysis() {
        assert_eq!(
            classify_instruction("한화 분석해줘"),
            RemoteCommandKind::Analysis
        );
        assert_eq!(
            classify_instruction("부서장 회의 소집"),
            RemoteCommandKind::Meeting
        );
        assert_eq!(
            classify_instruction("BTC 자동매매 시작"),
            RemoteCommandKind::ShadowControl
        );
        assert_eq!(
            classify_instruction("한화 10주 매수해"),
            RemoteCommandKind::PaperOrderProposal
        );
        assert_eq!(
            classify_instruction("서버 재시작"),
            RemoteCommandKind::SystemControl
        );
    }

    #[test]
    fn unauthorized_remote_user_is_rejected() {
        let (bridge, path) = bridge();
        enable(&bridge);
        let mut unauthorized = request("update:1", "한화 분석해줘");
        unauthorized.source_user_id = "999".to_owned();
        let error = ingest_instruction(&unauthorized, &bridge).expect_err("must reject");
        assert!(error.contains("허용되지 않은"));
        drop(bridge);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn analysis_is_queued_but_trading_waits_for_local_approval() {
        let (bridge, path) = bridge();
        enable(&bridge);
        let analysis = ingest_instruction(&request("update:2", "하이닉스 분석해줘"), &bridge)
            .expect("analysis");
        assert_eq!(analysis.status, "queued");
        let trading = ingest_instruction(&request("update:3", "괜찮으면 10주 매수해"), &bridge)
            .expect("trading");
        assert_eq!(trading.status, "awaiting_local_approval");
        assert_eq!(trading.command_kind, RemoteCommandKind::PaperOrderProposal);
        drop(bridge);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn identical_retry_is_idempotent_and_changed_retry_is_rejected() {
        let (bridge, path) = bridge();
        enable(&bridge);
        let first_request = request("update:4", "한화 분석해줘");
        let first = ingest_instruction(&first_request, &bridge).expect("first");
        let retry = ingest_instruction(&first_request, &bridge).expect("retry");
        assert_eq!(first.job_id, retry.job_id);
        let changed = request("update:4", "삼성전자 분석해줘");
        assert!(ingest_instruction(&changed, &bridge).is_err());
        drop(bridge);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn risky_job_can_only_move_through_local_approval_once() {
        let (bridge, path) = bridge();
        enable(&bridge);
        let job =
            ingest_instruction(&request("update:5", "한화 10주 매수해"), &bridge).expect("trading");
        let approved = transition_job(
            &bridge,
            &job.job_id,
            &["awaiting_local_approval"],
            "approved",
            "approved",
            "local_user",
            "내부 모의주문 후보 검토만 승인",
        )
        .expect("approval");
        assert_eq!(approved.status, "approved");
        assert!(transition_job(
            &bridge,
            &job.job_id,
            &["awaiting_local_approval"],
            "approved",
            "approved",
            "local_user",
            "중복 승인",
        )
        .is_err());
        drop(bridge);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn secrets_are_not_accepted_as_remote_instructions() {
        assert!(normalize_instruction("client_secret=do-not-store").is_err());
        assert!(normalize_instruction("일반 분석 요청").is_ok());
        assert!(!valid_telegram_id("-"));
        assert!(valid_telegram_id("-100123456789"));
    }
}
