use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::State;
#[cfg(test)]
use uuid::Uuid;

use crate::{
    engine_runtime::EngineRunRequest,
    governance::{
        acknowledge_alert, build_health_report, create_consistent_sqlite_backup, merge_alert,
        validate_audit_event, AlertSeverity, AuditEvent, HealthComponent, HealthReport,
        OperationalAlert,
    },
    paper_account::{execute_shadow_order, AppendOnlyLedger, LedgerEvent, ShadowOrderRequest},
    paper_trading::{self, ledger_id_for_currency, PaperAccountSnapshot},
    persistence::{now_ms, PersistenceBridge, SCHEMA_VERSION},
    trading::TradeSide,
};

const ALERT_DEDUPLICATION_WINDOW_MS: u64 = 15 * 60 * 1_000;
const PROVIDER_HEALTH_MAXIMUM_AGE_MS: u64 = 5 * 60 * 1_000;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineCandidateStatus {
    SafetyApproved,
    UserApproved,
    Submitted,
    Filled,
    Rejected,
    Cancelled,
    Expired,
}

impl EngineCandidateStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::SafetyApproved => "safety_approved",
            Self::UserApproved => "user_approved",
            Self::Submitted => "submitted",
            Self::Filled => "filled",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "safety_approved" => Ok(Self::SafetyApproved),
            "user_approved" => Ok(Self::UserApproved),
            "submitted" => Ok(Self::Submitted),
            "filled" => Ok(Self::Filled),
            "rejected" => Ok(Self::Rejected),
            "cancelled" => Ok(Self::Cancelled),
            "expired" => Ok(Self::Expired),
            _ => Err("저장된 엔진 주문 후보 상태를 해석하지 못했습니다.".to_owned()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineOrderCandidate {
    pub candidate_id: String,
    pub run_id: String,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub side: TradeSide,
    pub quantity: u64,
    pub quantity_scale: u64,
    pub reference_price_minor: u64,
    pub valid_until_ms: u64,
    pub status: EngineCandidateStatus,
    pub safety: Value,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCandidateCreateRequest {
    pub run_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineCandidateActionRequest {
    pub candidate_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationReport {
    pub checked_count: usize,
    pub repaired_count: usize,
    pub mismatch_count: usize,
    pub live_order_enabled: bool,
    pub state: ReconciliationState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationState {
    pub status: String,
    pub required_since_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub mismatch_count: usize,
    pub detail: String,
    pub candidate_actions_locked: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertAcknowledgeRequest {
    pub alert_id: String,
    pub response: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderHealthRecordRequest {
    pub component_id: String,
    pub critical: bool,
    pub healthy: bool,
    pub retry_action: String,
    pub detail: String,
    pub observed_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupReceipt {
    pub file_name: String,
    pub created_at_ms: u64,
    pub integrity_ok: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditExportReceipt {
    pub file_name: String,
    pub event_count: usize,
    pub sha256: String,
    pub created_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditExportDocument {
    schema: &'static str,
    exported_at_ms: u64,
    event_count: usize,
    events: Vec<AuditEvent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInspectRequest {
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInspection {
    pub file_name: String,
    pub integrity_ok: bool,
    pub schema_version: u32,
    pub supported_schema_version: u32,
    pub restore_ready: bool,
    pub blockers: Vec<String>,
    pub audit_event_count: u64,
    pub paper_ledger_event_count: u64,
    pub research_report_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInventoryEntry {
    pub file_name: String,
    pub created_at_ms: u64,
    pub size_bytes: u64,
    pub inspection: BackupInspection,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEvidenceReceipt {
    pub file_name: String,
    pub source_file_name: String,
    pub sha256: String,
    pub created_at_ms: u64,
    pub live_order_enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryEvidenceDocument {
    schema: &'static str,
    generated_at_ms: u64,
    backup: BackupInspection,
    reconciliation: ReconciliationState,
    latest_rehearsal_evidence: Option<RecoveryAuditEvidence>,
    live_order_enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryAuditEvidence {
    target_file_name: String,
    occurred_at_ms: u64,
    detail: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRehearsalReceipt {
    pub source_file_name: String,
    pub safety_backup_file_name: String,
    pub schema_version: u32,
    pub audit_event_count: u64,
    pub paper_ledger_event_count: u64,
    pub research_report_count: u64,
    pub krw_ledger_replayed: bool,
    pub usd_ledger_replayed: bool,
    pub isolated_copy_removed: bool,
    pub live_order_enabled: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn reconciliation_state(bridge: &PersistenceBridge) -> Result<ReconciliationState, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "재시작 대사 상태 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .query_row(
            "SELECT status,required_since_ms,completed_at_ms,mismatch_count,detail FROM runtime_reconciliation_state WHERE id=1",
            [],
            |row| {
                let status: String = row.get(0)?;
                Ok(ReconciliationState {
                    candidate_actions_locked: status != "ready",
                    status,
                    required_since_ms: row.get(1)?,
                    completed_at_ms: row.get(2)?,
                    mismatch_count: row.get::<_, u64>(3)? as usize,
                    detail: row.get(4)?,
                })
            },
        )
        .map_err(|error| format!("재시작 대사 상태를 읽지 못했습니다: {error}"))
}

fn require_reconciliation_ready(bridge: &PersistenceBridge) -> Result<(), String> {
    let state = reconciliation_state(bridge)?;
    if state.candidate_actions_locked {
        Err("앱 재시작 후 내부 원장 대사가 필요합니다. 대사를 완료하기 전에는 주문 후보 생성·승인을 할 수 없습니다.".to_owned())
    } else {
        Ok(())
    }
}

pub(crate) fn mark_runtime_reconciliation_required(
    bridge: &PersistenceBridge,
    required_at_ms: u64,
) -> Result<(), String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "재시작 대사 잠금을 설정하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "UPDATE runtime_reconciliation_state SET status='needs_reconciliation',required_since_ms=?1,completed_at_ms=NULL,mismatch_count=0,detail='앱 재시작 후 내부 원장 대사가 필요합니다.' WHERE id=1",
            params![required_at_ms],
        )
        .map_err(|error| format!("재시작 대사 잠금을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn runtime_reconciliation_status(
    bridge: State<'_, PersistenceBridge>,
) -> Result<ReconciliationState, String> {
    reconciliation_state(&bridge)
}

fn currency_for_market(market: &str, symbol: &str) -> Result<&'static str, String> {
    if market.contains("united_states") || market.contains("unitedstates") || market == "us" {
        Ok("USD")
    } else if market.contains("south_korea")
        || market == "korea"
        || market == "kr"
        || market.contains("crypto")
        || market == "coin"
        || symbol.starts_with("KRW-")
    {
        Ok("KRW")
    } else {
        Err("엔진 실행의 시장을 KRW/USD 내부 모의계좌로 매핑하지 못했습니다.".to_owned())
    }
}

fn side_text(side: TradeSide) -> &'static str {
    match side {
        TradeSide::Buy => "buy",
        TradeSide::Sell => "sell",
    }
}

fn parse_side(value: &str) -> Result<TradeSide, String> {
    match value {
        "buy" => Ok(TradeSide::Buy),
        "sell" => Ok(TradeSide::Sell),
        _ => Err("저장된 엔진 주문 방향이 올바르지 않습니다.".to_owned()),
    }
}

fn append_audit(bridge: &PersistenceBridge, event: AuditEvent) -> Result<(), String> {
    validate_audit_event(&event)?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "감사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "INSERT INTO audit_events(event_id,actor,action,target_id,previous_hash,next_hash,correlation_id,occurred_at_ms,detail) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![event.event_id,event.actor,event.action,event.target_id,event.previous_hash,event.next_hash,event.correlation_id,event.occurred_at_ms,event.detail],
        )
        .map_err(|error| format!("감사 사건을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

fn emit_alert(
    bridge: &PersistenceBridge,
    key: &str,
    severity: AlertSeverity,
    message: &str,
    observed_at_ms: u64,
) -> Result<OperationalAlert, String> {
    if !valid_id(key) || message.trim().is_empty() || message.chars().count() > 500 {
        return Err("운영 알림 식별자와 메시지가 올바르지 않습니다.".to_owned());
    }
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "운영 알림 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing = connection
        .query_row(
            "SELECT alert_id,deduplication_key,severity,message,first_seen_at_ms,last_seen_at_ms,occurrence_count,acknowledged_at_ms,response FROM operational_alerts WHERE deduplication_key=?1 ORDER BY last_seen_at_ms DESC LIMIT 1",
            params![key],
            row_alert,
        )
        .optional()
        .map_err(|error| format!("기존 운영 알림을 조회하지 못했습니다: {error}"))?;
    let incoming = OperationalAlert {
        alert_id: format!("alert:{key}:{observed_at_ms}"),
        deduplication_key: key.to_owned(),
        severity,
        message: message.to_owned(),
        first_seen_at_ms: observed_at_ms,
        last_seen_at_ms: observed_at_ms,
        occurrence_count: 1,
        acknowledged_at_ms: None,
        response: None,
    };
    let merged = merge_alert(existing.as_ref(), incoming, ALERT_DEDUPLICATION_WINDOW_MS)?;
    let severity = match merged.severity {
        AlertSeverity::Info => "info",
        AlertSeverity::Warning => "warning",
        AlertSeverity::Critical => "critical",
    };
    let transaction = connection
        .transaction()
        .map_err(|error| format!("운영 알림 저장을 시작하지 못했습니다: {error}"))?;
    if let Some(previous) = existing {
        if merged.first_seen_at_ms == previous.first_seen_at_ms {
            transaction.execute("UPDATE operational_alerts SET severity=?2,message=?3,last_seen_at_ms=?4,occurrence_count=?5,acknowledged_at_ms=NULL,response=NULL WHERE alert_id=?1", params![previous.alert_id,severity,merged.message,merged.last_seen_at_ms,merged.occurrence_count]).map_err(|error| format!("운영 알림을 병합하지 못했습니다: {error}"))?;
            transaction
                .commit()
                .map_err(|error| format!("운영 알림 병합을 확정하지 못했습니다: {error}"))?;
            return Ok(OperationalAlert {
                alert_id: previous.alert_id,
                ..merged
            });
        }
    }
    transaction.execute("INSERT INTO operational_alerts(alert_id,deduplication_key,severity,message,first_seen_at_ms,last_seen_at_ms,occurrence_count,acknowledged_at_ms,response) VALUES(?1,?2,?3,?4,?5,?6,?7,NULL,NULL)", params![merged.alert_id,merged.deduplication_key,severity,merged.message,merged.first_seen_at_ms,merged.last_seen_at_ms,merged.occurrence_count]).map_err(|error| format!("운영 알림을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("운영 알림 저장을 확정하지 못했습니다: {error}"))?;
    Ok(merged)
}

fn row_alert(row: &rusqlite::Row<'_>) -> rusqlite::Result<OperationalAlert> {
    let severity: String = row.get(2)?;
    Ok(OperationalAlert {
        alert_id: row.get(0)?,
        deduplication_key: row.get(1)?,
        severity: match severity.as_str() {
            "critical" => AlertSeverity::Critical,
            "warning" => AlertSeverity::Warning,
            _ => AlertSeverity::Info,
        },
        message: row.get(3)?,
        first_seen_at_ms: row.get(4)?,
        last_seen_at_ms: row.get(5)?,
        occurrence_count: row.get(6)?,
        acknowledged_at_ms: row.get(7)?,
        response: row.get(8)?,
    })
}

fn load_engine_candidate(
    bridge: &PersistenceBridge,
    candidate_id: &str,
) -> Result<EngineOrderCandidate, String> {
    if !valid_id(candidate_id) {
        return Err("유효한 엔진 주문 후보 ID가 필요합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 후보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .query_row(
            "SELECT candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms FROM engine_order_candidates WHERE candidate_id=?1",
            params![candidate_id],
            row_engine_candidate,
        )
        .optional()
        .map_err(|error| format!("엔진 주문 후보를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "엔진 주문 후보를 찾지 못했습니다.".to_owned())
        .and_then(engine_candidate_from_row)
}

type EngineCandidateRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    u64,
    u64,
    u64,
    u64,
    String,
    String,
    u64,
    u64,
);

fn row_engine_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<EngineCandidateRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
    ))
}

fn engine_candidate_from_row(row: EngineCandidateRow) -> Result<EngineOrderCandidate, String> {
    Ok(EngineOrderCandidate {
        candidate_id: row.0,
        run_id: row.1,
        symbol: row.2,
        market: row.3,
        currency: row.4,
        side: parse_side(&row.5)?,
        quantity: row.6,
        quantity_scale: row.7,
        reference_price_minor: row.8,
        valid_until_ms: row.9,
        status: EngineCandidateStatus::parse(&row.10)?,
        safety: serde_json::from_str(&row.11)
            .map_err(|error| format!("저장 안전 게이트를 해석하지 못했습니다: {error}"))?,
        created_at_ms: row.12,
        updated_at_ms: row.13,
    })
}

fn append_engine_transition(
    bridge: &PersistenceBridge,
    candidate_id: &str,
    expected: EngineCandidateStatus,
    next: EngineCandidateStatus,
    detail: Value,
    occurred_at_ms: u64,
) -> Result<(), String> {
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 후보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("엔진 후보 상태 저장을 시작하지 못했습니다: {error}"))?;
    let current: String = transaction
        .query_row(
            "SELECT status FROM engine_order_candidates WHERE candidate_id=?1",
            params![candidate_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("엔진 후보 상태를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "엔진 주문 후보를 찾지 못했습니다.".to_owned())?;
    if current != expected.as_str() {
        return Err(format!("엔진 후보 상태가 예상과 다릅니다: {current}"));
    }
    let index: u64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM engine_order_events WHERE candidate_id=?1",
            params![candidate_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("엔진 후보 사건 순번을 확인하지 못했습니다: {error}"))?;
    transaction.execute("INSERT INTO engine_order_events(candidate_id,event_index,event_type,event_json,occurred_at_ms) VALUES(?1,?2,?3,?4,?5)", params![candidate_id,index,next.as_str(),detail.to_string(),occurred_at_ms]).map_err(|error| format!("엔진 후보 사건을 저장하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "UPDATE engine_order_candidates SET status=?2,updated_at_ms=?3 WHERE candidate_id=?1",
            params![candidate_id, next.as_str(), occurred_at_ms],
        )
        .map_err(|error| format!("엔진 후보 상태를 갱신하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("엔진 후보 상태를 확정하지 못했습니다: {error}"))
}

fn ensure_active_strategy_protection(
    bridge: &PersistenceBridge,
    symbol: &str,
    currency: &str,
    side: TradeSide,
    evaluated_at_ms: u64,
) -> Result<bool, String> {
    if side == TradeSide::Sell {
        return Ok(false);
    }
    let Some(policy) = crate::risk_policy::active_policy(bridge)? else {
        return Ok(false);
    };
    let Some(decision) = crate::strategy_protection::evaluate_runtime_protection(
        bridge,
        &policy,
        symbol,
        currency,
        evaluated_at_ms,
    )?
    else {
        return Ok(false);
    };
    if !decision.can_open_new_position {
        let reasons = decision
            .triggers
            .iter()
            .map(|trigger| trigger.code.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "활성 전략 보호 정책이 엔진 신규 진입을 잠갔습니다: {reasons}"
        ));
    }
    Ok(true)
}

pub(crate) fn create_engine_candidate(
    bridge: &PersistenceBridge,
    run_id: &str,
    created_at_ms: u64,
) -> Result<EngineOrderCandidate, String> {
    require_reconciliation_ready(bridge)?;
    if !valid_id(run_id) {
        return Err("유효한 엔진 실행 ID가 필요합니다.".to_owned());
    }
    let (status, candidate_ready, market, input_json, report_json) = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection.query_row("SELECT COALESCE((SELECT e.status FROM engine_run_status_events e WHERE e.run_id=r.run_id ORDER BY e.occurred_at_ms DESC,e.event_id DESC LIMIT 1),r.status),r.candidate_ready,r.market,r.input_json,r.report_json FROM engine_runs r WHERE r.run_id=?1", params![run_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)? == 1,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?))).optional().map_err(|error| format!("엔진 실행을 조회하지 못했습니다: {error}"))?.ok_or_else(|| "엔진 실행을 찾지 못했습니다.".to_owned())?
    };
    if status != "completed" || !candidate_ready {
        return Err(
            "완료되어 후보 준비가 끝난 엔진 실행만 주문 후보로 만들 수 있습니다.".to_owned(),
        );
    }
    let request: EngineRunRequest = serde_json::from_str(&input_json)
        .map_err(|error| format!("엔진 입력을 해석하지 못했습니다: {error}"))?;
    let report: Value = serde_json::from_str(&report_json)
        .map_err(|error| format!("엔진 보고서를 해석하지 못했습니다: {error}"))?;
    let approved = report
        .pointer("/stages/pretrade/approved")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let quantity = report
        .pointer("/stages/pretrade/finalQuantity")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    if !approved || quantity == 0 || request.trade_plan.valid_until_ms <= created_at_ms {
        return Err(
            "사전 주문 게이트가 승인되지 않았거나 거래 계획 유효기간이 지났습니다.".to_owned(),
        );
    }
    let currency = currency_for_market(&market, &request.trade_plan.symbol)?;
    let quantity_scale = if request.trade_plan.symbol.starts_with("KRW-") {
        100_000_000
    } else {
        1
    };
    let account = paper_trading::load_or_open_account_for_currency(bridge, currency)?;
    let notional = u64::try_from(
        u128::from(request.pretrade_input.quote_price_minor) * u128::from(quantity)
            / u128::from(quantity_scale),
    )
    .map_err(|_| "엔진 후보 주문 금액이 지원 범위를 초과했습니다.".to_owned())?;
    match request.trade_plan.side {
        TradeSide::Buy if account.cash_minor < notional => {
            return Err("내부 모의계좌 예수금이 부족합니다.".to_owned())
        }
        TradeSide::Sell
            if account
                .positions
                .get(&request.trade_plan.symbol)
                .is_none_or(|position| {
                    position.quantity_scale != quantity_scale || position.quantity < quantity
                }) =>
        {
            return Err("내부 모의계좌 보유 수량이 부족합니다.".to_owned())
        }
        _ => {}
    }
    let protection_applied = ensure_active_strategy_protection(
        bridge,
        &request.trade_plan.symbol,
        currency,
        request.trade_plan.side,
        created_at_ms,
    )?;
    let candidate_id = format!("engine-cand-{run_id}");
    let mut checks = vec![
        "engine_candidate_ready",
        "deterministic_pretrade_approved",
        "internal_account_capacity",
        "live_transport_locked",
    ];
    if protection_applied {
        checks.push("active_strategy_protection_approved");
    }
    let safety = json!({"passed":true,"checks":checks,"costs":request.pretrade_input.costs,"liveOrderEnabled":false});
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 후보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("엔진 후보 저장을 시작하지 못했습니다: {error}"))?;
    transaction.execute("INSERT INTO engine_order_candidates(candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'safety_approved',?11,?12,?12)", params![candidate_id,run_id,request.trade_plan.symbol,market,currency,side_text(request.trade_plan.side),quantity,quantity_scale,request.pretrade_input.quote_price_minor,request.trade_plan.valid_until_ms,safety.to_string(),created_at_ms]).map_err(|error| if error.to_string().contains("UNIQUE constraint failed") { "이 엔진 실행의 주문 후보가 이미 있습니다.".to_owned() } else { format!("엔진 주문 후보를 저장하지 못했습니다: {error}") })?;
    for (index, event_type) in ["candidate_created", "safety_approved"].iter().enumerate() {
        transaction.execute("INSERT INTO engine_order_events(candidate_id,event_index,event_type,event_json,occurred_at_ms) VALUES(?1,?2,?3,?4,?5)", params![candidate_id,index as u64,event_type,json!({"runId":run_id,"liveOrderEnabled":false}).to_string(),created_at_ms]).map_err(|error| format!("엔진 후보 사건을 저장하지 못했습니다: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("엔진 후보 저장을 확정하지 못했습니다: {error}"))?;
    drop(connection);
    let candidate = load_engine_candidate(bridge, &candidate_id)?;
    append_audit(
        bridge,
        AuditEvent {
            event_id: format!("audit:candidate:{created_at_ms}:{run_id}"),
            actor: "local_user".to_owned(),
            action: "engine_candidate_created".to_owned(),
            target_id: candidate_id,
            previous_hash: None,
            next_hash: None,
            correlation_id: run_id.to_owned(),
            occurred_at_ms: created_at_ms,
            detail:
                "엔진 결과를 내부 모의주문 후보로 승격했습니다. 실전 주문 전송은 잠겨 있습니다."
                    .to_owned(),
        },
    )?;
    Ok(candidate)
}

#[tauri::command]
pub fn engine_order_candidate_create(
    request: EngineCandidateCreateRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<EngineOrderCandidate, String> {
    create_engine_candidate(&bridge, &request.run_id, now_ms()?)
}

#[tauri::command]
pub fn engine_order_candidates(
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<EngineOrderCandidate>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 후보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare("SELECT candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms FROM engine_order_candidates ORDER BY updated_at_ms DESC LIMIT 100").map_err(|error| format!("엔진 후보 목록을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], row_engine_candidate)
        .map_err(|error| format!("엔진 후보 목록을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        row.map_err(|error| format!("엔진 후보를 읽지 못했습니다: {error}"))
            .and_then(engine_candidate_from_row)
    })
    .collect()
}

fn approve_engine_candidate(
    bridge: &PersistenceBridge,
    candidate_id: &str,
    approved_at_ms: u64,
) -> Result<PaperAccountSnapshot, String> {
    require_reconciliation_ready(bridge)?;
    let candidate = load_engine_candidate(bridge, candidate_id)?;
    if candidate.status != EngineCandidateStatus::SafetyApproved {
        return Err("안전 승인 대기 중인 엔진 후보만 체결 승인할 수 있습니다.".to_owned());
    }
    if approved_at_ms > candidate.valid_until_ms {
        append_engine_transition(
            bridge,
            candidate_id,
            EngineCandidateStatus::SafetyApproved,
            EngineCandidateStatus::Expired,
            json!({"reason":"trade_plan_expired"}),
            approved_at_ms,
        )?;
        return Err("거래 계획 유효기간이 지나 후보를 만료 처리했습니다.".to_owned());
    }
    ensure_active_strategy_protection(
        bridge,
        &candidate.symbol,
        &candidate.currency,
        candidate.side,
        approved_at_ms,
    )?;
    let costs = serde_json::from_value(
        candidate
            .safety
            .get("costs")
            .cloned()
            .ok_or_else(|| "엔진 후보에 고정된 체결 비용이 없습니다.".to_owned())?,
    )
    .map_err(|error| format!("엔진 후보 체결 비용을 해석하지 못했습니다: {error}"))?;
    append_engine_transition(
        bridge,
        candidate_id,
        EngineCandidateStatus::SafetyApproved,
        EngineCandidateStatus::UserApproved,
        json!({"approvedBy":"local_user"}),
        approved_at_ms,
    )?;
    append_engine_transition(
        bridge,
        candidate_id,
        EngineCandidateStatus::UserApproved,
        EngineCandidateStatus::Submitted,
        json!({"transport":"internal_paper_ledger","liveOrderEnabled":false}),
        approved_at_ms,
    )?;
    let account = paper_trading::load_or_open_account_for_currency(bridge, &candidate.currency)?;
    let execution_at_ms = approved_at_ms.max(account.last_event_at_ms);
    let ledger_id = ledger_id_for_currency(&candidate.currency)?;
    let mut ledger = bridge.paper_ledger(ledger_id)?;
    let result = execute_shadow_order(
        &mut ledger,
        ShadowOrderRequest {
            account_id: account.account_id,
            order_id: format!("order-{candidate_id}"),
            idempotency_key: candidate_id.to_owned(),
            symbol: candidate.symbol.clone(),
            currency: candidate.currency.clone(),
            side: candidate.side,
            quantity: candidate.quantity,
            quantity_scale: candidate.quantity_scale,
            reference_price_minor: candidate.reference_price_minor,
            occurred_at_ms: execution_at_ms,
        },
        costs,
    );
    match result {
        Ok(account) => {
            append_engine_transition(
                bridge,
                candidate_id,
                EngineCandidateStatus::Submitted,
                EngineCandidateStatus::Filled,
                json!({"ledgerId":ledger_id,"quantity":candidate.quantity}),
                execution_at_ms,
            )?;
            append_audit(
                bridge,
                AuditEvent {
                    event_id: format!("audit:fill:{execution_at_ms}:{candidate_id}"),
                    actor: "local_user".to_owned(),
                    action: "internal_paper_order_filled".to_owned(),
                    target_id: candidate_id.to_owned(),
                    previous_hash: None,
                    next_hash: None,
                    correlation_id: candidate.run_id,
                    occurred_at_ms: execution_at_ms,
                    detail: "사용자 승인 후 내부 모의원장에만 체결했습니다.".to_owned(),
                },
            )?;
            Ok(paper_trading::snapshot(account))
        }
        Err(error) => {
            let _ = append_engine_transition(
                bridge,
                candidate_id,
                EngineCandidateStatus::Submitted,
                EngineCandidateStatus::Rejected,
                json!({"reason":error.message}),
                approved_at_ms,
            );
            let _ = emit_alert(
                bridge,
                "engine_candidate_fill_failed",
                AlertSeverity::Critical,
                "엔진 주문 후보의 내부 모의체결이 실패했습니다.",
                approved_at_ms,
            );
            Err(error.message)
        }
    }
}

#[tauri::command]
pub fn engine_order_candidate_approve(
    request: EngineCandidateActionRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<PaperAccountSnapshot, String> {
    approve_engine_candidate(&bridge, &request.candidate_id, now_ms()?)
}

#[tauri::command]
pub fn engine_order_candidate_reject(
    request: EngineCandidateActionRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    let now = now_ms()?;
    append_engine_transition(
        &bridge,
        &request.candidate_id,
        EngineCandidateStatus::SafetyApproved,
        EngineCandidateStatus::Rejected,
        json!({"reason":"user_rejected"}),
        now,
    )
}

#[tauri::command]
pub fn operations_runtime_reconcile(
    bridge: State<'_, PersistenceBridge>,
) -> Result<ReconciliationReport, String> {
    reconcile(&bridge, now_ms()?)
}

fn reconcile(
    bridge: &PersistenceBridge,
    observed_at_ms: u64,
) -> Result<ReconciliationReport, String> {
    let candidates = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "엔진 후보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement=connection.prepare("SELECT candidate_id,currency,status FROM engine_order_candidates WHERE status IN ('submitted','filled')").map_err(|error|format!("대사 대상을 준비하지 못했습니다: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| format!("대사 대상을 조회하지 못했습니다: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("대사 대상을 읽지 못했습니다: {error}"))?
    };
    let mut repaired = 0;
    let mut mismatches = 0;
    for (candidate_id, currency, status) in &candidates {
        let ledger = bridge.paper_ledger(ledger_id_for_currency(currency)?)?;
        let order_id = format!("order-{candidate_id}");
        let exists=ledger.events().iter().any(|event|matches!(event,LedgerEvent::OrderFilled{order_id:stored,..} if stored==&order_id));
        if status == "submitted" && exists {
            append_engine_transition(
                bridge,
                candidate_id,
                EngineCandidateStatus::Submitted,
                EngineCandidateStatus::Filled,
                json!({"reason":"ledger_reconciliation"}),
                observed_at_ms,
            )?;
            repaired += 1;
        } else if status == "submitted" {
            // 내부 원장 전송은 프로세스 안에서만 동기 실행된다. 재시작 후에도 체결 사건이
            // 없다면 외부에 남은 주문이 없으므로 미확정 제출을 취소해 재전송을 막는다.
            append_engine_transition(
                bridge,
                candidate_id,
                EngineCandidateStatus::Submitted,
                EngineCandidateStatus::Cancelled,
                json!({"reason":"missing_internal_fill_after_restart","liveOrderEnabled":false}),
                observed_at_ms,
            )?;
            repaired += 1;
        } else if status == "filled" && !exists {
            mismatches += 1;
            let key = format!("ledger_mismatch:{}", candidate_id.replace(':', "_"));
            let _ = emit_alert(
                bridge,
                &key,
                AlertSeverity::Critical,
                "체결 완료 상태와 내부 모의원장이 일치하지 않습니다.",
                observed_at_ms,
            )?;
        }
    }
    {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "대사 완료 상태를 저장하지 못했습니다.".to_owned())?;
        let (status, completed_at_ms, detail) = if mismatches == 0 {
            (
                "ready",
                Some(observed_at_ms),
                format!(
                    "내부 원장 {}건을 대사했고 자동 복구 {}건을 반영했습니다.",
                    candidates.len(),
                    repaired
                ),
            )
        } else {
            (
                "needs_reconciliation",
                None,
                format!("사용자 확인이 필요한 원장 불일치 {mismatches}건이 남아 있습니다."),
            )
        };
        connection
            .execute(
                "UPDATE runtime_reconciliation_state SET status=?1,completed_at_ms=?2,mismatch_count=?3,detail=?4 WHERE id=1",
                params![status, completed_at_ms, mismatches as u64, detail],
            )
            .map_err(|error| format!("대사 완료 상태를 저장하지 못했습니다: {error}"))?;
    }
    let state = reconciliation_state(bridge)?;
    Ok(ReconciliationReport {
        checked_count: candidates.len(),
        repaired_count: repaired,
        mismatch_count: mismatches,
        live_order_enabled: false,
        state,
    })
}

pub fn reconcile_for_shadow_soak(
    bridge: &PersistenceBridge,
) -> Result<ReconciliationReport, String> {
    reconcile(bridge, now_ms()?)
}

#[tauri::command]
pub fn operational_alerts(
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<OperationalAlert>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "운영 알림 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement=connection.prepare("SELECT alert_id,deduplication_key,severity,message,first_seen_at_ms,last_seen_at_ms,occurrence_count,acknowledged_at_ms,response FROM operational_alerts ORDER BY last_seen_at_ms DESC LIMIT 100").map_err(|error|format!("운영 알림 목록을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], row_alert)
        .map_err(|error| format!("운영 알림을 조회하지 못했습니다: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("운영 알림을 읽지 못했습니다: {error}"))
}

#[tauri::command]
pub fn operational_alert_acknowledge(
    request: AlertAcknowledgeRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<OperationalAlert, String> {
    let now = now_ms()?;
    let mut alert = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "운영 알림 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection.query_row("SELECT alert_id,deduplication_key,severity,message,first_seen_at_ms,last_seen_at_ms,occurrence_count,acknowledged_at_ms,response FROM operational_alerts WHERE alert_id=?1",params![request.alert_id],row_alert).optional().map_err(|error|format!("운영 알림을 조회하지 못했습니다: {error}"))?.ok_or_else(||"운영 알림을 찾지 못했습니다.".to_owned())?
    };
    acknowledge_alert(&mut alert, now, request.response.trim())?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "운영 알림 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "UPDATE operational_alerts SET acknowledged_at_ms=?2,response=?3 WHERE alert_id=?1",
            params![alert.alert_id, alert.acknowledged_at_ms, alert.response],
        )
        .map_err(|error| format!("운영 알림 확인을 저장하지 못했습니다: {error}"))?;
    Ok(alert)
}

#[tauri::command]
pub fn audit_event_history(
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<AuditEvent>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "감사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement=connection.prepare("SELECT event_id,actor,action,target_id,previous_hash,next_hash,correlation_id,occurred_at_ms,detail FROM audit_events ORDER BY occurred_at_ms DESC LIMIT 200").map_err(|error|format!("감사 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(AuditEvent {
                event_id: row.get(0)?,
                actor: row.get(1)?,
                action: row.get(2)?,
                target_id: row.get(3)?,
                previous_hash: row.get(4)?,
                next_hash: row.get(5)?,
                correlation_id: row.get(6)?,
                occurred_at_ms: row.get(7)?,
                detail: row.get(8)?,
            })
        })
        .map_err(|error| format!("감사 이력을 조회하지 못했습니다: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("감사 이력을 읽지 못했습니다: {error}"))
}

#[tauri::command]
pub fn audit_event_export(
    bridge: State<'_, PersistenceBridge>,
) -> Result<AuditExportReceipt, String> {
    export_audit_events(&bridge, now_ms()?)
}

fn export_audit_events(
    bridge: &PersistenceBridge,
    exported_at_ms: u64,
) -> Result<AuditExportReceipt, String> {
    let database_path = bridge
        .database_path
        .as_ref()
        .ok_or_else(|| "메모리 테스트 저장소는 감사 파일을 내보낼 수 없습니다.".to_owned())?;
    let parent = database_path
        .parent()
        .ok_or_else(|| "로컬 저장소 폴더를 확인하지 못했습니다.".to_owned())?;
    let export_dir = parent.join("exports");
    fs::create_dir_all(&export_dir)
        .map_err(|error| format!("감사 내보내기 폴더를 만들지 못했습니다: {error}"))?;

    let events = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "감사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let count: u64 = connection
            .query_row("SELECT COUNT(*) FROM audit_events", [], |row| row.get(0))
            .map_err(|error| format!("감사 사건 수를 확인하지 못했습니다: {error}"))?;
        if count > 10_000 {
            return Err("감사 내보내기는 한 번에 최대 10,000건까지 지원합니다.".to_owned());
        }
        let mut statement = connection
            .prepare("SELECT event_id,actor,action,target_id,previous_hash,next_hash,correlation_id,occurred_at_ms,detail FROM audit_events ORDER BY occurred_at_ms,event_id")
            .map_err(|error| format!("감사 내보내기를 준비하지 못했습니다: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(AuditEvent {
                    event_id: row.get(0)?,
                    actor: row.get(1)?,
                    action: row.get(2)?,
                    target_id: row.get(3)?,
                    previous_hash: row.get(4)?,
                    next_hash: row.get(5)?,
                    correlation_id: row.get(6)?,
                    occurred_at_ms: row.get(7)?,
                    detail: row.get(8)?,
                })
            })
            .map_err(|error| format!("감사 사건을 내보내지 못했습니다: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("감사 사건을 읽지 못했습니다: {error}"))?
    };
    for event in &events {
        validate_audit_event(event)?;
    }
    let document = AuditExportDocument {
        schema: "investa.audit-export.v1",
        exported_at_ms,
        event_count: events.len(),
        events,
    };
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("감사 내보내기 JSON을 만들지 못했습니다: {error}"))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let file_name = format!("investa-audit-{exported_at_ms}.json");
    let destination = export_dir.join(&file_name);
    let temporary = export_dir.join(format!(".{file_name}.tmp"));
    let write_result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("감사 임시 파일을 만들지 못했습니다: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("감사 파일을 안전하게 저장하지 못했습니다: {error}"))?;
        fs::rename(&temporary, &destination)
            .map_err(|error| format!("감사 파일 저장을 확정하지 못했습니다: {error}"))
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result?;
    if let Err(error) = append_audit(
        bridge,
        AuditEvent {
            event_id: format!("audit:export:{exported_at_ms}"),
            actor: "local_user".to_owned(),
            action: "audit_events_exported".to_owned(),
            target_id: file_name.clone(),
            previous_hash: None,
            next_hash: Some(sha256.clone()),
            correlation_id: format!("audit-export:{exported_at_ms}"),
            occurred_at_ms: exported_at_ms,
            detail: format!(
                "기존 감사 사건 {}건을 읽기 전용 JSON으로 내보냈습니다.",
                document.event_count
            ),
        },
    ) {
        let _ = fs::remove_file(&destination);
        return Err(error);
    }
    Ok(AuditExportReceipt {
        file_name,
        event_count: document.event_count,
        sha256,
        created_at_ms: exported_at_ms,
    })
}

#[tauri::command]
pub fn provider_health_record(
    request: ProviderHealthRecordRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    record_provider_health(&bridge, request, now_ms()?)
}

fn record_provider_health(
    bridge: &PersistenceBridge,
    request: ProviderHealthRecordRequest,
    received_at_ms: u64,
) -> Result<(), String> {
    if !valid_id(&request.component_id)
        || request.observed_at_ms == 0
        || request.observed_at_ms > received_at_ms.saturating_add(60_000)
        || request.retry_action.trim().is_empty()
        || request.detail.chars().count() > 500
    {
        return Err("공급자 상태 기록 형식이 올바르지 않습니다.".to_owned());
    }
    build_health_report(
        request.observed_at_ms,
        &[HealthComponent {
            component_id: request.component_id.clone(),
            critical: request.critical,
            healthy: request.healthy,
            last_success_at_ms: request.healthy.then_some(request.observed_at_ms),
            retry_action: request.retry_action.clone(),
            detail: request.detail.clone(),
        }],
    )?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "공급자 상태 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection.execute("INSERT INTO provider_health_events(event_id,component_id,critical,healthy,retry_action,detail,observed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![format!("health:{}:{}:{}",request.component_id,request.observed_at_ms,uuid::Uuid::new_v4().simple()),request.component_id,i64::from(request.critical),i64::from(request.healthy),request.retry_action,request.detail,request.observed_at_ms]).map_err(|error|format!("공급자 상태를 저장하지 못했습니다: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn operations_health_refresh(
    bridge: State<'_, PersistenceBridge>,
) -> Result<HealthReport, String> {
    refresh_local_health(&bridge, now_ms()?)
}

fn refresh_local_health(
    bridge: &PersistenceBridge,
    observed_at_ms: u64,
) -> Result<HealthReport, String> {
    let database_healthy = bridge
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 점검 잠금을 획득하지 못했습니다.".to_owned())?
        .query_row("PRAGMA quick_check(1)", [], |row| row.get::<_, String>(0))
        .map(|result| result == "ok")
        .unwrap_or(false);
    let krw_ledger_healthy = bridge.paper_ledger("paper-krw-v1").is_ok();
    let usd_ledger_healthy = bridge.paper_ledger("paper-usd-v1").is_ok();
    let checks = [
        ProviderHealthRecordRequest {
            component_id: "sqlite".to_owned(),
            critical: true,
            healthy: database_healthy,
            retry_action: "로컬 저장소 무결성 재검사".to_owned(),
            detail: if database_healthy {
                "SQLite quick_check 통과".to_owned()
            } else {
                "SQLite quick_check 실패".to_owned()
            },
            observed_at_ms,
        },
        ProviderHealthRecordRequest {
            component_id: "paper_ledger_krw".to_owned(),
            critical: true,
            healthy: krw_ledger_healthy,
            retry_action: "KRW 내부 모의원장 재생".to_owned(),
            detail: if krw_ledger_healthy {
                "KRW append-only 사건 재생 통과".to_owned()
            } else {
                "KRW append-only 사건 재생 실패".to_owned()
            },
            observed_at_ms,
        },
        ProviderHealthRecordRequest {
            component_id: "paper_ledger_usd".to_owned(),
            critical: true,
            healthy: usd_ledger_healthy,
            retry_action: "USD 내부 모의원장 재생".to_owned(),
            detail: if usd_ledger_healthy {
                "USD append-only 사건 재생 통과".to_owned()
            } else {
                "USD append-only 사건 재생 실패".to_owned()
            },
            observed_at_ms,
        },
    ];
    for check in checks {
        let component_id = check.component_id.clone();
        let healthy = check.healthy;
        record_provider_health(bridge, check, observed_at_ms)?;
        if !healthy {
            let _ = emit_alert(
                bridge,
                &format!("health_failure_{component_id}"),
                AlertSeverity::Critical,
                &format!("핵심 로컬 구성요소 {component_id} 점검에 실패했습니다."),
                observed_at_ms,
            )?;
        }
    }
    provider_health_report_at(bridge, observed_at_ms, PROVIDER_HEALTH_MAXIMUM_AGE_MS)
}

pub fn refresh_local_health_for_shadow_soak(
    bridge: &PersistenceBridge,
) -> Result<HealthReport, String> {
    refresh_local_health(bridge, now_ms()?)
}

#[tauri::command]
pub fn provider_health_report(
    bridge: State<'_, PersistenceBridge>,
) -> Result<HealthReport, String> {
    provider_health_report_at(&bridge, now_ms()?, PROVIDER_HEALTH_MAXIMUM_AGE_MS)
}

fn provider_health_report_at(
    bridge: &PersistenceBridge,
    generated_at_ms: u64,
    maximum_age_ms: u64,
) -> Result<HealthReport, String> {
    if maximum_age_ms == 0 {
        return Err("공급자 상태 최대 유효시간은 0보다 커야 합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "공급자 상태 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement=connection.prepare("SELECT component_id,critical,healthy,retry_action,detail,observed_at_ms FROM provider_health_events p WHERE event_id=(SELECT event_id FROM provider_health_events WHERE component_id=p.component_id ORDER BY observed_at_ms DESC,event_id DESC LIMIT 1) ORDER BY component_id").map_err(|error|format!("공급자 상태 보고서를 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            let reported_healthy = row.get::<_, i64>(2)? == 1;
            let observed_at_ms: u64 = row.get(5)?;
            let fresh = generated_at_ms.saturating_sub(observed_at_ms) <= maximum_age_ms;
            let healthy = reported_healthy && fresh;
            let stored_detail: String = row.get(4)?;
            Ok(HealthComponent {
                component_id: row.get(0)?,
                critical: row.get::<_, i64>(1)? == 1,
                healthy,
                last_success_at_ms: reported_healthy.then_some(observed_at_ms),
                retry_action: row.get(3)?,
                detail: if fresh {
                    stored_detail
                } else {
                    format!("상태 만료: {stored_detail}")
                },
            })
        })
        .map_err(|error| format!("공급자 상태를 조회하지 못했습니다: {error}"))?;
    let components = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("공급자 상태를 읽지 못했습니다: {error}"))?;
    drop(statement);
    drop(connection);
    build_health_report(generated_at_ms, &components)
}

#[tauri::command]
pub fn local_backup_create(bridge: State<'_, PersistenceBridge>) -> Result<BackupReceipt, String> {
    create_local_backup(&bridge, now_ms()?)
}

#[tauri::command]
pub fn local_backup_inspect(
    request: BackupInspectRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<BackupInspection, String> {
    inspect_local_backup(&bridge, request.file_name.trim())
}

#[tauri::command]
pub fn local_backup_rehearse(
    request: BackupInspectRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RecoveryRehearsalReceipt, String> {
    rehearse_local_backup(&bridge, request.file_name.trim(), now_ms()?)
}

#[tauri::command]
pub fn local_backup_inventory(
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<BackupInventoryEntry>, String> {
    backup_inventory(&bridge)
}

#[tauri::command]
pub fn local_recovery_evidence_export(
    request: BackupInspectRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RecoveryEvidenceReceipt, String> {
    export_recovery_evidence(&bridge, request.file_name.trim(), now_ms()?)
}

fn backup_inventory(bridge: &PersistenceBridge) -> Result<Vec<BackupInventoryEntry>, String> {
    let database_path = bridge
        .database_path
        .as_ref()
        .ok_or_else(|| "메모리 테스트 저장소는 백업 목록을 조회할 수 없습니다.".to_owned())?;
    let backup_dir = database_path
        .parent()
        .ok_or_else(|| "로컬 저장소 폴더를 확인하지 못했습니다.".to_owned())?
        .join("backups");
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(&backup_dir)
        .map_err(|error| format!("백업 목록을 읽지 못했습니다: {error}"))?
    {
        let item = item.map_err(|error| format!("백업 항목을 읽지 못했습니다: {error}"))?;
        if !item
            .file_type()
            .map_err(|error| format!("백업 항목 종류를 확인하지 못했습니다: {error}"))?
            .is_file()
        {
            continue;
        }
        let file_name = item.file_name().to_string_lossy().into_owned();
        let Some(timestamp) = file_name
            .strip_prefix("investa-")
            .and_then(|value| value.strip_suffix(".sqlite3"))
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let size_bytes = item
            .metadata()
            .map_err(|error| format!("백업 크기를 확인하지 못했습니다: {error}"))?
            .len();
        let inspection =
            inspect_local_backup(bridge, &file_name).unwrap_or_else(|_| BackupInspection {
                file_name: file_name.clone(),
                integrity_ok: false,
                schema_version: 0,
                supported_schema_version: SCHEMA_VERSION,
                restore_ready: false,
                blockers: vec!["백업 파일을 읽기 전용으로 검사하지 못했습니다.".to_owned()],
                audit_event_count: 0,
                paper_ledger_event_count: 0,
                research_report_count: 0,
            });
        entries.push(BackupInventoryEntry {
            file_name,
            created_at_ms: timestamp,
            size_bytes,
            inspection,
        });
    }
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.created_at_ms));
    entries.truncate(100);
    Ok(entries)
}

fn export_recovery_evidence(
    bridge: &PersistenceBridge,
    file_name: &str,
    generated_at_ms: u64,
) -> Result<RecoveryEvidenceReceipt, String> {
    let backup = inspect_local_backup(bridge, file_name)?;
    let reconciliation = reconciliation_state(bridge)?;
    let latest_rehearsal_evidence = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "복구 훈련 감사 조회 잠금을 획득하지 못했습니다.".to_owned())?;
        connection.query_row(
            "SELECT target_id,occurred_at_ms,detail FROM audit_events WHERE action='local_backup_rehearsed' AND target_id=?1 ORDER BY occurred_at_ms DESC,event_id DESC LIMIT 1",
            params![file_name],
            |row| Ok(RecoveryAuditEvidence { target_file_name: row.get(0)?, occurred_at_ms: row.get(1)?, detail: row.get(2)? }),
        ).optional().map_err(|error| format!("복구 훈련 감사 근거를 읽지 못했습니다: {error}"))?
    };
    let document = RecoveryEvidenceDocument {
        schema: "investa.recovery-evidence.v1",
        generated_at_ms,
        backup,
        reconciliation,
        latest_rehearsal_evidence,
        live_order_enabled: false,
    };
    let serialized = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("복구 증거를 직렬화하지 못했습니다: {error}"))?;
    let sha256 = format!("{:x}", Sha256::digest(&serialized));
    let database_path = bridge
        .database_path
        .as_ref()
        .ok_or_else(|| "메모리 테스트 저장소는 복구 증거를 내보낼 수 없습니다.".to_owned())?;
    let export_dir = database_path
        .parent()
        .ok_or_else(|| "로컬 저장소 폴더를 확인하지 못했습니다.".to_owned())?
        .join("exports");
    fs::create_dir_all(&export_dir)
        .map_err(|error| format!("복구 증거 폴더를 만들지 못했습니다: {error}"))?;
    let exported_file_name = format!("recovery-evidence-{generated_at_ms}.json");
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(export_dir.join(&exported_file_name))
        .map_err(|error| format!("복구 증거 파일을 만들지 못했습니다: {error}"))?;
    output
        .write_all(&serialized)
        .map_err(|error| format!("복구 증거 파일을 쓰지 못했습니다: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("복구 증거 파일을 동기화하지 못했습니다: {error}"))?;
    append_audit(bridge, AuditEvent {
        event_id: format!("audit:recovery-evidence:{generated_at_ms}"), actor: "local_user".to_owned(), action: "local_recovery_evidence_exported".to_owned(), target_id: file_name.to_owned(), previous_hash: None, next_hash: Some(sha256.clone()), correlation_id: format!("recovery-evidence:{generated_at_ms}"), occurred_at_ms: generated_at_ms, detail: "백업 사전검사·격리훈련 감사·재시작 잠금 상태만 포함한 복구 증거 JSON을 내보냈습니다.".to_owned(),
    })?;
    Ok(RecoveryEvidenceReceipt {
        file_name: exported_file_name,
        source_file_name: file_name.to_owned(),
        sha256,
        created_at_ms: generated_at_ms,
        live_order_enabled: false,
    })
}

fn inspect_local_backup(
    bridge: &PersistenceBridge,
    file_name: &str,
) -> Result<BackupInspection, String> {
    let timestamp = file_name
        .strip_prefix("investa-")
        .and_then(|value| value.strip_suffix(".sqlite3"))
        .ok_or_else(|| "Investa가 생성한 백업 파일명만 검사할 수 있습니다.".to_owned())?;
    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Investa가 생성한 백업 파일명만 검사할 수 있습니다.".to_owned());
    }
    let database_path = bridge
        .database_path
        .as_ref()
        .ok_or_else(|| "메모리 테스트 저장소는 백업 파일을 검사할 수 없습니다.".to_owned())?;
    let parent = database_path
        .parent()
        .ok_or_else(|| "로컬 저장소 폴더를 확인하지 못했습니다.".to_owned())?;
    let backup_dir = parent.join("backups");
    let backup_path = backup_dir.join(file_name);
    let canonical_dir = backup_dir
        .canonicalize()
        .map_err(|error| format!("백업 폴더를 확인하지 못했습니다: {error}"))?;
    let canonical_path = backup_path
        .canonicalize()
        .map_err(|error| format!("검사할 백업 파일을 찾지 못했습니다: {error}"))?;
    if canonical_path.parent() != Some(canonical_dir.as_path()) {
        return Err("앱 데이터 백업 폴더 밖의 파일은 검사할 수 없습니다.".to_owned());
    }

    let connection = Connection::open_with_flags(
        &canonical_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("백업을 읽기 전용으로 열지 못했습니다: {error}"))?;
    let integrity: String = connection
        .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
        .map_err(|error| format!("백업 무결성을 검사하지 못했습니다: {error}"))?;
    let schema_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| format!("백업 스키마 버전을 확인하지 못했습니다: {error}"))?;
    let integrity_ok = integrity == "ok";
    let mut blockers = Vec::new();
    if !integrity_ok {
        blockers.push(format!("SQLite quick_check 실패: {integrity}"));
    }
    if schema_version == 0 {
        blockers.push("Investa 스키마 버전이 기록되지 않은 파일입니다.".to_owned());
    } else if schema_version > SCHEMA_VERSION {
        blockers.push(format!(
            "현재 앱보다 새로운 백업입니다. 백업 {schema_version}, 앱 {SCHEMA_VERSION}"
        ));
    }
    let table_count = |table: &str| -> Result<u64, String> {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                params![table],
                |row| row.get(0),
            )
            .map_err(|error| format!("백업 테이블 구성을 확인하지 못했습니다: {error}"))?;
        if !exists {
            return Ok(0);
        }
        let sql = format!("SELECT COUNT(*) FROM {table}");
        connection
            .query_row(&sql, [], |row| row.get(0))
            .map_err(|error| format!("백업 기록 수를 확인하지 못했습니다: {error}"))
    };
    let audit_event_count = table_count("audit_events")?;
    let paper_ledger_event_count = table_count("paper_ledger_events")?;
    let research_report_count = table_count("research_reports")?;
    Ok(BackupInspection {
        file_name: file_name.to_owned(),
        integrity_ok,
        schema_version,
        supported_schema_version: SCHEMA_VERSION,
        restore_ready: blockers.is_empty(),
        blockers,
        audit_event_count,
        paper_ledger_event_count,
        research_report_count,
    })
}

fn remove_rehearsal_artifacts(database_path: &Path, rehearsal_dir: &Path) -> Result<(), String> {
    for path in [
        database_path.to_path_buf(),
        database_path.with_extension("sqlite3-wal"),
        database_path.with_extension("sqlite3-shm"),
    ] {
        if path.exists() {
            fs::remove_file(&path)
                .map_err(|error| format!("격리 복구 임시 파일을 지우지 못했습니다: {error}"))?;
        }
    }
    if rehearsal_dir.exists() {
        fs::remove_dir(rehearsal_dir)
            .map_err(|error| format!("격리 복구 임시 폴더를 지우지 못했습니다: {error}"))?;
    }
    Ok(())
}

fn rehearse_local_backup(
    bridge: &PersistenceBridge,
    file_name: &str,
    rehearsed_at_ms: u64,
) -> Result<RecoveryRehearsalReceipt, String> {
    let inspection = inspect_local_backup(bridge, file_name)?;
    if !inspection.restore_ready {
        return Err(format!(
            "복원 사전검사를 통과하지 못했습니다: {}",
            inspection.blockers.join(" · ")
        ));
    }
    // 복구 훈련 직전의 운영 DB도 일관 백업으로 남긴다. 실제 DB 교체는 하지 않는다.
    let safety_backup = create_local_backup(bridge, rehearsed_at_ms)?;
    let current_database = bridge
        .database_path
        .as_ref()
        .ok_or_else(|| "메모리 테스트 저장소는 복구 훈련을 실행할 수 없습니다.".to_owned())?;
    let app_data_dir = current_database
        .parent()
        .ok_or_else(|| "로컬 저장소 폴더를 확인하지 못했습니다.".to_owned())?;
    let backup_dir = app_data_dir.join("backups");
    let canonical_backup_dir = backup_dir
        .canonicalize()
        .map_err(|error| format!("백업 폴더를 확인하지 못했습니다: {error}"))?;
    let source = backup_dir
        .join(file_name)
        .canonicalize()
        .map_err(|error| format!("훈련할 백업을 다시 확인하지 못했습니다: {error}"))?;
    if source.parent() != Some(canonical_backup_dir.as_path()) {
        return Err("앱 데이터 백업 폴더 밖의 파일은 복구 훈련에 사용할 수 없습니다.".to_owned());
    }
    let rehearsal_root = app_data_dir.join("recovery-rehearsals");
    fs::create_dir_all(&rehearsal_root)
        .map_err(|error| format!("격리 복구 상위 폴더를 만들지 못했습니다: {error}"))?;
    let rehearsal_dir = rehearsal_root.join(format!("rehearsal-{rehearsed_at_ms}"));
    fs::create_dir(&rehearsal_dir)
        .map_err(|error| format!("새 격리 복구 폴더를 만들지 못했습니다: {error}"))?;
    let rehearsal_database = rehearsal_dir.join("investa-rehearsal.sqlite3");
    if let Err(error) = fs::copy(&source, &rehearsal_database) {
        let _ = remove_rehearsal_artifacts(&rehearsal_database, &rehearsal_dir);
        return Err(format!(
            "복구 훈련용 DB 복사본을 만들지 못했습니다: {error}"
        ));
    }

    let rehearsal_result = (|| -> Result<(u32, bool, bool), String> {
        let rehearsal = PersistenceBridge::open(&rehearsal_database)?;
        let krw_replayed = rehearsal
            .paper_ledger(ledger_id_for_currency("KRW")?)
            .map(|_| true)?;
        let usd_replayed = rehearsal
            .paper_ledger(ledger_id_for_currency("USD")?)
            .map(|_| true)?;
        let connection = rehearsal
            .connection
            .lock()
            .map_err(|_| "격리 복구 DB 잠금을 획득하지 못했습니다.".to_owned())?;
        let integrity: String = connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(|error| format!("격리 복구 DB 무결성을 검사하지 못했습니다: {error}"))?;
        if integrity != "ok" {
            return Err(format!("격리 복구 DB quick_check 실패: {integrity}"));
        }
        let schema_version = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| format!("격리 복구 DB 스키마를 확인하지 못했습니다: {error}"))?;
        for (table, expected) in [
            ("audit_events", inspection.audit_event_count),
            ("paper_ledger_events", inspection.paper_ledger_event_count),
            ("research_reports", inspection.research_report_count),
        ] {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            let actual: u64 = connection
                .query_row(&sql, [], |row| row.get(0))
                .map_err(|error| format!("격리 복구 기록 수를 확인하지 못했습니다: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "격리 복구 후 {table} 기록 수가 {expected}건에서 {actual}건으로 달라졌습니다."
                ));
            }
        }
        drop(connection);
        drop(rehearsal);
        Ok((schema_version, krw_replayed, usd_replayed))
    })();
    let cleanup_result = remove_rehearsal_artifacts(&rehearsal_database, &rehearsal_dir);
    let (schema_version, krw_ledger_replayed, usd_ledger_replayed) = rehearsal_result?;
    cleanup_result?;
    append_audit(
        bridge,
        AuditEvent {
            event_id: format!("audit:recovery-rehearsal:{rehearsed_at_ms}"),
            actor: "local_user".to_owned(),
            action: "local_backup_rehearsed".to_owned(),
            target_id: file_name.to_owned(),
            previous_hash: None,
            next_hash: None,
            correlation_id: format!("recovery-rehearsal:{rehearsed_at_ms}"),
            occurred_at_ms: rehearsed_at_ms,
            detail: "운영 DB를 교체하지 않고 격리 복사본의 마이그레이션·무결성·KRW/USD 원장 재생을 검증했습니다.".to_owned(),
        },
    )?;
    Ok(RecoveryRehearsalReceipt {
        source_file_name: file_name.to_owned(),
        safety_backup_file_name: safety_backup.file_name,
        schema_version,
        audit_event_count: inspection.audit_event_count,
        paper_ledger_event_count: inspection.paper_ledger_event_count,
        research_report_count: inspection.research_report_count,
        krw_ledger_replayed,
        usd_ledger_replayed,
        isolated_copy_removed: true,
        live_order_enabled: false,
    })
}

fn create_local_backup(bridge: &PersistenceBridge, now: u64) -> Result<BackupReceipt, String> {
    let database_path = bridge
        .database_path
        .clone()
        .ok_or_else(|| "메모리 테스트 저장소는 파일 백업을 만들 수 없습니다.".to_owned())?;
    let parent = database_path
        .parent()
        .ok_or_else(|| "로컬 저장소 폴더를 확인하지 못했습니다.".to_owned())?;
    let backup_dir = parent.join("backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|error| format!("백업 폴더를 만들지 못했습니다: {error}"))?;
    let file_name = format!("investa-{now}.sqlite3");
    let destination = backup_dir.join(&file_name);
    {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "백업 중 로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        create_consistent_sqlite_backup(&connection, &destination)?;
    }
    append_audit(
        bridge,
        AuditEvent {
            event_id: format!("audit:backup:{now}"),
            actor: "local_user".to_owned(),
            action: "local_backup_created".to_owned(),
            target_id: file_name.clone(),
            previous_hash: None,
            next_hash: None,
            correlation_id: format!("backup:{now}"),
            occurred_at_ms: now,
            detail: "앱 데이터 폴더에 SQLite 일관 백업을 만들고 무결성을 확인했습니다.".to_owned(),
        },
    )?;
    Ok(BackupReceipt {
        file_name,
        created_at_ms: now,
        integrity_ok: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alerts_are_deduplicated_and_acknowledged() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        emit_alert(
            &bridge,
            "provider_down",
            AlertSeverity::Warning,
            "시세 공급자 응답이 없습니다.",
            100,
        )
        .expect("first");
        let merged = emit_alert(
            &bridge,
            "provider_down",
            AlertSeverity::Warning,
            "시세 공급자 응답이 없습니다.",
            200,
        )
        .expect("second");
        assert_eq!(merged.occurrence_count, 2);
        let connection = bridge.connection.lock().expect("lock");
        let count: u64 = connection
            .query_row("SELECT COUNT(*) FROM operational_alerts", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn reconciliation_reports_clean_empty_state() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let report = reconcile(&bridge, 100).expect("reconcile");
        assert_eq!(report.checked_count, 0);
        assert_eq!(report.mismatch_count, 0);
        assert!(!report.live_order_enabled);
        assert!(!report.state.candidate_actions_locked);
    }

    #[test]
    fn restart_lock_blocks_candidate_actions_until_clean_reconciliation() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        mark_runtime_reconciliation_required(&bridge, 50).expect("lock");
        let locked = reconciliation_state(&bridge).expect("locked state");
        assert!(locked.candidate_actions_locked);
        assert!(require_reconciliation_ready(&bridge).is_err());

        let report = reconcile(&bridge, 60).expect("clean reconciliation");
        assert_eq!(report.state.status, "ready");
        assert!(!report.state.candidate_actions_locked);
        require_reconciliation_ready(&bridge).expect("candidate actions unlocked");
    }

    #[test]
    fn approved_engine_candidate_fills_only_the_internal_ledger() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let now = now_ms().expect("time");
        {
            let connection = bridge.connection.lock().expect("lock");
            connection.execute("INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms) VALUES('run-fill','idem-fill','completed','005930','south_korea',1,'{}','{}',?1,?1)",params![now]).expect("run");
            connection.execute("INSERT INTO engine_order_candidates(candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms) VALUES('engine-cand-run-fill','run-fill','005930','south_korea','KRW','buy',1,1,70000,?1,'safety_approved',?2,?3,?3)",params![now + 60_000,json!({"costs":{"buyFeeBps":0.0,"sellFeeBps":0.0,"sellTaxBps":0.0,"slippageBps":0.0}}).to_string(),now]).expect("candidate");
            for (index, event_type) in ["candidate_created", "safety_approved"].iter().enumerate() {
                connection.execute("INSERT INTO engine_order_events(candidate_id,event_index,event_type,event_json,occurred_at_ms) VALUES('engine-cand-run-fill',?1,?2,'{}',?3)",params![index as u64,event_type,now]).expect("event");
            }
        }
        let snapshot =
            approve_engine_candidate(&bridge, "engine-cand-run-fill", now + 1).expect("approve");
        assert_eq!(snapshot.account.positions["005930"].quantity, 1);
        assert!(!snapshot.live_order_enabled);
        let candidate = load_engine_candidate(&bridge, "engine-cand-run-fill").expect("candidate");
        assert_eq!(candidate.status, EngineCandidateStatus::Filled);
        let connection = bridge.connection.lock().expect("lock");
        let audit_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE action='internal_paper_order_filled'",
                [],
                |row| row.get(0),
            )
            .expect("audit");
        assert_eq!(audit_count, 1);
    }

    #[test]
    fn reconciliation_repairs_a_crash_after_ledger_append() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let now = now_ms().expect("time");
        let account =
            paper_trading::load_or_open_account_for_currency(&bridge, "KRW").expect("account");
        {
            let connection = bridge.connection.lock().expect("lock");
            connection.execute("INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms) VALUES('run-reconcile','idem-reconcile','completed','005930','korea',1,'{}','{}',?1,?1)",params![now]).expect("run");
            connection.execute("INSERT INTO engine_order_candidates(candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms) VALUES('engine-cand-run-reconcile','run-reconcile','005930','korea','KRW','buy',1,1,70000,?1,'submitted','{}',?2,?2)",params![now+60_000,now]).expect("candidate");
            for (index, event_type) in [
                "candidate_created",
                "safety_approved",
                "user_approved",
                "submitted",
            ]
            .iter()
            .enumerate()
            {
                connection.execute("INSERT INTO engine_order_events(candidate_id,event_index,event_type,event_json,occurred_at_ms) VALUES('engine-cand-run-reconcile',?1,?2,'{}',?3)",params![index as u64,event_type,now]).expect("event");
            }
        }
        let mut ledger = bridge
            .paper_ledger(ledger_id_for_currency("KRW").expect("ledger id"))
            .expect("ledger");
        execute_shadow_order(
            &mut ledger,
            ShadowOrderRequest {
                account_id: account.account_id,
                order_id: "order-engine-cand-run-reconcile".to_owned(),
                idempotency_key: "engine-cand-run-reconcile".to_owned(),
                symbol: "005930".to_owned(),
                currency: "KRW".to_owned(),
                side: TradeSide::Buy,
                quantity: 1,
                quantity_scale: 1,
                reference_price_minor: 70000,
                occurred_at_ms: now.max(account.last_event_at_ms),
            },
            crate::simulation::TradingCosts {
                buy_fee_bps: 0.0,
                sell_fee_bps: 0.0,
                sell_tax_bps: 0.0,
                slippage_bps: 0.0,
            },
        )
        .expect("ledger fill");
        let report = reconcile(&bridge, now + 1).expect("reconcile");
        assert_eq!(report.repaired_count, 1);
        assert_eq!(
            load_engine_candidate(&bridge, "engine-cand-run-reconcile")
                .expect("candidate")
                .status,
            EngineCandidateStatus::Filled
        );
    }

    #[test]
    fn reconciliation_cancels_an_internal_submission_without_a_ledger_fill() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let now = 1_000;
        {
            let connection = bridge.connection.lock().expect("lock");
            connection.execute("INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms) VALUES('run-no-fill','idem-no-fill','completed','005930','korea',1,'{}','{}',?1,?1)",params![now]).expect("run");
            connection.execute("INSERT INTO engine_order_candidates(candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms) VALUES('engine-cand-run-no-fill','run-no-fill','005930','korea','KRW','buy',1,1,70000,60000,'submitted','{}',?1,?1)",params![now]).expect("candidate");
            for (index, event_type) in [
                "candidate_created",
                "safety_approved",
                "user_approved",
                "submitted",
            ]
            .iter()
            .enumerate()
            {
                connection.execute("INSERT INTO engine_order_events(candidate_id,event_index,event_type,event_json,occurred_at_ms) VALUES('engine-cand-run-no-fill',?1,?2,'{}',?3)",params![index as u64,event_type,now]).expect("event");
            }
        }

        let report = reconcile(&bridge, now + 1).expect("reconcile");
        assert_eq!(report.repaired_count, 1);
        assert_eq!(report.mismatch_count, 0);
        assert_eq!(
            load_engine_candidate(&bridge, "engine-cand-run-no-fill")
                .expect("candidate")
                .status,
            EngineCandidateStatus::Cancelled
        );
    }

    #[test]
    fn provider_health_latest_event_controls_readiness() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        {
            let connection = bridge.connection.lock().expect("lock");
            connection.execute("INSERT INTO provider_health_events(event_id,component_id,critical,healthy,retry_action,detail,observed_at_ms) VALUES('h1','ledger',1,1,'retry','ok',1),('h2','ledger',1,0,'retry','mismatch',2)",[]).expect("events");
        }
        let connection = bridge.connection.lock().expect("lock");
        let latest:i64=connection.query_row("SELECT healthy FROM provider_health_events WHERE component_id='ledger' ORDER BY observed_at_ms DESC LIMIT 1",[],|row|row.get(0)).expect("latest");
        assert_eq!(latest, 0);
        drop(connection);
        let report = provider_health_report_at(&bridge, 3, 10).expect("report");
        assert!(!report.automated_trading_ready);
        assert_eq!(report.components.len(), 1);
    }

    #[test]
    fn stale_provider_success_is_not_trading_ready() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        {
            let connection = bridge.connection.lock().expect("lock");
            connection.execute("INSERT INTO provider_health_events(event_id,component_id,critical,healthy,retry_action,detail,observed_at_ms) VALUES('stale','market',1,1,'refresh','last success',100)",[]).expect("event");
        }
        let report = provider_health_report_at(&bridge, 1_000, 100).expect("report");
        assert!(!report.automated_trading_ready);
        assert!(!report.components[0].healthy);
        assert!(report.components[0].detail.contains("만료"));
    }

    #[test]
    fn local_health_refresh_records_sqlite_and_both_ledgers() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let report = refresh_local_health(&bridge, 1_000).expect("refresh");
        assert!(report.automated_trading_ready);
        assert_eq!(report.components.len(), 3);
        assert!(report.components.iter().all(|component| component.healthy));
        let count: u64 = bridge
            .connection
            .lock()
            .expect("lock")
            .query_row("SELECT COUNT(*) FROM provider_health_events", [], |row| {
                row.get(0)
            })
            .expect("health count");
        assert_eq!(count, 3);
    }

    #[test]
    fn file_backup_is_checked_and_audited() {
        let test_root =
            std::env::temp_dir().join(format!("investa-runtime-backup-{}", std::process::id()));
        fs::create_dir_all(&test_root).expect("test dir");
        let database_path = test_root.join("investa.sqlite3");
        let bridge = PersistenceBridge::open(&database_path).expect("database");
        let receipt = create_local_backup(&bridge, 123_456).expect("backup");
        assert!(receipt.integrity_ok);
        let backup_path = test_root.join("backups").join(&receipt.file_name);
        assert!(backup_path.is_file());
        let audit_count: u64 = bridge
            .connection
            .lock()
            .expect("lock")
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE action='local_backup_created'",
                [],
                |row| row.get(0),
            )
            .expect("audit");
        assert_eq!(audit_count, 1);
        drop(bridge);
        fs::remove_file(&backup_path).expect("remove backup");
        fs::remove_dir(test_root.join("backups")).expect("remove backup dir");
        for path in [
            database_path.clone(),
            test_root.join("investa.sqlite3-wal"),
            test_root.join("investa.sqlite3-shm"),
        ] {
            if path.exists() {
                fs::remove_file(path).expect("remove database artifact");
            }
        }
        fs::remove_dir(test_root).expect("remove test dir");
    }

    #[test]
    fn backup_preflight_is_read_only_and_rejects_path_traversal() {
        let test_root = std::env::temp_dir().join(format!(
            "investa-backup-preflight-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&test_root).expect("test dir");
        let database_path = test_root.join("investa.sqlite3");
        let bridge = PersistenceBridge::open(&database_path).expect("database");
        let receipt = create_local_backup(&bridge, 223_344).expect("backup");
        let backup_path = test_root.join("backups").join(&receipt.file_name);
        let before = fs::metadata(&backup_path).expect("before metadata").len();
        let inspection = inspect_local_backup(&bridge, &receipt.file_name).expect("inspect");
        let after = fs::metadata(&backup_path).expect("after metadata").len();
        assert!(inspection.integrity_ok);
        assert!(inspection.restore_ready);
        assert_eq!(inspection.schema_version, SCHEMA_VERSION);
        assert_eq!(before, after);
        assert!(inspect_local_backup(&bridge, "../investa-223344.sqlite3").is_err());
        assert!(inspect_local_backup(&bridge, "other.sqlite3").is_err());

        drop(bridge);
        fs::remove_file(backup_path).expect("remove backup");
        fs::remove_dir(test_root.join("backups")).expect("remove backup dir");
        for path in [
            database_path.clone(),
            test_root.join("investa.sqlite3-wal"),
            test_root.join("investa.sqlite3-shm"),
        ] {
            if path.exists() {
                fs::remove_file(path).expect("remove database artifact");
            }
        }
        fs::remove_dir(test_root).expect("remove test dir");
    }

    #[test]
    fn backup_inventory_is_bounded_and_recovery_evidence_contains_no_database_payload() {
        let test_root = std::env::temp_dir().join(format!(
            "investa-backup-inventory-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&test_root).expect("test dir");
        let database_path = test_root.join("investa.sqlite3");
        let bridge = PersistenceBridge::open(&database_path).expect("database");
        let older = create_local_backup(&bridge, 10).expect("older backup");
        let newer = create_local_backup(&bridge, 20).expect("newer backup");
        fs::write(
            test_root.join("backups").join("investa-15.sqlite3"),
            b"not a sqlite database",
        )
        .expect("corrupt fixture");
        fs::write(test_root.join("backups").join("notes.txt"), b"ignore").expect("noise");
        fs::create_dir(test_root.join("backups").join("investa-30.sqlite3")).expect("noise dir");

        let inventory = backup_inventory(&bridge).expect("inventory");
        assert_eq!(inventory.len(), 3);
        assert_eq!(inventory[0].file_name, newer.file_name);
        assert_eq!(inventory[2].file_name, older.file_name);
        assert!(inventory[0].inspection.restore_ready);
        assert!(!inventory[1].inspection.restore_ready);
        assert_eq!(inventory[1].inspection.schema_version, 0);
        assert!(inventory[2].inspection.restore_ready);

        let source_path = test_root.join("backups").join(&newer.file_name);
        let source_size = fs::metadata(&source_path).expect("source metadata").len();
        let receipt = export_recovery_evidence(&bridge, &newer.file_name, 30).expect("evidence");
        assert!(!receipt.live_order_enabled);
        assert_eq!(
            source_size,
            fs::metadata(&source_path).expect("source after").len()
        );
        let evidence = fs::read_to_string(test_root.join("exports").join(&receipt.file_name))
            .expect("read evidence");
        let parsed: Value = serde_json::from_str(&evidence).expect("parse evidence");
        assert_eq!(parsed["schema"], "investa.recovery-evidence.v1");
        assert_eq!(parsed["backup"]["fileName"], newer.file_name);
        assert!(parsed.get("events").is_none());

        drop(bridge);
        fs::remove_dir_all(&test_root).expect("remove test root");
    }

    #[test]
    fn recovery_rehearsal_replays_an_isolated_copy_and_preserves_the_live_database() {
        let test_root = std::env::temp_dir().join(format!(
            "investa-recovery-rehearsal-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&test_root).expect("test dir");
        let database_path = test_root.join("investa.sqlite3");
        let bridge = PersistenceBridge::open(&database_path).expect("database");
        paper_trading::load_or_open_account_for_currency(&bridge, "KRW").expect("krw ledger");
        paper_trading::load_or_open_account_for_currency(&bridge, "USD").expect("usd ledger");
        let source = create_local_backup(&bridge, 300).expect("source backup");
        let live_before: u64 = bridge
            .connection
            .lock()
            .expect("lock")
            .query_row("SELECT COUNT(*) FROM paper_ledger_events", [], |row| {
                row.get(0)
            })
            .expect("live count");

        let receipt = rehearse_local_backup(&bridge, &source.file_name, 301).expect("rehearsal");
        assert_eq!(receipt.schema_version, SCHEMA_VERSION);
        assert!(receipt.krw_ledger_replayed && receipt.usd_ledger_replayed);
        assert!(receipt.isolated_copy_removed);
        assert!(!receipt.live_order_enabled);
        assert!(!test_root
            .join("recovery-rehearsals")
            .join("rehearsal-301")
            .exists());
        let live_after: u64 = bridge
            .connection
            .lock()
            .expect("lock")
            .query_row("SELECT COUNT(*) FROM paper_ledger_events", [], |row| {
                row.get(0)
            })
            .expect("live count");
        assert_eq!(live_before, live_after);

        drop(bridge);
        for name in [source.file_name, receipt.safety_backup_file_name] {
            fs::remove_file(test_root.join("backups").join(name)).expect("remove backup");
        }
        fs::remove_dir(test_root.join("backups")).expect("remove backup dir");
        fs::remove_dir(test_root.join("recovery-rehearsals")).expect("remove rehearsal root");
        for path in [
            database_path.clone(),
            test_root.join("investa.sqlite3-wal"),
            test_root.join("investa.sqlite3-shm"),
        ] {
            if path.exists() {
                fs::remove_file(path).expect("remove database artifact");
            }
        }
        fs::remove_dir(test_root).expect("remove test dir");
    }

    #[test]
    fn audit_export_is_bounded_to_app_data_and_preserves_source_events() {
        let test_root =
            std::env::temp_dir().join(format!("investa-audit-export-{}", Uuid::new_v4().simple()));
        fs::create_dir_all(&test_root).expect("test dir");
        let database_path = test_root.join("investa.sqlite3");
        let bridge = PersistenceBridge::open(&database_path).expect("database");
        append_audit(
            &bridge,
            AuditEvent {
                event_id: "audit:test:1".to_owned(),
                actor: "test".to_owned(),
                action: "tested".to_owned(),
                target_id: "fixture".to_owned(),
                previous_hash: None,
                next_hash: None,
                correlation_id: "test:1".to_owned(),
                occurred_at_ms: 10,
                detail: "내보내기 원본".to_owned(),
            },
        )
        .expect("source event");

        let receipt = export_audit_events(&bridge, 20).expect("export");
        assert_eq!(receipt.event_count, 1);
        assert_eq!(receipt.sha256.len(), 64);
        let export_path = test_root.join("exports").join(&receipt.file_name);
        let document: Value = serde_json::from_slice(&fs::read(&export_path).expect("read export"))
            .expect("parse export");
        assert_eq!(document["schema"], "investa.audit-export.v1");
        assert_eq!(document["eventCount"], 1);
        assert_eq!(document["events"][0]["eventId"], "audit:test:1");
        let original_count: u64 = bridge
            .connection
            .lock()
            .expect("lock")
            .query_row(
                "SELECT COUNT(*) FROM audit_events WHERE event_id='audit:test:1' AND detail='내보내기 원본'",
                [],
                |row| row.get(0),
            )
            .expect("original count");
        assert_eq!(original_count, 1);

        drop(bridge);
        fs::remove_file(export_path).expect("remove export");
        fs::remove_dir(test_root.join("exports")).expect("remove export dir");
        for path in [
            database_path.clone(),
            test_root.join("investa.sqlite3-wal"),
            test_root.join("investa.sqlite3-shm"),
        ] {
            if path.exists() {
                fs::remove_file(path).expect("remove database artifact");
            }
        }
        fs::remove_dir(test_root).expect("remove test dir");
    }
}
