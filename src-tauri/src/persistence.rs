use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::{
    backtest::{BacktestConfig, BacktestResult, PriceBar},
    paper_account::{replay_ledger, AppendOnlyLedger, LedgerError, LedgerErrorCode, LedgerEvent},
    research::{ResearchReport, StrategyReview},
};

pub(crate) const SCHEMA_VERSION: u32 = 31;
const MAX_HISTORY_LIMIT: u16 = 100;

pub struct PersistenceBridge {
    pub(crate) connection: Mutex<Connection>,
    pub(crate) database_path: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceStatus {
    pub available: bool,
    pub schema_version: u32,
    pub integrity_ok: bool,
    pub research_report_count: u64,
    pub dataset_count: u64,
    pub backtest_run_count: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramSourceRecord {
    pub peer_id: i64,
    pub title: String,
    pub username: Option<String>,
    pub enabled: bool,
    pub last_message_id: Option<i32>,
    pub updated_at_ms: u64,
}

pub struct TelegramMessageRevision<'a> {
    pub peer_id: i64,
    pub message_id: i32,
    pub posted_at_ms: u64,
    pub edited_at_ms: Option<u64>,
    pub ingested_at_ms: u64,
    pub content_hash: &'a str,
    pub text: &'a str,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramEvidenceItem {
    pub peer_id: i64,
    pub source_title: String,
    pub source_username: Option<String>,
    pub message_id: i32,
    pub posted_at_ms: u64,
    pub edited_at_ms: Option<u64>,
    pub ingested_at_ms: u64,
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchRunSummary {
    pub experiment_id: String,
    pub trace_id: String,
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    pub currency: String,
    pub provider: String,
    pub interval: String,
    pub adjusted: bool,
    pub bar_count: u64,
    pub total_return_bps: i64,
    pub max_drawdown_bps: u64,
    pub win_rate_bps: Option<u64>,
    pub completed_trade_count: u64,
    pub created_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchRunDetail {
    pub experiment_id: String,
    pub record: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRecordSummary {
    pub record_id: String,
    pub kind: String,
    pub status: String,
    pub market: String,
    pub title: String,
    pub symbol: String,
    pub currency: String,
    pub requested_at_ms: Option<u64>,
    pub completed_at_ms: u64,
    pub price_low_minor: Option<u64>,
    pub price_high_minor: Option<u64>,
    pub total_return_bps: Option<i64>,
    pub max_drawdown_bps: Option<u64>,
    pub win_rate_bps: Option<u64>,
    pub completed_trade_count: Option<u64>,
    pub classification: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestReplayEntry {
    pub experiment_id: String,
    pub classification: String,
    pub title: String,
    pub symbol: String,
    pub currency: String,
    pub side: String,
    pub occurred_at_ms: u64,
    pub reference_price_minor: u64,
    pub execution_price_minor: u64,
    pub quantity: u64,
    pub fee_minor: u64,
    pub tax_minor: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestReplayRun {
    pub experiment_id: String,
    pub classification: String,
    pub title: String,
    pub symbol: String,
    pub currency: String,
    pub initial_cash_minor: u64,
    pub final_cash_minor: u64,
    pub final_equity_minor: u64,
    pub realized_pnl_minor: i64,
    pub total_return_bps: i64,
    pub open_position_quantity: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestReplayHistory {
    pub runs: Vec<BacktestReplayRun>,
    pub entries: Vec<BacktestReplayEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisNoteRequest {
    pub record_id: String,
    pub kind: String,
    pub status: String,
    pub market: String,
    pub title: String,
    pub symbol: Option<String>,
    pub currency: Option<String>,
    pub requested_at_ms: Option<u64>,
    pub content: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisRecordDetail {
    pub summary: AnalysisRecordSummary,
    pub record: Value,
}

struct StoredAnalysisRow {
    record_id: String,
    title: String,
    symbol: String,
    currency: String,
    provider: String,
    requested_at_ms: Option<u64>,
    completed_at_ms: u64,
    total_return_bps: i64,
    max_drawdown_bps: u64,
    win_rate_bps: Option<u64>,
    completed_trade_count: u64,
    bars_json: String,
    record_json: String,
}

pub struct PersistBacktest<'a> {
    pub report: &'a ResearchReport,
    pub review: &'a StrategyReview,
    pub bars: &'a [PriceBar],
    pub config: &'a BacktestConfig,
    pub result: &'a BacktestResult,
    pub provider: &'a str,
    pub interval: &'a str,
    pub adjusted: bool,
    pub warnings: &'a [String],
    pub requested_at_ms: Option<u64>,
    pub classification: &'a str,
}

pub struct SqliteLedger<'a> {
    bridge: &'a PersistenceBridge,
    ledger_id: String,
    events: Vec<LedgerEvent>,
}

fn storage_error(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}

pub(crate) fn now_ms() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| storage_error("현재 시각을 확인하지 못했습니다", error))?
        .as_millis()
        .try_into()
        .map_err(|error| storage_error("현재 시각이 지원 범위를 초과했습니다", error))
}

fn serialize(value: &impl Serialize, label: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| storage_error(label, error))
}

fn analysis_summary(row: &StoredAnalysisRow) -> Result<AnalysisRecordSummary, String> {
    let bars_value: Value = serde_json::from_str(&row.bars_json)
        .map_err(|error| storage_error("저장된 분석 가격 데이터를 해석하지 못했습니다", error))?;
    let bars = bars_value["bars"]
        .as_array()
        .ok_or_else(|| "저장된 분석 가격 데이터에 가격봉이 없습니다.".to_owned())?;
    let price_low_minor = bars
        .iter()
        .flat_map(|bar| [bar["openMinor"].as_u64(), bar["closeMinor"].as_u64()])
        .flatten()
        .min()
        .ok_or_else(|| "저장된 분석 가격 데이터에서 최저가를 확인하지 못했습니다.".to_owned())?;
    let price_high_minor = bars
        .iter()
        .flat_map(|bar| [bar["openMinor"].as_u64(), bar["closeMinor"].as_u64()])
        .flatten()
        .max()
        .ok_or_else(|| "저장된 분석 가격 데이터에서 최고가를 확인하지 못했습니다.".to_owned())?;
    let record: Value = serde_json::from_str(&row.record_json)
        .map_err(|error| storage_error("저장된 분석 결과를 해석하지 못했습니다", error))?;
    let serialized_market = record["report"]["strategyCandidate"]["market"]
        .as_str()
        .unwrap_or_default();
    let market = if row.provider.contains("UPBIT") || row.symbol.starts_with("KRW-") {
        "coin"
    } else if serialized_market == "united_states" || row.currency == "USD" {
        "us"
    } else {
        "kr"
    };
    let classification = record["classification"]
        .as_str()
        .unwrap_or("research_experiment");
    Ok(AnalysisRecordSummary {
        record_id: row.record_id.clone(),
        kind: "strategy".to_owned(),
        status: "completed".to_owned(),
        market: market.to_owned(),
        title: row.title.clone(),
        symbol: row.symbol.clone(),
        currency: row.currency.clone(),
        requested_at_ms: row.requested_at_ms,
        completed_at_ms: row.completed_at_ms,
        price_low_minor: Some(price_low_minor),
        price_high_minor: Some(price_high_minor),
        total_return_bps: Some(row.total_return_bps),
        max_drawdown_bps: Some(row.max_drawdown_bps),
        win_rate_bps: row.win_rate_bps,
        completed_trade_count: Some(row.completed_trade_count),
        classification: classification.to_owned(),
    })
}

fn initialize(connection: &Connection) -> Result<(), String> {
    let current_version: u32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|error| storage_error("로컬 저장소 스키마 버전을 확인하지 못했습니다", error))?;
    if current_version > SCHEMA_VERSION {
        return Err(format!(
            "이 앱이 지원하는 버전보다 새로운 로컬 저장소입니다. 앱을 업데이트해 주세요. (DB {current_version}, 앱 {SCHEMA_VERSION})"
        ));
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;
             PRAGMA busy_timeout = 5000;
             CREATE TABLE IF NOT EXISTS research_reports (
                 trace_id TEXT PRIMARY KEY NOT NULL,
                 strategy_id TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 currency TEXT NOT NULL,
                 report_json TEXT NOT NULL,
                 review_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS datasets (
                 dataset_id TEXT PRIMARY KEY NOT NULL,
                 provider TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 currency TEXT NOT NULL,
                 interval TEXT NOT NULL,
                 adjusted INTEGER NOT NULL CHECK(adjusted IN (0, 1)),
                 bar_count INTEGER NOT NULL CHECK(bar_count > 0),
                 first_period_start_ms INTEGER NOT NULL CHECK(first_period_start_ms >= 0),
                 last_available_at_ms INTEGER NOT NULL CHECK(last_available_at_ms >= first_period_start_ms),
                 ingested_at_ms INTEGER NOT NULL CHECK(ingested_at_ms >= 0),
                 bars_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS backtest_runs (
                 experiment_id TEXT PRIMARY KEY NOT NULL,
                 trace_id TEXT NOT NULL REFERENCES research_reports(trace_id),
                 dataset_id TEXT NOT NULL REFERENCES datasets(dataset_id),
                 strategy_id TEXT NOT NULL,
                 strategy_name TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 currency TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 interval TEXT NOT NULL,
                 adjusted INTEGER NOT NULL CHECK(adjusted IN (0, 1)),
                 bar_count INTEGER NOT NULL CHECK(bar_count > 0),
                 total_return_bps INTEGER NOT NULL,
                 max_drawdown_bps INTEGER NOT NULL CHECK(max_drawdown_bps >= 0),
                 win_rate_bps INTEGER CHECK(win_rate_bps IS NULL OR win_rate_bps BETWEEN 0 AND 10000),
                 completed_trade_count INTEGER NOT NULL CHECK(completed_trade_count >= 0),
                 classification TEXT NOT NULL DEFAULT 'research_experiment'
                     CHECK(classification IN ('system_check', 'research_experiment', 'promotion_candidate')),
                 record_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS backtest_runs_created_at
                 ON backtest_runs(created_at_ms DESC, experiment_id DESC);
             CREATE TABLE IF NOT EXISTS walk_forward_runs (
                 validation_run_id TEXT PRIMARY KEY NOT NULL,
                 source_experiment_id TEXT NOT NULL REFERENCES backtest_runs(experiment_id),
                 fold_count INTEGER NOT NULL CHECK(fold_count BETWEEN 2 AND 5),
                 strategy_trial_count INTEGER NOT NULL CHECK(strategy_trial_count > 0),
                 report_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS walk_forward_runs_source_created_at
                 ON walk_forward_runs(source_experiment_id, created_at_ms DESC, validation_run_id DESC);
             CREATE TABLE IF NOT EXISTS codex_agent_threads (
                 agent_id TEXT PRIMARY KEY NOT NULL,
                 thread_id TEXT UNIQUE NOT NULL,
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS paper_ledger_events (
                 ledger_id TEXT NOT NULL,
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('account_opened', 'order_filled')),
                 event_json TEXT NOT NULL,
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 PRIMARY KEY(ledger_id, event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS paper_order_candidates (
                 candidate_id TEXT PRIMARY KEY NOT NULL,
                 experiment_id TEXT NOT NULL REFERENCES backtest_runs(experiment_id),
                 trace_id TEXT NOT NULL,
                 symbol TEXT NOT NULL,
                 currency TEXT NOT NULL,
                 side TEXT NOT NULL CHECK(side IN ('buy', 'sell')),
                 quantity INTEGER NOT NULL CHECK(quantity > 0),
                 reference_price_minor INTEGER NOT NULL CHECK(reference_price_minor > 0),
                 observed_at_ms INTEGER NOT NULL CHECK(observed_at_ms >= 0),
                 source TEXT NOT NULL CHECK(source IN ('manual', 'shadow_engine')),
                 status TEXT NOT NULL CHECK(status IN ('safety_approved', 'user_approved', 'submitted', 'partially_filled', 'filled', 'rejected', 'cancelled', 'expired')),
                 safety_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE UNIQUE INDEX IF NOT EXISTS paper_order_candidate_active
                 ON paper_order_candidates(experiment_id, symbol, side)
                 WHERE status IN ('safety_approved', 'user_approved', 'submitted', 'partially_filled');
             CREATE TABLE IF NOT EXISTS paper_order_events (
                 candidate_id TEXT NOT NULL REFERENCES paper_order_candidates(candidate_id),
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('candidate_created', 'safety_approved', 'user_approved', 'submitted', 'partially_filled', 'filled', 'cancelled', 'rejected', 'expired')),
                 event_json TEXT NOT NULL,
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                 PRIMARY KEY(candidate_id, event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS manual_paper_orders (
                 order_id TEXT PRIMARY KEY NOT NULL,
                 market TEXT NOT NULL CHECK(market IN ('kr', 'us', 'coin')),
                 symbol TEXT NOT NULL,
                 currency TEXT NOT NULL CHECK(currency IN ('KRW', 'USD')),
                 side TEXT NOT NULL CHECK(side IN ('buy', 'sell')),
                 order_type TEXT NOT NULL CHECK(order_type IN ('limit')),
                 quantity INTEGER NOT NULL CHECK(quantity > 0),
                 limit_price_minor INTEGER NOT NULL CHECK(limit_price_minor > 0),
                 status TEXT NOT NULL CHECK(status IN ('pending', 'cancelled')),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS manual_paper_orders_updated_at
                 ON manual_paper_orders(updated_at_ms DESC, order_id DESC);
             CREATE TABLE IF NOT EXISTS internal_execution_plans (
                 execution_id TEXT PRIMARY KEY NOT NULL CHECK(length(execution_id) BETWEEN 1 AND 128),
                 idempotency_key TEXT UNIQUE NOT NULL CHECK(length(idempotency_key) BETWEEN 1 AND 128),
                 status TEXT NOT NULL CHECK(status IN ('working','partially_filled','filled','cancelled','expired')),
                 request_json TEXT NOT NULL CHECK(length(request_json) BETWEEN 2 AND 100000),
                 state_json TEXT NOT NULL CHECK(length(state_json) BETWEEN 2 AND 500000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS internal_execution_plans_updated
                 ON internal_execution_plans(updated_at_ms DESC,execution_id DESC);
             CREATE TABLE IF NOT EXISTS internal_execution_events (
                 event_id TEXT PRIMARY KEY NOT NULL CHECK(length(event_id) BETWEEN 1 AND 128),
                 execution_id TEXT NOT NULL REFERENCES internal_execution_plans(execution_id),
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('fill','reprice','cancel','expire')),
                 event_json TEXT NOT NULL CHECK(length(event_json) BETWEEN 2 AND 100000),
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0),
                 UNIQUE(execution_id,event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS shadow_watches (
                 watch_id TEXT PRIMARY KEY NOT NULL,
                 experiment_id TEXT NOT NULL REFERENCES backtest_runs(experiment_id),
                 enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                 interval_seconds INTEGER NOT NULL CHECK(interval_seconds BETWEEN 15 AND 86400),
                 last_checked_at_ms INTEGER,
                 last_signal_key TEXT,
                 status TEXT NOT NULL CHECK(status IN ('watching', 'stopped', 'error')),
                 last_error TEXT,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS strategy_deployments (
                 deployment_id TEXT PRIMARY KEY NOT NULL CHECK(length(deployment_id) BETWEEN 1 AND 128),
                 idempotency_key TEXT UNIQUE NOT NULL CHECK(length(idempotency_key) BETWEEN 1 AND 128),
                 slot_key TEXT NOT NULL CHECK(length(slot_key) BETWEEN 1 AND 256),
                 experiment_id TEXT NOT NULL REFERENCES backtest_runs(experiment_id),
                 validation_run_id TEXT NOT NULL REFERENCES walk_forward_runs(validation_run_id),
                 strategy_id TEXT NOT NULL CHECK(length(strategy_id) BETWEEN 1 AND 128),
                 strategy_schema_version TEXT NOT NULL CHECK(length(strategy_schema_version) BETWEEN 1 AND 32),
                 plugin_id TEXT NOT NULL CHECK(length(plugin_id) BETWEEN 1 AND 128),
                 plugin_version INTEGER NOT NULL CHECK(plugin_version > 0),
                 dataset_id TEXT NOT NULL REFERENCES datasets(dataset_id),
                 evidence_sha256 TEXT NOT NULL CHECK(length(evidence_sha256) = 64),
                 status TEXT NOT NULL CHECK(status IN ('awaiting_approval','canary','canary_passed','paper_active','stopped','superseded','rolled_back','rejected')),
                 revision INTEGER NOT NULL CHECK(revision > 0),
                 canary_policy_json TEXT NOT NULL CHECK(length(canary_policy_json) BETWEEN 2 AND 100000),
                 evidence_json TEXT NOT NULL CHECK(length(evidence_json) BETWEEN 2 AND 1000000),
                 previous_deployment_id TEXT REFERENCES strategy_deployments(deployment_id),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE UNIQUE INDEX IF NOT EXISTS strategy_deployments_active_slot
                 ON strategy_deployments(slot_key) WHERE status = 'paper_active';
             CREATE INDEX IF NOT EXISTS strategy_deployments_slot_time
                 ON strategy_deployments(slot_key,updated_at_ms DESC,deployment_id DESC);
             CREATE TABLE IF NOT EXISTS strategy_deployment_events (
                 event_id TEXT PRIMARY KEY NOT NULL CHECK(length(event_id) BETWEEN 1 AND 128),
                 deployment_id TEXT NOT NULL REFERENCES strategy_deployments(deployment_id),
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('candidate_created','canary_approved','canary_observed','performance_observed','canary_passed','auto_stopped','paper_approved','superseded','rollback_approved','rejected')),
                 event_json TEXT NOT NULL CHECK(length(event_json) BETWEEN 2 AND 200000),
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0),
                 UNIQUE(deployment_id,event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS workflow_jobs (
                 job_id TEXT PRIMARY KEY NOT NULL,
                 topic TEXT NOT NULL,
                 importance TEXT NOT NULL CHECK(importance IN ('normal', 'important')),
                 stage TEXT NOT NULL,
                 status TEXT NOT NULL CHECK(status IN ('active', 'interrupted', 'cancelled', 'completed')),
                 selected_departments_json TEXT NOT NULL,
                 reports_json TEXT NOT NULL,
                 synthesis_json TEXT,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS workflow_jobs_updated_at
                 ON workflow_jobs(updated_at_ms DESC, job_id DESC);
             CREATE TABLE IF NOT EXISTS meeting_paper_handoffs (
                 handoff_id TEXT PRIMARY KEY NOT NULL CHECK(length(handoff_id) BETWEEN 1 AND 128),
                 workflow_job_id TEXT NOT NULL UNIQUE REFERENCES workflow_jobs(job_id),
                 analysis_record_id TEXT NOT NULL REFERENCES analysis_notes(record_id),
                 symbol TEXT NOT NULL CHECK(length(symbol) BETWEEN 1 AND 32),
                 strategy TEXT NOT NULL CHECK(length(strategy) BETWEEN 1 AND 500),
                 experiment_id TEXT REFERENCES backtest_runs(experiment_id),
                 paper_candidate_id TEXT REFERENCES paper_order_candidates(candidate_id),
                 engine_run_id TEXT UNIQUE REFERENCES engine_runs(run_id),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS meeting_paper_handoffs_updated_at
                 ON meeting_paper_handoffs(updated_at_ms DESC, handoff_id DESC);
             CREATE TABLE IF NOT EXISTS analysis_notes (
                 record_id TEXT PRIMARY KEY NOT NULL,
                 kind TEXT NOT NULL CHECK(kind IN ('instrument', 'meeting', 'strategy')),
                 status TEXT NOT NULL CHECK(status IN ('completed', 'blocked', 'held', 'error')),
                 market TEXT NOT NULL CHECK(market IN ('kr', 'us', 'coin', 'securities_futures', 'crypto_futures', 'mixed')),
                 title TEXT NOT NULL,
                 symbol TEXT,
                 currency TEXT,
                 requested_at_ms INTEGER CHECK(requested_at_ms IS NULL OR requested_at_ms >= 0),
                 content_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS analysis_notes_created_at
                 ON analysis_notes(created_at_ms DESC, record_id DESC);
             CREATE TABLE IF NOT EXISTS engine_runs (
                 run_id TEXT PRIMARY KEY NOT NULL,
                 idempotency_key TEXT UNIQUE NOT NULL,
                 status TEXT NOT NULL CHECK(status IN ('completed', 'blocked', 'cancelled', 'interrupted')),
                 symbol TEXT NOT NULL,
                 market TEXT NOT NULL,
                 candidate_ready INTEGER NOT NULL CHECK(candidate_ready IN (0, 1)),
                 input_json TEXT NOT NULL,
                 report_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS engine_runs_updated_at
                 ON engine_runs(updated_at_ms DESC, run_id DESC);
             CREATE TABLE IF NOT EXISTS engine_run_status_events (
                 event_id TEXT PRIMARY KEY,
                 run_id TEXT NOT NULL,
                 status TEXT NOT NULL CHECK(status IN ('cancelled','interrupted')),
                 reason TEXT NOT NULL,
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0),
                 FOREIGN KEY(run_id) REFERENCES engine_runs(run_id)
             );
             CREATE INDEX IF NOT EXISTS engine_run_status_events_run_time
                 ON engine_run_status_events(run_id, occurred_at_ms DESC, event_id DESC);
             CREATE TABLE IF NOT EXISTS engine_order_candidates (
                 candidate_id TEXT PRIMARY KEY NOT NULL,
                 run_id TEXT NOT NULL UNIQUE REFERENCES engine_runs(run_id),
                 symbol TEXT NOT NULL,
                 market TEXT NOT NULL,
                 currency TEXT NOT NULL CHECK(currency IN ('KRW','USD')),
                 side TEXT NOT NULL CHECK(side IN ('buy','sell')),
                 quantity INTEGER NOT NULL CHECK(quantity > 0),
                 quantity_scale INTEGER NOT NULL CHECK(quantity_scale > 0),
                 reference_price_minor INTEGER NOT NULL CHECK(reference_price_minor > 0),
                 valid_until_ms INTEGER NOT NULL CHECK(valid_until_ms > 0),
                 status TEXT NOT NULL CHECK(status IN ('safety_approved','user_approved','submitted','filled','rejected','cancelled','expired')),
                 safety_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS engine_order_candidates_updated_at
                 ON engine_order_candidates(updated_at_ms DESC, candidate_id DESC);
             CREATE TABLE IF NOT EXISTS engine_order_events (
                 candidate_id TEXT NOT NULL REFERENCES engine_order_candidates(candidate_id),
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('candidate_created','safety_approved','user_approved','submitted','filled','rejected','cancelled','expired','reconciled')),
                 event_json TEXT NOT NULL,
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                 PRIMARY KEY(candidate_id,event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS operational_alerts (
                 alert_id TEXT PRIMARY KEY NOT NULL,
                 deduplication_key TEXT NOT NULL,
                 severity TEXT NOT NULL CHECK(severity IN ('info','warning','critical')),
                 message TEXT NOT NULL,
                 first_seen_at_ms INTEGER NOT NULL CHECK(first_seen_at_ms > 0),
                 last_seen_at_ms INTEGER NOT NULL CHECK(last_seen_at_ms >= first_seen_at_ms),
                 occurrence_count INTEGER NOT NULL CHECK(occurrence_count > 0),
                 acknowledged_at_ms INTEGER,
                 response TEXT
             ) STRICT;
             CREATE INDEX IF NOT EXISTS operational_alerts_last_seen
                 ON operational_alerts(last_seen_at_ms DESC,alert_id DESC);
             CREATE TABLE IF NOT EXISTS audit_events (
                 event_id TEXT PRIMARY KEY NOT NULL,
                 actor TEXT NOT NULL,
                 action TEXT NOT NULL,
                 target_id TEXT NOT NULL,
                 previous_hash TEXT,
                 next_hash TEXT,
                 correlation_id TEXT NOT NULL,
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0),
                 detail TEXT NOT NULL
             ) STRICT;
             CREATE INDEX IF NOT EXISTS audit_events_occurred_at
                 ON audit_events(occurred_at_ms DESC,event_id DESC);
             CREATE TABLE IF NOT EXISTS provider_health_events (
                 event_id TEXT PRIMARY KEY NOT NULL,
                 component_id TEXT NOT NULL,
                 critical INTEGER NOT NULL CHECK(critical IN (0,1)),
                 healthy INTEGER NOT NULL CHECK(healthy IN (0,1)),
                 retry_action TEXT NOT NULL,
                 detail TEXT NOT NULL,
                 observed_at_ms INTEGER NOT NULL CHECK(observed_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS provider_health_component_time
                 ON provider_health_events(component_id,observed_at_ms DESC,event_id DESC);
             CREATE TABLE IF NOT EXISTS runtime_reconciliation_state (
                 id INTEGER PRIMARY KEY NOT NULL CHECK(id = 1),
                 status TEXT NOT NULL CHECK(status IN ('needs_reconciliation', 'ready')),
                 required_since_ms INTEGER,
                 completed_at_ms INTEGER,
                 mismatch_count INTEGER NOT NULL DEFAULT 0 CHECK(mismatch_count >= 0),
                 detail TEXT NOT NULL
             ) STRICT;
             INSERT OR IGNORE INTO runtime_reconciliation_state
                 (id,status,required_since_ms,completed_at_ms,mismatch_count,detail)
             VALUES (1,'ready',NULL,NULL,0,'초기 스키마 준비 완료');
             CREATE TABLE IF NOT EXISTS risk_policy_versions (
                 policy_id TEXT PRIMARY KEY NOT NULL,
                 status TEXT NOT NULL CHECK(status IN ('recommended', 'active', 'retired')),
                 policy_json TEXT NOT NULL,
                 evidence_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 approved_at_ms INTEGER
             ) STRICT;
             CREATE UNIQUE INDEX IF NOT EXISTS risk_policy_single_active
                 ON risk_policy_versions(status) WHERE status = 'active';
             CREATE TABLE IF NOT EXISTS strategy_protection_decisions (
                 decision_id INTEGER PRIMARY KEY,
                 policy_id TEXT NOT NULL,
                 target_symbol TEXT NOT NULL CHECK(length(target_symbol) BETWEEN 1 AND 32),
                 can_open_new_position INTEGER NOT NULL CHECK(can_open_new_position IN (0, 1)),
                 decision_json TEXT NOT NULL CHECK(length(decision_json) BETWEEN 2 AND 100000),
                 evaluated_at_ms INTEGER NOT NULL CHECK(evaluated_at_ms > 0),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= evaluated_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS strategy_protection_decisions_time
                 ON strategy_protection_decisions(evaluated_at_ms DESC, decision_id DESC);
             CREATE TABLE IF NOT EXISTS portfolio_risk_snapshots (
                 snapshot_id TEXT PRIMARY KEY NOT NULL CHECK(length(snapshot_id) BETWEEN 1 AND 128),
                 as_of_ms INTEGER NOT NULL CHECK(as_of_ms > 0),
                 currency TEXT NOT NULL CHECK(length(currency) BETWEEN 3 AND 8),
                 request_json TEXT NOT NULL CHECK(length(request_json) BETWEEN 2 AND 2000000),
                 report_json TEXT NOT NULL CHECK(length(report_json) BETWEEN 2 AND 1000000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= as_of_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS portfolio_risk_snapshots_created
                 ON portfolio_risk_snapshots(created_at_ms DESC, snapshot_id DESC);
             CREATE TABLE IF NOT EXISTS kis_paper_order_audit (
                 request_id TEXT PRIMARY KEY NOT NULL,
                 action TEXT NOT NULL CHECK(action IN ('submit', 'cancel', 'reconcile')),
                 symbol TEXT,
                 remote_order_id TEXT,
                 status TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS kis_paper_order_audit_created_at
                 ON kis_paper_order_audit(created_at_ms DESC, request_id DESC);
             CREATE TABLE IF NOT EXISTS futures_paper_events (
                 ledger_id TEXT NOT NULL,
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('account_opened', 'position_opened', 'position_marked', 'position_closed')),
                 request_id TEXT UNIQUE,
                 event_json TEXT NOT NULL,
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms >= 0),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 PRIMARY KEY(ledger_id, event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS crypto_risk_policy_changes (
                 change_id TEXT PRIMARY KEY NOT NULL CHECK(length(change_id) BETWEEN 1 AND 128),
                 policy_revision TEXT NOT NULL UNIQUE CHECK(length(policy_revision) BETWEEN 1 AND 128),
                 leverage_enabled INTEGER NOT NULL CHECK(leverage_enabled IN (0, 1)),
                 maximum_leverage_bps INTEGER NOT NULL CHECK(maximum_leverage_bps BETWEEN 10000 AND 20000),
                 reason TEXT NOT NULL CHECK(length(reason) BETWEEN 3 AND 500),
                 confirmation_recorded INTEGER NOT NULL CHECK(confirmation_recorded = 1),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS crypto_risk_policy_changes_created
                 ON crypto_risk_policy_changes(created_at_ms DESC, change_id DESC);
             CREATE TABLE IF NOT EXISTS futures_lifecycle_events (
                 event_id TEXT PRIMARY KEY NOT NULL CHECK(length(event_id) BETWEEN 1 AND 128),
                 request_id TEXT NOT NULL UNIQUE CHECK(length(request_id) BETWEEN 1 AND 128),
                 contract_symbol TEXT NOT NULL CHECK(length(contract_symbol) BETWEEN 1 AND 64),
                 event_type TEXT NOT NULL CHECK(event_type IN ('daily_settlement', 'expiry_close', 'manual_rollover')),
                 event_json TEXT NOT NULL CHECK(length(event_json) BETWEEN 2 AND 100000),
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS futures_lifecycle_events_contract_time
                 ON futures_lifecycle_events(contract_symbol, occurred_at_ms DESC, event_id DESC);
             CREATE TABLE IF NOT EXISTS operational_drills (
                 drill_id TEXT PRIMARY KEY NOT NULL CHECK(length(drill_id) BETWEEN 1 AND 128),
                 scenario TEXT NOT NULL CHECK(scenario IN ('order_rejected','partial_fill','stale_market_data','broker_outage','loss_limit','reconciliation_mismatch')),
                 result_json TEXT NOT NULL CHECK(length(result_json) BETWEEN 2 AND 100000),
                 executed_at_ms INTEGER NOT NULL CHECK(executed_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS operational_drills_executed
                 ON operational_drills(executed_at_ms DESC, drill_id DESC);
             CREATE TABLE IF NOT EXISTS publicity_article_revisions (
                 article_id TEXT NOT NULL CHECK(length(article_id) BETWEEN 1 AND 128),
                 revision INTEGER NOT NULL CHECK(revision > 0),
                 title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 300),
                 body_markdown TEXT NOT NULL CHECK(length(body_markdown) BETWEEN 1 AND 200000),
                 media_json TEXT NOT NULL CHECK(length(media_json) BETWEEN 2 AND 100000),
                 links_json TEXT NOT NULL CHECK(length(links_json) BETWEEN 2 AND 100000),
                 masking_confirmed INTEGER NOT NULL CHECK(masking_confirmed IN (0,1)),
                 status TEXT NOT NULL CHECK(status IN ('draft','rejected','approved','private_saved')),
                 review_note TEXT,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 PRIMARY KEY(article_id, revision)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS publicity_article_revisions_latest
                 ON publicity_article_revisions(article_id, revision DESC);
             CREATE TABLE IF NOT EXISTS shadow_soak_audits (
                 run_id TEXT PRIMARY KEY NOT NULL CHECK(length(run_id) BETWEEN 1 AND 128),
                 sample_count INTEGER NOT NULL CHECK(sample_count >= 2),
                 audit_json TEXT NOT NULL CHECK(length(audit_json) BETWEEN 2 AND 100000),
                 simulated_timeline INTEGER NOT NULL CHECK(simulated_timeline IN (0,1)),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS futures_paper_events_created_at
                 ON futures_paper_events(created_at_ms DESC, ledger_id, event_index DESC);
             CREATE TABLE IF NOT EXISTS telegram_sources (
                 peer_id INTEGER PRIMARY KEY NOT NULL,
                 title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 256),
                 username TEXT CHECK(username IS NULL OR length(username) BETWEEN 1 AND 64),
                 enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                 last_message_id INTEGER,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS telegram_sources_enabled
                 ON telegram_sources(enabled, title);
             CREATE TABLE IF NOT EXISTS telegram_message_revisions (
                 peer_id INTEGER NOT NULL REFERENCES telegram_sources(peer_id),
                 message_id INTEGER NOT NULL CHECK(message_id > 0),
                 posted_at_ms INTEGER NOT NULL CHECK(posted_at_ms >= 0),
                 edited_at_ms INTEGER CHECK(edited_at_ms IS NULL OR edited_at_ms >= posted_at_ms),
                 ingested_at_ms INTEGER NOT NULL CHECK(ingested_at_ms >= posted_at_ms),
                 content_hash TEXT NOT NULL CHECK(length(content_hash) = 64),
                 text TEXT NOT NULL CHECK(length(text) BETWEEN 1 AND 20000),
                 PRIMARY KEY(peer_id, message_id, content_hash)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS telegram_message_revisions_point_in_time
                 ON telegram_message_revisions(posted_at_ms DESC, peer_id, message_id DESC);
             CREATE TABLE IF NOT EXISTS forecast_dataset_audits (
                 audit_id TEXT PRIMARY KEY NOT NULL CHECK(length(audit_id) BETWEEN 1 AND 128),
                 dataset_id TEXT NOT NULL CHECK(length(dataset_id) BETWEEN 1 AND 128),
                 asset_contract_id TEXT NOT NULL CHECK(length(asset_contract_id) BETWEEN 1 AND 128),
                 asset_class TEXT NOT NULL CHECK(asset_class IN ('korea_stock', 'united_states_stock', 'equity_future', 'index_future', 'crypto_spot', 'crypto_perpetual')),
                 valid INTEGER NOT NULL CHECK(valid IN (0, 1)),
                 audit_json TEXT NOT NULL CHECK(length(audit_json) BETWEEN 2 AND 1000000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS forecast_dataset_audits_dataset_created
                 ON forecast_dataset_audits(dataset_id, created_at_ms DESC);
             CREATE TABLE IF NOT EXISTS probability_forecasts (
                 forecast_id TEXT PRIMARY KEY NOT NULL CHECK(length(forecast_id) BETWEEN 1 AND 128),
                 model_id TEXT NOT NULL CHECK(length(model_id) BETWEEN 1 AND 128),
                 model_version TEXT NOT NULL CHECK(length(model_version) BETWEEN 1 AND 128),
                 dataset_id TEXT NOT NULL CHECK(length(dataset_id) BETWEEN 1 AND 128),
                 asset_contract_id TEXT NOT NULL CHECK(length(asset_contract_id) BETWEEN 1 AND 128),
                 asset_class TEXT NOT NULL CHECK(asset_class IN ('korea_stock', 'united_states_stock', 'equity_future', 'index_future', 'crypto_spot', 'crypto_perpetual')),
                 horizon_ms INTEGER NOT NULL CHECK(horizon_ms > 0),
                 evidence_mode TEXT NOT NULL CHECK(evidence_mode IN ('full_features', 'price_only_fallback', 'unavailable')),
                 forecast_json TEXT NOT NULL CHECK(length(forecast_json) BETWEEN 2 AND 100000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS probability_forecasts_trace
                 ON probability_forecasts(asset_class, model_id, model_version, dataset_id, horizon_ms, created_at_ms DESC);
             CREATE TABLE IF NOT EXISTS forecast_calibration_runs (
                 calibration_id TEXT PRIMARY KEY NOT NULL CHECK(length(calibration_id) BETWEEN 1 AND 128),
                 asset_class TEXT NOT NULL CHECK(asset_class IN ('korea_stock', 'united_states_stock', 'equity_future', 'index_future', 'crypto_spot', 'crypto_perpetual')),
                 model_id TEXT NOT NULL CHECK(length(model_id) BETWEEN 1 AND 128),
                 model_version TEXT NOT NULL CHECK(length(model_version) BETWEEN 1 AND 128),
                 dataset_id TEXT NOT NULL CHECK(length(dataset_id) BETWEEN 1 AND 128),
                 horizon_ms INTEGER NOT NULL CHECK(horizon_ms > 0),
                 sample_count INTEGER NOT NULL CHECK(sample_count > 0),
                 metrics_json TEXT NOT NULL CHECK(length(metrics_json) BETWEEN 2 AND 100000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS forecast_calibration_runs_trace
                 ON forecast_calibration_runs(asset_class, model_id, model_version, dataset_id, horizon_ms, created_at_ms DESC);
             CREATE TABLE IF NOT EXISTS ml_dataset_manifests (
                 manifest_id TEXT PRIMARY KEY NOT NULL CHECK(length(manifest_id) BETWEEN 1 AND 128),
                 audit_id TEXT NOT NULL REFERENCES forecast_dataset_audits(audit_id),
                 dataset_id TEXT NOT NULL CHECK(length(dataset_id) BETWEEN 1 AND 128),
                 asset_class TEXT NOT NULL CHECK(asset_class IN ('korea_stock', 'united_states_stock', 'equity_future', 'index_future', 'crypto_spot', 'crypto_perpetual')),
                 content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
                 feature_schema_sha256 TEXT NOT NULL CHECK(length(feature_schema_sha256) = 64),
                 sample_count INTEGER NOT NULL CHECK(sample_count > 0),
                 feature_count INTEGER NOT NULL CHECK(feature_count > 0),
                 manifest_json TEXT NOT NULL CHECK(length(manifest_json) BETWEEN 2 AND 1000000),
                 payload_json TEXT NOT NULL CHECK(length(payload_json) BETWEEN 2 AND 67108864),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS ml_dataset_manifests_dataset_created
                 ON ml_dataset_manifests(dataset_id, created_at_ms DESC);
             CREATE TABLE IF NOT EXISTS ml_dataset_shard_sets (
                 shard_set_id TEXT PRIMARY KEY NOT NULL CHECK(length(shard_set_id) BETWEEN 1 AND 128),
                 dataset_id TEXT NOT NULL CHECK(length(dataset_id) BETWEEN 1 AND 128),
                 asset_class TEXT NOT NULL CHECK(asset_class IN ('korea_stock', 'united_states_stock', 'equity_future', 'index_future', 'crypto_spot', 'crypto_perpetual')),
                 feature_schema_sha256 TEXT NOT NULL CHECK(length(feature_schema_sha256) = 64),
                 combined_content_sha256 TEXT NOT NULL CHECK(length(combined_content_sha256) = 64),
                 shard_count INTEGER NOT NULL CHECK(shard_count BETWEEN 2 AND 64),
                 sample_count INTEGER NOT NULL CHECK(sample_count > 0),
                 feature_count INTEGER NOT NULL CHECK(feature_count > 0),
                 record_json TEXT NOT NULL CHECK(length(record_json) BETWEEN 2 AND 1000000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS ml_dataset_shard_sets_created
                 ON ml_dataset_shard_sets(created_at_ms DESC, shard_set_id);
             CREATE TABLE IF NOT EXISTS ml_training_jobs (
                 job_id TEXT PRIMARY KEY NOT NULL CHECK(length(job_id) BETWEEN 1 AND 128),
                 manifest_id TEXT NOT NULL REFERENCES ml_dataset_manifests(manifest_id),
                 algorithm TEXT NOT NULL CHECK(algorithm IN ('lightgbm', 'xgboost', 'chronos', 'timesfm')),
                 contract_version TEXT NOT NULL CHECK(length(contract_version) BETWEEN 1 AND 64),
                 input_sha256 TEXT NOT NULL CHECK(length(input_sha256) = 64),
                 status TEXT NOT NULL CHECK(status IN ('prepared', 'completed', 'failed')),
                 request_json TEXT NOT NULL CHECK(length(request_json) BETWEEN 2 AND 1000000),
                 result_json TEXT,
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS ml_training_jobs_manifest_created
                 ON ml_training_jobs(manifest_id, created_at_ms DESC);
             CREATE TABLE IF NOT EXISTS ml_training_job_sources (
                 job_id TEXT PRIMARY KEY NOT NULL REFERENCES ml_training_jobs(job_id),
                 source_kind TEXT NOT NULL CHECK(source_kind IN ('manifest', 'shard_set')),
                 source_id TEXT NOT NULL CHECK(length(source_id) BETWEEN 1 AND 128),
                 source_content_sha256 TEXT NOT NULL CHECK(length(source_content_sha256) = 64),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS ml_training_job_sources_source
                 ON ml_training_job_sources(source_kind, source_id, created_at_ms DESC);
             INSERT OR IGNORE INTO ml_training_job_sources
                 (job_id, source_kind, source_id, source_content_sha256, created_at_ms)
             SELECT jobs.job_id, 'manifest', jobs.manifest_id, manifests.content_sha256, jobs.created_at_ms
             FROM ml_training_jobs AS jobs
             INNER JOIN ml_dataset_manifests AS manifests
                 ON manifests.manifest_id = jobs.manifest_id;
             CREATE TABLE IF NOT EXISTS ml_model_versions (
                 model_id TEXT NOT NULL CHECK(length(model_id) BETWEEN 1 AND 128),
                 model_version TEXT NOT NULL CHECK(length(model_version) BETWEEN 1 AND 128),
                 job_id TEXT NOT NULL UNIQUE REFERENCES ml_training_jobs(job_id),
                 asset_class TEXT NOT NULL CHECK(asset_class IN ('korea_stock', 'united_states_stock', 'equity_future', 'index_future', 'crypto_spot', 'crypto_perpetual')),
                 algorithm TEXT NOT NULL CHECK(algorithm IN ('lightgbm', 'xgboost', 'chronos', 'timesfm')),
                 artifact_format TEXT NOT NULL CHECK(artifact_format IN ('lightgbm_text', 'xgboost_json', 'safetensors', 'onnx')),
                 artifact_sha256 TEXT NOT NULL CHECK(length(artifact_sha256) = 64),
                 status TEXT NOT NULL CHECK(status = 'candidate_review'),
                 metrics_json TEXT NOT NULL CHECK(length(metrics_json) BETWEEN 2 AND 100000),
                 record_json TEXT NOT NULL CHECK(length(record_json) BETWEEN 2 AND 1000000),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 PRIMARY KEY(model_id, model_version)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS ml_model_versions_created
                 ON ml_model_versions(created_at_ms DESC, model_id, model_version);
             CREATE TABLE IF NOT EXISTS remote_control_policy (
                 id INTEGER PRIMARY KEY NOT NULL CHECK(id = 1),
                 enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                 allowed_user_ids_json TEXT NOT NULL,
                 analysis_enabled INTEGER NOT NULL CHECK(analysis_enabled IN (0, 1)),
                 meeting_enabled INTEGER NOT NULL CHECK(meeting_enabled IN (0, 1)),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
             ) STRICT;
             INSERT OR IGNORE INTO remote_control_policy
                 (id, enabled, allowed_user_ids_json, analysis_enabled, meeting_enabled, updated_at_ms)
             VALUES (1, 0, '[]', 1, 1, 0);
             CREATE TABLE IF NOT EXISTS remote_control_jobs (
                 job_id TEXT PRIMARY KEY NOT NULL CHECK(length(job_id) BETWEEN 1 AND 128),
                 source TEXT NOT NULL CHECK(source IN ('telegram', 'cloud_relay', 'local_test')),
                 source_request_id TEXT NOT NULL CHECK(length(source_request_id) BETWEEN 1 AND 128),
                 source_user_id TEXT NOT NULL CHECK(length(source_user_id) BETWEEN 1 AND 64),
                 source_chat_id TEXT NOT NULL CHECK(length(source_chat_id) BETWEEN 1 AND 64),
                 request_hash TEXT NOT NULL CHECK(length(request_hash) = 64),
                 command_kind TEXT NOT NULL CHECK(command_kind IN ('status', 'analysis', 'meeting', 'paper_order_proposal', 'shadow_control', 'system_control')),
                 instruction TEXT NOT NULL CHECK(length(instruction) BETWEEN 1 AND 4000),
                 status TEXT NOT NULL CHECK(status IN ('queued', 'awaiting_local_approval', 'approved', 'rejected', 'cancelled', 'dispatched', 'completed', 'failed')),
                 provider_id TEXT,
                 approval_reason TEXT,
                 received_at_ms INTEGER NOT NULL CHECK(received_at_ms > 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= received_at_ms),
                 UNIQUE(source, source_request_id)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS remote_control_jobs_updated
                 ON remote_control_jobs(updated_at_ms DESC, job_id DESC);
             CREATE TABLE IF NOT EXISTS remote_control_job_events (
                 job_id TEXT NOT NULL REFERENCES remote_control_jobs(job_id),
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('received', 'queued', 'approval_required', 'approved', 'rejected', 'cancelled', 'dispatched', 'completed', 'failed')),
                 actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 64),
                 detail TEXT NOT NULL CHECK(length(detail) BETWEEN 1 AND 1000),
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0),
                 PRIMARY KEY(job_id, event_index)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS workspace_preferences (
                 singleton_id INTEGER PRIMARY KEY NOT NULL CHECK(singleton_id = 1),
                 preferences_json TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= 0)
             ) STRICT;
             INSERT OR IGNORE INTO workspace_preferences(singleton_id, preferences_json, updated_at_ms)
             VALUES(1, '{\"displayTimezone\":\"Asia/Seoul\",\"quietHoursStart\":23,\"quietHoursEnd\":7,\"staleAfterSeconds\":300,\"notifyWarning\":true,\"notifyCritical\":true}', 0);
             CREATE TABLE IF NOT EXISTS market_stream_checkpoints (
                 stream_id TEXT PRIMARY KEY NOT NULL CHECK(length(stream_id) BETWEEN 1 AND 64),
                 state_json TEXT NOT NULL CHECK(length(state_json) BETWEEN 2 AND 2000000),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms > 0)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS pit_provider_pages (
                 page_id TEXT PRIMARY KEY NOT NULL CHECK(length(page_id) = 64),
                 provider TEXT NOT NULL CHECK(provider IN ('upbit_spot','binance_spot','binance_usdm','binance_coinm')),
                 symbol TEXT NOT NULL CHECK(length(symbol) BETWEEN 5 AND 32),
                 interval TEXT NOT NULL CHECK(interval IN ('minute1','minute3','minute5','minute15','minute30','hour1','hour4','day1')),
                 requested_start_ms INTEGER NOT NULL CHECK(requested_start_ms > 0),
                 requested_end_exclusive_ms INTEGER NOT NULL CHECK(requested_end_exclusive_ms > requested_start_ms),
                 page_json TEXT NOT NULL CHECK(length(page_json) BETWEEN 2 AND 16777216),
                 fetched_at_ms INTEGER NOT NULL CHECK(fetched_at_ms > 0)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS pit_provider_pages_lookup
                 ON pit_provider_pages(provider, symbol, interval, requested_start_ms, requested_end_exclusive_ms);
             CREATE TABLE IF NOT EXISTS pit_price_observations (
                 record_id TEXT PRIMARY KEY NOT NULL CHECK(length(record_id) BETWEEN 1 AND 256),
                 provider TEXT NOT NULL CHECK(provider IN ('upbit_spot','binance_spot','binance_usdm','binance_coinm')),
                 symbol TEXT NOT NULL CHECK(length(symbol) BETWEEN 5 AND 32),
                 interval TEXT NOT NULL CHECK(interval IN ('minute1','minute3','minute5','minute15','minute30','hour1','hour4','day1')),
                 bar_end_ms INTEGER NOT NULL CHECK(bar_end_ms > 0),
                 available_at_ms INTEGER NOT NULL CHECK(available_at_ms >= bar_end_ms),
                 ingested_at_ms INTEGER NOT NULL CHECK(ingested_at_ms >= available_at_ms),
                 source TEXT NOT NULL CHECK(length(source) BETWEEN 1 AND 64),
                 source_revision TEXT NOT NULL CHECK(length(source_revision) = 71),
                 close_scaled INTEGER NOT NULL CHECK(close_scaled > 0),
                 price_scale INTEGER NOT NULL CHECK(price_scale > 0),
                 final_bar INTEGER NOT NULL CHECK(final_bar = 1),
                 UNIQUE(provider, symbol, interval, bar_end_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS pit_price_observations_lookup
                 ON pit_price_observations(provider, symbol, interval, bar_end_ms);
             CREATE TABLE IF NOT EXISTS pit_collection_jobs (
                 job_id TEXT PRIMARY KEY NOT NULL CHECK(length(job_id) BETWEEN 1 AND 128),
                 idempotency_key TEXT NOT NULL UNIQUE CHECK(length(idempotency_key) BETWEEN 1 AND 128),
                 request_hash TEXT NOT NULL CHECK(length(request_hash) = 64),
                 provider TEXT NOT NULL CHECK(provider IN ('upbit_spot','binance_spot','binance_usdm','binance_coinm')),
                 symbol TEXT NOT NULL CHECK(length(symbol) BETWEEN 5 AND 32),
                 interval TEXT NOT NULL CHECK(interval IN ('minute1','minute3','minute5','minute15','minute30','hour1','hour4','day1')),
                 requested_start_ms INTEGER NOT NULL CHECK(requested_start_ms > 0),
                 requested_end_exclusive_ms INTEGER NOT NULL CHECK(requested_end_exclusive_ms > requested_start_ms),
                 page_size INTEGER NOT NULL CHECK(page_size BETWEEN 1 AND 1000),
                 status TEXT NOT NULL CHECK(status IN ('queued','running','retry_wait','completed','failed','cancelled')),
                 cursor_start_ms INTEGER NOT NULL CHECK(cursor_start_ms >= requested_start_ms),
                 cursor_end_exclusive_ms INTEGER NOT NULL CHECK(cursor_end_exclusive_ms <= requested_end_exclusive_ms),
                 page_count INTEGER NOT NULL DEFAULT 0 CHECK(page_count >= 0),
                 observation_count INTEGER NOT NULL DEFAULT 0 CHECK(observation_count >= 0),
                 failure_count INTEGER NOT NULL DEFAULT 0 CHECK(failure_count BETWEEN 0 AND 4),
                 next_retry_at_ms INTEGER CHECK(next_retry_at_ms IS NULL OR next_retry_at_ms > 0),
                 last_error TEXT CHECK(last_error IS NULL OR length(last_error) BETWEEN 1 AND 500),
                 created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                 updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms >= created_at_ms)
             ) STRICT;
             CREATE INDEX IF NOT EXISTS pit_collection_jobs_status
                 ON pit_collection_jobs(status, next_retry_at_ms, updated_at_ms);
             CREATE TABLE IF NOT EXISTS pit_provider_rate_limits (
                 provider TEXT PRIMARY KEY NOT NULL CHECK(provider IN ('upbit_spot','binance_spot','binance_usdm','binance_coinm')),
                 next_allowed_at_ms INTEGER NOT NULL CHECK(next_allowed_at_ms > 0)
             ) STRICT;
             CREATE TABLE IF NOT EXISTS pit_collection_job_events (
                 job_id TEXT NOT NULL REFERENCES pit_collection_jobs(job_id),
                 event_index INTEGER NOT NULL CHECK(event_index >= 0),
                 event_type TEXT NOT NULL CHECK(event_type IN ('created','claimed','recovered','page_stored','retry_scheduled','completed','failed','cancelled','released')),
                 detail_json TEXT NOT NULL CHECK(length(detail_json) BETWEEN 2 AND 4000),
                 occurred_at_ms INTEGER NOT NULL CHECK(occurred_at_ms > 0),
                 PRIMARY KEY(job_id, event_index)
             ) STRICT;
             ",
        )
        .map_err(|error| storage_error("로컬 연구 저장소를 초기화하지 못했습니다", error))?;
    if current_version < 4 {
        connection
            .execute_batch(
                "ALTER TABLE backtest_runs ADD COLUMN requested_at_ms INTEGER
                 CHECK(requested_at_ms IS NULL OR requested_at_ms >= 0);",
            )
            .map_err(|error| storage_error("분석 요청 시각 스키마를 확장하지 못했습니다", error))?;
    }
    if current_version < 9 {
        let has_classification: u32 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('backtest_runs') WHERE name = 'classification'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| storage_error("백테스트 분류 열을 확인하지 못했습니다", error))?;
        if has_classification == 0 {
            connection
                .execute_batch(
                    "ALTER TABLE backtest_runs ADD COLUMN classification TEXT NOT NULL
                     DEFAULT 'research_experiment'
                     CHECK(classification IN ('system_check', 'research_experiment', 'promotion_candidate'));",
                )
                .map_err(|error| storage_error("백테스트 분류 스키마를 확장하지 못했습니다", error))?;
        }
    }
    if current_version < 21 {
        let analysis_schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = 'analysis_notes'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| storage_error("분석 기록 스키마를 확인하지 못했습니다", error))?;
        if !analysis_schema.contains("securities_futures") {
            connection
                .execute_batch(
                    "BEGIN IMMEDIATE;
                     DROP INDEX IF EXISTS analysis_notes_created_at;
                     ALTER TABLE analysis_notes RENAME TO analysis_notes_v20;
                     CREATE TABLE analysis_notes (
                         record_id TEXT PRIMARY KEY NOT NULL,
                         kind TEXT NOT NULL CHECK(kind IN ('instrument', 'meeting', 'strategy')),
                         status TEXT NOT NULL CHECK(status IN ('completed', 'blocked', 'held', 'error')),
                         market TEXT NOT NULL CHECK(market IN ('kr', 'us', 'coin', 'securities_futures', 'crypto_futures', 'mixed')),
                         title TEXT NOT NULL,
                         symbol TEXT,
                         currency TEXT,
                         requested_at_ms INTEGER CHECK(requested_at_ms IS NULL OR requested_at_ms >= 0),
                         content_json TEXT NOT NULL,
                         created_at_ms INTEGER NOT NULL CHECK(created_at_ms >= 0)
                     ) STRICT;
                     INSERT INTO analysis_notes
                         (record_id, kind, status, market, title, symbol, currency, requested_at_ms, content_json, created_at_ms)
                     SELECT record_id, kind, status, market, title, symbol, currency, requested_at_ms, content_json, created_at_ms
                     FROM analysis_notes_v20;
                     DROP TABLE analysis_notes_v20;
                     CREATE INDEX analysis_notes_created_at
                         ON analysis_notes(created_at_ms DESC, record_id DESC);
                     COMMIT;",
                )
                .map_err(|error| {
                    let _ = connection.execute_batch("ROLLBACK;");
                    storage_error("선물 분석 기록 스키마를 확장하지 못했습니다", error)
                })?;
        }
    }
    if current_version == 30 {
        let has_experiment_id: u32 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('meeting_paper_handoffs') WHERE name = 'experiment_id'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| storage_error("회의 백테스트 계보 열을 확인하지 못했습니다", error))?;
        if has_experiment_id == 0 {
            connection
                .execute_batch(
                    "ALTER TABLE meeting_paper_handoffs ADD COLUMN experiment_id TEXT REFERENCES backtest_runs(experiment_id);
                     ALTER TABLE meeting_paper_handoffs ADD COLUMN paper_candidate_id TEXT REFERENCES paper_order_candidates(candidate_id);
                     ",
                )
                .map_err(|error| storage_error("회의 백테스트·후보 계보를 확장하지 못했습니다", error))?;
        }
    }
    connection
        .execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS meeting_paper_handoffs_experiment
                 ON meeting_paper_handoffs(experiment_id) WHERE experiment_id IS NOT NULL;
             CREATE UNIQUE INDEX IF NOT EXISTS meeting_paper_handoffs_candidate
                 ON meeting_paper_handoffs(paper_candidate_id) WHERE paper_candidate_id IS NOT NULL;",
        )
        .map_err(|error| storage_error("회의 백테스트·후보 계보 인덱스를 만들지 못했습니다", error))?;
    connection
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| storage_error("로컬 저장소 스키마 버전을 기록하지 못했습니다", error))?;
    Ok(())
}

fn immutable_value(
    transaction: &Transaction<'_>,
    table: &str,
    key_column: &str,
    key: &str,
    value_column: &str,
) -> Result<Option<String>, String> {
    let sql = format!("SELECT {value_column} FROM {table} WHERE {key_column} = ?1");
    transaction
        .query_row(&sql, params![key], |row| row.get(0))
        .optional()
        .map_err(|error| storage_error("기존 불변 기록을 확인하지 못했습니다", error))
}

fn ensure_same(existing: Option<String>, expected: &str, label: &str) -> Result<bool, String> {
    match existing {
        Some(existing) if existing == expected => Ok(false),
        Some(_) => Err(format!(
            "같은 {label} 식별자에 다른 내용이 이미 저장되어 있습니다. 새 식별자로 다시 실행해 주세요."
        )),
        None => Ok(true),
    }
}

impl PersistenceBridge {
    pub fn open(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| "로컬 연구 저장소의 상위 경로를 찾지 못했습니다.".to_owned())?;
        fs::create_dir_all(parent)
            .map_err(|error| storage_error("로컬 데이터 폴더를 만들지 못했습니다", error))?;
        let connection = Connection::open(path)
            .map_err(|error| storage_error("로컬 연구 저장소를 열지 못했습니다", error))?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| storage_error("로컬 저장소 WAL 모드를 설정하지 못했습니다", error))?;
        initialize(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            database_path: Some(path.to_path_buf()),
        })
    }

    pub fn replace_telegram_sources(&self, sources: &[TelegramSourceRecord]) -> Result<(), String> {
        if sources.len() > 50 {
            return Err("텔레그램 수집 채널은 최대 50개까지 선택할 수 있습니다.".to_owned());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| storage_error("텔레그램 채널 저장을 시작하지 못했습니다", error))?;
        transaction
            .execute("UPDATE telegram_sources SET enabled = 0", [])
            .map_err(|error| {
                storage_error("기존 텔레그램 채널 선택을 초기화하지 못했습니다", error)
            })?;
        for source in sources {
            if source.title.trim().is_empty() || source.title.chars().count() > 256 {
                return Err("텔레그램 채널 이름 형식이 올바르지 않습니다.".to_owned());
            }
            transaction.execute(
                "INSERT INTO telegram_sources
                    (peer_id, title, username, enabled, last_message_id, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)
                 ON CONFLICT(peer_id) DO UPDATE SET
                    title = excluded.title,
                    username = excluded.username,
                    enabled = 1,
                    updated_at_ms = excluded.updated_at_ms",
                params![
                    source.peer_id,
                    source.title.trim(),
                    source.username.as_deref(),
                    source.last_message_id,
                    source.updated_at_ms,
                ],
            ).map_err(|error| storage_error("텔레그램 채널 선택을 저장하지 못했습니다", error))?;
        }
        transaction
            .commit()
            .map_err(|error| storage_error("텔레그램 채널 선택을 확정하지 못했습니다", error))
    }

    pub fn telegram_sources(&self) -> Result<Vec<TelegramSourceRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT peer_id, title, username, enabled, last_message_id, updated_at_ms
             FROM telegram_sources WHERE enabled = 1 ORDER BY title, peer_id",
            )
            .map_err(|error| storage_error("텔레그램 채널 조회를 준비하지 못했습니다", error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(TelegramSourceRecord {
                    peer_id: row.get(0)?,
                    title: row.get(1)?,
                    username: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? == 1,
                    last_message_id: row.get(4)?,
                    updated_at_ms: row.get(5)?,
                })
            })
            .map_err(|error| storage_error("텔레그램 채널을 조회하지 못했습니다", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("텔레그램 채널을 읽지 못했습니다", error))
    }

    pub fn persist_telegram_revisions(
        &self,
        peer_id: i64,
        revisions: &[TelegramMessageRevision<'_>],
        last_message_id: Option<i32>,
        synced_at_ms: u64,
    ) -> Result<u64, String> {
        if revisions.len() > 200 {
            return Err("한 번에 저장할 텔레그램 메시지는 최대 200개입니다.".to_owned());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let transaction = connection
            .transaction()
            .map_err(|error| storage_error("텔레그램 메시지 저장을 시작하지 못했습니다", error))?;
        let mut inserted = 0_u64;
        for revision in revisions {
            if revision.peer_id != peer_id
                || revision.message_id <= 0
                || revision.text.trim().is_empty()
                || revision.text.chars().count() > 20_000
                || revision.content_hash.len() != 64
            {
                return Err("텔레그램 메시지 저장 형식이 올바르지 않습니다.".to_owned());
            }
            inserted += transaction.execute(
                "INSERT OR IGNORE INTO telegram_message_revisions
                    (peer_id, message_id, posted_at_ms, edited_at_ms, ingested_at_ms, content_hash, text)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    revision.peer_id,
                    revision.message_id,
                    revision.posted_at_ms,
                    revision.edited_at_ms,
                    revision.ingested_at_ms,
                    revision.content_hash,
                    revision.text,
                ],
            ).map_err(|error| storage_error("텔레그램 메시지를 저장하지 못했습니다", error))? as u64;
        }
        transaction
            .execute(
                "UPDATE telegram_sources
             SET last_message_id = CASE
                    WHEN ?2 IS NULL THEN last_message_id
                    WHEN last_message_id IS NULL OR ?2 > last_message_id THEN ?2
                    ELSE last_message_id END,
                 updated_at_ms = ?3
             WHERE peer_id = ?1 AND enabled = 1",
                params![peer_id, last_message_id, synced_at_ms],
            )
            .map_err(|error| {
                storage_error("텔레그램 채널 동기화 시각을 저장하지 못했습니다", error)
            })?;
        transaction
            .commit()
            .map_err(|error| storage_error("텔레그램 메시지 저장을 확정하지 못했습니다", error))?;
        Ok(inserted)
    }

    pub fn telegram_evidence(
        &self,
        as_of_ms: u64,
        since_ms: u64,
        limit: u16,
    ) -> Result<Vec<TelegramEvidenceItem>, String> {
        if limit == 0 || limit > 100 || since_ms > as_of_ms {
            return Err("텔레그램 근거 조회 범위가 올바르지 않습니다.".to_owned());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection.prepare(
            "SELECT r.peer_id, s.title, s.username, r.message_id, r.posted_at_ms,
                    r.edited_at_ms, r.ingested_at_ms, r.text
             FROM telegram_message_revisions r
             JOIN telegram_sources s ON s.peer_id = r.peer_id
             WHERE s.enabled = 1
               AND r.posted_at_ms BETWEEN ?1 AND ?2
               AND r.ingested_at_ms <= ?2
               AND (r.edited_at_ms IS NULL OR r.edited_at_ms <= ?2)
               AND NOT EXISTS (
                   SELECT 1 FROM telegram_message_revisions newer
                   WHERE newer.peer_id = r.peer_id
                     AND newer.message_id = r.message_id
                     AND COALESCE(newer.edited_at_ms, newer.posted_at_ms) > COALESCE(r.edited_at_ms, r.posted_at_ms)
                     AND COALESCE(newer.edited_at_ms, newer.posted_at_ms) <= ?2
               )
             ORDER BY r.posted_at_ms DESC, r.peer_id, r.message_id DESC
             LIMIT ?3",
        ).map_err(|error| storage_error("텔레그램 근거 조회를 준비하지 못했습니다", error))?;
        let rows = statement
            .query_map(params![since_ms, as_of_ms, limit], |row| {
                Ok(TelegramEvidenceItem {
                    peer_id: row.get(0)?,
                    source_title: row.get(1)?,
                    source_username: row.get(2)?,
                    message_id: row.get(3)?,
                    posted_at_ms: row.get(4)?,
                    edited_at_ms: row.get(5)?,
                    ingested_at_ms: row.get(6)?,
                    text: row.get(7)?,
                })
            })
            .map_err(|error| storage_error("텔레그램 근거를 조회하지 못했습니다", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("텔레그램 근거를 읽지 못했습니다", error))
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Result<Self, String> {
        let connection = Connection::open_in_memory()
            .map_err(|error| storage_error("테스트 저장소를 열지 못했습니다", error))?;
        initialize(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
            database_path: None,
        })
    }

    pub fn persist_backtest(&self, input: PersistBacktest<'_>) -> Result<(), String> {
        if !matches!(
            input.classification,
            "system_check" | "research_experiment" | "promotion_candidate"
        ) {
            return Err("백테스트 분류가 올바르지 않습니다.".to_owned());
        }
        let created_at_ms = now_ms()?;
        if input
            .requested_at_ms
            .is_some_and(|requested_at_ms| requested_at_ms > created_at_ms)
        {
            return Err("분석 요청 시각은 완료 시각보다 늦을 수 없습니다.".to_owned());
        }
        let report_json = serialize(input.report, "연구 보고서를 직렬화하지 못했습니다")?;
        let review_json = serialize(input.review, "연구 검토 결과를 직렬화하지 못했습니다")?;
        // dataset_id는 봉 값뿐 아니라 공급자·간격·수정주가 조건까지 같은
        // 불변 스냅샷을 가리킨다. 같은 봉 배열을 다른 조건으로 재사용하는 것도 충돌이다.
        let bars_json = serialize(
            &json!({
                "provider": input.provider,
                "interval": input.interval,
                "adjusted": input.adjusted,
                "bars": input.bars,
            }),
            "가격 데이터셋을 직렬화하지 못했습니다",
        )?;
        let record_json = serialize(
            &json!({
                "report": input.report,
                "review": input.review,
                "config": input.config,
                "result": input.result,
                "provider": input.provider,
                "interval": input.interval,
                "adjusted": input.adjusted,
                "warnings": input.warnings,
                "classification": input.classification,
            }),
            "백테스트 기록을 직렬화하지 못했습니다",
        )?;
        let first_bar = input
            .bars
            .first()
            .ok_or_else(|| "빈 가격 데이터셋은 저장할 수 없습니다.".to_owned())?;
        let last_bar = input
            .bars
            .last()
            .ok_or_else(|| "빈 가격 데이터셋은 저장할 수 없습니다.".to_owned())?;
        let bar_count = u64::try_from(input.bars.len())
            .map_err(|error| storage_error("가격 데이터 개수가 지원 범위를 초과했습니다", error))?;
        let trade_count = u64::try_from(input.result.completed_trade_count)
            .map_err(|error| storage_error("거래 개수가 지원 범위를 초과했습니다", error))?;
        let strategy = &input.report.strategy_candidate;

        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let transaction = connection.transaction().map_err(|error| {
            storage_error("백테스트 저장 트랜잭션을 시작하지 못했습니다", error)
        })?;

        if ensure_same(
            immutable_value(
                &transaction,
                "research_reports",
                "trace_id",
                &input.report.trace_id,
                "report_json",
            )?,
            &report_json,
            "연구 추적",
        )? {
            transaction
                .execute(
                    "INSERT INTO research_reports
                     (trace_id, strategy_id, symbol, currency, report_json, review_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        input.report.trace_id,
                        strategy.strategy_id,
                        strategy.symbol,
                        strategy.currency,
                        report_json,
                        review_json,
                        created_at_ms,
                    ],
                )
                .map_err(|error| storage_error("연구 보고서를 저장하지 못했습니다", error))?;
        }

        if ensure_same(
            immutable_value(
                &transaction,
                "datasets",
                "dataset_id",
                &input.config.dataset_id,
                "bars_json",
            )?,
            &bars_json,
            "데이터셋",
        )? {
            transaction
                .execute(
                    "INSERT INTO datasets
                     (dataset_id, provider, symbol, currency, interval, adjusted, bar_count,
                      first_period_start_ms, last_available_at_ms, ingested_at_ms, bars_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        input.config.dataset_id,
                        input.provider,
                        first_bar.symbol,
                        first_bar.currency,
                        input.interval,
                        input.adjusted,
                        bar_count,
                        first_bar.period_start_ms,
                        last_bar.available_at_ms,
                        last_bar.ingested_at_ms,
                        bars_json,
                        created_at_ms,
                    ],
                )
                .map_err(|error| storage_error("가격 데이터셋을 저장하지 못했습니다", error))?;
        }

        if ensure_same(
            immutable_value(
                &transaction,
                "backtest_runs",
                "experiment_id",
                &input.config.experiment_id,
                "record_json",
            )?,
            &record_json,
            "실험",
        )? {
            transaction
                .execute(
                    "INSERT INTO backtest_runs
                     (experiment_id, trace_id, dataset_id, strategy_id, strategy_name, symbol,
                      currency, provider, interval, adjusted, bar_count, total_return_bps,
                      max_drawdown_bps, win_rate_bps, completed_trade_count, record_json,
                      created_at_ms, requested_at_ms, classification)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                    params![
                        input.config.experiment_id,
                        input.report.trace_id,
                        input.config.dataset_id,
                        strategy.strategy_id,
                        strategy.name,
                        strategy.symbol,
                        strategy.currency,
                        input.provider,
                        input.interval,
                        input.adjusted,
                        bar_count,
                        input.result.total_return_bps,
                        input.result.max_drawdown_bps,
                        input.result.win_rate_bps,
                        trade_count,
                        record_json,
                        created_at_ms,
                        input.requested_at_ms,
                        input.classification,
                    ],
                )
                .map_err(|error| storage_error("백테스트 결과를 저장하지 못했습니다", error))?;
        }

        transaction
            .commit()
            .map_err(|error| storage_error("백테스트 저장을 확정하지 못했습니다", error))
    }

    pub fn paper_ledger(&self, ledger_id: &str) -> Result<SqliteLedger<'_>, String> {
        if !valid_local_identifier(ledger_id) {
            return Err("유효한 모의계좌 원장 식별자가 필요합니다.".to_owned());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT event_json FROM paper_ledger_events
                 WHERE ledger_id = ?1 ORDER BY event_index ASC",
            )
            .map_err(|error| storage_error("모의계좌 원장 조회를 준비하지 못했습니다", error))?;
        let rows = statement
            .query_map(params![ledger_id], |row| row.get::<_, String>(0))
            .map_err(|error| storage_error("모의계좌 원장을 조회하지 못했습니다", error))?;
        let mut events = Vec::new();
        for row in rows {
            let event_json =
                row.map_err(|error| storage_error("모의계좌 원장 사건을 읽지 못했습니다", error))?;
            events.push(serde_json::from_str(&event_json).map_err(|error| {
                storage_error("저장된 모의계좌 원장 사건을 해석하지 못했습니다", error)
            })?);
        }
        drop(statement);
        drop(connection);
        if !events.is_empty() {
            replay_ledger(&events).map_err(|error| {
                format!(
                    "저장된 모의계좌 원장의 무결성 검증에 실패했습니다: {}",
                    error.message
                )
            })?;
        }
        Ok(SqliteLedger {
            bridge: self,
            ledger_id: ledger_id.to_owned(),
            events,
        })
    }

    fn append_paper_event(
        &self,
        ledger_id: &str,
        expected_index: usize,
        event: &LedgerEvent,
    ) -> Result<(), String> {
        let event_json = serialize(event, "모의계좌 원장 사건을 직렬화하지 못했습니다")?;
        let event_type = match event {
            LedgerEvent::AccountOpened { .. } => "account_opened",
            LedgerEvent::OrderFilled { .. } => "order_filled",
        };
        let expected_index = u64::try_from(expected_index).map_err(|error| {
            storage_error("모의계좌 원장 순번이 지원 범위를 초과했습니다", error)
        })?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let transaction = connection.transaction().map_err(|error| {
            storage_error("모의계좌 원장 트랜잭션을 시작하지 못했습니다", error)
        })?;
        let current_count: u64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM paper_ledger_events WHERE ledger_id = ?1",
                params![ledger_id],
                |row| row.get(0),
            )
            .map_err(|error| storage_error("모의계좌 원장 순번을 확인하지 못했습니다", error))?;
        if current_count != expected_index {
            return Err(
                "모의계좌 원장이 다른 작업에서 변경되었습니다. 다시 불러온 뒤 재시도해 주세요."
                    .to_owned(),
            );
        }
        transaction
            .execute(
                "INSERT INTO paper_ledger_events
                 (ledger_id, event_index, event_type, event_json, occurred_at_ms, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    ledger_id,
                    expected_index,
                    event_type,
                    event_json,
                    event.occurred_at_ms(),
                    now_ms()?,
                ],
            )
            .map_err(|error| storage_error("모의계좌 원장 사건을 추가하지 못했습니다", error))?;
        transaction
            .commit()
            .map_err(|error| storage_error("모의계좌 원장 사건을 확정하지 못했습니다", error))
    }

    pub fn codex_threads(&self) -> Result<Vec<(String, String)>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare("SELECT agent_id, thread_id FROM codex_agent_threads ORDER BY agent_id")
            .map_err(|error| storage_error("Codex 대화 연결 조회를 준비하지 못했습니다", error))?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|error| storage_error("Codex 대화 연결을 조회하지 못했습니다", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("Codex 대화 연결을 읽지 못했습니다", error))
    }

    pub fn save_codex_thread(&self, agent_id: &str, thread_id: &str) -> Result<(), String> {
        if agent_id.is_empty()
            || agent_id.len() > 64
            || thread_id.is_empty()
            || thread_id.len() > 128
        {
            return Err("유효한 직원·Codex thread 식별자가 필요합니다.".to_owned());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .execute(
                "INSERT INTO codex_agent_threads(agent_id, thread_id, updated_at_ms)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(agent_id) DO UPDATE SET
                    thread_id = excluded.thread_id,
                    updated_at_ms = excluded.updated_at_ms",
                params![agent_id, thread_id, now_ms()?],
            )
            .map_err(|error| storage_error("Codex 대화 연결을 저장하지 못했습니다", error))?;
        Ok(())
    }

    pub fn remove_codex_thread(&self, agent_id: &str) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .execute(
                "DELETE FROM codex_agent_threads WHERE agent_id = ?1",
                params![agent_id],
            )
            .map_err(|error| storage_error("Codex 대화 연결을 초기화하지 못했습니다", error))?;
        Ok(())
    }

    fn status(&self) -> Result<PersistenceStatus, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let integrity: String = connection
            .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
            .map_err(|error| storage_error("로컬 저장소 무결성을 확인하지 못했습니다", error))?;
        let count = |table: &str| -> Result<u64, String> {
            let sql = format!("SELECT COUNT(*) FROM {table}");
            connection
                .query_row(&sql, [], |row| row.get(0))
                .map_err(|error| storage_error("로컬 저장 기록 수를 확인하지 못했습니다", error))
        };
        let integrity_ok = integrity == "ok";
        Ok(PersistenceStatus {
            available: integrity_ok,
            schema_version: SCHEMA_VERSION,
            integrity_ok,
            research_report_count: count("research_reports")?,
            dataset_count: count("datasets")?,
            backtest_run_count: count("backtest_runs")?,
            message: if integrity_ok {
                "로컬 연구 저장소 정상".to_owned()
            } else {
                "로컬 연구 저장소 무결성 확인 필요".to_owned()
            },
        })
    }

    fn history(&self, limit: u16) -> Result<Vec<ResearchRunSummary>, String> {
        if limit == 0 || limit > MAX_HISTORY_LIMIT {
            return Err(format!("조회 개수는 1~{MAX_HISTORY_LIMIT}개여야 합니다."));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT experiment_id, trace_id, strategy_id, strategy_name, symbol, currency,
                        provider, interval, adjusted, bar_count, total_return_bps,
                        max_drawdown_bps, win_rate_bps, completed_trade_count, created_at_ms
                 FROM backtest_runs
                 ORDER BY created_at_ms DESC, experiment_id DESC
                 LIMIT ?1",
            )
            .map_err(|error| storage_error("연구 기록 조회를 준비하지 못했습니다", error))?;
        let rows = statement
            .query_map(params![limit], |row| {
                Ok(ResearchRunSummary {
                    experiment_id: row.get(0)?,
                    trace_id: row.get(1)?,
                    strategy_id: row.get(2)?,
                    strategy_name: row.get(3)?,
                    symbol: row.get(4)?,
                    currency: row.get(5)?,
                    provider: row.get(6)?,
                    interval: row.get(7)?,
                    adjusted: row.get(8)?,
                    bar_count: row.get(9)?,
                    total_return_bps: row.get(10)?,
                    max_drawdown_bps: row.get(11)?,
                    win_rate_bps: row.get(12)?,
                    completed_trade_count: row.get(13)?,
                    created_at_ms: row.get(14)?,
                })
            })
            .map_err(|error| storage_error("연구 기록을 조회하지 못했습니다", error))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("연구 기록을 읽지 못했습니다", error))
    }

    fn detail(&self, experiment_id: &str) -> Result<ResearchRunDetail, String> {
        if experiment_id.is_empty() || experiment_id.len() > 128 {
            return Err("유효한 실험 식별자가 필요합니다.".to_owned());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let record_json: String = connection
            .query_row(
                "SELECT record_json FROM backtest_runs WHERE experiment_id = ?1",
                params![experiment_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage_error("연구 상세 기록을 조회하지 못했습니다", error))?
            .ok_or_else(|| "해당 실험 기록을 찾지 못했습니다.".to_owned())?;
        let record = serde_json::from_str(&record_json)
            .map_err(|error| storage_error("저장된 연구 기록을 해석하지 못했습니다", error))?;
        Ok(ResearchRunDetail {
            experiment_id: experiment_id.to_owned(),
            record,
        })
    }

    fn save_analysis_note(&self, request: AnalysisNoteRequest) -> Result<(), String> {
        if !valid_local_identifier(&request.record_id)
            || !matches!(request.kind.as_str(), "instrument" | "meeting" | "strategy")
            || !matches!(
                request.status.as_str(),
                "completed" | "blocked" | "held" | "error"
            )
            || !matches!(
                request.market.as_str(),
                "kr" | "us" | "coin" | "securities_futures" | "crypto_futures" | "mixed"
            )
            || request.title.trim().is_empty()
            || request.title.chars().count() > 240
            || request.title.chars().any(char::is_control)
            || request
                .symbol
                .as_ref()
                .is_some_and(|value| value.len() > 24 || value.chars().any(char::is_control))
            || request.currency.as_ref().is_some_and(|value| {
                value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase())
            })
        {
            return Err(
                "분석 기록의 식별자·분류·상태·제목·종목 정보가 올바르지 않습니다.".to_owned(),
            );
        }
        let created_at_ms = now_ms()?;
        if request
            .requested_at_ms
            .is_some_and(|value| value > created_at_ms)
        {
            return Err("분석 요청 시각은 저장 시각보다 늦을 수 없습니다.".to_owned());
        }
        let content_json = serialize(&request.content, "분석 내용을 직렬화하지 못했습니다")?;
        if content_json.len() > 1_000_000 {
            return Err("분석 기록이 로컬 저장 한도를 초과했습니다.".to_owned());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 분석 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT content_json FROM analysis_notes WHERE record_id = ?1",
                params![request.record_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| storage_error("기존 분석 기록을 확인하지 못했습니다", error))?;
        if let Some(existing) = existing {
            return if existing == content_json {
                Ok(())
            } else {
                Err("같은 분석 기록 ID를 다른 내용으로 덮어쓸 수 없습니다.".to_owned())
            };
        }
        connection.execute(
            "INSERT INTO analysis_notes (record_id, kind, status, market, title, symbol, currency, requested_at_ms, content_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![request.record_id, request.kind, request.status, request.market, request.title.trim(), request.symbol, request.currency, request.requested_at_ms, content_json, created_at_ms],
        ).map_err(|error| storage_error("분석 기록을 저장하지 못했습니다", error))?;
        Ok(())
    }

    fn analysis_history(&self, limit: u16) -> Result<Vec<AnalysisRecordSummary>, String> {
        if limit == 0 || limit > MAX_HISTORY_LIMIT {
            return Err(format!("조회 개수는 1~{MAX_HISTORY_LIMIT}개여야 합니다."));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT b.experiment_id, b.strategy_name, b.symbol, b.currency, b.provider,
                        b.requested_at_ms, b.created_at_ms, b.total_return_bps,
                        b.max_drawdown_bps, b.win_rate_bps, b.completed_trade_count,
                        d.bars_json, b.record_json
                 FROM backtest_runs b
                 INNER JOIN datasets d ON d.dataset_id = b.dataset_id
                 ORDER BY b.created_at_ms DESC, b.experiment_id DESC
                 LIMIT ?1",
            )
            .map_err(|error| storage_error("분석 기록 조회를 준비하지 못했습니다", error))?;
        let rows = statement
            .query_map(params![limit], |row| {
                Ok(StoredAnalysisRow {
                    record_id: row.get(0)?,
                    title: row.get(1)?,
                    symbol: row.get(2)?,
                    currency: row.get(3)?,
                    provider: row.get(4)?,
                    requested_at_ms: row.get(5)?,
                    completed_at_ms: row.get(6)?,
                    total_return_bps: row.get(7)?,
                    max_drawdown_bps: row.get(8)?,
                    win_rate_bps: row.get(9)?,
                    completed_trade_count: row.get(10)?,
                    bars_json: row.get(11)?,
                    record_json: row.get(12)?,
                })
            })
            .map_err(|error| storage_error("분석 기록을 조회하지 못했습니다", error))?;
        let stored = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("분석 기록을 읽지 못했습니다", error))?;
        let mut summaries = stored
            .iter()
            .map(analysis_summary)
            .collect::<Result<Vec<_>, _>>()?;
        let mut note_statement = connection.prepare(
            "SELECT record_id, kind, status, market, title, COALESCE(symbol, ''), COALESCE(currency, ''), requested_at_ms, created_at_ms
             FROM analysis_notes ORDER BY created_at_ms DESC, record_id DESC LIMIT ?1"
        ).map_err(|error| storage_error("일반 분석 기록 조회를 준비하지 못했습니다", error))?;
        let notes = note_statement
            .query_map(params![limit], |row| {
                Ok(AnalysisRecordSummary {
                    record_id: row.get(0)?,
                    kind: row.get(1)?,
                    status: row.get(2)?,
                    market: row.get(3)?,
                    title: row.get(4)?,
                    symbol: row.get(5)?,
                    currency: row.get(6)?,
                    requested_at_ms: row.get(7)?,
                    completed_at_ms: row.get(8)?,
                    price_low_minor: None,
                    price_high_minor: None,
                    total_return_bps: None,
                    max_drawdown_bps: None,
                    win_rate_bps: None,
                    completed_trade_count: None,
                    classification: "research_experiment".to_owned(),
                })
            })
            .map_err(|error| storage_error("일반 분석 기록을 조회하지 못했습니다", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("일반 분석 기록을 읽지 못했습니다", error))?;
        summaries.extend(notes);
        summaries.sort_by(|left, right| {
            right
                .completed_at_ms
                .cmp(&left.completed_at_ms)
                .then_with(|| right.record_id.cmp(&left.record_id))
        });
        summaries.truncate(limit as usize);
        Ok(summaries)
    }

    fn analysis_detail(&self, record_id: &str) -> Result<AnalysisRecordDetail, String> {
        if !valid_local_identifier(record_id) {
            return Err("유효한 분석 기록 식별자가 필요합니다.".to_owned());
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let note = connection.query_row(
            "SELECT record_id, kind, status, market, title, COALESCE(symbol, ''), COALESCE(currency, ''), requested_at_ms, created_at_ms, content_json
             FROM analysis_notes WHERE record_id = ?1",
            params![record_id],
            |row| Ok((AnalysisRecordSummary {
                record_id: row.get(0)?, kind: row.get(1)?, status: row.get(2)?, market: row.get(3)?, title: row.get(4)?, symbol: row.get(5)?, currency: row.get(6)?, requested_at_ms: row.get(7)?, completed_at_ms: row.get(8)?,
                price_low_minor: None, price_high_minor: None, total_return_bps: None, max_drawdown_bps: None, win_rate_bps: None, completed_trade_count: None,
                classification: "research_experiment".to_owned(),
            }, row.get::<_, String>(9)?)),
        ).optional().map_err(|error| storage_error("일반 분석 상세를 조회하지 못했습니다", error))?;
        if let Some((summary, content_json)) = note {
            let record = serde_json::from_str(&content_json)
                .map_err(|error| storage_error("일반 분석 내용을 해석하지 못했습니다", error))?;
            return Ok(AnalysisRecordDetail { summary, record });
        }
        let stored = connection
            .query_row(
                "SELECT b.experiment_id, b.strategy_name, b.symbol, b.currency, b.provider,
                        b.requested_at_ms, b.created_at_ms, b.total_return_bps,
                        b.max_drawdown_bps, b.win_rate_bps, b.completed_trade_count,
                        d.bars_json, b.record_json
                 FROM backtest_runs b
                 INNER JOIN datasets d ON d.dataset_id = b.dataset_id
                 WHERE b.experiment_id = ?1",
                params![record_id],
                |row| {
                    Ok(StoredAnalysisRow {
                        record_id: row.get(0)?,
                        title: row.get(1)?,
                        symbol: row.get(2)?,
                        currency: row.get(3)?,
                        provider: row.get(4)?,
                        requested_at_ms: row.get(5)?,
                        completed_at_ms: row.get(6)?,
                        total_return_bps: row.get(7)?,
                        max_drawdown_bps: row.get(8)?,
                        win_rate_bps: row.get(9)?,
                        completed_trade_count: row.get(10)?,
                        bars_json: row.get(11)?,
                        record_json: row.get(12)?,
                    })
                },
            )
            .optional()
            .map_err(|error| storage_error("분석 상세 기록을 조회하지 못했습니다", error))?
            .ok_or_else(|| "해당 분석 기록을 찾지 못했습니다.".to_owned())?;
        let summary = analysis_summary(&stored)?;
        let record = serde_json::from_str(&stored.record_json)
            .map_err(|error| storage_error("저장된 분석 결과를 해석하지 못했습니다", error))?;
        Ok(AnalysisRecordDetail { summary, record })
    }

    fn backtest_replay_history(&self, limit: u16) -> Result<BacktestReplayHistory, String> {
        if limit == 0 || limit > MAX_HISTORY_LIMIT {
            return Err(format!("조회 개수는 1~{MAX_HISTORY_LIMIT}개여야 합니다."));
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| "로컬 연구 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT experiment_id, classification, strategy_name, symbol, currency, record_json
             FROM backtest_runs ORDER BY created_at_ms DESC, experiment_id DESC",
            )
            .map_err(|error| {
                storage_error("백테스트 재생 원장 조회를 준비하지 못했습니다", error)
            })?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(|error| storage_error("백테스트 재생 원장을 조회하지 못했습니다", error))?;
        let mut runs = Vec::new();
        let mut entries = Vec::new();
        for row in rows {
            let (experiment_id, classification, title, symbol, currency, record_json) =
                row.map_err(|error| storage_error("백테스트 재생 원장을 읽지 못했습니다", error))?;
            let record: Value = serde_json::from_str(&record_json).map_err(|error| {
                storage_error("백테스트 체결 기록을 해석하지 못했습니다", error)
            })?;
            let result = &record["result"];
            runs.push(BacktestReplayRun {
                experiment_id: experiment_id.clone(),
                classification: classification.clone(),
                title: title.clone(),
                symbol: symbol.clone(),
                currency: currency.clone(),
                initial_cash_minor: result["initialCashMinor"].as_u64().unwrap_or_default(),
                final_cash_minor: result["finalCashMinor"].as_u64().unwrap_or_default(),
                final_equity_minor: result["finalEquityMinor"].as_u64().unwrap_or_default(),
                realized_pnl_minor: result["realizedPnlMinor"].as_i64().unwrap_or_default(),
                total_return_bps: result["totalReturnBps"].as_i64().unwrap_or_default(),
                open_position_quantity: result["openPositionQuantity"].as_u64().unwrap_or_default(),
            });
            let fills = record["result"]["fills"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            for fill in fills.into_iter().rev() {
                entries.push(BacktestReplayEntry {
                    experiment_id: experiment_id.clone(),
                    classification: classification.clone(),
                    title: title.clone(),
                    symbol: symbol.clone(),
                    currency: currency.clone(),
                    side: fill["side"].as_str().unwrap_or("unknown").to_owned(),
                    occurred_at_ms: fill["periodStartMs"].as_u64().unwrap_or_default(),
                    reference_price_minor: fill["referencePriceMinor"].as_u64().unwrap_or_default(),
                    execution_price_minor: fill["executionPriceMinor"].as_u64().unwrap_or_default(),
                    quantity: fill["quantity"].as_u64().unwrap_or_default(),
                    fee_minor: fill["feeMinor"].as_u64().unwrap_or_default(),
                    tax_minor: fill["taxMinor"].as_u64().unwrap_or_default(),
                });
            }
            if runs.len() >= limit as usize {
                break;
            }
        }
        Ok(BacktestReplayHistory { runs, entries })
    }
}

fn valid_local_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

impl AppendOnlyLedger for SqliteLedger<'_> {
    fn events(&self) -> &[LedgerEvent] {
        &self.events
    }

    fn append(&mut self, event: LedgerEvent) -> Result<(), LedgerError> {
        self.bridge
            .append_paper_event(&self.ledger_id, self.events.len(), &event)
            .map_err(|message| LedgerError {
                code: LedgerErrorCode::AppendFailed,
                message,
            })?;
        self.events.push(event);
        Ok(())
    }
}

#[tauri::command]
pub fn persistence_status(
    bridge: State<'_, PersistenceBridge>,
) -> Result<PersistenceStatus, String> {
    bridge.status()
}

#[tauri::command]
pub fn research_run_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<ResearchRunSummary>, String> {
    bridge.history(limit)
}

#[tauri::command]
pub fn research_run_detail(
    experiment_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<ResearchRunDetail, String> {
    bridge.detail(&experiment_id)
}

#[tauri::command]
pub fn analysis_record_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<AnalysisRecordSummary>, String> {
    bridge.analysis_history(limit)
}

#[tauri::command]
pub fn analysis_record_detail(
    record_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<AnalysisRecordDetail, String> {
    bridge.analysis_detail(&record_id)
}

#[tauri::command]
pub fn analysis_note_save(
    request: AnalysisNoteRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    bridge.save_analysis_note(request)
}

#[tauri::command]
pub fn paper_ledger_history(
    currency: String,
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<Value>, String> {
    if !matches!(currency.as_str(), "KRW" | "USD") || limit == 0 || limit > MAX_HISTORY_LIMIT {
        return Err("원장 통화와 조회 개수를 확인해 주세요.".to_owned());
    }
    let ledger = bridge.paper_ledger(crate::paper_trading::ledger_id_for_currency(&currency)?)?;
    let start = ledger.events.len().saturating_sub(limit as usize);
    ledger.events[start..]
        .iter()
        .rev()
        .map(|event| {
            serde_json::to_value(event)
                .map_err(|error| storage_error("원장 사건을 직렬화하지 못했습니다", error))
        })
        .collect()
}

#[tauri::command]
pub fn backtest_replay_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<BacktestReplayHistory, String> {
    bridge.backtest_replay_history(limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backtest::run_backtest,
        paper_account::{execute_shadow_order, open_paper_account, ShadowOrderRequest},
        research::{
            CrossDirection, EvidenceKind, Market, ReferenceEvidence, SignalSpec, StrategySpec,
        },
        simulation::TradingCosts,
        trading::TradeSide,
    };

    fn fixture() -> (
        ResearchReport,
        StrategyReview,
        Vec<PriceBar>,
        BacktestConfig,
        BacktestResult,
    ) {
        let report = ResearchReport {
            trace_id: "trace-persistence-1".to_owned(),
            request: "이동평균 전략을 검토해줘".to_owned(),
            evidence: vec![ReferenceEvidence {
                evidence_id: "evidence-1".to_owned(),
                kind: EvidenceKind::Paper,
                source_url: "https://example.com/paper".to_owned(),
                revision: None,
                license: None,
                summary: "재현 가능한 테스트 근거입니다.".to_owned(),
                claimed_result: None,
            }],
            strategy_candidate: StrategySpec {
                schema_version: "1".to_owned(),
                strategy_id: "strategy-persistence-1".to_owned(),
                name: "저장소 테스트 전략".to_owned(),
                market: Market::Korea,
                symbol: "005930".to_owned(),
                currency: "KRW".to_owned(),
                hypothesis: "단기 이동평균이 장기 이동평균을 상향 돌파하면 추세가 이어집니다."
                    .to_owned(),
                source_evidence_ids: vec!["evidence-1".to_owned()],
                entry_signal: SignalSpec::MovingAverageCross {
                    fast_window: 2,
                    slow_window: 3,
                    direction: CrossDirection::Above,
                },
                exit_signal: SignalSpec::MovingAverageCross {
                    fast_window: 2,
                    slow_window: 3,
                    direction: CrossDirection::Below,
                },
                limitations: vec!["테스트 데이터만 사용합니다.".to_owned()],
                unknowns: vec![],
            },
        };
        let review = crate::research::review_research_report(&report);
        let closes = [100_u64, 99, 98, 101, 104, 102, 97, 96];
        let bars = closes
            .iter()
            .enumerate()
            .map(|(index, close)| PriceBar {
                symbol: "005930".to_owned(),
                currency: "KRW".to_owned(),
                source: "FIXTURE".to_owned(),
                period_start_ms: (index as u64 + 1) * 1_000,
                period_end_ms: (index as u64 + 1) * 1_000 + 900,
                available_at_ms: (index as u64 + 1) * 1_000 + 900,
                ingested_at_ms: 20_000,
                open_minor: *close,
                high_minor: *close,
                low_minor: *close,
                close_minor: *close,
                volume: 1_000,
            })
            .collect::<Vec<_>>();
        let config = BacktestConfig {
            experiment_id: "experiment-persistence-1".to_owned(),
            dataset_id: "dataset-persistence-1".to_owned(),
            code_version: "test".to_owned(),
            initial_cash_minor: 100_000,
            order_quantity: 1,
            quantity_scale: 1,
            close_open_position_at_end: true,
            costs: TradingCosts {
                buy_fee_bps: 0.0,
                sell_fee_bps: 0.0,
                sell_tax_bps: 0.0,
                slippage_bps: 0.0,
            },
            risk_limits: None,
        };
        let result = run_backtest(&report.strategy_candidate, &bars, &config)
            .expect("fixture backtest should succeed");
        (report, review, bars, config, result)
    }

    #[test]
    fn persists_and_reads_an_immutable_backtest_record() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        let (report, review, bars, config, result) = fixture();
        let input = || PersistBacktest {
            report: &report,
            review: &review,
            bars: &bars,
            config: &config,
            result: &result,
            provider: "FIXTURE",
            interval: "1d",
            adjusted: true,
            warnings: &[],
            requested_at_ms: Some(1_000),
            classification: "research_experiment",
        };

        bridge
            .persist_backtest(input())
            .expect("first write succeeds");
        bridge
            .persist_backtest(input())
            .expect("identical retry is idempotent");

        let status = bridge.status().expect("status should load");
        assert!(status.integrity_ok);
        assert_eq!(status.research_report_count, 1);
        assert_eq!(status.dataset_count, 1);
        assert_eq!(status.backtest_run_count, 1);
        let history = bridge.history(10).expect("history should load");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].experiment_id, config.experiment_id);
        assert_eq!(history[0].bar_count, bars.len() as u64);
        let detail = bridge
            .detail(&config.experiment_id)
            .expect("detail should load");
        assert_eq!(detail.record["report"]["traceId"], report.trace_id);
        let analysis = bridge
            .analysis_history(10)
            .expect("analysis history should load");
        assert_eq!(analysis.len(), 1);
        assert_eq!(analysis[0].record_id, config.experiment_id);
        assert_eq!(analysis[0].market, "kr");
        assert_eq!(analysis[0].classification, "research_experiment");
        assert_eq!(analysis[0].requested_at_ms, Some(1_000));
        assert_eq!(
            analysis[0].price_low_minor,
            Some(
                bars.iter()
                    .flat_map(|bar| [bar.open_minor, bar.close_minor])
                    .min()
                    .expect("low")
            )
        );
        assert_eq!(
            analysis[0].price_high_minor,
            Some(
                bars.iter()
                    .flat_map(|bar| [bar.open_minor, bar.close_minor])
                    .max()
                    .expect("high")
            )
        );
        let analysis_detail = bridge
            .analysis_detail(&config.experiment_id)
            .expect("analysis detail should load");
        assert_eq!(analysis_detail.record["report"]["request"], report.request);
        let replay = bridge.backtest_replay_history(100).expect("replay history");
        assert_eq!(replay.runs.len(), 1);
        assert_eq!(replay.runs[0].final_equity_minor, result.final_equity_minor);
        assert_eq!(replay.entries.len(), result.fills.len());
        assert!(replay
            .entries
            .iter()
            .all(|entry| entry.experiment_id == config.experiment_id));
    }

    #[test]
    fn saves_blocked_and_meeting_analysis_without_inventing_prices() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        bridge
            .save_analysis_note(AnalysisNoteRequest {
                record_id: "analysis-meeting-1".to_owned(),
                kind: "meeting".to_owned(),
                status: "held".to_owned(),
                market: "mixed".to_owned(),
                title: "복합 시장 안건".to_owned(),
                symbol: None,
                currency: None,
                requested_at_ms: Some(1),
                content: json!({"type": "meeting", "synthesis": {"decision": "hold"}}),
            })
            .expect("save note");
        let history = bridge.analysis_history(10).expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].kind, "meeting");
        assert_eq!(history[0].status, "held");
        assert_eq!(history[0].price_low_minor, None);
        let detail = bridge
            .analysis_detail("analysis-meeting-1")
            .expect("detail");
        assert_eq!(detail.record["type"], "meeting");
    }

    #[test]
    fn saves_securities_and_crypto_futures_analysis_separately() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        for (record_id, market) in [
            ("analysis-index-future-1", "securities_futures"),
            ("analysis-perpetual-1", "crypto_futures"),
        ] {
            bridge
                .save_analysis_note(AnalysisNoteRequest {
                    record_id: record_id.to_owned(),
                    kind: "instrument".to_owned(),
                    status: "completed".to_owned(),
                    market: market.to_owned(),
                    title: format!("{market} 분석"),
                    symbol: None,
                    currency: None,
                    requested_at_ms: Some(1),
                    content: json!({"type": "role_report"}),
                })
                .expect("futures note should save");
        }
        let history = bridge.analysis_history(10).expect("history");
        assert!(history
            .iter()
            .any(|item| item.market == "securities_futures"));
        assert!(history.iter().any(|item| item.market == "crypto_futures"));
    }

    #[test]
    fn rejects_identifier_reuse_with_different_content_atomically() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        let (report, review, bars, config, result) = fixture();
        bridge
            .persist_backtest(PersistBacktest {
                report: &report,
                review: &review,
                bars: &bars,
                config: &config,
                result: &result,
                provider: "FIXTURE",
                interval: "1d",
                adjusted: true,
                warnings: &[],
                requested_at_ms: None,
                classification: "research_experiment",
            })
            .expect("first write succeeds");

        let mut changed_bars = bars.clone();
        changed_bars[0].close_minor += 1;
        let error = bridge
            .persist_backtest(PersistBacktest {
                report: &report,
                review: &review,
                bars: &changed_bars,
                config: &config,
                result: &result,
                provider: "FIXTURE",
                interval: "1d",
                adjusted: true,
                warnings: &[],
                requested_at_ms: None,
                classification: "research_experiment",
            })
            .expect_err("dataset collision must fail");
        assert!(error.contains("데이터셋"));
        assert_eq!(bridge.status().expect("status").backtest_run_count, 1);
    }

    #[test]
    fn validates_history_and_detail_inputs() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        assert!(bridge.history(0).is_err());
        assert!(bridge.history(MAX_HISTORY_LIMIT + 1).is_err());
        assert!(bridge.detail("").is_err());
        assert!(bridge.detail("missing").is_err());
        assert!(bridge.analysis_history(0).is_err());
        assert!(bridge.analysis_detail("").is_err());
        assert!(bridge.analysis_detail("missing").is_err());
    }

    #[test]
    fn telegram_revisions_are_deduplicated_and_point_in_time_safe() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        bridge
            .replace_telegram_sources(&[TelegramSourceRecord {
                peer_id: -1_001_234_567_890,
                title: "투자 뉴스".to_owned(),
                username: Some("investment_news".to_owned()),
                enabled: true,
                last_message_id: None,
                updated_at_ms: 1_000,
            }])
            .expect("source should save");
        let initial = TelegramMessageRevision {
            peer_id: -1_001_234_567_890,
            message_id: 7,
            posted_at_ms: 1_000,
            edited_at_ms: None,
            ingested_at_ms: 1_100,
            content_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            text: "초기 뉴스",
        };
        assert_eq!(
            bridge
                .persist_telegram_revisions(initial.peer_id, &[initial], Some(7), 1_100)
                .expect("initial revision"),
            1
        );
        let duplicate = TelegramMessageRevision {
            peer_id: -1_001_234_567_890,
            message_id: 7,
            posted_at_ms: 1_000,
            edited_at_ms: None,
            ingested_at_ms: 1_200,
            content_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            text: "초기 뉴스",
        };
        assert_eq!(
            bridge
                .persist_telegram_revisions(duplicate.peer_id, &[duplicate], Some(7), 1_200)
                .expect("duplicate revision"),
            0
        );
        let edited = TelegramMessageRevision {
            peer_id: -1_001_234_567_890,
            message_id: 7,
            posted_at_ms: 1_000,
            edited_at_ms: Some(2_000),
            ingested_at_ms: 2_100,
            content_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            text: "수정 뉴스",
        };
        bridge
            .persist_telegram_revisions(edited.peer_id, &[edited], Some(7), 2_100)
            .expect("edited revision");

        let before_edit = bridge
            .telegram_evidence(1_500, 0, 10)
            .expect("point in time before edit");
        assert_eq!(before_edit.len(), 1);
        assert_eq!(before_edit[0].text, "초기 뉴스");
        let after_edit = bridge
            .telegram_evidence(2_200, 0, 10)
            .expect("point in time after edit");
        assert_eq!(after_edit.len(), 1);
        assert_eq!(after_edit[0].text, "수정 뉴스");
    }

    #[test]
    fn telegram_source_selection_disables_unselected_sources_without_deleting_history() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        let source = |peer_id, title: &str| TelegramSourceRecord {
            peer_id,
            title: title.to_owned(),
            username: None,
            enabled: true,
            last_message_id: None,
            updated_at_ms: 1_000,
        };
        bridge
            .replace_telegram_sources(&[
                source(-1_001_000_000_001, "A"),
                source(-1_001_000_000_002, "B"),
            ])
            .expect("sources should save");
        bridge
            .replace_telegram_sources(&[source(-1_001_000_000_002, "B")])
            .expect("selection should replace");
        let selected = bridge.telegram_sources().expect("sources should load");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].title, "B");
        let all_count: u64 = bridge
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM telegram_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(all_count, 2);
    }

    #[test]
    fn refuses_to_downgrade_a_newer_database_schema() {
        let connection = Connection::open_in_memory().expect("database should open");
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .expect("future schema should set");
        let error = initialize(&connection).expect_err("future schema must be rejected");
        assert!(error.contains("앱을 업데이트"));
    }

    #[test]
    fn upgrades_analysis_notes_to_futures_markets_without_losing_history() {
        let connection = Connection::open_in_memory().expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE analysis_notes (
                    record_id TEXT PRIMARY KEY NOT NULL,
                    kind TEXT NOT NULL CHECK(kind IN ('instrument', 'meeting', 'strategy')),
                    status TEXT NOT NULL CHECK(status IN ('completed', 'blocked', 'held', 'error')),
                    market TEXT NOT NULL CHECK(market IN ('kr', 'us', 'coin', 'mixed')),
                    title TEXT NOT NULL,
                    symbol TEXT,
                    currency TEXT,
                    requested_at_ms INTEGER,
                    content_json TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                 ) STRICT;
                 INSERT INTO analysis_notes VALUES
                    ('legacy-analysis', 'meeting', 'completed', 'mixed', '기존 안건', NULL, NULL, 1, '{}', 2);
                 PRAGMA user_version = 20;",
            )
            .expect("legacy schema");
        initialize(&connection).expect("migration should succeed");
        let legacy_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM analysis_notes WHERE record_id = 'legacy-analysis'",
                [],
                |row| row.get(0),
            )
            .expect("legacy row");
        connection
            .execute(
                "INSERT INTO analysis_notes VALUES ('future-analysis', 'instrument', 'completed', 'crypto_futures', '코인 선물', NULL, NULL, 1, '{}', 3)",
                [],
            )
            .expect("new futures market should pass the constraint");
        assert_eq!(legacy_count, 1);
    }

    #[test]
    fn upgrades_version_twenty_one_with_market_stream_checkpoints() {
        let connection = Connection::open_in_memory().expect("database should open");
        connection
            .pragma_update(None, "user_version", 21)
            .expect("legacy version");
        initialize(&connection).expect("migration should succeed");
        let table_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='market_stream_checkpoints'",
                [],
                |row| row.get(0),
            )
            .expect("checkpoint table");
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(table_count, 1);
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn upgrades_version_thirty_with_meeting_backtest_lineage_columns() {
        let connection = Connection::open_in_memory().expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE meeting_paper_handoffs (
                    handoff_id TEXT PRIMARY KEY NOT NULL,
                    workflow_job_id TEXT NOT NULL UNIQUE,
                    analysis_record_id TEXT NOT NULL,
                    symbol TEXT NOT NULL,
                    strategy TEXT NOT NULL,
                    engine_run_id TEXT UNIQUE,
                    created_at_ms INTEGER NOT NULL,
                    updated_at_ms INTEGER NOT NULL
                 ) STRICT;
                 PRAGMA user_version = 30;",
            )
            .expect("version 30 schema");
        initialize(&connection).expect("migration should succeed");
        let experiment_column_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('meeting_paper_handoffs') WHERE name='experiment_id'",
                [],
                |row| row.get(0),
            )
            .expect("experiment column");
        let candidate_column_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('meeting_paper_handoffs') WHERE name='paper_candidate_id'",
                [],
                |row| row.get(0),
            )
            .expect("candidate column");
        assert_eq!(experiment_column_count, 1);
        assert_eq!(candidate_column_count, 1);
    }

    #[test]
    fn upgrades_a_version_one_database_without_removing_existing_ledger_events() {
        let connection = Connection::open_in_memory().expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE paper_ledger_events (
                    ledger_id TEXT NOT NULL,
                    event_index INTEGER NOT NULL,
                    event_type TEXT NOT NULL,
                    event_json TEXT NOT NULL,
                    occurred_at_ms INTEGER NOT NULL,
                    created_at_ms INTEGER NOT NULL,
                    PRIMARY KEY(ledger_id, event_index)
                 ) STRICT;
                 INSERT INTO paper_ledger_events VALUES
                    ('legacy-paper', 0, 'account_opened', '{}', 1, 1);
                 PRAGMA user_version = 1;",
            )
            .expect("legacy schema");
        initialize(&connection).expect("migration should succeed");
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        let legacy_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM paper_ledger_events WHERE ledger_id = 'legacy-paper'",
                [],
                |row| row.get(0),
            )
            .expect("legacy row");
        let requested_at_column_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('backtest_runs') WHERE name = 'requested_at_ms'",
                [],
                |row| row.get(0),
            )
            .expect("requested time column");
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(legacy_count, 1);
        assert_eq!(requested_at_column_count, 1);
    }

    #[test]
    fn persists_and_replaces_codex_agent_thread_mapping() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        bridge
            .save_codex_thread("paper-researcher", "thread-1")
            .expect("mapping should save");
        assert_eq!(
            bridge.codex_threads().expect("mapping should load"),
            vec![("paper-researcher".to_owned(), "thread-1".to_owned())]
        );
        bridge
            .save_codex_thread("paper-researcher", "thread-2")
            .expect("mapping should update");
        assert_eq!(bridge.codex_threads().expect("mapping")[0].1, "thread-2");
        bridge
            .remove_codex_thread("paper-researcher")
            .expect("mapping should delete");
        assert!(bridge.codex_threads().expect("mapping").is_empty());
    }

    #[test]
    fn reopens_and_replays_an_append_only_paper_ledger() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        {
            let mut ledger = bridge
                .paper_ledger("paper-krw")
                .expect("ledger should open");
            open_paper_account(
                &mut ledger,
                "paper-account".to_owned(),
                "KRW".to_owned(),
                1_000_000,
                1_000,
            )
            .expect("account should open");
            execute_shadow_order(
                &mut ledger,
                ShadowOrderRequest {
                    account_id: "paper-account".to_owned(),
                    order_id: "order-1".to_owned(),
                    idempotency_key: "idem-1".to_owned(),
                    symbol: "005930".to_owned(),
                    currency: "KRW".to_owned(),
                    side: TradeSide::Buy,
                    quantity: 2,
                    quantity_scale: 1,
                    reference_price_minor: 70_000,
                    occurred_at_ms: 2_000,
                },
                TradingCosts {
                    buy_fee_bps: 0.0,
                    sell_fee_bps: 0.0,
                    sell_tax_bps: 0.0,
                    slippage_bps: 0.0,
                },
            )
            .expect("paper order should fill");
        }

        let reopened = bridge
            .paper_ledger("paper-krw")
            .expect("ledger should reopen");
        let state = replay_ledger(reopened.events()).expect("ledger should replay");
        assert_eq!(state.event_count, 2);
        assert_eq!(state.cash_minor, 860_000);
        assert_eq!(state.positions["005930"].quantity, 2);
    }

    #[test]
    fn rejects_concurrent_stale_paper_ledger_append() {
        let bridge = PersistenceBridge::in_memory().expect("database should initialize");
        let mut first = bridge.paper_ledger("paper-krw").expect("first ledger");
        let mut stale = bridge.paper_ledger("paper-krw").expect("stale ledger");
        open_paper_account(
            &mut first,
            "paper-account".to_owned(),
            "KRW".to_owned(),
            1_000_000,
            1_000,
        )
        .expect("first append should succeed");
        let error = open_paper_account(
            &mut stale,
            "other-account".to_owned(),
            "KRW".to_owned(),
            1_000_000,
            1_000,
        )
        .expect_err("stale append must fail");
        assert_eq!(error.code, LedgerErrorCode::AppendFailed);
    }
}
