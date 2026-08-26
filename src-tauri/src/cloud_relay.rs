use std::{thread, time::Duration};

use hmac::{Hmac, Mac};
use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

use crate::{
    persistence::{now_ms, PersistenceBridge},
    remote_control::{self, RemoteControlJob, RemoteInstructionRequest},
};

const CREDENTIAL_SERVICE: &str = "Investa.CloudRelay";
const CONFIG_ACCOUNT: &str = "relay-configuration";
const REQUEST_TIMEOUT_SECONDS: u64 = 12;
const MIN_SHARED_SECRET_BYTES: usize = 32;
const MAX_RESULT_CHARS: usize = 12_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRelayConfigRequest {
    pub base_url: String,
    pub device_id: String,
    pub shared_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CloudRelayConfig {
    base_url: String,
    device_id: String,
    shared_secret: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRelayStatus {
    pub configured: bool,
    pub reachable: bool,
    pub base_url: Option<String>,
    pub device_id: Option<String>,
    pub live_order_enabled: bool,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRelayResultRequest {
    pub relay_job_id: String,
    pub local_job_id: String,
    pub status: String,
    pub result_text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudRelayPullResult {
    pub received: bool,
    pub local_job: Option<RemoteControlJob>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayJobEnvelope {
    job: Option<RelayJob>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayJob {
    job_id: String,
    source_request_id: String,
    source_user_id: String,
    source_chat_id: String,
    instruction: String,
    received_at_ms: u64,
}

fn credential_entry() -> Result<Entry, String> {
    Entry::new(CREDENTIAL_SERVICE, CONFIG_ACCOUNT)
        .map_err(|_| "Windows 자격 증명 저장소를 열지 못했습니다.".to_owned())
}

fn load_config() -> Result<Option<CloudRelayConfig>, String> {
    match credential_entry()?.get_password() {
        Ok(value) => serde_json::from_str(&value).map(Some).map_err(|_| {
            "저장된 Cloud relay 설정이 손상되었습니다. 삭제 후 다시 연결해 주세요.".to_owned()
        }),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("Windows 자격 증명 저장소를 읽지 못했습니다.".to_owned()),
    }
}

pub(crate) fn is_configured() -> bool {
    matches!(load_config(), Ok(Some(_)))
}

fn validate_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn validate_base_url(value: &str) -> Result<String, String> {
    let mut url = Url::parse(value.trim())
        .map_err(|_| "Cloud relay 주소 형식을 확인해 주세요.".to_owned())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(
            "Cloud relay는 경로·자격정보가 없는 HTTPS 서비스 주소만 연결할 수 있습니다.".to_owned(),
        );
    }
    url.set_path("");
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn validate_config(request: CloudRelayConfigRequest) -> Result<CloudRelayConfig, String> {
    let base_url = validate_base_url(&request.base_url)?;
    let device_id = request.device_id.trim().to_owned();
    let shared_secret = request.shared_secret.trim().to_owned();
    if !validate_identifier(&device_id)
        || shared_secret.as_bytes().len() < MIN_SHARED_SECRET_BYTES
        || shared_secret.len() > 512
        || !shared_secret.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err("장치 ID와 32바이트 이상의 relay 공유 비밀값을 확인해 주세요.".to_owned());
    }
    Ok(CloudRelayConfig {
        base_url,
        device_id,
        shared_secret,
    })
}

fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .build()
        .map_err(|_| "Cloud relay HTTP 클라이언트를 준비하지 못했습니다.".to_owned())
}

fn canonical_request(timestamp: &str, nonce: &str, method: &str, path: &str, body: &str) -> String {
    let body_hash = format!("{:x}", Sha256::digest(body.as_bytes()));
    format!(
        "{timestamp}\n{nonce}\n{}\n{path}\n{body_hash}",
        method.to_ascii_uppercase()
    )
}

fn request_signature(
    config: &CloudRelayConfig,
    timestamp: &str,
    nonce: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Result<String, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(config.shared_secret.as_bytes())
        .map_err(|_| "Cloud relay 서명 키를 처리하지 못했습니다.".to_owned())?;
    mac.update(canonical_request(timestamp, nonce, method, path, body).as_bytes());
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

async fn signed_post(
    config: &CloudRelayConfig,
    path: &str,
    body: &str,
) -> Result<reqwest::Response, String> {
    let timestamp = now_ms()?.to_string();
    let nonce = Uuid::new_v4().simple().to_string();
    let signature = request_signature(config, &timestamp, &nonce, "POST", path, body)?;
    client()?
        .post(format!("{}{path}", config.base_url))
        .header("content-type", "application/json")
        .header("x-investa-timestamp", timestamp)
        .header("x-investa-nonce", nonce)
        .header("x-investa-signature", signature)
        .body(body.to_owned())
        .send()
        .await
        .map_err(|_| "Cloud relay에 연결하지 못했습니다.".to_owned())
}

fn safe_response_error(status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            "Cloud relay 주소, 공유 비밀값과 서비스 권한을 확인해 주세요.".to_owned()
        }
        StatusCode::CONFLICT => "Cloud relay가 재전송 요청을 안전하게 차단했습니다.".to_owned(),
        StatusCode::TOO_MANY_REQUESTS => {
            "Cloud relay 요청 한도를 초과했습니다. 잠시 후 다시 시도해 주세요.".to_owned()
        }
        _ => "Cloud relay가 요청을 처리하지 못했습니다.".to_owned(),
    }
}

async fn health_status(config: &CloudRelayConfig) -> Result<(), String> {
    let response = client()?
        .get(format!("{}/healthz", config.base_url))
        .send()
        .await
        .map_err(|_| "Cloud relay 상태를 확인하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err("Cloud relay 상태 응답을 확인하지 못했습니다.".to_owned());
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|_| "Cloud relay 상태 응답 형식이 올바르지 않습니다.".to_owned())?;
    if payload.get("ok").and_then(serde_json::Value::as_bool) != Some(true)
        || payload
            .get("liveOrderEnabled")
            .and_then(serde_json::Value::as_bool)
            != Some(false)
    {
        return Err("연결 대상이 Investa 안전 relay인지 확인하지 못했습니다.".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub async fn cloud_relay_status() -> Result<CloudRelayStatus, String> {
    let Some(config) = load_config()? else {
        return Ok(CloudRelayStatus {
            configured: false,
            reachable: false,
            base_url: None,
            device_id: None,
            live_order_enabled: false,
            message: "Google Cloud relay 연결 정보가 없습니다.".to_owned(),
        });
    };
    let reachable = health_status(&config).await.is_ok();
    Ok(CloudRelayStatus {
        configured: true,
        reachable,
        base_url: Some(config.base_url),
        device_id: Some(config.device_id),
        live_order_enabled: false,
        message: if reachable {
            "Cloud relay 연결됨 · HMAC 서명·재전송 차단 · 실전 주문 잠금".to_owned()
        } else {
            "Cloud relay 설정은 저장됐지만 현재 상태 확인에 실패했습니다.".to_owned()
        },
    })
}

#[tauri::command]
pub async fn cloud_relay_save_configuration(
    request: CloudRelayConfigRequest,
) -> Result<CloudRelayStatus, String> {
    let config = validate_config(request)?;
    health_status(&config).await?;
    let serialized = serde_json::to_string(&config)
        .map_err(|_| "Cloud relay 설정을 저장하지 못했습니다.".to_owned())?;
    credential_entry()?.set_password(&serialized).map_err(|_| {
        "Cloud relay 설정을 Windows 자격 증명 관리자에 저장하지 못했습니다.".to_owned()
    })?;
    cloud_relay_status().await
}

#[tauri::command]
pub async fn cloud_relay_delete_configuration() -> Result<CloudRelayStatus, String> {
    match credential_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => cloud_relay_status().await,
        Err(_) => Err("Cloud relay 설정을 삭제하지 못했습니다.".to_owned()),
    }
}

#[tauri::command]
pub async fn cloud_relay_pull_job(
    bridge: State<'_, PersistenceBridge>,
) -> Result<CloudRelayPullResult, String> {
    pull_job(&bridge).await
}

async fn pull_job(bridge: &PersistenceBridge) -> Result<CloudRelayPullResult, String> {
    let config = load_config()?.ok_or_else(|| "Cloud relay 연결 정보가 없습니다.".to_owned())?;
    let body = serde_json::to_string(&serde_json::json!({ "deviceId": config.device_id }))
        .map_err(|_| "Cloud relay 요청을 만들지 못했습니다.".to_owned())?;
    let response = signed_post(&config, "/v1/jobs/pull", &body).await?;
    if !response.status().is_success() {
        return Err(safe_response_error(response.status()));
    }
    let envelope: RelayJobEnvelope = response
        .json()
        .await
        .map_err(|_| "Cloud relay 작업 응답 형식이 올바르지 않습니다.".to_owned())?;
    let Some(job) = envelope.job else {
        return Ok(CloudRelayPullResult {
            received: false,
            local_job: None,
            message: "대기 중인 원격 작업이 없습니다.".to_owned(),
        });
    };
    if !validate_identifier(&job.job_id) || !validate_identifier(&job.source_request_id) {
        return Err("Cloud relay 작업 식별자 형식이 올바르지 않습니다.".to_owned());
    }
    let local_job = remote_control::ingest_cloud_instruction(
        RemoteInstructionRequest {
            source: "cloud_relay".to_owned(),
            source_request_id: job.job_id.clone(),
            source_user_id: job.source_user_id,
            source_chat_id: job.source_chat_id,
            instruction: job.instruction,
            received_at_ms: job.received_at_ms,
            provider_id: Some("local_codex".to_owned()),
        },
        &bridge,
    )?;
    submit_result(
        &config,
        CloudRelayResultRequest {
            relay_job_id: job.job_id,
            local_job_id: local_job.job_id.clone(),
            status: local_job.status.clone(),
            result_text: local_job
                .approval_reason
                .clone()
                .unwrap_or_else(|| "Investa 로컬 작업 큐에 등록했습니다.".to_owned()),
        },
    )
    .await?;
    Ok(CloudRelayPullResult {
        received: true,
        local_job: Some(local_job),
        message: "원격 지시를 로컬 안전 정책으로 검증해 등록했습니다.".to_owned(),
    })
}

async fn submit_result(
    config: &CloudRelayConfig,
    request: CloudRelayResultRequest,
) -> Result<(), String> {
    if !validate_identifier(&request.relay_job_id)
        || !validate_identifier(&request.local_job_id)
        || !matches!(
            request.status.as_str(),
            "queued"
                | "awaiting_local_approval"
                | "approved"
                | "rejected"
                | "cancelled"
                | "completed"
                | "failed"
        )
        || request.result_text.chars().count() > MAX_RESULT_CHARS
    {
        return Err("Cloud relay 결과 형식을 확인해 주세요.".to_owned());
    }
    let relay_status = if request.status == "queued" {
        "accepted"
    } else {
        request.status.as_str()
    };
    let path = format!("/v1/jobs/{}/result", request.relay_job_id);
    let body = serde_json::to_string(&serde_json::json!({
        "deviceId": config.device_id,
        "localJobId": request.local_job_id,
        "status": relay_status,
        "resultText": request.result_text,
    }))
    .map_err(|_| "Cloud relay 결과를 만들지 못했습니다.".to_owned())?;
    let response = signed_post(config, &path, &body).await?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(safe_response_error(response.status()))
    }
}

#[tauri::command]
pub async fn cloud_relay_submit_result(request: CloudRelayResultRequest) -> Result<(), String> {
    let config = load_config()?.ok_or_else(|| "Cloud relay 연결 정보가 없습니다.".to_owned())?;
    submit_result(&config, request).await
}

pub fn start_polling(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(15));
        if !matches!(load_config(), Ok(Some(_))) {
            continue;
        }
        let bridge = app.state::<PersistenceBridge>();
        let _ = tauri::async_runtime::block_on(pull_job(&bridge));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_https_origin_only() {
        assert_eq!(
            validate_base_url("https://relay.example.run.app/").expect("url"),
            "https://relay.example.run.app"
        );
        assert!(validate_base_url("http://relay.example.com").is_err());
        assert!(validate_base_url("https://relay.example.com/path").is_err());
        assert!(validate_base_url("https://user:secret@relay.example.com").is_err());
    }

    #[test]
    fn canonical_signature_matches_relay_contract() {
        let config = CloudRelayConfig {
            base_url: "https://relay.example.com".to_owned(),
            device_id: "desktop-1".to_owned(),
            shared_secret: "a-secure-shared-secret-that-is-long-enough".to_owned(),
        };
        assert_eq!(
            request_signature(
                &config,
                "1787600000000",
                "nonce_1234567890abcdef",
                "POST",
                "/v1/jobs/pull",
                "{\"deviceId\":\"pc-1\"}"
            )
            .expect("signature"),
            "1858eb9ef2222313e500b849471b540fcb5c9c85eb7cc484c1a2170b9c0c5b2b"
        );
    }
}
