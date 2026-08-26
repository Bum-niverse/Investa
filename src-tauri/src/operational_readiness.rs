use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::persistence::{now_ms, PersistenceBridge};

const MAX_TEXT_LEN: usize = 8_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePreferences {
    pub display_timezone: String,
    pub quiet_hours_start: u8,
    pub quiet_hours_end: u8,
    pub stale_after_seconds: u32,
    pub notify_warning: bool,
    pub notify_critical: bool,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            display_timezone: "Asia/Seoul".to_owned(),
            quiet_hours_start: 23,
            quiet_hours_end: 7,
            stale_after_seconds: 300,
            notify_warning: true,
            notify_critical: true,
        }
    }
}

fn validate_preferences(value: &WorkspacePreferences) -> Result<(), String> {
    if !matches!(
        value.display_timezone.as_str(),
        "Asia/Seoul" | "America/New_York" | "UTC"
    ) {
        return Err("표시 시간대는 KST, ET 또는 UTC만 선택할 수 있습니다.".to_owned());
    }
    if value.quiet_hours_start > 23 || value.quiet_hours_end > 23 {
        return Err("방해 금지 시간은 0~23시여야 합니다.".to_owned());
    }
    if !(30..=86_400).contains(&value.stale_after_seconds) {
        return Err("데이터 만료 기준은 30초~24시간이어야 합니다.".to_owned());
    }
    Ok(())
}

#[tauri::command]
pub fn workspace_preferences_get(
    bridge: State<'_, PersistenceBridge>,
) -> Result<WorkspacePreferences, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "설정 저장소 잠금에 실패했습니다.".to_owned())?;
    let raw = connection
        .query_row(
            "SELECT preferences_json FROM workspace_preferences WHERE singleton_id=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("워크스페이스 설정을 읽지 못했습니다: {error}"))?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("워크스페이스 설정 형식이 올바르지 않습니다: {error}"))
}

