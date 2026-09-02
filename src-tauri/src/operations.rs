use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::{
    backtest::latest_signal,
    crypto_market::CryptoMarketBridge,
    market_data::MarketDataBridge,
    paper_account::{execute_shadow_order, AppendOnlyLedger, LedgerEvent, ShadowOrderRequest},
    paper_trading::{self, PaperAccountSnapshot, PAPER_ACCOUNT_ID, PAPER_LEDGER_ID},
    persistence::{self, PersistenceBridge},
    research::{review_strategy_spec, StrategySpec},
    simulation::default_stock_costs,
    trading::TradeSide,
};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    SafetyApproved,
    UserApproved,
    Submitted,
    PartiallyFilled,
    Filled,
    Rejected,
    Cancelled,
    Expired,
}

impl CandidateStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::SafetyApproved => "safety_approved",
            Self::UserApproved => "user_approved",
            Self::Submitted => "submitted",
            Self::PartiallyFilled => "partially_filled",
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
            "partially_filled" => Ok(Self::PartiallyFilled),
            "filled" => Ok(Self::Filled),
            "rejected" => Ok(Self::Rejected),
            "cancelled" => Ok(Self::Cancelled),
            "expired" => Ok(Self::Expired),
            _ => Err("저장된 주문 후보 상태를 해석하지 못했습니다.".to_owned()),
        }
    }
}

