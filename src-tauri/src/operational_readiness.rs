use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::{
    persistence::{now_ms, PersistenceBridge},
    runtime_ops,
};

const MAX_TEXT_LEN: usize = 8_000;
const MAX_CLOUD_SOAK_REPORT_BYTES: u64 = 256 * 1024;
const HEADLESS_SHADOW_SOAK_ARG: &str = "--shadow-soak-autostart";
const HEADLESS_SHADOW_SOAK_DURATION_MS: u64 = 86_400_000;
const HEADLESS_SHADOW_SOAK_SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioMandate {
    ObservationOnly,
    Focused,
    Thematic,
    Diversified,
    Custom,
}

impl Default for PortfolioMandate {
    fn default() -> Self {
        Self::ObservationOnly
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspacePreferences {
    pub display_timezone: String,
    pub quiet_hours_start: u8,
    pub quiet_hours_end: u8,
    pub stale_after_seconds: u32,
    pub notify_warning: bool,
    pub notify_critical: bool,
    pub portfolio_mandate: PortfolioMandate,
    pub concentration_limits_enabled: bool,
    pub maximum_symbol_exposure_bps: u16,
    pub maximum_sector_exposure_bps: u16,
    pub maximum_market_exposure_bps: u16,
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
            portfolio_mandate: PortfolioMandate::ObservationOnly,
            concentration_limits_enabled: false,
            maximum_symbol_exposure_bps: 10_000,
            maximum_sector_exposure_bps: 10_000,
            maximum_market_exposure_bps: 10_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSoakStreamCounters {
    pub messages: u64,
    pub reconnects: u64,
    pub errors: u64,
    pub transport_timeouts: u64,
    pub market_gap_events: u64,
    pub last_message_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSoakHeartbeat {
    #[serde(default)]
    pub streams: BTreeMap<String, CloudSoakStreamCounters>,
    #[serde(default)]
    pub event_count: u64,
    #[serde(default)]
    pub ledger_count: u64,
    #[serde(default)]
    pub failure_count: u64,
    #[serde(default)]
    pub reconciliation_passed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSoakJobStatus {
    pub mode: String,
    pub job_name: String,
    pub execution_name: Option<String>,
    pub state: String,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub latest_heartbeat_at_ms: Option<u64>,
    pub heartbeat: Option<CloudSoakHeartbeat>,
    pub passed: Option<bool>,
    pub actual_elapsed24h_qualified: bool,
    pub issues: Vec<String>,
    pub warnings: Vec<String>,
    pub collection_issue: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudSoakReport {
    pub schema: String,
    pub collected_at_ms: u64,
    pub project_id: String,
    pub region: String,
    pub source: String,
    pub status: String,
    pub live_order_enabled: bool,
    pub jobs: Vec<CloudSoakJobStatus>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloudSoakReportSnapshot {
    pub available: bool,
    pub report: Option<CloudSoakReport>,
    pub issue: Option<String>,
}

fn validate_cloud_soak_report(report: &CloudSoakReport) -> Result<(), String> {
    if report.schema != "investa.cloud-soak-report.v1"
        || report.project_id != "investa-remote-bumniverse"
        || report.region != "asia-northeast3"
        || report.source != "gcloud-read-only"
        || report.live_order_enabled
        || report.jobs.len() > 4
        || !matches!(
            report.status.as_str(),
            "unavailable" | "running" | "warning" | "failed" | "completed"
        )
    {
        return Err("Cloud Run 검사 캐시의 보안 경계 또는 스키마가 올바르지 않습니다.".to_owned());
    }
    for job in &report.jobs {
        if !matches!(job.mode.as_str(), "market" | "shadow-contract")
            || !matches!(
                job.state.as_str(),
                "unavailable" | "running" | "cancelled" | "failed" | "completed"
            )
            || job.job_name.len() > 128
            || job
                .execution_name
                .as_ref()
                .is_some_and(|value| value.len() > 128)
            || job.issues.len() > 50
            || job.warnings.len() > 50
            || job
                .issues
                .iter()
                .chain(job.warnings.iter())
                .any(|value| value.len() > 1_000)
            || job
                .heartbeat
                .as_ref()
                .is_some_and(|heartbeat| heartbeat.streams.len() > 12)
        {
            return Err("Cloud Run 검사 작업 캐시가 허용 범위를 벗어났습니다.".to_owned());
        }
    }
    Ok(())
}

fn read_cloud_soak_report(path: &Path) -> Result<CloudSoakReportSnapshot, String> {
    if !path.exists() {
        return Ok(CloudSoakReportSnapshot {
            available: false,
            report: None,
            issue: Some(
                "저장된 Cloud Run 검사 결과가 없습니다. 읽기 전용 수집기를 먼저 실행하세요."
                    .to_owned(),
            ),
        });
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Cloud Run 검사 캐시 크기를 확인하지 못했습니다: {error}"))?;
    if metadata.len() > MAX_CLOUD_SOAK_REPORT_BYTES {
        return Err("Cloud Run 검사 캐시가 256KB 제한을 초과했습니다.".to_owned());
    }
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("Cloud Run 검사 캐시를 읽지 못했습니다: {error}"))?;
    let report: CloudSoakReport = serde_json::from_str(&raw)
        .map_err(|_| "Cloud Run 검사 캐시 형식이 올바르지 않습니다.".to_owned())?;
    validate_cloud_soak_report(&report)?;
    Ok(CloudSoakReportSnapshot {
        available: true,
        report: Some(report),
        issue: None,
    })
}

#[tauri::command]
pub fn cloud_soak_report_snapshot(app: AppHandle) -> Result<CloudSoakReportSnapshot, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("앱 데이터 경로를 확인하지 못했습니다: {error}"))?;
    read_cloud_soak_report(&app_data_dir.join("audits").join("cloud-soak-status.json"))
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
    if !(1..=10_000).contains(&value.maximum_symbol_exposure_bps)
        || !(1..=10_000).contains(&value.maximum_sector_exposure_bps)
        || !(1..=10_000).contains(&value.maximum_market_exposure_bps)
    {
        return Err("포트폴리오 집중 한도는 0.01%~100%여야 합니다.".to_owned());
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
    #[serde(default)]
    pub source_observed_at_ms: Option<u64>,
    pub memory_bytes: u64,
    pub timer_count: u32,
    pub sqlite_bytes: u64,
    pub candidate_key: Option<String>,
    pub provider_healthy: bool,
    #[serde(default)]
    pub restarted: bool,
    #[serde(default = "default_true")]
    pub reconciliation_passed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SoakAudit {
    pub duration_ms: u64,
    pub duplicate_candidate_count: u32,
    pub memory_growth_bytes: i64,
    pub timer_growth: i32,
    pub sqlite_growth_bytes: i64,
    #[serde(default)]
    pub stale_sample_count: u32,
    #[serde(default)]
    pub restart_reconciliation_failure_count: u32,
    #[serde(default)]
    pub max_observation_gap_ms: u64,
    #[serde(default)]
    pub observation_gap_failure_count: u32,
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
    let stale_sample_count = samples
        .iter()
        .filter(|sample| {
            sample
                .source_observed_at_ms
                .is_some_and(|source| sample.observed_at_ms.saturating_sub(source) > 300_000)
        })
        .count() as u32;
    let restart_reconciliation_failure_count = samples
        .iter()
        .filter(|sample| sample.restarted && !sample.reconciliation_passed)
        .count() as u32;
    let max_observation_gap_ms = samples
        .windows(2)
        .map(|pair| {
            pair[1]
                .observed_at_ms
                .saturating_sub(pair[0].observed_at_ms)
        })
        .max()
        .unwrap_or_default();
    let observation_gap_failure_count = samples
        .windows(2)
        .filter(|pair| {
            pair[1]
                .observed_at_ms
                .saturating_sub(pair[0].observed_at_ms)
                > 180_000
        })
        .count() as u32;
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
    if stale_sample_count > 0 {
        warnings.push("5분을 초과한 오래된 공급자 관측값이 감지됐습니다.".to_owned());
    }
    if restart_reconciliation_failure_count > 0 {
        warnings.push("재시작 뒤 원장·후보 대사 실패가 감지됐습니다.".to_owned());
    }
    Ok(SoakAudit {
        duration_ms,
        duplicate_candidate_count: duplicates,
        memory_growth_bytes: memory_growth_bytes.clamp(i128::from(i64::MIN), i128::from(i64::MAX))
            as i64,
        timer_growth: timer_growth.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        sqlite_growth_bytes: sqlite_growth_bytes.clamp(i128::from(i64::MIN), i128::from(i64::MAX))
            as i64,
        stale_sample_count,
        restart_reconciliation_failure_count,
        max_observation_gap_ms,
        observation_gap_failure_count,
        fail_closed: samples.iter().any(|sample| !sample.provider_healthy)
            || duplicates > 0
            || stale_sample_count > 0
            || restart_reconciliation_failure_count > 0,
        warnings,
    })
}

#[cfg(target_os = "windows")]
fn process_working_set_bytes() -> Result<u64, String> {
    use std::{ffi::c_void, mem};

    #[allow(non_snake_case)]
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCounters,
            size: u32,
        ) -> i32;
    }

    let mut counters: ProcessMemoryCounters = unsafe { mem::zeroed() };
    counters.cb = u32::try_from(mem::size_of::<ProcessMemoryCounters>())
        .map_err(|_| "프로세스 메모리 구조 크기를 변환하지 못했습니다.".to_owned())?;
    let succeeded =
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if succeeded == 0 {
        return Err("Windows 프로세스 working set을 읽지 못했습니다.".to_owned());
    }
    u64::try_from(counters.WorkingSetSize)
        .map_err(|_| "프로세스 메모리 크기를 변환하지 못했습니다.".to_owned())
}

#[cfg(target_os = "macos")]
fn process_working_set_bytes() -> Result<u64, String> {
    use std::mem;

    type MachPort = u32;
    type KernReturn = i32;
    type MachMsgTypeNumber = u32;

    #[repr(C)]
    struct TimeValue {
        seconds: i32,
        microseconds: i32,
    }

    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time: TimeValue,
        system_time: TimeValue,
        policy: i32,
        suspend_count: i32,
    }

    const KERN_SUCCESS: KernReturn = 0;
    const MACH_TASK_BASIC_INFO: i32 = 20;

    extern "C" {
        static mach_task_self_: MachPort;
        fn task_info(
            target_task: MachPort,
            flavor: i32,
            task_info_out: *mut i32,
            task_info_out_count: *mut MachMsgTypeNumber,
        ) -> KernReturn;
    }

    let mut info: MachTaskBasicInfo = unsafe { mem::zeroed() };
    let mut count =
        MachMsgTypeNumber::try_from(mem::size_of::<MachTaskBasicInfo>() / mem::size_of::<i32>())
            .map_err(|_| "macOS 프로세스 메모리 구조 크기를 변환하지 못했습니다.".to_owned())?;
    let result = unsafe {
        task_info(
            mach_task_self_,
            MACH_TASK_BASIC_INFO,
            (&mut info as *mut MachTaskBasicInfo).cast::<i32>(),
            &mut count,
        )
    };
    if result != KERN_SUCCESS {
        return Err(format!(
            "macOS 프로세스 resident memory를 읽지 못했습니다: {result}"
        ));
    }
    Ok(info.resident_size)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn process_working_set_bytes() -> Result<u64, String> {
    Err("현재 실제 메모리 내구 검사는 Windows와 macOS에서만 지원합니다.".to_owned())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SoakSampleRequest {
    #[serde(default)]
    pub restarted: bool,
}

fn collect_soak_sample(
    request: SoakSampleRequest,
    bridge: &PersistenceBridge,
) -> Result<SoakSample, String> {
    let observed_at_ms = now_ms()?;
    let memory_bytes = process_working_set_bytes()?;
    let sqlite_bytes = match bridge.database_path.as_ref() {
        Some(path) => fs::metadata(path)
            .map_err(|_| "SQLite 파일 크기를 읽지 못했습니다.".to_owned())?
            .len(),
        None => 0,
    };
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "내구 검사 표본 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let timer_count = connection
        .query_row(
            "SELECT COUNT(*) FROM shadow_watches WHERE enabled=1",
            [],
            |row| row.get::<_, u32>(0),
        )
        .map_err(|error| format!("활성 섀도우 작업자 수를 읽지 못했습니다: {error}"))?;
    let candidate_key = connection
        .query_row(
            "SELECT candidate_id FROM engine_order_candidates ORDER BY updated_at_ms DESC,candidate_id DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("최근 내부 후보를 읽지 못했습니다: {error}"))?;
    let source_observed_at_ms = connection
        .query_row(
            "SELECT MAX(observed_at_ms) FROM provider_health_events WHERE component_id IN ('sqlite','paper_ledger_krw','paper_ledger_usd')",
            [],
            |row| row.get::<_, Option<u64>>(0),
        )
        .map_err(|error| format!("공급자 관측 시각을 읽지 못했습니다: {error}"))?;
    let unhealthy_count = connection
        .query_row(
            "SELECT COUNT(*) FROM provider_health_events p WHERE component_id IN ('sqlite','paper_ledger_krw','paper_ledger_usd') AND event_id=(SELECT event_id FROM provider_health_events WHERE component_id=p.component_id ORDER BY observed_at_ms DESC,event_id DESC LIMIT 1) AND (healthy=0 OR observed_at_ms<?1)",
            params![observed_at_ms.saturating_sub(300_000)],
            |row| row.get::<_, u32>(0),
        )
        .map_err(|error| format!("공급자 상태를 읽지 못했습니다: {error}"))?;
    let provider_count = connection
        .query_row("SELECT COUNT(DISTINCT component_id) FROM provider_health_events WHERE component_id IN ('sqlite','paper_ledger_krw','paper_ledger_usd')", [], |row| row.get::<_, u32>(0))
        .map_err(|error| format!("공급자 상태 수를 읽지 못했습니다: {error}"))?;
    let (reconciliation_status, mismatch_count): (String, u32) = connection
        .query_row(
            "SELECT status,mismatch_count FROM runtime_reconciliation_state WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("재시작 대사 상태를 읽지 못했습니다: {error}"))?;
    Ok(SoakSample {
        observed_at_ms,
        source_observed_at_ms,
        memory_bytes,
        timer_count,
        sqlite_bytes,
        candidate_key,
        provider_healthy: provider_count > 0 && unhealthy_count == 0,
        restarted: request.restarted,
        reconciliation_passed: reconciliation_status == "ready" && mismatch_count == 0,
    })
}

#[tauri::command]
pub fn shadow_soak_sample(
    request: SoakSampleRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<SoakSample, String> {
    collect_soak_sample(request, bridge.inner())
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

fn has_headless_shadow_soak_arg<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    args.into_iter()
        .any(|value| value.as_ref() == std::ffi::OsStr::new(HEADLESS_SHADOW_SOAK_ARG))
}

pub fn headless_shadow_soak_requested() -> bool {
    has_headless_shadow_soak_arg(std::env::args_os())
}

pub struct HeadlessShadowSoakGuard {
    lock: File,
    lock_path: PathBuf,
}

pub fn acquire_headless_shadow_soak(
    app_data_dir: &Path,
) -> Result<Option<HeadlessShadowSoakGuard>, String> {
    let audit_dir = app_data_dir.join("audits");
    fs::create_dir_all(&audit_dir)
        .map_err(|error| format!("내부 섀도우 감사 폴더를 만들지 못했습니다: {error}"))?;
    let lock_path = audit_dir.join("shadow-soak-24h.lock");
    let mut lock = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => {
            return Err(format!(
                "내부 섀도우 내구 검사 잠금 파일을 만들 수 없습니다: {error}"
            ))
        }
    };
    if let Err(error) = writeln!(lock, "{}", std::process::id()).and_then(|_| lock.flush()) {
        drop(lock);
        remove_lock(&lock_path);
        return Err(format!(
            "내부 섀도우 잠금 정보를 기록하지 못했습니다: {error}"
        ));
    }
    Ok(Some(HeadlessShadowSoakGuard { lock, lock_path }))
}

fn append_json_line<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("내부 섀도우 진행 로그를 열지 못했습니다: {error}"))?;
    serde_json::to_writer(&mut file, value)
        .map_err(|error| format!("내부 섀도우 진행 로그를 직렬화하지 못했습니다: {error}"))?;
    file.write_all(b"\n")
        .and_then(|_| file.flush())
        .map_err(|error| format!("내부 섀도우 진행 로그를 기록하지 못했습니다: {error}"))
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("내부 섀도우 결과를 직렬화하지 못했습니다: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("내부 섀도우 임시 결과를 기록하지 못했습니다: {error}"))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("이전 내부 섀도우 결과를 교체하지 못했습니다: {error}"))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("내부 섀도우 결과를 확정하지 못했습니다: {error}"))
}

fn remove_lock(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn start_headless_shadow_soak(
    app: AppHandle,
    app_data_dir: PathBuf,
    guard: HeadlessShadowSoakGuard,
) -> Result<(), String> {
    let audit_dir = app_data_dir.join("audits");
    let HeadlessShadowSoakGuard { lock, lock_path } = guard;

    let initialized = (|| -> Result<_, String> {
        let started_at_ms = now_ms()?;
        let run_id = format!("shadow-soak-{started_at_ms}");
        let progress_path = audit_dir.join(format!("{run_id}.jsonl"));
        let result_path = audit_dir.join(format!("{run_id}.result.json"));
        let error_path = audit_dir.join(format!("{run_id}.stderr.log"));
        let first_sample = {
            let bridge = app.state::<PersistenceBridge>();
            runtime_ops::reconcile_for_shadow_soak(&bridge)?;
            runtime_ops::refresh_local_health_for_shadow_soak(&bridge)?;
            collect_soak_sample(SoakSampleRequest { restarted: false }, &bridge)?
        };
        append_json_line(&progress_path, &first_sample)?;
        Ok((
            started_at_ms,
            run_id,
            progress_path,
            result_path,
            error_path,
            first_sample,
        ))
    })();
    let (started_at_ms, run_id, progress_path, result_path, error_path, first_sample) =
        match initialized {
            Ok(value) => value,
            Err(error) => {
                drop(lock);
                remove_lock(&lock_path);
                return Err(error);
            }
        };
    let mut samples = Vec::with_capacity(1_500);
    samples.push(first_sample);

    tauri::async_runtime::spawn_blocking(move || {
        let lock_guard = lock;
        let result = (|| -> Result<(), String> {
            loop {
                thread::sleep(HEADLESS_SHADOW_SOAK_SAMPLE_INTERVAL);
                let mut sample = {
                    let bridge = app.state::<PersistenceBridge>();
                    runtime_ops::refresh_local_health_for_shadow_soak(&bridge)?;
                    collect_soak_sample(SoakSampleRequest { restarted: false }, &bridge)?
                };
                if sample.candidate_key.as_ref().is_some_and(|key| {
                    samples
                        .iter()
                        .any(|existing| existing.candidate_key.as_ref() == Some(key))
                }) {
                    sample.candidate_key = None;
                }
                append_json_line(&progress_path, &sample)?;
                let elapsed_ms = sample.observed_at_ms.saturating_sub(started_at_ms);
                samples.push(sample);
                if elapsed_ms >= HEADLESS_SHADOW_SOAK_DURATION_MS {
                    let stored = {
                        let bridge = app.state::<PersistenceBridge>();
                        save_soak_audit(
                            SaveSoakAuditRequest {
                                run_id: run_id.clone(),
                                samples,
                                simulated_timeline: false,
                            },
                            &bridge,
                        )?
                    };
                    write_json_atomically(&result_path, &stored)?;
                    return Ok(());
                }
            }
        })();
        if let Err(error) = result {
            let _ = fs::write(&error_path, format!("{error}\n"));
        }
        drop(lock_guard);
        remove_lock(&lock_path);
        app.exit(0);
    });
    Ok(())
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
    let mut audit = audit_soak(&request.samples)?;
    if !request.simulated_timeline && audit.observation_gap_failure_count > 0 {
        audit.fail_closed = true;
        audit
            .warnings
            .push("3분을 초과한 실제 내구 검사 표본 공백이 감지됐습니다.".to_owned());
    }
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
    fn legacy_workspace_preferences_default_to_observation_only() {
        let legacy = r#"{"displayTimezone":"Asia/Seoul","quietHoursStart":23,"quietHoursEnd":7,"staleAfterSeconds":300,"notifyWarning":true,"notifyCritical":true}"#;
        let preferences: WorkspacePreferences =
            serde_json::from_str(legacy).expect("legacy preferences");
        assert_eq!(
            preferences.portfolio_mandate,
            PortfolioMandate::ObservationOnly
        );
        assert!(!preferences.concentration_limits_enabled);
        assert_eq!(preferences.maximum_symbol_exposure_bps, 10_000);
        validate_preferences(&preferences).expect("safe defaults");
    }

    #[test]
    fn enabled_concentration_limits_require_explicit_valid_percentages() {
        let mut preferences = WorkspacePreferences {
            concentration_limits_enabled: true,
            portfolio_mandate: PortfolioMandate::Thematic,
            maximum_symbol_exposure_bps: 7_500,
            maximum_sector_exposure_bps: 9_000,
            maximum_market_exposure_bps: 10_000,
            ..WorkspacePreferences::default()
        };
        validate_preferences(&preferences).expect("explicit thematic limits");
        preferences.maximum_sector_exposure_bps = 0;
        assert!(validate_preferences(&preferences).is_err());
    }

    #[test]
    fn cloud_soak_cache_is_fixed_schema_and_live_order_stays_locked() {
        let root = std::env::temp_dir().join(format!(
            "investa-cloud-soak-cache-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).expect("cache test directory");
        let path = root.join("cloud-soak-status.json");
        fs::write(
            &path,
            r#"{
              "schema":"investa.cloud-soak-report.v1",
              "collectedAtMs":1788307200000,
              "projectId":"investa-remote-bumniverse",
              "region":"asia-northeast3",
              "source":"gcloud-read-only",
              "status":"running",
              "liveOrderEnabled":false,
              "jobs":[{
                "mode":"market","jobName":"investa-market-soak-24h-v2","executionName":"execution-1",
                "state":"running","startedAtMs":1788307200000,"completedAtMs":null,"elapsedMs":60000,
                "latestHeartbeatAtMs":1788307260000,
                "heartbeat":{"streams":{"upbit_spot":{"messages":10,"reconnects":0,"errors":0,"transportTimeouts":0,"marketGapEvents":0,"lastMessageAtMs":1788307260000}}},
                "passed":null,"actualElapsed24hQualified":false,"issues":[],"warnings":[],"collectionIssue":null
              }]
            }"#,
        )
        .expect("write cache fixture");
        let snapshot = read_cloud_soak_report(&path).expect("read cache");
        assert!(snapshot.available);
        let report = snapshot.report.expect("report");
        assert!(!report.live_order_enabled);
        assert_eq!(
            report.jobs[0]
                .heartbeat
                .as_ref()
                .expect("heartbeat")
                .streams["upbit_spot"]
                .messages,
            10
        );
        fs::remove_dir_all(root).expect("remove cache test directory");
    }

    #[test]
    fn cloud_soak_cache_rejects_unknown_fields_and_live_order_enablement() {
        let report: CloudSoakReport = serde_json::from_str(r#"{
          "schema":"investa.cloud-soak-report.v1","collectedAtMs":1,"projectId":"investa-remote-bumniverse",
          "region":"asia-northeast3","source":"gcloud-read-only","status":"running","liveOrderEnabled":true,"jobs":[]
        }"#).expect("parse bounded schema");
        assert!(validate_cloud_soak_report(&report).is_err());
        assert!(serde_json::from_str::<CloudSoakReport>(r#"{
          "schema":"investa.cloud-soak-report.v1","collectedAtMs":1,"projectId":"investa-remote-bumniverse",
          "region":"asia-northeast3","source":"gcloud-read-only","status":"running","liveOrderEnabled":false,"jobs":[],"token":"forbidden"
        }"#).is_err());
    }

    #[test]
    fn headless_shadow_soak_requires_an_explicit_exact_flag() {
        assert!(has_headless_shadow_soak_arg([
            "investa",
            "--shadow-soak-autostart"
        ]));
        assert!(!has_headless_shadow_soak_arg(["investa"]));
        assert!(!has_headless_shadow_soak_arg([
            "investa",
            "--shadow-soak-autostart=true"
        ]));
    }

    #[test]
    fn headless_shadow_soak_lock_is_acquired_before_database_startup() {
        let root = std::env::temp_dir().join(format!(
            "investa-shadow-soak-lock-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let guard = acquire_headless_shadow_soak(&root)
            .expect("lock acquisition")
            .expect("first runner should acquire the lock");
        assert!(acquire_headless_shadow_soak(&root)
            .expect("duplicate lock check")
            .is_none());
        assert_eq!(
            fs::read_to_string(&guard.lock_path)
                .expect("lock pid")
                .trim(),
            std::process::id().to_string()
        );
        let lock_path = guard.lock_path.clone();
        drop(guard);
        remove_lock(&lock_path);
        fs::remove_dir_all(root).expect("remove lock test directory");
    }

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
                source_observed_at_ms: Some(1),
                memory_bytes: 10,
                timer_count: 1,
                sqlite_bytes: 10,
                candidate_key: Some("bar-1".to_owned()),
                provider_healthy: true,
                restarted: false,
                reconciliation_passed: true,
            },
            SoakSample {
                observed_at_ms: 2,
                source_observed_at_ms: Some(2),
                memory_bytes: 20,
                timer_count: 2,
                sqlite_bytes: 20,
                candidate_key: Some("bar-1".to_owned()),
                provider_healthy: false,
                restarted: false,
                reconciliation_passed: true,
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
                source_observed_at_ms: Some(1 + hour * 3_600_000),
                memory_bytes: 100_000_000 + hour * 100_000,
                timer_count: 4,
                sqlite_bytes: 10_000_000 + hour * 1_000,
                candidate_key: Some(format!("completed-bar-{hour}")),
                provider_healthy: true,
                restarted: hour == 12,
                reconciliation_passed: true,
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

    #[test]
    fn soak_audit_fails_closed_on_stale_data_and_restart_mismatch() {
        let result = audit_soak(&[
            SoakSample {
                observed_at_ms: 1_000_000,
                source_observed_at_ms: Some(1_000_000),
                memory_bytes: 1,
                timer_count: 1,
                sqlite_bytes: 1,
                candidate_key: None,
                provider_healthy: true,
                restarted: false,
                reconciliation_passed: true,
            },
            SoakSample {
                observed_at_ms: 1_400_001,
                source_observed_at_ms: Some(1_000_000),
                memory_bytes: 1,
                timer_count: 1,
                sqlite_bytes: 1,
                candidate_key: None,
                provider_healthy: true,
                restarted: true,
                reconciliation_passed: false,
            },
        ])
        .expect("audit");
        assert_eq!(result.stale_sample_count, 1);
        assert_eq!(result.restart_reconciliation_failure_count, 1);
        assert!(result.fail_closed);
    }

    #[test]
    fn actual_soak_save_fails_closed_when_sampling_stops_for_over_three_minutes() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let stored = save_soak_audit(
            SaveSoakAuditRequest {
                run_id: "actual-gap".to_owned(),
                samples: vec![
                    SoakSample {
                        observed_at_ms: 1,
                        source_observed_at_ms: Some(1),
                        memory_bytes: 1,
                        timer_count: 1,
                        sqlite_bytes: 1,
                        candidate_key: None,
                        provider_healthy: true,
                        restarted: false,
                        reconciliation_passed: true,
                    },
                    SoakSample {
                        observed_at_ms: 240_002,
                        source_observed_at_ms: Some(240_002),
                        memory_bytes: 1,
                        timer_count: 1,
                        sqlite_bytes: 1,
                        candidate_key: None,
                        provider_healthy: true,
                        restarted: true,
                        reconciliation_passed: true,
                    },
                ],
                simulated_timeline: false,
            },
            &bridge,
        )
        .expect("save actual gap");
        assert_eq!(stored.audit.observation_gap_failure_count, 1);
        assert!(stored.audit.fail_closed);
        assert!(stored
            .audit
            .warnings
            .iter()
            .any(|warning| warning.contains("표본 공백")));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn runtime_soak_sample_reads_real_process_and_local_health_without_secrets() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let observed_at_ms = now_ms().expect("now");
        {
            let connection = bridge.connection.lock().expect("connection");
            for component in ["sqlite", "paper_ledger_krw", "paper_ledger_usd"] {
                connection.execute(
                    "INSERT INTO provider_health_events(event_id,component_id,critical,healthy,retry_action,detail,observed_at_ms) VALUES(?1,?2,1,1,'retry','ok',?3)",
                    params![format!("health-{component}"), component, observed_at_ms],
                ).expect("health event");
            }
        }
        let sample = collect_soak_sample(SoakSampleRequest { restarted: true }, &bridge)
            .expect("runtime sample");
        assert!(sample.memory_bytes > 0);
        assert_eq!(sample.sqlite_bytes, 0);
        assert!(sample.provider_healthy);
        assert!(sample.restarted && sample.reconciliation_passed);
        let serialized = serde_json::to_string(&sample).expect("serialize");
        assert!(!serialized.to_ascii_lowercase().contains("secret"));
        assert!(!serialized.to_ascii_lowercase().contains("token"));
    }
}