#[tauri::command]
pub fn workspace_preferences_save(
    bridge: State<'_, PersistenceBridge>,
    preferences: WorkspacePreferences,
) -> Result<WorkspacePreferences, String> {
    validate_preferences(&preferences)?;
    let raw = serde_json::to_string(&preferences)
        .map_err(|error| format!("설정을 직렬화하지 못했습니다: {error}"))?;
    let observed_at_ms = now_ms()?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "설정 저장소 잠금에 실패했습니다.".to_owned())?;
    connection.execute(
        "INSERT INTO workspace_preferences(singleton_id,preferences_json,updated_at_ms) VALUES(1,?1,?2)
         ON CONFLICT(singleton_id) DO UPDATE SET preferences_json=excluded.preferences_json,updated_at_ms=excluded.updated_at_ms",
        rusqlite::params![raw, observed_at_ms],
    ).map_err(|error| format!("워크스페이스 설정을 저장하지 못했습니다: {error}"))?;
    Ok(preferences)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketAssetClass {
    KrEquity,
    UsEquity,
    CoinSpot,
    EquityFuture,
    CoinFuture,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketOrderInput {
    pub asset_class: MarketAssetClass,
    pub currency: String,
    pub price_minor: u64,
    pub quantity_base_units: u64,
    pub quantity_scale: u64,
    pub lot_size_base_units: u64,
    pub price_tick_minor: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedMarketOrder {
    pub currency: String,
    pub price_minor: u64,
    pub quantity_base_units: u64,
    pub notional_minor: u64,
    pub warnings: Vec<String>,
}

pub fn normalize_market_order(input: &MarketOrderInput) -> Result<NormalizedMarketOrder, String> {
    let expected_currency = match input.asset_class {
        MarketAssetClass::KrEquity | MarketAssetClass::CoinSpot => "KRW",
        MarketAssetClass::UsEquity => "USD",
        MarketAssetClass::EquityFuture | MarketAssetClass::CoinFuture => input.currency.as_str(),
    };
    if input.currency != expected_currency
        || input.price_minor == 0
        || input.quantity_base_units == 0
        || input.quantity_scale == 0
        || input.lot_size_base_units == 0
        || input.price_tick_minor == 0
    {
        return Err("통화·가격·수량·호가·주문 단위가 시장 규칙과 맞지 않습니다.".to_owned());
    }
    if input.quantity_base_units % input.lot_size_base_units != 0 {
        return Err("수량이 시장의 최소 주문 단위와 맞지 않습니다.".to_owned());
    }
    if input.price_minor % input.price_tick_minor != 0 {
        return Err("가격이 시장의 호가 단위와 맞지 않습니다.".to_owned());
    }
    let notional = u128::from(input.price_minor)
        .checked_mul(u128::from(input.quantity_base_units))
        .ok_or_else(|| "주문 금액이 지원 범위를 초과했습니다.".to_owned())?
        / u128::from(input.quantity_scale);
    Ok(NormalizedMarketOrder {
        currency: input.currency.clone(),
        price_minor: input.price_minor,
        quantity_base_units: input.quantity_base_units,
        notional_minor: u64::try_from(notional)
            .map_err(|_| "주문 금액이 지원 범위를 초과했습니다.".to_owned())?,
        warnings: if matches!(
            input.asset_class,
            MarketAssetClass::EquityFuture | MarketAssetClass::CoinFuture
        ) {
            vec!["공식 상품 마스터가 연결되기 전에는 내부 sandbox 가정만 사용합니다.".to_owned()]
        } else {
            Vec::new()
        },
    })
}

#[tauri::command]
pub fn market_order_normalize(request: MarketOrderInput) -> Result<NormalizedMarketOrder, String> {
    normalize_market_order(&request)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFailure {
    pub status_code: Option<u16>,
    pub timed_out: bool,
    pub attempt: u8,
    pub mutating: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RetryDecision {
    pub retry: bool,
    pub delay_ms: u64,
    pub fail_closed: bool,
    pub reason: String,
}

pub fn retry_decision(failure: &ProviderFailure) -> RetryDecision {
    let retryable = failure.timed_out
        || failure.status_code == Some(429)
        || failure
            .status_code
            .is_some_and(|value| (500..=599).contains(&value));
    let retry = retryable && failure.attempt < 4 && !failure.mutating;
    let delay_ms = if retry {
        500_u64.saturating_mul(2_u64.pow(u32::from(failure.attempt.min(4))))
    } else {
        0
    };
    RetryDecision {
        retry,
        delay_ms,
        fail_closed: !retry || failure.mutating,
        reason: if failure.mutating {
            "변경 요청은 자동 재시도하지 않고 멱등성 키로 상태를 대사합니다."
        } else if retry {
            "조회 요청을 제한 횟수 안에서 지수 백오프로 재시도합니다."
        } else {
            "재시도 한도를 초과해 신규 진입을 잠급니다."
        }
        .to_owned(),
    }
}

#[tauri::command]
pub fn provider_retry_decision(request: ProviderFailure) -> RetryDecision {
    retry_decision(&request)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    OfficialFact,
    NewsFact,
    CommunitySentiment,
    DiffusionMetric,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SynthesisEvidence {
    pub evidence_id: String,
    pub source_revision_id: String,
    pub kind: EvidenceKind,
    pub text: String,
    pub observed_at_ms: u64,
    pub available_at_ms: u64,
    pub corroborated: bool,
    pub suspected_automation: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceSynthesis {
    pub facts: Vec<SynthesisEvidence>,
    pub sentiment: Vec<SynthesisEvidence>,
    pub diffusion: Vec<SynthesisEvidence>,
    pub excluded_evidence_ids: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn synthesize_evidence(
    as_of_ms: u64,
    input: Vec<SynthesisEvidence>,
) -> Result<EvidenceSynthesis, String> {
    let mut seen = BTreeSet::new();
    let mut facts = Vec::new();
    let mut sentiment = Vec::new();
    let mut diffusion = Vec::new();
    let mut excluded = Vec::new();
    for item in input {
        if item.evidence_id.trim().is_empty()
            || item.source_revision_id.trim().is_empty()
            || item.text.trim().is_empty()
            || item.text.len() > MAX_TEXT_LEN
        {
            return Err("근거 ID·원천 리비전·본문 길이를 확인해 주세요.".to_owned());
        }
        if item.available_at_ms > as_of_ms
            || !seen.insert((item.source_revision_id.clone(), item.text.clone()))
        {
            excluded.push(item.evidence_id);
            continue;
        }
        match item.kind {
            EvidenceKind::OfficialFact | EvidenceKind::NewsFact => facts.push(item),
            EvidenceKind::CommunitySentiment => sentiment.push(item),
            EvidenceKind::DiffusionMetric => diffusion.push(item),
        }
    }
    let mut warnings = Vec::new();
    if sentiment.iter().any(|item| item.suspected_automation) {
        warnings.push(
            "커뮤니티 자료에 자동화 계정 의심 표본이 포함되어 심리 근거를 낮게 평가합니다."
                .to_owned(),
        );
    }
    if facts.iter().any(|item| !item.corroborated) {
        warnings
            .push("독립 출처로 확인되지 않은 뉴스 사실은 미확인 주장으로 유지합니다.".to_owned());
    }
    Ok(EvidenceSynthesis {
        facts,
        sentiment,
        diffusion,
        excluded_evidence_ids: excluded,
        warnings,
    })
}

#[tauri::command]
pub fn evidence_synthesis_preview(
    as_of_ms: u64,
    evidence: Vec<SynthesisEvidence>,
) -> Result<EvidenceSynthesis, String> {
    synthesize_evidence(as_of_ms, evidence)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    StrategySignal,
    UserManual,
    PeriodEnd,
    Liquidation,
    Rollover,
    Unknown,
}

impl ExitReason {
    pub fn compatible(raw: Option<&str>, is_legacy: bool) -> Self {
        match raw {
            Some("stop_loss") => Self::StopLoss,
            Some("take_profit") => Self::TakeProfit,
            Some("strategy_signal") => Self::StrategySignal,
            Some("period_end") => Self::PeriodEnd,
            Some("liquidation") => Self::Liquidation,
            Some("rollover") => Self::Rollover,
            Some("user_manual") | None if is_legacy => Self::UserManual,
            _ => Self::Unknown,
        }
    }
}

#[tauri::command]
pub fn paper_exit_reason_resolve(raw: Option<String>, legacy: bool) -> ExitReason {
    ExitReason::compatible(raw.as_deref(), legacy)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SoakSample {
    pub observed_at_ms: u64,
    pub memory_bytes: u64,
    pub timer_count: u32,
    pub sqlite_bytes: u64,
    pub candidate_key: Option<String>,
    pub provider_healthy: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SoakAudit {
    pub duration_ms: u64,
    pub duplicate_candidate_count: u32,
    pub memory_growth_bytes: i64,
    pub timer_growth: i32,
    pub sqlite_growth_bytes: i64,
    pub fail_closed: bool,
    pub warnings: Vec<String>,
}

pub fn audit_soak(samples: &[SoakSample]) -> Result<SoakAudit, String> {
    let first = samples
        .first()
        .ok_or_else(|| "내구 검사 표본이 필요합니다.".to_owned())?;
    let last = samples
        .last()
        .ok_or_else(|| "내구 검사 표본이 필요합니다.".to_owned())?;
    if samples
        .windows(2)
        .any(|pair| pair[0].observed_at_ms >= pair[1].observed_at_ms)
    {
        return Err("내구 검사 표본은 시각 오름차순이어야 합니다.".to_owned());
    }
    let mut keys = BTreeSet::new();
    let mut duplicates = 0;
    for key in samples
        .iter()
        .filter_map(|sample| sample.candidate_key.as_deref())
    {
        if !keys.insert(key) {
            duplicates += 1;
        }
    }
    let duration_ms = last.observed_at_ms.saturating_sub(first.observed_at_ms);
    let memory_growth_bytes = i128::from(last.memory_bytes) - i128::from(first.memory_bytes);
    let sqlite_growth_bytes = i128::from(last.sqlite_bytes) - i128::from(first.sqlite_bytes);
    let timer_growth = i64::from(last.timer_count) - i64::from(first.timer_count);
    let mut warnings = Vec::new();
    if duration_ms < 86_400_000 {
        warnings.push("24시간 표본이 아니므로 장시간 완료 판정은 보류합니다.".to_owned());
    }
    if duplicates > 0 {
        warnings.push("동일 완료봉 후보 중복이 감지됐습니다.".to_owned());
    }
    if memory_growth_bytes > 128 * 1024 * 1024 {
        warnings.push("메모리 증가량이 128MB 기준을 초과했습니다.".to_owned());
    }
    if timer_growth > 4 {
        warnings.push("타이머 증가량이 기준을 초과했습니다.".to_owned());
    }
    Ok(SoakAudit {
        duration_ms,
        duplicate_candidate_count: duplicates,
        memory_growth_bytes: memory_growth_bytes.clamp(i128::from(i64::MIN), i128::from(i64::MAX))
            as i64,
        timer_growth: timer_growth.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        sqlite_growth_bytes: sqlite_growth_bytes.clamp(i128::from(i64::MIN), i128::from(i64::MAX))
            as i64,
        fail_closed: samples.iter().any(|sample| !sample.provider_healthy) || duplicates > 0,
        warnings,
    })
}

#[tauri::command]
pub fn shadow_soak_audit(samples: Vec<SoakSample>) -> Result<SoakAudit, String> {
    audit_soak(&samples)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveSoakAuditRequest {
    pub run_id: String,
    pub samples: Vec<SoakSample>,
    pub simulated_timeline: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredSoakAudit {
    pub run_id: String,
    pub sample_count: usize,
    pub audit: SoakAudit,
    pub simulated_timeline: bool,
    pub actual_elapsed_qualified: bool,
    pub created_at_ms: u64,
}

fn valid_run_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn save_soak_audit(
    request: SaveSoakAuditRequest,
    bridge: &PersistenceBridge,
) -> Result<StoredSoakAudit, String> {
    if !valid_run_id(&request.run_id)
        || request.samples.len() < 2
        || request.samples.len() > 100_000
    {
        return Err("내구 검사 실행 ID와 표본 수를 확인해 주세요.".to_owned());
    }
    let audit = audit_soak(&request.samples)?;
    let created_at_ms = now_ms()?;
    let stored = StoredSoakAudit {
        run_id: request.run_id,
        sample_count: request.samples.len(),
        actual_elapsed_qualified: !request.simulated_timeline && audit.duration_ms >= 86_400_000,
        simulated_timeline: request.simulated_timeline,
        audit,
        created_at_ms,
    };
    let json = serde_json::to_string(&stored)
        .map_err(|_| "내구 검사 결과를 직렬화하지 못했습니다.".to_owned())?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "내구 검사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing: Option<String> = connection
        .query_row(
            "SELECT audit_json FROM shadow_soak_audits WHERE run_id=?1",
            params![stored.run_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("기존 내구 검사 결과를 확인하지 못했습니다: {error}"))?;
    if let Some(existing) = existing {
        let existing: StoredSoakAudit = serde_json::from_str(&existing)
            .map_err(|_| "저장된 내구 검사 결과를 해석하지 못했습니다.".to_owned())?;
        if existing.sample_count == stored.sample_count
            && existing.audit == stored.audit
            && existing.simulated_timeline == stored.simulated_timeline
        {
            return Ok(existing);
        }
        return Err("같은 실행 ID에 다른 내구 검사 결과가 이미 저장되어 있습니다.".to_owned());
    }
    connection.execute("INSERT INTO shadow_soak_audits(run_id,sample_count,audit_json,simulated_timeline,created_at_ms) VALUES(?1,?2,?3,?4,?5)",params![stored.run_id,stored.sample_count,json,stored.simulated_timeline,stored.created_at_ms])
        .map_err(|error|format!("내구 검사 결과를 저장하지 못했습니다: {error}"))?;
    Ok(stored)
}

#[tauri::command]
pub fn shadow_soak_audit_save(
    request: SaveSoakAuditRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StoredSoakAudit, String> {
    save_soak_audit(request, bridge.inner())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationsDashboardSnapshot {
    pub observed_at_ms: u64,
    pub source_timestamps_ms: BTreeMap<String, u64>,
    pub counts: BTreeMap<String, u64>,
    pub live_order_enabled: bool,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub fn operations_dashboard_snapshot(
    bridge: State<'_, PersistenceBridge>,
) -> Result<OperationsDashboardSnapshot, String> {
    let observed_at_ms = now_ms()?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "운영 저장소 잠금에 실패했습니다.".to_owned())?;
    let queries = [
        ("engineRuns", "SELECT COUNT(*) FROM engine_runs"),
        (
            "pendingCandidates",
            "SELECT COUNT(*) FROM engine_order_candidates WHERE status='safety_approved'",
        ),
        (
            "unacknowledgedAlerts",
            "SELECT COUNT(*) FROM operational_alerts WHERE acknowledged_at_ms IS NULL",
        ),
        (
            "activeShadowWatches",
            "SELECT COUNT(*) FROM shadow_watches WHERE enabled=1",
        ),
        (
            "savedForecasts",
            "SELECT COUNT(*) FROM probability_forecasts",
        ),
    ];
    let mut counts = BTreeMap::new();
    for (key, sql) in queries {
        let value: u64 = connection
            .query_row(sql, [], |row| row.get(0))
            .map_err(|error| format!("운영 지표를 집계하지 못했습니다: {error}"))?;
        counts.insert(key.to_owned(), value);
    }
    let mut source_timestamps_ms = BTreeMap::new();
    for (key, sql) in [
        (
            "engine",
            "SELECT COALESCE(MAX(updated_at_ms),0) FROM engine_runs",
        ),
        (
            "provider",
            "SELECT COALESCE(MAX(observed_at_ms),0) FROM provider_health_events",
        ),
        (
            "forecast",
            "SELECT COALESCE(MAX(created_at_ms),0) FROM probability_forecasts",
        ),
    ] {
        let value: u64 = connection
            .query_row(sql, [], |row| row.get(0))
            .map_err(|error| format!("운영 관측 시각을 읽지 못했습니다: {error}"))?;
        source_timestamps_ms.insert(key.to_owned(), value);
    }
    let warnings = source_timestamps_ms
        .iter()
        .filter(|(_, value)| **value == 0)
        .map(|(key, _)| format!("{key} 관측 기록이 없습니다."))
        .collect();
    Ok(OperationsDashboardSnapshot {
        observed_at_ms,
        source_timestamps_ms,
        counts,
        live_order_enabled: false,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_rules_reject_wrong_currency_and_lot() {
        let mut request = MarketOrderInput {
            asset_class: MarketAssetClass::UsEquity,
            currency: "KRW".to_owned(),
            price_minor: 100,
            quantity_base_units: 1,
            quantity_scale: 1,
            lot_size_base_units: 1,
            price_tick_minor: 1,
        };
        assert!(normalize_market_order(&request).is_err());
        request.asset_class = MarketAssetClass::KrEquity;
        request.quantity_base_units = 3;
        request.lot_size_base_units = 2;
        assert!(normalize_market_order(&request).is_err());
    }

    #[test]
    fn market_rules_preserve_krw_usd_and_fractional_coin_units() {
        for request in [
            MarketOrderInput {
                asset_class: MarketAssetClass::KrEquity,
                currency: "KRW".to_owned(),
                price_minor: 70_000,
                quantity_base_units: 2,
                quantity_scale: 1,
                lot_size_base_units: 1,
                price_tick_minor: 1,
            },
            MarketOrderInput {
                asset_class: MarketAssetClass::UsEquity,
                currency: "USD".to_owned(),
                price_minor: 20_050,
                quantity_base_units: 3,
                quantity_scale: 1,
                lot_size_base_units: 1,
                price_tick_minor: 1,
            },
            MarketOrderInput {
                asset_class: MarketAssetClass::CoinSpot,
                currency: "KRW".to_owned(),
                price_minor: 100_000_000,
                quantity_base_units: 10_000_000,
                quantity_scale: 100_000_000,
                lot_size_base_units: 1,
                price_tick_minor: 1,
            },
        ] {
            let normalized = normalize_market_order(&request).expect("representative market order");
            assert_eq!(normalized.currency, request.currency);
            assert!(normalized.notional_minor > 0);
        }
    }

    #[test]
    fn retry_is_bounded_and_mutations_fail_closed() {
        assert!(
            retry_decision(&ProviderFailure {
                status_code: Some(429),
                timed_out: false,
                attempt: 1,
                mutating: false
            })
            .retry
        );
        let mutation = retry_decision(&ProviderFailure {
            status_code: Some(503),
            timed_out: false,
            attempt: 0,
            mutating: true,
        });
        assert!(!mutation.retry && mutation.fail_closed);
    }

    #[test]
    fn synthesis_separates_sources_and_blocks_future_and_duplicate_revisions() {
        let base = SynthesisEvidence {
            evidence_id: "a".to_owned(),
            source_revision_id: "r1".to_owned(),
            kind: EvidenceKind::NewsFact,
            text: "확인 필요".to_owned(),
            observed_at_ms: 1,
            available_at_ms: 2,
            corroborated: false,
            suspected_automation: false,
        };
        let mut future = base.clone();
        future.evidence_id = "b".to_owned();
        future.source_revision_id = "r2".to_owned();
        future.available_at_ms = 20;
        let mut social = base.clone();
        social.evidence_id = "c".to_owned();
        social.source_revision_id = "r3".to_owned();
        social.kind = EvidenceKind::CommunitySentiment;
        social.text = "긍정".to_owned();
        social.suspected_automation = true;
        let output = synthesize_evidence(10, vec![base, future, social]).expect("synthesis");
        assert_eq!(output.facts.len(), 1);
        assert_eq!(output.sentiment.len(), 1);
        assert_eq!(output.excluded_evidence_ids, vec!["b"]);
        assert_eq!(output.warnings.len(), 2);
    }

    #[test]
    fn legacy_sell_is_manual_and_only_explicit_stop_is_stop_loss() {
        assert_eq!(ExitReason::compatible(None, true), ExitReason::UserManual);
        assert_eq!(
            ExitReason::compatible(Some("stop_loss"), false),
            ExitReason::StopLoss
        );
        assert_eq!(
            ExitReason::compatible(Some("sell"), false),
            ExitReason::Unknown
        );
    }

    #[test]
    fn soak_audit_detects_duplicate_and_provider_failure() {
        let result = audit_soak(&[
            SoakSample {
                observed_at_ms: 1,
                memory_bytes: 10,
                timer_count: 1,
                sqlite_bytes: 10,
                candidate_key: Some("bar-1".to_owned()),
                provider_healthy: true,
            },
            SoakSample {
                observed_at_ms: 2,
                memory_bytes: 20,
                timer_count: 2,
                sqlite_bytes: 20,
                candidate_key: Some("bar-1".to_owned()),
                provider_healthy: false,
            },
        ])
        .expect("audit");
        assert_eq!(result.duplicate_candidate_count, 1);
        assert!(result.fail_closed);
    }

    #[test]
    fn twenty_four_hour_replay_has_no_duplicate_but_is_not_actual_elapsed_time() {
        let samples = (0..=24)
            .map(|hour| SoakSample {
                observed_at_ms: 1 + hour * 3_600_000,
                memory_bytes: 100_000_000 + hour * 100_000,
                timer_count: 4,
                sqlite_bytes: 10_000_000 + hour * 1_000,
                candidate_key: Some(format!("completed-bar-{hour}")),
                provider_healthy: true,
            })
            .collect::<Vec<_>>();
        let bridge = PersistenceBridge::in_memory().expect("database");
        let stored = save_soak_audit(
            SaveSoakAuditRequest {
                run_id: "simulated-24h-replay".into(),
                samples: samples.clone(),
                simulated_timeline: true,
            },
            &bridge,
        )
        .expect("save 24h replay");
        assert_eq!(stored.audit.duration_ms, 86_400_000);
        assert_eq!(stored.audit.duplicate_candidate_count, 0);
        assert!(!stored.audit.fail_closed);
        assert!(stored.audit.warnings.is_empty());
        assert!(!stored.actual_elapsed_qualified);
        let replay = save_soak_audit(
            SaveSoakAuditRequest {
                run_id: "simulated-24h-replay".into(),
                samples,
                simulated_timeline: true,
            },
            &bridge,
        )
        .expect("idempotent save");
        assert_eq!(stored, replay);
        let connection = bridge.connection.lock().expect("connection");
        let count: u64 = connection
            .query_row("SELECT COUNT(*) FROM shadow_soak_audits", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 1);
    }
}