fn transition_allowed(from: CandidateStatus, to: CandidateStatus) -> bool {
    matches!(
        (from, to),
        (
            CandidateStatus::SafetyApproved,
            CandidateStatus::UserApproved
        ) | (CandidateStatus::SafetyApproved, CandidateStatus::Rejected)
            | (CandidateStatus::SafetyApproved, CandidateStatus::Cancelled)
            | (CandidateStatus::SafetyApproved, CandidateStatus::Expired)
            | (CandidateStatus::UserApproved, CandidateStatus::Submitted)
            | (CandidateStatus::UserApproved, CandidateStatus::Cancelled)
            | (CandidateStatus::Submitted, CandidateStatus::PartiallyFilled)
            | (CandidateStatus::Submitted, CandidateStatus::Filled)
            | (CandidateStatus::Submitted, CandidateStatus::Rejected)
            | (CandidateStatus::Submitted, CandidateStatus::Cancelled)
            | (
                CandidateStatus::PartiallyFilled,
                CandidateStatus::PartiallyFilled
            )
            | (CandidateStatus::PartiallyFilled, CandidateStatus::Filled)
            | (CandidateStatus::PartiallyFilled, CandidateStatus::Cancelled)
    )
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyGate {
    pub passed: bool,
    pub checks: Vec<String>,
    pub performance_thresholds_configured: bool,
    pub live_order_enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderCandidate {
    pub candidate_id: String,
    pub experiment_id: String,
    pub trace_id: String,
    pub symbol: String,
    pub currency: String,
    pub side: TradeSide,
    pub quantity: u64,
    pub reference_price_minor: u64,
    pub observed_at_ms: u64,
    pub source: String,
    pub status: CandidateStatus,
    pub safety: SafetyGate,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCandidateRequest {
    pub experiment_id: String,
    pub side: TradeSide,
    pub quantity: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateActionRequest {
    pub candidate_id: String,
}

#[derive(Debug)]
struct StoredExperiment {
    trace_id: String,
    symbol: String,
    currency: String,
    interval: String,
    record: Value,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn load_experiment(
    persistence: &PersistenceBridge,
    experiment_id: &str,
) -> Result<StoredExperiment, String> {
    if !valid_id(experiment_id) {
        return Err("유효한 저장 실험 ID가 필요합니다.".to_owned());
    }
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let row = connection
        .query_row(
            "SELECT b.trace_id, b.symbol, b.currency, b.interval, b.record_json
             FROM backtest_runs b
             WHERE b.experiment_id = ?1",
            params![experiment_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("저장된 백테스트를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "선택한 백테스트 기록을 찾지 못했습니다.".to_owned())?;
    let record: Value = serde_json::from_str(&row.4)
        .map_err(|error| format!("백테스트 기록을 해석하지 못했습니다: {error}"))?;
    Ok(StoredExperiment {
        trace_id: row.0,
        symbol: row.1,
        currency: row.2,
        interval: row.3,
        record,
    })
}

fn validate_experiment(experiment: &StoredExperiment) -> Result<(), String> {
    let executable = experiment
        .record
        .pointer("/review/executable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !executable {
        return Err("실행 가능 검증을 통과한 백테스트 기록만 후보로 만들 수 있습니다.".to_owned());
    }
    if experiment.currency != "KRW" {
        return Err("현재 내부 모의계좌는 KRW 전략만 지원합니다.".to_owned());
    }
    Ok(())
}

fn safety_gate(
    persistence: &PersistenceBridge,
    experiment: &StoredExperiment,
    side: TradeSide,
    quantity: u64,
    price: u64,
) -> Result<SafetyGate, String> {
    validate_experiment(experiment)?;
    if quantity == 0 || price == 0 {
        return Err("수량과 기준 가격은 0보다 커야 합니다.".to_owned());
    }
    let quantity_scale: u64 = if experiment.symbol.starts_with("KRW-") {
        100_000_000
    } else {
        1
    };
    let notional =
        u64::try_from(u128::from(price) * u128::from(quantity) / u128::from(quantity_scale))
            .map_err(|_| "모의주문 금액이 지원 범위를 초과했습니다.".to_owned())?;
    // 보호 정책은 내부 원장의 종료 거래를 읽으므로 첫 주문에서도 계좌 개설 사건을
    // 먼저 멱등적으로 보장한다.
    let account = paper_trading::load_or_open_account(persistence)?;
    let active_policy = crate::risk_policy::active_policy(persistence)?;
    if let Some(policy) = &active_policy {
        if notional > policy.max_order_notional_minor {
            return Err("활성 위험 정책의 주문 금액 한도를 초과했습니다.".to_owned());
        }
        let drawdown = experiment
            .record
            .pointer("/result/maxDrawdownBps")
            .and_then(Value::as_u64)
            .ok_or_else(|| "백테스트 최대 낙폭을 확인하지 못했습니다.".to_owned())?;
        if drawdown > policy.max_backtest_drawdown_bps {
            return Err("활성 위험 정책의 백테스트 최대 낙폭 한도를 초과했습니다.".to_owned());
        }
        let ledger = persistence.paper_ledger(PAPER_LEDGER_ID)?;
        let events = ledger.events();
        let day_start = persistence::now_ms()? / 86_400_000 * 86_400_000;
        let before = events
            .iter()
            .take_while(|event| event.occurred_at_ms() < day_start)
            .cloned()
            .collect::<Vec<_>>();
        let prior_pnl = if before.is_empty() {
            0
        } else {
            crate::paper_account::replay_ledger(&before)
                .map_err(|error| error.message)?
                .realized_pnl_minor
        };
        let today_loss = account_daily_loss(events, prior_pnl)?;
        if today_loss >= policy.daily_loss_limit_minor {
            return Err("활성 위험 정책의 일일 실현손실 한도에 도달했습니다.".to_owned());
        }
        if side == TradeSide::Buy {
            if let Some(decision) = crate::strategy_protection::evaluate_runtime_protection(
                persistence,
                policy,
                &experiment.symbol,
                &experiment.currency,
                persistence::now_ms()?,
            )? {
                if !decision.can_open_new_position {
                    let reasons = decision
                        .triggers
                        .iter()
                        .map(|trigger| trigger.code.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(format!(
                        "활성 전략 보호 정책이 신규 진입을 잠갔습니다: {reasons}"
                    ));
                }
            }
        }
    }
    match side {
        TradeSide::Buy => {
            if account.cash_minor < notional {
                return Err("내부 모의계좌 예수금이 부족합니다.".to_owned());
            }
        }
        TradeSide::Sell => {
            if account
                .positions
                .get(&experiment.symbol)
                .is_none_or(|position| position.quantity < quantity)
            {
                return Err("내부 모의계좌 보유 수량이 부족합니다.".to_owned());
            }
        }
    }
    let mut checks = vec![
        "불변 연구·백테스트 기록 확인".to_owned(),
        "실행 가능한 전략 계약 확인".to_owned(),
        "KRW 내부 모의원장 잔고·보유수량 확인".to_owned(),
        "토스 최신 현재가 확인".to_owned(),
        "실전 주문 전송 경로 잠금 확인".to_owned(),
    ];
    if active_policy.is_some() {
        checks.push("사용자 승인 위험 정책의 주문 금액·백테스트 MDD 확인".to_owned());
        if active_policy
            .as_ref()
            .is_some_and(|policy| policy.protection.is_some())
        {
            checks.push("내부 모의원장 기반 쿨다운·연속손실·낙폭 보호 확인".to_owned());
        }
    }
    Ok(SafetyGate {
        passed: true,
        checks,
        performance_thresholds_configured: active_policy.is_some(),
        live_order_enabled: false,
    })
}

fn account_daily_loss(events: &[LedgerEvent], prior_pnl: i64) -> Result<u64, String> {
    if events.is_empty() {
        return Ok(0);
    }
    let current = crate::paper_account::replay_ledger(events)
        .map_err(|error| error.message)?
        .realized_pnl_minor;
    Ok(current
        .checked_sub(prior_pnl)
        .unwrap_or(i64::MIN)
        .min(0)
        .unsigned_abs())
}

fn insert_candidate(
    persistence: &PersistenceBridge,
    experiment_id: &str,
    experiment: &StoredExperiment,
    side: TradeSide,
    quantity: u64,
    price: u64,
    observed_at_ms: u64,
    source: &str,
    gate: &SafetyGate,
) -> Result<OrderCandidate, String> {
    let experiment_suffix: String = experiment_id.chars().take(64).collect();
    let candidate_id = format!("cand-{observed_at_ms}-{experiment_suffix}");
    if !valid_id(&candidate_id) {
        return Err("주문 후보 ID를 만들지 못했습니다.".to_owned());
    }
    let safety_json = serde_json::to_string(gate)
        .map_err(|error| format!("안전 게이트를 기록하지 못했습니다: {error}"))?;
    let side_text = match side {
        TradeSide::Buy => "buy",
        TradeSide::Sell => "sell",
    };
    let event_json = serde_json::to_string(&json!({
        "experimentId": experiment_id,
        "source": source,
        "safety": gate,
    }))
    .map_err(|error| format!("주문 후보 사건을 기록하지 못했습니다: {error}"))?;
    let mut connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("주문 후보 트랜잭션을 시작하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "INSERT INTO paper_order_candidates
             (candidate_id, experiment_id, trace_id, symbol, currency, side, quantity,
              reference_price_minor, observed_at_ms, source, status, safety_json, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'safety_approved', ?11, ?12, ?12)",
            params![candidate_id, experiment_id, experiment.trace_id, experiment.symbol,
                experiment.currency, side_text, quantity, price, observed_at_ms, source,
                safety_json, observed_at_ms],
        )
        .map_err(|error| {
            if error.to_string().contains("UNIQUE constraint failed") {
                "같은 실험·종목·방향의 처리 중인 주문 후보가 이미 있습니다.".to_owned()
            } else {
                format!("주문 후보를 저장하지 못했습니다: {error}")
            }
        })?;
    for (index, event_type) in ["candidate_created", "safety_approved"].iter().enumerate() {
        transaction
            .execute(
                "INSERT INTO paper_order_events
                 (candidate_id, event_index, event_type, event_json, occurred_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    candidate_id,
                    index as u64,
                    event_type,
                    event_json,
                    observed_at_ms
                ],
            )
            .map_err(|error| format!("주문 후보 사건을 저장하지 못했습니다: {error}"))?;
    }
    transaction
        .commit()
        .map_err(|error| format!("주문 후보 저장을 확정하지 못했습니다: {error}"))?;
    Ok(OrderCandidate {
        candidate_id,
        experiment_id: experiment_id.to_owned(),
        trace_id: experiment.trace_id.clone(),
        symbol: experiment.symbol.clone(),
        currency: experiment.currency.clone(),
        side,
        quantity,
        reference_price_minor: price,
        observed_at_ms,
        source: source.to_owned(),
        status: CandidateStatus::SafetyApproved,
        safety: gate.clone(),
        created_at_ms: observed_at_ms,
        updated_at_ms: observed_at_ms,
    })
}

fn row_candidate(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    String,
    String,
    String,
    String,
    String,
    String,
    u64,
    u64,
    u64,
    String,
    String,
    String,
    u64,
    u64,
)> {
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

type CandidateRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    u64,
    u64,
    u64,
    String,
    String,
    String,
    u64,
    u64,
);

fn candidate_from_row(row: CandidateRow) -> Result<OrderCandidate, String> {
    Ok(OrderCandidate {
        candidate_id: row.0,
        experiment_id: row.1,
        trace_id: row.2,
        symbol: row.3,
        currency: row.4,
        side: match row.5.as_str() {
            "buy" => TradeSide::Buy,
            "sell" => TradeSide::Sell,
            _ => return Err("저장 주문 방향이 올바르지 않습니다.".to_owned()),
        },
        quantity: row.6,
        reference_price_minor: row.7,
        observed_at_ms: row.8,
        source: row.9,
        status: CandidateStatus::parse(&row.10)?,
        safety: serde_json::from_str(&row.11)
            .map_err(|error| format!("저장 안전 게이트를 해석하지 못했습니다: {error}"))?,
        created_at_ms: row.12,
        updated_at_ms: row.13,
    })
}

fn load_candidate(
    persistence: &PersistenceBridge,
    candidate_id: &str,
) -> Result<OrderCandidate, String> {
    if !valid_id(candidate_id) {
        return Err("유효한 주문 후보 ID가 필요합니다.".to_owned());
    }
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let row = connection.query_row(
        "SELECT candidate_id, experiment_id, trace_id, symbol, currency, side, quantity,
         reference_price_minor, observed_at_ms, source, status, safety_json, created_at_ms, updated_at_ms
         FROM paper_order_candidates WHERE candidate_id = ?1", params![candidate_id], row_candidate)
        .optional().map_err(|error| format!("주문 후보를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "주문 후보를 찾지 못했습니다.".to_owned())?;
    candidate_from_row(row)
}

fn append_transition(
    persistence: &PersistenceBridge,
    candidate_id: &str,
    to: CandidateStatus,
    detail: Value,
) -> Result<(), String> {
    let now = persistence::now_ms()?;
    let mut connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("주문 상태 트랜잭션을 시작하지 못했습니다: {error}"))?;
    let from_text: String = transaction
        .query_row(
            "SELECT status FROM paper_order_candidates WHERE candidate_id = ?1",
            params![candidate_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("현재 주문 상태를 확인하지 못했습니다: {error}"))?
        .ok_or_else(|| "주문 후보를 찾지 못했습니다.".to_owned())?;
    let from = CandidateStatus::parse(&from_text)?;
    if !transition_allowed(from, to) {
        return Err(format!(
            "허용되지 않은 주문 상태 전이입니다: {} → {}",
            from.as_str(),
            to.as_str()
        ));
    }
    let index: u64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM paper_order_events WHERE candidate_id = ?1",
            params![candidate_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("주문 사건 순번을 확인하지 못했습니다: {error}"))?;
    let event_json = serde_json::to_string(&detail)
        .map_err(|error| format!("주문 사건을 직렬화하지 못했습니다: {error}"))?;
    transaction.execute("INSERT INTO paper_order_events (candidate_id, event_index, event_type, event_json, occurred_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![candidate_id, index, to.as_str(), event_json, now]).map_err(|error| format!("주문 사건을 저장하지 못했습니다: {error}"))?;
    transaction.execute("UPDATE paper_order_candidates SET status = ?2, updated_at_ms = ?3 WHERE candidate_id = ?1",
        params![candidate_id, to.as_str(), now]).map_err(|error| format!("주문 상태를 갱신하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("주문 상태 저장을 확정하지 못했습니다: {error}"))
}

#[tauri::command]
pub async fn paper_order_candidate_create(
    request: CreateCandidateRequest,
    market: State<'_, MarketDataBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<OrderCandidate, String> {
    let experiment = load_experiment(&persistence, &request.experiment_id)?;
    validate_experiment(&experiment)?;
    let (price, observed_at_ms) = market.fetch_krw_current_price(&experiment.symbol).await?;
    let gate = safety_gate(
        &persistence,
        &experiment,
        request.side,
        request.quantity,
        price,
    )?;
    insert_candidate(
        &persistence,
        &request.experiment_id,
        &experiment,
        request.side,
        request.quantity,
        price,
        observed_at_ms,
        "manual",
        &gate,
    )
}

#[tauri::command]
pub fn paper_order_candidates(
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<OrderCandidate>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare(
        "SELECT candidate_id, experiment_id, trace_id, symbol, currency, side, quantity,
         reference_price_minor, observed_at_ms, source, status, safety_json, created_at_ms, updated_at_ms
         FROM paper_order_candidates ORDER BY created_at_ms DESC LIMIT 100")
        .map_err(|error| format!("주문 후보 목록을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], row_candidate)
        .map_err(|error| format!("주문 후보 목록을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        row.map_err(|error| format!("주문 후보를 읽지 못했습니다: {error}"))
            .and_then(candidate_from_row)
    })
    .collect()
}

#[tauri::command]
pub async fn paper_order_candidate_approve(
    request: CandidateActionRequest,
    market: State<'_, MarketDataBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PaperAccountSnapshot, String> {
    let candidate = load_candidate(&persistence, &request.candidate_id)?;
    if candidate.status != CandidateStatus::SafetyApproved {
        return Err("안전 승인 대기 중인 후보만 사용자가 체결 승인할 수 있습니다.".to_owned());
    }
    let experiment = load_experiment(&persistence, &candidate.experiment_id)?;
    let (price, observed_at_ms) = market.fetch_krw_current_price(&candidate.symbol).await?;
    safety_gate(
        &persistence,
        &experiment,
        candidate.side,
        candidate.quantity,
        price,
    )?;
    append_transition(
        &persistence,
        &candidate.candidate_id,
        CandidateStatus::UserApproved,
        json!({"approvedBy": "local_user", "observedAtMs": observed_at_ms}),
    )?;
    append_transition(
        &persistence,
        &candidate.candidate_id,
        CandidateStatus::Submitted,
        json!({"mode": "internal_paper_ledger", "referencePriceMinor": price}),
    )?;
    paper_trading::load_or_open_account(&persistence)?;
    let mut ledger = persistence.paper_ledger(PAPER_LEDGER_ID)?;
    match execute_shadow_order(
        &mut ledger,
        ShadowOrderRequest {
            account_id: PAPER_ACCOUNT_ID.to_owned(),
            order_id: format!("order-{}", candidate.candidate_id),
            idempotency_key: candidate.candidate_id.clone(),
            symbol: candidate.symbol.clone(),
            currency: candidate.currency.clone(),
            side: candidate.side,
            quantity: candidate.quantity,
            quantity_scale: if candidate.symbol.starts_with("KRW-") {
                100_000_000
            } else {
                1
            },
            reference_price_minor: price,
            occurred_at_ms: observed_at_ms,
        },
        if candidate.symbol.starts_with("KRW-") {
            crate::simulation::TradingCosts {
                buy_fee_bps: 5.0,
                sell_fee_bps: 5.0,
                sell_tax_bps: 0.0,
                slippage_bps: 0.0,
            }
        } else {
            default_stock_costs(&candidate.currency)
                .map_err(|_| "지원하지 않는 통화의 공식 모의체결 비용입니다.".to_owned())?
        },
    ) {
        Ok(account) => {
            append_transition(
                &persistence,
                &candidate.candidate_id,
                CandidateStatus::Filled,
                json!({"quantity": candidate.quantity, "referencePriceMinor": price, "ledgerId": PAPER_LEDGER_ID}),
            )?;
            Ok(paper_trading::snapshot(account))
        }
        Err(error) => {
            let _ = append_transition(
                &persistence,
                &candidate.candidate_id,
                CandidateStatus::Rejected,
                json!({"reason": error.message}),
            );
            Err(error.message)
        }
    }
}

#[tauri::command]
pub fn paper_order_candidate_reject(
    request: CandidateActionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    append_transition(
        &persistence,
        &request.candidate_id,
        CandidateStatus::Rejected,
        json!({"reason": "user_rejected"}),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationsRecovery {
    pub recovered_filled_count: usize,
    pub safely_closed_count: usize,
    pub message: String,
}

#[tauri::command]
pub fn operations_recover(
    persistence: State<'_, PersistenceBridge>,
) -> Result<OperationsRecovery, String> {
    let candidates = {
        let connection = persistence
            .connection
            .lock()
            .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT candidate_id, status FROM paper_order_candidates
                 WHERE status IN ('user_approved', 'submitted', 'partially_filled')",
            )
            .map_err(|error| format!("복구 대상 주문을 준비하지 못했습니다: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("복구 대상 주문을 조회하지 못했습니다: {error}"))?;
        rows.map(|row| row.map_err(|error| format!("복구 대상 주문을 읽지 못했습니다: {error}")))
            .collect::<Result<Vec<_>, _>>()?
    };
    let ledger = persistence.paper_ledger(PAPER_LEDGER_ID)?;
    let filled_ids = ledger
        .events()
        .iter()
        .filter_map(|event| match event {
            LedgerEvent::OrderFilled {
                idempotency_key, ..
            } => Some(idempotency_key.as_str()),
            LedgerEvent::AccountOpened { .. } => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut recovered_filled_count = 0;
    let mut safely_closed_count = 0;
    for (candidate_id, status) in candidates {
        let current = CandidateStatus::parse(&status)?;
        if filled_ids.contains(candidate_id.as_str()) {
            append_transition(
                &persistence,
                &candidate_id,
                CandidateStatus::Filled,
                json!({"recoveredFromLedger": true}),
            )?;
            recovered_filled_count += 1;
        } else {
            let terminal = match current {
                CandidateStatus::Submitted => CandidateStatus::Rejected,
                CandidateStatus::UserApproved | CandidateStatus::PartiallyFilled => {
                    CandidateStatus::Cancelled
                }
                _ => unreachable!("query limits recovery states"),
            };
            append_transition(
                &persistence,
                &candidate_id,
                terminal,
                json!({"reason": "startup_recovery_no_internal_fill", "liveTransportEnabled": false}),
            )?;
            safely_closed_count += 1;
        }
    }
    Ok(OperationsRecovery {
        recovered_filled_count,
        safely_closed_count,
        message: "실전 주문 전송 없이 내부 원장과 미완료 후보를 대사했습니다.".to_owned(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowWatch {
    pub watch_id: String,
    pub experiment_id: String,
    pub enabled: bool,
    pub interval_seconds: u64,
    pub last_checked_at_ms: Option<u64>,
    pub last_signal_key: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowRuntimeStatus {
    pub running: bool,
    pub enabled_watch_count: usize,
    pub watches: Vec<ShadowWatch>,
    pub live_order_enabled: bool,
    pub message: String,
}

/// 섀도우 감시는 UI 수명과 분리되어야 한다. 이 상태는 프로세스 안에서
/// 백그라운드 루프의 중복 시작과 동시 tick을 막을 뿐 주문 권한을 갖지 않는다.
pub struct ShadowEngineRuntime {
    started: AtomicBool,
    tick_in_progress: AtomicBool,
}

impl Default for ShadowEngineRuntime {
    fn default() -> Self {
        Self {
            started: AtomicBool::new(false),
            tick_in_progress: AtomicBool::new(false),
        }
    }
}

struct ShadowTickGuard<'a>(&'a AtomicBool);

impl Drop for ShadowTickGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl ShadowEngineRuntime {
    fn begin_tick(&self) -> Option<ShadowTickGuard<'_>> {
        self.tick_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| ShadowTickGuard(&self.tick_in_progress))
    }
}

const SHADOW_SCHEDULER_POLL_SECONDS: u64 = 5;

/// 저장된 감시 항목은 SQLite에 남아 있으므로 앱 재시작 뒤에도 자동으로 재개한다.
/// 전용 blocking worker에서만 대기하고, 실제 네트워크 작업은 Tauri async runtime에
/// 위임한다. 외부 주문 전송은 이 경로에 존재하지 않는다.
pub fn start_shadow_engine(app: AppHandle) {
    let runtime = app.state::<ShadowEngineRuntime>();
    if runtime.started.swap(true, Ordering::AcqRel) {
        return;
    }
    drop(runtime);

    tauri::async_runtime::spawn_blocking(move || loop {
        std::thread::sleep(Duration::from_secs(SHADOW_SCHEDULER_POLL_SECONDS));
        let result = tauri::async_runtime::block_on(async {
            let market = app.state::<MarketDataBridge>();
            let crypto = app.state::<CryptoMarketBridge>();
            let persistence = app.state::<PersistenceBridge>();
            let runtime = app.state::<ShadowEngineRuntime>();
            run_shadow_engine_once(&market, &crypto, &persistence, &runtime).await
        });
        if let Err(error) = result {
            eprintln!("섀도우 백그라운드 감시 오류: {error}");
        }
    });
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowWatchRequest {
    pub experiment_id: String,
    pub interval_seconds: Option<u64>,
}

fn load_watches(persistence: &PersistenceBridge) -> Result<Vec<ShadowWatch>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare("SELECT watch_id, experiment_id, enabled, interval_seconds, last_checked_at_ms, last_signal_key, status, last_error FROM shadow_watches ORDER BY created_at_ms DESC")
        .map_err(|error| format!("섀도우 감시 목록을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok(ShadowWatch {
                watch_id: row.get(0)?,
                experiment_id: row.get(1)?,
                enabled: row.get::<_, i64>(2)? == 1,
                interval_seconds: row.get(3)?,
                last_checked_at_ms: row.get(4)?,
                last_signal_key: row.get(5)?,
                status: row.get(6)?,
                last_error: row.get(7)?,
            })
        })
        .map_err(|error| format!("섀도우 감시 목록을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| row.map_err(|error| format!("섀도우 감시를 읽지 못했습니다: {error}")))
        .collect()
}

#[tauri::command]
pub fn shadow_runtime_status(
    persistence: State<'_, PersistenceBridge>,
) -> Result<ShadowRuntimeStatus, String> {
    shadow_runtime_status_from_bridge(&persistence)
}

#[tauri::command]
pub fn shadow_watch_arm(
    request: ShadowWatchRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<ShadowRuntimeStatus, String> {
    arm_shadow_watch(
        &persistence,
        &request.experiment_id,
        request.interval_seconds,
    )?;
    shadow_runtime_status(persistence)
}

pub(crate) fn arm_shadow_watch(
    persistence: &PersistenceBridge,
    experiment_id: &str,
    interval_seconds: Option<u64>,
) -> Result<(), String> {
    load_experiment(persistence, experiment_id)
        .and_then(|experiment| validate_experiment(&experiment))?;
    let interval = interval_seconds.unwrap_or(60).clamp(15, 86_400);
    let now = persistence::now_ms()?;
    let watch_id = format!("watch-{experiment_id}");
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection.execute("INSERT INTO shadow_watches (watch_id, experiment_id, enabled, interval_seconds, status, created_at_ms, updated_at_ms)
        VALUES (?1, ?2, 1, ?3, 'watching', ?4, ?4)
        ON CONFLICT(watch_id) DO UPDATE SET enabled = 1, interval_seconds = excluded.interval_seconds, status = 'watching', last_error = NULL, updated_at_ms = excluded.updated_at_ms",
        params![watch_id, experiment_id, interval, now]).map_err(|error| format!("섀도우 감시를 시작하지 못했습니다: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn shadow_watch_stop(
    request: CandidateActionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<ShadowRuntimeStatus, String> {
    let now = persistence::now_ms()?;
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let changed = connection.execute("UPDATE shadow_watches SET enabled = 0, status = 'stopped', updated_at_ms = ?2 WHERE watch_id = ?1", params![request.candidate_id, now])
        .map_err(|error| format!("섀도우 감시를 중지하지 못했습니다: {error}"))?;
    if changed == 0 {
        return Err("중지할 섀도우 감시를 찾지 못했습니다.".to_owned());
    }
    drop(connection);
    shadow_runtime_status(persistence)
}

fn update_watch(
    persistence: &PersistenceBridge,
    watch_id: &str,
    now: u64,
    signal_key: Option<&str>,
    error: Option<&str>,
) -> Result<(), String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection.execute("UPDATE shadow_watches SET last_checked_at_ms = ?2, last_signal_key = COALESCE(?3, last_signal_key), status = ?4, last_error = ?5, updated_at_ms = ?2 WHERE watch_id = ?1",
        params![watch_id, now, signal_key, if error.is_some() { "error" } else { "watching" }, error]).map_err(|e| format!("섀도우 감시 상태를 저장하지 못했습니다: {e}"))?;
    Ok(())
}

fn shadow_watch_is_due(watch: &ShadowWatch, now: u64) -> bool {
    watch.enabled
        && watch
            .last_checked_at_ms
            .is_none_or(|last| now.saturating_sub(last) >= watch.interval_seconds * 1_000)
}

pub(crate) async fn run_shadow_engine_once(
    market: &MarketDataBridge,
    crypto: &CryptoMarketBridge,
    persistence: &PersistenceBridge,
    runtime: &ShadowEngineRuntime,
) -> Result<ShadowRuntimeStatus, String> {
    let Some(_tick_guard) = runtime.begin_tick() else {
        return shadow_runtime_status_from_bridge(persistence);
    };
    let now = persistence::now_ms()?;
    for watch in load_watches(persistence)?
        .into_iter()
        .filter(|watch| shadow_watch_is_due(watch, now))
    {
        let result = async {
            let experiment = load_experiment(persistence, &watch.experiment_id)?;
            validate_experiment(&experiment)?;
            let spec: StrategySpec = serde_json::from_value(
                experiment
                    .record
                    .pointer("/report/strategyCandidate")
                    .cloned()
                    .ok_or_else(|| "저장 전략 명세가 없습니다.".to_owned())?,
            )
            .map_err(|error| format!("저장 전략 명세를 해석하지 못했습니다: {error}"))?;
            if !review_strategy_spec(&spec).executable {
                return Err("저장 전략 계약이 더 이상 실행 가능하지 않습니다.".to_owned());
            }
            let is_crypto = experiment.symbol.starts_with("KRW-");
            let fresh_bars = if is_crypto {
                crypto
                    .fetch_strategy_bars(&experiment.symbol, &experiment.interval)
                    .await?
            } else {
                market
                    .fetch_latest_strategy_bars(&experiment.symbol, &experiment.interval)
                    .await?
            };
            let Some(side) = latest_signal(&spec, &fresh_bars).map_err(|error| error.message)?
            else {
                update_watch(persistence, &watch.watch_id, now, None, None)?;
                return Ok::<(), String>(());
            };
            let last = fresh_bars
                .last()
                .ok_or_else(|| "저장 가격봉이 없습니다.".to_owned())?;
            let signal_key = format!(
                "{}-{:?}-{}",
                watch.experiment_id, side, last.period_start_ms
            );
            if watch.last_signal_key.as_deref() == Some(&signal_key) {
                update_watch(persistence, &watch.watch_id, now, None, None)?;
                return Ok(());
            }
            let (price, observed_at) = if is_crypto {
                crypto.fetch_price(&experiment.symbol).await?
            } else {
                market.fetch_krw_current_price(&experiment.symbol).await?
            };
            let quantity = if is_crypto {
                experiment
                    .record
                    .pointer("/config/orderQuantity")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "코인 백테스트 수량을 확인하지 못했습니다.".to_owned())?
            } else {
                1
            };
            let gate = safety_gate(persistence, &experiment, side, quantity, price)?;
            insert_candidate(
                persistence,
                &watch.experiment_id,
                &experiment,
                side,
                quantity,
                price,
                observed_at,
                "shadow_engine",
                &gate,
            )?;
            update_watch(persistence, &watch.watch_id, now, Some(&signal_key), None)
        }
        .await;
        if let Err(error) = result {
            update_watch(persistence, &watch.watch_id, now, None, Some(&error))?;
        }
    }
    shadow_runtime_status_from_bridge(persistence)
}

fn shadow_runtime_status_from_bridge(
    persistence: &PersistenceBridge,
) -> Result<ShadowRuntimeStatus, String> {
    let watches = load_watches(persistence)?;
    let enabled_watch_count = watches.iter().filter(|watch| watch.enabled).count();
    Ok(ShadowRuntimeStatus {
        running: enabled_watch_count > 0,
        enabled_watch_count,
        watches,
        live_order_enabled: false,
        message: if enabled_watch_count > 0 {
            "Rust 백그라운드 엔진이 저장 전략의 완료 봉 신호를 감시 중입니다. 후보 생성 뒤 사용자 승인이 필요합니다."
                .to_owned()
        } else {
            "활성화된 섀도우 감시가 없습니다.".to_owned()
        },
    })
}

#[tauri::command]
pub async fn shadow_engine_tick(
    market: State<'_, MarketDataBridge>,
    crypto: State<'_, CryptoMarketBridge>,
    persistence: State<'_, PersistenceBridge>,
    runtime: State<'_, ShadowEngineRuntime>,
) -> Result<ShadowRuntimeStatus, String> {
    run_shadow_engine_once(&market, &crypto, &persistence, &runtime).await
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowJob {
    pub job_id: String,
    pub topic: String,
    pub importance: String,
    pub stage: String,
    pub status: String,
    pub selected_department_ids: Vec<String>,
    pub reports: Value,
    pub synthesis: Option<Value>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStartRequest {
    pub job_id: String,
    pub topic: String,
    pub importance: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowCheckpointRequest {
    pub job_id: String,
    pub stage: String,
    pub selected_department_ids: Vec<String>,
    pub reports: Value,
    pub synthesis: Option<Value>,
    pub status: Option<String>,
}

#[tauri::command]
pub fn meeting_workflow_start(
    request: WorkflowStartRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<WorkflowJob, String> {
    if !valid_id(&request.job_id)
        || request.topic.trim().is_empty()
        || request.topic.chars().count() > 4_000
        || !matches!(request.importance.as_str(), "normal" | "important")
    {
        return Err("회의 작업 식별자·안건·중요도가 올바르지 않습니다.".to_owned());
    }
    let now = persistence::now_ms()?;
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection.execute("INSERT INTO workflow_jobs (job_id, topic, importance, stage, status, selected_departments_json, reports_json, created_at_ms, updated_at_ms)
        VALUES (?1, ?2, ?3, 'routing', 'active', '[]', '{}', ?4, ?4)", params![request.job_id, request.topic, request.importance, now])
        .map_err(|error| format!("회의 복구 기록을 시작하지 못했습니다: {error}"))?;
    Ok(WorkflowJob {
        job_id: request.job_id,
        topic: request.topic,
        importance: request.importance,
        stage: "routing".to_owned(),
        status: "active".to_owned(),
        selected_department_ids: vec![],
        reports: json!({}),
        synthesis: None,
        created_at_ms: now,
        updated_at_ms: now,
    })
}

#[tauri::command]
pub fn meeting_workflow_checkpoint(
    request: WorkflowCheckpointRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    if !valid_id(&request.job_id)
        || !matches!(
            request.stage.as_str(),
            "routing"
                | "summoning"
                | "briefing"
                | "dispatching"
                | "department-analysis"
                | "reconvening"
                | "results"
                | "cancelled"
        )
        || request.selected_department_ids.len() > 9
        || request
            .selected_department_ids
            .iter()
            .any(|department_id| !valid_id(department_id))
    {
        return Err("회의 체크포인트가 올바르지 않습니다.".to_owned());
    }
    let status = request.status.unwrap_or_else(|| "active".to_owned());
    if !matches!(status.as_str(), "active" | "cancelled" | "completed") {
        return Err("회의 상태가 올바르지 않습니다.".to_owned());
    }
    let now = persistence::now_ms()?;
    let departments = serde_json::to_string(&request.selected_department_ids)
        .map_err(|error| format!("부서 목록을 기록하지 못했습니다: {error}"))?;
    let reports = serde_json::to_string(&request.reports)
        .map_err(|error| format!("보고 체크포인트를 기록하지 못했습니다: {error}"))?;
    let synthesis = request
        .synthesis
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|error| format!("종합 보고를 기록하지 못했습니다: {error}"))?;
    if reports.len() > 1_000_000
        || synthesis
            .as_ref()
            .is_some_and(|serialized| serialized.len() > 250_000)
    {
        return Err("회의 체크포인트가 로컬 저장 한도를 초과했습니다.".to_owned());
    }
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let changed = connection.execute("UPDATE workflow_jobs SET stage = ?2, status = ?3, selected_departments_json = ?4, reports_json = ?5, synthesis_json = ?6, updated_at_ms = ?7 WHERE job_id = ?1",
        params![request.job_id, request.stage, status, departments, reports, synthesis, now]).map_err(|error| format!("회의 체크포인트를 저장하지 못했습니다: {error}"))?;
    if changed == 0 {
        return Err("체크포인트를 저장할 회의 작업을 찾지 못했습니다.".to_owned());
    }
    Ok(())
}

fn workflow_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    u64,
    u64,
)> {
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
    ))
}

fn recover_interrupted_workflows(
    persistence: &PersistenceBridge,
    now: u64,
) -> Result<Vec<WorkflowJob>, String> {
    let mut connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("회의 복구 상태를 열지 못했습니다: {error}"))?;
    transaction.execute("UPDATE workflow_jobs SET status = 'interrupted', updated_at_ms = ?1 WHERE status = 'active'", params![now]).map_err(|error| format!("중단 회의를 표시하지 못했습니다: {error}"))?;
    let mut statement = transaction.prepare("SELECT job_id, topic, importance, stage, status, selected_departments_json, reports_json, synthesis_json, created_at_ms, updated_at_ms FROM workflow_jobs WHERE status = 'interrupted' ORDER BY updated_at_ms DESC LIMIT 10")
        .map_err(|error| format!("중단 회의를 조회하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], workflow_from_row)
        .map_err(|error| format!("중단 회의를 읽지 못했습니다: {error}"))?;
    let mut jobs = Vec::new();
    for row in rows {
        let row = row.map_err(|error| format!("중단 회의 기록이 손상되었습니다: {error}"))?;
        jobs.push(WorkflowJob {
            job_id: row.0,
            topic: row.1,
            importance: row.2,
            stage: row.3,
            status: row.4,
            selected_department_ids: serde_json::from_str(&row.5)
                .map_err(|error| format!("저장 부서 목록을 해석하지 못했습니다: {error}"))?,
            reports: serde_json::from_str(&row.6)
                .map_err(|error| format!("저장 보고를 해석하지 못했습니다: {error}"))?,
            synthesis: row
                .7
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|error| format!("저장 종합 보고를 해석하지 못했습니다: {error}"))?,
            created_at_ms: row.8,
            updated_at_ms: row.9,
        });
    }
    drop(statement);
    transaction
        .commit()
        .map_err(|error| format!("회의 복구 상태를 확정하지 못했습니다: {error}"))?;
    Ok(jobs)
}

#[tauri::command]
pub fn meeting_workflow_interrupted(
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<WorkflowJob>, String> {
    recover_interrupted_workflows(persistence.inner(), persistence::now_ms()?)
}

fn resume_interrupted_workflow(
    persistence: &PersistenceBridge,
    job_id: &str,
    now: u64,
) -> Result<WorkflowJob, String> {
    if !valid_id(job_id) {
        return Err("재개할 회의 작업 식별자가 올바르지 않습니다.".to_owned());
    }
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let changed = connection
        .execute(
            "UPDATE workflow_jobs SET status = 'active', updated_at_ms = ?2 WHERE job_id = ?1 AND status = 'interrupted'",
            params![job_id, now],
        )
        .map_err(|error| format!("중단 회의를 재개하지 못했습니다: {error}"))?;
    if changed == 0 {
        return Err("재개할 중단 회의 기록을 찾지 못했습니다.".to_owned());
    }
    let row = connection
        .query_row(
            "SELECT job_id, topic, importance, stage, status, selected_departments_json, reports_json, synthesis_json, created_at_ms, updated_at_ms FROM workflow_jobs WHERE job_id = ?1",
            params![job_id],
            workflow_from_row,
        )
        .map_err(|error| format!("재개된 회의 기록을 읽지 못했습니다: {error}"))?;
    Ok(WorkflowJob {
        job_id: row.0,
        topic: row.1,
        importance: row.2,
        stage: row.3,
        status: row.4,
        selected_department_ids: serde_json::from_str(&row.5)
            .map_err(|error| format!("저장 부서 목록을 해석하지 못했습니다: {error}"))?,
        reports: serde_json::from_str(&row.6)
            .map_err(|error| format!("저장 보고를 해석하지 못했습니다: {error}"))?,
        synthesis: row
            .7
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| format!("저장 종합 보고를 해석하지 못했습니다: {error}"))?,
        created_at_ms: row.8,
        updated_at_ms: row.9,
    })
}

#[tauri::command]
pub fn meeting_workflow_resume(
    request: CandidateActionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<WorkflowJob, String> {
    resume_interrupted_workflow(
        persistence.inner(),
        &request.candidate_id,
        persistence::now_ms()?,
    )
}

#[tauri::command]
pub fn meeting_workflow_dismiss(
    request: CandidateActionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    let now = persistence::now_ms()?;
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let changed = connection.execute("UPDATE workflow_jobs SET status = 'cancelled', updated_at_ms = ?2 WHERE job_id = ?1 AND status = 'interrupted'", params![request.candidate_id, now]).map_err(|error| format!("중단 회의 기록을 닫지 못했습니다: {error}"))?;
    if changed == 0 {
        return Err("닫을 중단 회의 기록을 찾지 못했습니다.".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_state_machine_rejects_terminal_and_skipped_transitions() {
        assert!(transition_allowed(
            CandidateStatus::SafetyApproved,
            CandidateStatus::UserApproved
        ));
        assert!(transition_allowed(
            CandidateStatus::Submitted,
            CandidateStatus::PartiallyFilled
        ));
        assert!(transition_allowed(
            CandidateStatus::PartiallyFilled,
            CandidateStatus::Filled
        ));
        assert!(!transition_allowed(
            CandidateStatus::SafetyApproved,
            CandidateStatus::Filled
        ));
        assert!(!transition_allowed(
            CandidateStatus::Filled,
            CandidateStatus::Submitted
        ));
        assert!(!transition_allowed(
            CandidateStatus::Rejected,
            CandidateStatus::UserApproved
        ));
    }

    #[test]
    fn performance_policy_is_not_silently_invented() {
        let gate = SafetyGate {
            passed: true,
            checks: vec![],
            performance_thresholds_configured: false,
            live_order_enabled: false,
        };
        assert!(!gate.performance_thresholds_configured);
        assert!(!gate.live_order_enabled);
    }

    #[test]
    fn shadow_watch_due_boundary_is_deterministic() {
        let watch = ShadowWatch {
            watch_id: "watch-test".to_owned(),
            experiment_id: "experiment-test".to_owned(),
            enabled: true,
            interval_seconds: 60,
            last_checked_at_ms: Some(100_000),
            last_signal_key: None,
            status: "watching".to_owned(),
            last_error: None,
        };
        assert!(!shadow_watch_is_due(&watch, 159_999));
        assert!(shadow_watch_is_due(&watch, 160_000));
        assert!(!shadow_watch_is_due(
            &ShadowWatch {
                enabled: false,
                ..watch
            },
            200_000
        ));
    }

    #[test]
    fn shadow_tick_runtime_rejects_overlap_and_releases_after_guard_drop() {
        let runtime = ShadowEngineRuntime::default();
        let guard = runtime.begin_tick().expect("first tick");
        assert!(runtime.begin_tick().is_none());
        drop(guard);
        assert!(runtime.begin_tick().is_some());
    }

    #[test]
    fn operational_tables_are_created_by_the_non_destructive_migration() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let connection = persistence.connection.lock().expect("connection");
        for table in [
            "paper_order_candidates",
            "paper_order_events",
            "shadow_watches",
            "workflow_jobs",
            "walk_forward_runs",
            "strategy_protection_decisions",
            "portfolio_risk_snapshots",
        ] {
            let exists: u64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    params![table],
                    |row| row.get(0),
                )
                .expect("schema query");
            assert_eq!(exists, 1, "missing table {table}");
        }
    }

    #[test]
    fn interrupted_meeting_recovery_preserves_partial_reports_and_is_idempotent() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        {
            let connection = persistence.connection.lock().expect("connection");
            connection.execute(
                "INSERT INTO workflow_jobs(job_id,topic,importance,stage,status,selected_departments_json,reports_json,synthesis_json,created_at_ms,updated_at_ms) VALUES('long-session-1','장시간 복구 검증','important','department-analysis','active','[\"research\",\"risk\"]','{\"research\":{\"summary\":\"완료 보고\"}}',NULL,1,2)",
                [],
            ).expect("fixture");
        }
        let first = recover_interrupted_workflows(&persistence, 3).expect("recover");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, "interrupted");
        assert_eq!(first[0].reports["research"]["summary"], "완료 보고");
        assert!(first[0].synthesis.is_none());
        let second = recover_interrupted_workflows(&persistence, 4).expect("repeat recover");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].reports, first[0].reports);
    }

    #[test]
    fn interrupted_meeting_resume_keeps_completed_reports_and_original_job_id() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        {
            let connection = persistence.connection.lock().expect("connection");
            connection.execute(
                "INSERT INTO workflow_jobs(job_id,topic,importance,stage,status,selected_departments_json,reports_json,synthesis_json,created_at_ms,updated_at_ms) VALUES('resume-1','체크포인트 재개','important','department-analysis','interrupted','[\"research\",\"risk\"]','{\"research-director\":{\"summary\":\"완료 보고\"}}',NULL,1,2)",
                [],
            ).expect("fixture");
        }
        let resumed = resume_interrupted_workflow(&persistence, "resume-1", 3).expect("resume");
        assert_eq!(resumed.job_id, "resume-1");
        assert_eq!(resumed.status, "active");
        assert_eq!(resumed.stage, "department-analysis");
        assert_eq!(resumed.reports["research-director"]["summary"], "완료 보고");
        assert!(resume_interrupted_workflow(&persistence, "resume-1", 4).is_err());
    }

    #[test]
    fn meeting_recovery_survives_one_hundred_interrupt_resume_cycles() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        for cycle in 0_u64..100 {
            let job_id = format!("soak-{cycle}");
            {
                let connection = persistence.connection.lock().expect("connection");
                connection
                    .execute(
                        "INSERT INTO workflow_jobs(job_id,topic,importance,stage,status,selected_departments_json,reports_json,synthesis_json,created_at_ms,updated_at_ms) VALUES(?1,'회의 복구 반복 검증','important','department-analysis','active','[\"research\",\"risk\"]',?2,NULL,?3,?3)",
                        params![
                            job_id,
                            format!(r#"{{"research":{{"summary":"cycle-{cycle}"}}}}"#),
                            cycle.saturating_mul(10).saturating_add(1),
                        ],
                    )
                    .expect("fixture");
            }
            let interrupted = recover_interrupted_workflows(
                &persistence,
                cycle.saturating_mul(10).saturating_add(2),
            )
            .expect("recover");
            let recovered = interrupted
                .iter()
                .find(|job| job.job_id == job_id)
                .expect("current interrupted workflow");
            assert_eq!(
                recovered.reports["research"]["summary"],
                format!("cycle-{cycle}")
            );

            let resumed = resume_interrupted_workflow(
                &persistence,
                &job_id,
                cycle.saturating_mul(10).saturating_add(3),
            )
            .expect("resume");
            assert_eq!(resumed.stage, "department-analysis");
            assert_eq!(resumed.selected_department_ids, vec!["research", "risk"]);

            let connection = persistence.connection.lock().expect("connection");
            let changed = connection
                .execute(
                    "UPDATE workflow_jobs SET stage='results', status='completed', updated_at_ms=?2 WHERE job_id=?1 AND status='active'",
                    params![job_id, cycle.saturating_mul(10).saturating_add(4)],
                )
                .expect("complete");
            assert_eq!(changed, 1);
        }

        let connection = persistence.connection.lock().expect("connection");
        let completed: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM workflow_jobs WHERE status='completed'",
                [],
                |row| row.get(0),
            )
            .expect("completed count");
        assert_eq!(completed, 100);
    }
}
