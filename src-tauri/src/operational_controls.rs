use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    paper_account::{replay_ledger, AppendOnlyLedger, LedgerEvent},
    paper_trading::ledger_id_for_currency,
    persistence::PersistenceBridge,
    strategy_protection::closed_trades_from_ledger,
    trading::TradeSide,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskMark {
    pub symbol: String,
    pub market: String,
    pub sector: String,
    pub mark_price_minor: u64,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperRiskLimits {
    pub policy_revision: String,
    pub day_start_ms: u64,
    pub maximum_trade_loss_minor: u64,
    pub maximum_daily_loss_minor: u64,
    pub maximum_drawdown_bps: u64,
    pub maximum_symbol_exposure_bps: u64,
    pub maximum_sector_exposure_bps: u64,
    pub maximum_market_exposure_bps: u64,
    pub maximum_consecutive_losses: usize,
    pub maximum_mark_age_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperRiskMonitorRequest {
    pub currency: String,
    pub as_of_ms: u64,
    pub marks: Vec<RiskMark>,
    pub limits: PaperRiskLimits,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExposureBucket {
    pub key: String,
    pub notional_minor: u64,
    pub exposure_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperRiskMonitorReport {
    pub currency: String,
    pub policy_revision: String,
    pub as_of_ms: u64,
    pub current_equity_minor: i64,
    pub realized_pnl_minor: i64,
    pub unrealized_pnl_minor: i64,
    pub largest_trade_loss_minor: u64,
    pub daily_realized_pnl_minor: i64,
    pub maximum_drawdown_bps: u64,
    pub consecutive_loss_count: usize,
    pub symbol_exposure: Vec<ExposureBucket>,
    pub sector_exposure: Vec<ExposureBucket>,
    pub market_exposure: Vec<ExposureBucket>,
    pub violations: Vec<String>,
    pub recommendation: String,
    pub new_entries_allowed: bool,
    pub live_order_allowed: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn validate_risk_request(request: &PaperRiskMonitorRequest) -> Result<(), String> {
    if !matches!(request.currency.as_str(), "KRW" | "USD")
        || request.as_of_ms == 0
        || request.limits.day_start_ms > request.as_of_ms
        || !valid_id(&request.limits.policy_revision)
        || request.limits.maximum_trade_loss_minor == 0
        || request.limits.maximum_daily_loss_minor == 0
        || !(1..=10_000).contains(&request.limits.maximum_drawdown_bps)
        || !(1..=10_000).contains(&request.limits.maximum_symbol_exposure_bps)
        || !(1..=10_000).contains(&request.limits.maximum_sector_exposure_bps)
        || !(1..=10_000).contains(&request.limits.maximum_market_exposure_bps)
        || request.limits.maximum_consecutive_losses == 0
        || request.limits.maximum_mark_age_ms == 0
        || request.marks.len() > 10_000
    {
        return Err("모의 위험 감시의 통화·시각·정책 리비전과 한도를 확인해 주세요.".to_owned());
    }
    let mut symbols = BTreeSet::new();
    for mark in &request.marks {
        if mark.symbol.trim().is_empty()
            || mark.symbol.len() > 32
            || mark.market.trim().is_empty()
            || mark.market.len() > 64
            || mark.sector.trim().is_empty()
            || mark.sector.len() > 64
            || mark.mark_price_minor == 0
            || mark.observed_at_ms == 0
            || mark.observed_at_ms > request.as_of_ms
            || !symbols.insert(mark.symbol.clone())
        {
            return Err("위험 감시 시세는 종목별로 하나이며 시장·섹터·과거 또는 현재 관측 시각이 필요합니다.".to_owned());
        }
    }
    Ok(())
}

fn exposure_buckets(
    values: BTreeMap<String, u64>,
    denominator: u64,
) -> Result<Vec<ExposureBucket>, String> {
    values
        .into_iter()
        .map(|(key, notional_minor)| {
            let exposure_bps = u64::try_from(
                u128::from(notional_minor).saturating_mul(10_000) / u128::from(denominator.max(1)),
            )
            .map_err(|_| "위험 노출 비중이 지원 범위를 초과했습니다.".to_owned())?;
            Ok(ExposureBucket {
                key,
                notional_minor,
                exposure_bps,
            })
        })
        .collect()
}

pub(crate) fn evaluate_paper_risk(
    events: &[LedgerEvent],
    request: PaperRiskMonitorRequest,
) -> Result<PaperRiskMonitorReport, String> {
    validate_risk_request(&request)?;
    let account = replay_ledger(events).map_err(|error| error.message)?;
    if account.currency != request.currency {
        return Err("위험 감시 통화와 모의원장 통화가 일치하지 않습니다.".to_owned());
    }
    let marks = request
        .marks
        .iter()
        .map(|mark| (mark.symbol.as_str(), mark))
        .collect::<BTreeMap<_, _>>();
    let mut symbol_values = BTreeMap::new();
    let mut sector_values = BTreeMap::new();
    let mut market_values = BTreeMap::new();
    let mut marked_value = 0_u64;
    let mut unrealized = 0_i128;
    let mut violations = Vec::new();
    for position in account.positions.values() {
        let mark = marks
            .get(position.symbol.as_str())
            .ok_or_else(|| format!("{}의 위험 감시 시세가 없습니다.", position.symbol))?;
        if request.as_of_ms.saturating_sub(mark.observed_at_ms) > request.limits.maximum_mark_age_ms
        {
            violations.push(format!("{} 시세 지연", position.symbol));
        }
        let value = u64::try_from(
            u128::from(mark.mark_price_minor) * u128::from(position.quantity)
                / u128::from(position.quantity_scale),
        )
        .map_err(|_| "평가금액이 지원 범위를 초과했습니다.".to_owned())?;
        marked_value = marked_value
            .checked_add(value)
            .ok_or_else(|| "평가금액 합계가 지원 범위를 초과했습니다.".to_owned())?;
        unrealized += i128::from(value) - i128::from(position.cost_basis_minor);
        symbol_values.insert(position.symbol.clone(), value);
        *sector_values.entry(mark.sector.clone()).or_insert(0_u64) = sector_values
            .get(&mark.sector)
            .copied()
            .unwrap_or_default()
            .checked_add(value)
            .ok_or_else(|| "섹터 노출 합계가 지원 범위를 초과했습니다.".to_owned())?;
        *market_values.entry(mark.market.clone()).or_insert(0_u64) = market_values
            .get(&mark.market)
            .copied()
            .unwrap_or_default()
            .checked_add(value)
            .ok_or_else(|| "시장 노출 합계가 지원 범위를 초과했습니다.".to_owned())?;
    }
    let current_equity = i128::from(account.cash_minor) + i128::from(marked_value);
    if current_equity <= 0 {
        violations.push("모의계좌 평가자산이 0 이하".to_owned());
    }
    let denominator = u64::try_from(current_equity.max(1))
        .map_err(|_| "평가자산이 지원 범위를 초과했습니다.".to_owned())?;
    let symbol_exposure = exposure_buckets(symbol_values, denominator)?;
    let sector_exposure = exposure_buckets(sector_values, denominator)?;
    let market_exposure = exposure_buckets(market_values, denominator)?;
    if symbol_exposure
        .iter()
        .any(|item| item.exposure_bps > request.limits.maximum_symbol_exposure_bps)
    {
        violations.push("종목 노출 한도 초과".to_owned());
    }
    if sector_exposure
        .iter()
        .any(|item| item.exposure_bps > request.limits.maximum_sector_exposure_bps)
    {
        violations.push("섹터 노출 한도 초과".to_owned());
    }
    if market_exposure
        .iter()
        .any(|item| item.exposure_bps > request.limits.maximum_market_exposure_bps)
    {
        violations.push("시장 노출 한도 초과".to_owned());
    }

    let (initial_equity, closed) = closed_trades_from_ledger(events)?;
    let largest_trade_loss = closed
        .iter()
        .filter_map(|trade| trade.net_pnl_minor.checked_neg())
        .max()
        .unwrap_or_default() as u64;
    let daily_realized = closed
        .iter()
        .filter(|trade| trade.closed_at_ms >= request.limits.day_start_ms)
        .try_fold(0_i64, |sum, trade| sum.checked_add(trade.net_pnl_minor))
        .ok_or_else(|| "당일 손익 합계가 지원 범위를 초과했습니다.".to_owned())?;
    let mut cumulative = i128::from(initial_equity);
    let mut peak = cumulative;
    let mut maximum_drawdown_bps = 0_u64;
    let mut consecutive_losses = 0_usize;
    for trade in &closed {
        cumulative += i128::from(trade.net_pnl_minor);
        peak = peak.max(cumulative);
        if peak > 0 && cumulative < peak {
            let drawdown = u64::try_from((peak - cumulative) * 10_000 / peak)
                .map_err(|_| "최대 낙폭이 지원 범위를 초과했습니다.".to_owned())?;
            maximum_drawdown_bps = maximum_drawdown_bps.max(drawdown);
        }
        consecutive_losses = if trade.net_pnl_minor < 0 {
            consecutive_losses + 1
        } else {
            0
        };
    }
    if largest_trade_loss > request.limits.maximum_trade_loss_minor {
        violations.push("거래당 손실 한도 초과".to_owned());
    }
    if daily_realized < 0 && daily_realized.unsigned_abs() > request.limits.maximum_daily_loss_minor
    {
        violations.push("일일 손실 한도 초과".to_owned());
    }
    if maximum_drawdown_bps > request.limits.maximum_drawdown_bps {
        violations.push("최대 낙폭 한도 초과".to_owned());
    }
    if consecutive_losses >= request.limits.maximum_consecutive_losses {
        violations.push("연속 손실 한도 도달".to_owned());
    }
    let severe = violations.iter().any(|violation| {
        violation.contains("손실")
            || violation.contains("낙폭")
            || violation.contains("지연")
            || violation.contains("0 이하")
    });
    let recommendation = if severe {
        "stop"
    } else if violations.is_empty() {
        "continue"
    } else {
        "reduce"
    };
    Ok(PaperRiskMonitorReport {
        currency: request.currency,
        policy_revision: request.limits.policy_revision,
        as_of_ms: request.as_of_ms,
        current_equity_minor: i64::try_from(current_equity)
            .map_err(|_| "현재 평가자산이 지원 범위를 초과했습니다.".to_owned())?,
        realized_pnl_minor: account.realized_pnl_minor,
        unrealized_pnl_minor: i64::try_from(unrealized)
            .map_err(|_| "미실현손익이 지원 범위를 초과했습니다.".to_owned())?,
        largest_trade_loss_minor: largest_trade_loss,
        daily_realized_pnl_minor: daily_realized,
        maximum_drawdown_bps,
        consecutive_loss_count: consecutive_losses,
        symbol_exposure,
        sector_exposure,
        market_exposure,
        recommendation: recommendation.to_owned(),
        new_entries_allowed: violations.is_empty(),
        violations,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn paper_risk_monitor_evaluate(
    request: PaperRiskMonitorRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<PaperRiskMonitorReport, String> {
    let ledger = bridge.paper_ledger(ledger_id_for_currency(&request.currency)?)?;
    evaluate_paper_risk(ledger.events(), request)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionObservation {
    pub event_id: String,
    pub order_id: String,
    pub idempotency_key: String,
    pub symbol: String,
    pub market: String,
    pub side: TradeSide,
    pub reference_price_minor: u64,
    pub average_fill_price_minor: u64,
    pub requested_quantity: u64,
    pub filled_quantity: u64,
    pub quantity_scale: u64,
    pub fee_minor: u64,
    pub tax_minor: u64,
    pub fx_cost_minor: u64,
    pub funding_minor: i64,
    pub submitted_at_ms: u64,
    pub first_fill_at_ms: u64,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeQualityRequest {
    pub observations: Vec<ExecutionObservation>,
    pub maximum_execution_loss_bps: u64,
    pub maximum_latency_ms: u64,
    pub maximum_order_quantity: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionQualityItem {
    pub event_id: String,
    pub execution_loss_minor: i64,
    pub execution_loss_bps: i64,
    pub explicit_cost_minor: i64,
    pub latency_ms: u64,
    pub partial_fill: bool,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeQualityReport {
    pub items: Vec<ExecutionQualityItem>,
    pub duplicate_order_count: usize,
    pub duplicate_idempotency_count: usize,
    pub abnormal_order_count: usize,
    pub total_execution_loss_minor: i64,
    pub pause_new_orders_candidate: bool,
    pub pause_reasons: Vec<String>,
    pub ledger_mutated: bool,
    pub live_order_allowed: bool,
}

pub fn analyze_trade_quality(request: TradeQualityRequest) -> Result<TradeQualityReport, String> {
    if request.observations.is_empty()
        || request.observations.len() > 100_000
        || request.maximum_execution_loss_bps == 0
        || request.maximum_latency_ms == 0
        || request.maximum_order_quantity == 0
    {
        return Err("거래품질 관측과 비용·지연·수량 임계치를 확인해 주세요.".to_owned());
    }
    let mut order_ids = BTreeSet::new();
    let mut idempotency_keys = BTreeSet::new();
    let mut duplicate_orders = 0;
    let mut duplicate_keys = 0;
    let mut abnormal_orders = 0;
    let mut total_loss = 0_i64;
    let mut items = Vec::with_capacity(request.observations.len());
    for item in request.observations {
        if !valid_id(&item.event_id)
            || !valid_id(&item.order_id)
            || !valid_id(&item.idempotency_key)
            || item.symbol.trim().is_empty()
            || item.market.trim().is_empty()
            || item.reference_price_minor == 0
            || item.average_fill_price_minor == 0
            || item.requested_quantity == 0
            || item.filled_quantity > item.requested_quantity
            || item.quantity_scale == 0
            || item.submitted_at_ms == 0
            || item.first_fill_at_ms < item.submitted_at_ms
            || item.completed_at_ms < item.first_fill_at_ms
        {
            return Err("거래품질 관측의 식별자·가격·수량·시각 순서를 확인해 주세요.".to_owned());
        }
        let mut flags = Vec::new();
        if !order_ids.insert(item.order_id.clone()) {
            duplicate_orders += 1;
            flags.push("duplicate_order_id".to_owned());
        }
        if !idempotency_keys.insert(item.idempotency_key.clone()) {
            duplicate_keys += 1;
            flags.push("duplicate_idempotency_key".to_owned());
        }
        if item.requested_quantity > request.maximum_order_quantity {
            abnormal_orders += 1;
            flags.push("excess_quantity".to_owned());
        }
        let adverse_price = match item.side {
            TradeSide::Buy => {
                i128::from(item.average_fill_price_minor) - i128::from(item.reference_price_minor)
            }
            TradeSide::Sell => {
                i128::from(item.reference_price_minor) - i128::from(item.average_fill_price_minor)
            }
        };
        let price_loss =
            adverse_price * i128::from(item.filled_quantity) / i128::from(item.quantity_scale);
        let explicit = i128::from(item.fee_minor)
            + i128::from(item.tax_minor)
            + i128::from(item.fx_cost_minor)
            + i128::from(item.funding_minor);
        let execution_loss = price_loss + explicit;
        let reference_notional = i128::from(item.reference_price_minor)
            * i128::from(item.filled_quantity)
            / i128::from(item.quantity_scale);
        let execution_loss_bps = if reference_notional == 0 {
            0
        } else {
            i64::try_from(execution_loss * 10_000 / reference_notional)
                .map_err(|_| "실행 손실 비율이 지원 범위를 초과했습니다.".to_owned())?
        };
        let latency = item.completed_at_ms - item.submitted_at_ms;
        if execution_loss_bps > request.maximum_execution_loss_bps as i64 {
            flags.push("execution_quality_degraded".to_owned());
        }
        if latency > request.maximum_latency_ms {
            flags.push("latency_threshold_exceeded".to_owned());
        }
        if item.filled_quantity < item.requested_quantity {
            flags.push("partial_fill".to_owned());
        }
        let execution_loss_minor = i64::try_from(execution_loss)
            .map_err(|_| "실행 손실이 지원 범위를 초과했습니다.".to_owned())?;
        total_loss = total_loss
            .checked_add(execution_loss_minor)
            .ok_or_else(|| "실행 손실 합계가 지원 범위를 초과했습니다.".to_owned())?;
        items.push(ExecutionQualityItem {
            event_id: item.event_id,
            execution_loss_minor,
            execution_loss_bps,
            explicit_cost_minor: i64::try_from(explicit)
                .map_err(|_| "명시적 비용이 지원 범위를 초과했습니다.".to_owned())?,
            latency_ms: latency,
            partial_fill: item.filled_quantity < item.requested_quantity,
            flags,
        });
    }
    let mut reasons = Vec::new();
    if duplicate_orders > 0 || duplicate_keys > 0 {
        reasons.push("중복 주문 식별자 감지".to_owned());
    }
    if abnormal_orders > 0 {
        reasons.push("주문 수량 임계치 초과".to_owned());
    }
    if items.iter().any(|item| {
        item.flags.iter().any(|flag| {
            matches!(
                flag.as_str(),
                "execution_quality_degraded" | "latency_threshold_exceeded"
            )
        })
    }) {
        reasons.push("체결 품질 임계치 초과".to_owned());
    }
    Ok(TradeQualityReport {
        items,
        duplicate_order_count: duplicate_orders,
        duplicate_idempotency_count: duplicate_keys,
        abnormal_order_count: abnormal_orders,
        total_execution_loss_minor: total_loss,
        pause_new_orders_candidate: !reasons.is_empty(),
        pause_reasons: reasons,
        ledger_mutated: false,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn trade_quality_analyze(request: TradeQualityRequest) -> Result<TradeQualityReport, String> {
    analyze_trade_quality(request)
}

const CRYPTO_RISK_CONFIRMATION: &str = "레버리지 위험을 확인했습니다";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CryptoRiskPolicyChange {
    pub change_id: String,
    pub policy_revision: String,
    pub leverage_enabled: bool,
    pub maximum_leverage_bps: u64,
    pub reason: String,
    pub confirmation_recorded: bool,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCryptoRiskPolicyRequest {
    pub change_id: String,
    pub policy_revision: String,
    pub leverage_enabled: bool,
    pub maximum_leverage_bps: u64,
    pub reason: String,
    pub confirmation: String,
    pub created_at_ms: u64,
}

fn validate_crypto_policy(request: &SaveCryptoRiskPolicyRequest) -> Result<(), String> {
    if !valid_id(&request.change_id)
        || !valid_id(&request.policy_revision)
        || !(10_000..=20_000).contains(&request.maximum_leverage_bps)
        || request.reason.trim().len() < 3
        || request.reason.len() > 500
        || request.confirmation != CRYPTO_RISK_CONFIRMATION
        || request.created_at_ms == 0
    {
        return Err("코인 위험정책은 고유 리비전·1~2배 한도·변경 사유와 정확한 위험 확인 문구가 필요합니다.".to_owned());
    }
    if !request.leverage_enabled && request.maximum_leverage_bps != 10_000 {
        return Err("레버리지를 끈 정책의 최대 배율은 1배여야 합니다.".to_owned());
    }
    Ok(())
}

fn crypto_policy_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CryptoRiskPolicyChange> {
    Ok(CryptoRiskPolicyChange {
        change_id: row.get(0)?,
        policy_revision: row.get(1)?,
        leverage_enabled: row.get(2)?,
        maximum_leverage_bps: row.get(3)?,
        reason: row.get(4)?,
        confirmation_recorded: row.get(5)?,
        created_at_ms: row.get(6)?,
    })
}

#[tauri::command]
pub fn crypto_risk_policy_change_save(
    request: SaveCryptoRiskPolicyRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<CryptoRiskPolicyChange, String> {
    validate_crypto_policy(&request)?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "코인 위험정책 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing = connection.query_row(
        "SELECT change_id,policy_revision,leverage_enabled,maximum_leverage_bps,reason,confirmation_recorded,created_at_ms FROM crypto_risk_policy_changes WHERE change_id=?1",
        params![request.change_id], crypto_policy_row,
    ).optional().map_err(|error| format!("기존 코인 위험정책을 확인하지 못했습니다: {error}"))?;
    if let Some(existing) = existing {
        if existing.policy_revision == request.policy_revision
            && existing.leverage_enabled == request.leverage_enabled
            && existing.maximum_leverage_bps == request.maximum_leverage_bps
            && existing.reason == request.reason.trim()
            && existing.created_at_ms == request.created_at_ms
        {
            return Ok(existing);
        }
        return Err("같은 변경 식별자에 다른 코인 위험정책이 이미 저장되어 있습니다.".to_owned());
    }
    connection.execute(
        "INSERT INTO crypto_risk_policy_changes(change_id,policy_revision,leverage_enabled,maximum_leverage_bps,reason,confirmation_recorded,created_at_ms) VALUES(?1,?2,?3,?4,?5,1,?6)",
        params![request.change_id, request.policy_revision, request.leverage_enabled, request.maximum_leverage_bps, request.reason.trim(), request.created_at_ms],
    ).map_err(|error| format!("코인 위험정책을 저장하지 못했습니다: {error}"))?;
    connection.query_row(
        "SELECT change_id,policy_revision,leverage_enabled,maximum_leverage_bps,reason,confirmation_recorded,created_at_ms FROM crypto_risk_policy_changes WHERE change_id=?1",
        params![request.change_id], crypto_policy_row,
    ).map_err(|error| format!("저장된 코인 위험정책을 읽지 못했습니다: {error}"))
}

#[tauri::command]
pub fn crypto_risk_policy_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<CryptoRiskPolicyChange>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "코인 위험정책 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare(
        "SELECT change_id,policy_revision,leverage_enabled,maximum_leverage_bps,reason,confirmation_recorded,created_at_ms FROM crypto_risk_policy_changes ORDER BY created_at_ms DESC,change_id DESC LIMIT ?1"
    ).map_err(|error| format!("코인 위험정책 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 200)], crypto_policy_row)
        .map_err(|error| format!("코인 위험정책 이력을 조회하지 못했습니다: {error}"))?
        .map(|row| row.map_err(|error| format!("코인 위험정책 이력을 읽지 못했습니다: {error}")))
        .collect();
    rows
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationsDrillScenario {
    OrderRejected,
    PartialFill,
    StaleMarketData,
    BrokerOutage,
    LossLimit,
    ReconciliationMismatch,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationsDrillRequest {
    pub drill_id: String,
    pub scenario: OperationsDrillScenario,
    pub observation: String,
    pub executed_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationsDrillResult {
    pub drill_id: String,
    pub scenario: OperationsDrillScenario,
    pub severity: String,
    pub observation: String,
    pub recommended_actions: Vec<String>,
    pub kill_switch_required: bool,
    pub new_entries_allowed: bool,
    pub cancellation_allowed: bool,
    pub executed_at_ms: u64,
    pub live_order_allowed: bool,
}

fn drill_scenario_db(scenario: OperationsDrillScenario) -> &'static str {
    match scenario {
        OperationsDrillScenario::OrderRejected => "order_rejected",
        OperationsDrillScenario::PartialFill => "partial_fill",
        OperationsDrillScenario::StaleMarketData => "stale_market_data",
        OperationsDrillScenario::BrokerOutage => "broker_outage",
        OperationsDrillScenario::LossLimit => "loss_limit",
        OperationsDrillScenario::ReconciliationMismatch => "reconciliation_mismatch",
    }
}

pub fn evaluate_operations_drill(
    request: OperationsDrillRequest,
) -> Result<OperationsDrillResult, String> {
    if !valid_id(&request.drill_id)
        || request.observation.trim().len() < 3
        || request.observation.len() > 1_000
        || request.executed_at_ms == 0
    {
        return Err("운영 훈련의 ID·관측 내용·실행 시각을 확인해 주세요.".to_owned());
    }
    let (severity, kill_switch_required, new_entries_allowed, actions) = match request.scenario {
        OperationsDrillScenario::OrderRejected => (
            "warning",
            false,
            true,
            vec![
                "거부 사유 확인",
                "동일 주문 자동 재전송 금지",
                "원장 상태 확인",
            ],
        ),
        OperationsDrillScenario::PartialFill => (
            "warning",
            false,
            false,
            vec![
                "잔량 자동 재전송 금지",
                "부분체결 원장 대사",
                "취소 가능 여부 확인",
            ],
        ),
        OperationsDrillScenario::StaleMarketData => (
            "critical",
            true,
            false,
            vec![
                "신규 진입 중단",
                "시세 공급자 복구 확인",
                "최신 관측 시각 재검증",
            ],
        ),
        OperationsDrillScenario::BrokerOutage => (
            "critical",
            true,
            false,
            vec![
                "신규 진입 중단",
                "미체결 주문 조회",
                "브로커 복구 후 전체 대사",
            ],
        ),
        OperationsDrillScenario::LossLimit => (
            "critical",
            true,
            false,
            vec![
                "신규 진입 중단",
                "손실 한도 원인 확인",
                "사용자 승인 전 재개 금지",
            ],
        ),
        OperationsDrillScenario::ReconciliationMismatch => (
            "critical",
            true,
            false,
            vec![
                "신규 진입 중단",
                "내부·외부 원장 비교",
                "불일치 해소 전 재전송 금지",
            ],
        ),
    };
    Ok(OperationsDrillResult {
        drill_id: request.drill_id,
        scenario: request.scenario,
        severity: severity.to_owned(),
        observation: request.observation.trim().to_owned(),
        recommended_actions: actions.into_iter().map(str::to_owned).collect(),
        kill_switch_required,
        new_entries_allowed,
        cancellation_allowed: true,
        executed_at_ms: request.executed_at_ms,
        live_order_allowed: false,
    })
}

fn execute_operations_drill(
    request: OperationsDrillRequest,
    bridge: &PersistenceBridge,
) -> Result<OperationsDrillResult, String> {
    let result = evaluate_operations_drill(request)?;
    let json = serde_json::to_string(&result)
        .map_err(|_| "운영 훈련 결과를 직렬화하지 못했습니다.".to_owned())?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "운영 훈련 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("운영 훈련 저장을 시작하지 못했습니다: {error}"))?;
    let existing: Option<String> = transaction
        .query_row(
            "SELECT result_json FROM operational_drills WHERE drill_id=?1",
            params![result.drill_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("기존 운영 훈련을 확인하지 못했습니다: {error}"))?;
    if let Some(existing) = existing {
        if existing == json {
            return Ok(result);
        }
        return Err("같은 훈련 ID에 다른 결과가 이미 저장되어 있습니다.".to_owned());
    }
    transaction.execute(
        "INSERT INTO operational_drills(drill_id,scenario,result_json,executed_at_ms) VALUES(?1,?2,?3,?4)",
        params![result.drill_id,drill_scenario_db(result.scenario),json,result.executed_at_ms],
    ).map_err(|error| format!("운영 훈련 결과를 저장하지 못했습니다: {error}"))?;
    let alert_id = format!("drill-{}", result.drill_id);
    transaction.execute(
        "INSERT OR IGNORE INTO operational_alerts(alert_id,deduplication_key,severity,message,first_seen_at_ms,last_seen_at_ms,occurrence_count,acknowledged_at_ms,response) VALUES(?1,?2,?3,?4,?5,?5,1,NULL,NULL)",
        params![alert_id,format!("operations-drill:{}",result.drill_id),result.severity,format!("운영 훈련 · {}",result.observation),result.executed_at_ms],
    ).map_err(|error| format!("운영 훈련 알림을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("운영 훈련 결과를 확정하지 못했습니다: {error}"))?;
    Ok(result)
}

#[tauri::command]
pub fn operations_drill_execute(
    request: OperationsDrillRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<OperationsDrillResult, String> {
    execute_operations_drill(request, bridge.inner())
}

#[tauri::command]
pub fn operations_drill_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<OperationsDrillResult>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "운영 훈련 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare("SELECT result_json FROM operational_drills ORDER BY executed_at_ms DESC,drill_id DESC LIMIT ?1")
        .map_err(|error| format!("운영 훈련 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 200)], |row| row.get::<_, String>(0))
        .map_err(|error| format!("운영 훈련 이력을 조회하지 못했습니다: {error}"))?
        .map(|row| {
            let json = row.map_err(|error| format!("운영 훈련 이력을 읽지 못했습니다: {error}"))?;
            serde_json::from_str(&json)
                .map_err(|_| "저장된 운영 훈련을 해석하지 못했습니다.".to_owned())
        })
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        paper_account::{execute_shadow_order, open_paper_account, InMemoryLedger},
        simulation::TradingCosts,
    };

    #[test]
    fn crypto_leverage_policy_requires_exact_confirmation_and_is_idempotent() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = SaveCryptoRiskPolicyRequest {
            change_id: "crypto-risk-1".into(),
            policy_revision: "crypto-v1".into(),
            leverage_enabled: true,
            maximum_leverage_bps: 15_000,
            reason: "격리된 모의환경 검증".into(),
            confirmation: CRYPTO_RISK_CONFIRMATION.into(),
            created_at_ms: 1,
        };
        validate_crypto_policy(&request).expect("valid policy");
        let connection = bridge.connection.lock().expect("connection");
        connection.execute(
            "INSERT INTO crypto_risk_policy_changes(change_id,policy_revision,leverage_enabled,maximum_leverage_bps,reason,confirmation_recorded,created_at_ms) VALUES(?1,?2,?3,?4,?5,1,?6)",
            params![request.change_id, request.policy_revision, request.leverage_enabled, request.maximum_leverage_bps, request.reason, request.created_at_ms],
        ).expect("insert");
        let count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM crypto_risk_policy_changes",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(count, 1);
        assert!(!connection
            .query_row(
                "SELECT confirmation_recorded FROM crypto_risk_policy_changes",
                [],
                |row| row.get::<_, bool>(0)
            )
            .expect("confirmation")
            .eq(&false));
    }

    #[test]
    fn operational_drills_fail_closed_but_keep_cancellation_available() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        for scenario in [
            OperationsDrillScenario::PartialFill,
            OperationsDrillScenario::StaleMarketData,
            OperationsDrillScenario::BrokerOutage,
            OperationsDrillScenario::LossLimit,
            OperationsDrillScenario::ReconciliationMismatch,
        ] {
            let result = execute_operations_drill(
                OperationsDrillRequest {
                    drill_id: format!("drill-{:?}", scenario).to_ascii_lowercase(),
                    scenario,
                    observation: "격리된 운영 장애 훈련".into(),
                    executed_at_ms: 1,
                },
                &bridge,
            )
            .expect("drill");
            assert!(!result.new_entries_allowed);
            assert!(result.cancellation_allowed);
            assert!(!result.live_order_allowed);
        }
        let connection = bridge.connection.lock().expect("connection");
        let drill_count: u64 = connection
            .query_row("SELECT COUNT(*) FROM operational_drills", [], |row| {
                row.get(0)
            })
            .expect("drill count");
        let alert_count: u64 = connection
            .query_row("SELECT COUNT(*) FROM operational_alerts", [], |row| {
                row.get(0)
            })
            .expect("alert count");
        assert_eq!(drill_count, 5);
        assert_eq!(alert_count, 5);
    }

    #[test]
    fn risk_monitor_uses_ledger_losses_marks_and_exposure_without_live_permission() {
        let mut ledger = InMemoryLedger::new();
        open_paper_account(&mut ledger, "paper".into(), "KRW".into(), 1_000_000, 1).unwrap();
        execute_shadow_order(
            &mut ledger,
            crate::paper_account::ShadowOrderRequest {
                account_id: "paper".into(),
                order_id: "buy-1".into(),
                idempotency_key: "key-1".into(),
                symbol: "TEST".into(),
                currency: "KRW".into(),
                side: TradeSide::Buy,
                quantity: 10,
                quantity_scale: 1,
                reference_price_minor: 10_000,
                occurred_at_ms: 2,
            },
            TradingCosts {
                buy_fee_bps: 0.0,
                sell_fee_bps: 0.0,
                sell_tax_bps: 0.0,
                slippage_bps: 0.0,
            },
        )
        .unwrap();
        let report = evaluate_paper_risk(
            ledger.events(),
            PaperRiskMonitorRequest {
                currency: "KRW".into(),
                as_of_ms: 10,
                marks: vec![RiskMark {
                    symbol: "TEST".into(),
                    market: "kr".into(),
                    sector: "tech".into(),
                    mark_price_minor: 9_000,
                    observed_at_ms: 9,
                }],
                limits: PaperRiskLimits {
                    policy_revision: "risk-v1".into(),
                    day_start_ms: 1,
                    maximum_trade_loss_minor: 100_000,
                    maximum_daily_loss_minor: 100_000,
                    maximum_drawdown_bps: 5_000,
                    maximum_symbol_exposure_bps: 500,
                    maximum_sector_exposure_bps: 500,
                    maximum_market_exposure_bps: 500,
                    maximum_consecutive_losses: 3,
                    maximum_mark_age_ms: 5,
                },
            },
        )
        .unwrap();
        assert_eq!(report.unrealized_pnl_minor, -10_000);
        assert!(!report.new_entries_allowed);
        assert!(!report.live_order_allowed);
        assert!(report.violations.iter().any(|item| item.contains("노출")));
    }

    #[test]
    fn trade_quality_separates_price_and_explicit_costs_and_flags_duplicates() {
        let observation = ExecutionObservation {
            event_id: "event-1".into(),
            order_id: "order-1".into(),
            idempotency_key: "key-1".into(),
            symbol: "TEST".into(),
            market: "kr".into(),
            side: TradeSide::Buy,
            reference_price_minor: 1000,
            average_fill_price_minor: 1010,
            requested_quantity: 10,
            filled_quantity: 5,
            quantity_scale: 1,
            fee_minor: 3,
            tax_minor: 0,
            fx_cost_minor: 0,
            funding_minor: 0,
            submitted_at_ms: 1,
            first_fill_at_ms: 2,
            completed_at_ms: 100,
        };
        let mut duplicate = observation.clone();
        duplicate.event_id = "event-2".into();
        let report = analyze_trade_quality(TradeQualityRequest {
            observations: vec![observation, duplicate],
            maximum_execution_loss_bps: 10,
            maximum_latency_ms: 50,
            maximum_order_quantity: 100,
        })
        .unwrap();
        assert_eq!(report.duplicate_order_count, 1);
        assert_eq!(report.duplicate_idempotency_count, 1);
        assert!(report.pause_new_orders_candidate);
        assert!(!report.ledger_mutated);
    }
}
