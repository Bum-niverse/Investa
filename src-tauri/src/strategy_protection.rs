use std::collections::BTreeMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    paper_account::{AppendOnlyLedger, LedgerEvent},
    persistence::{self, PersistenceBridge},
    risk_policy::RiskPolicy,
    trading::TradeSide,
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TradeExitKind {
    Signal,
    StopLoss,
    TakeProfit,
    Manual,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedTradeObservation {
    pub trade_id: String,
    pub symbol: String,
    pub closed_at_ms: u64,
    pub net_pnl_minor: i64,
    pub exit_kind: TradeExitKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyProtectionPolicy {
    pub policy_id: String,
    pub lookback_ms: u64,
    pub lock_duration_ms: u64,
    pub cooldown_ms: u64,
    pub maximum_stop_loss_count: usize,
    pub maximum_consecutive_loss_count: usize,
    pub maximum_drawdown_bps: u64,
    pub minimum_symbol_trade_count: usize,
    pub minimum_symbol_net_pnl_minor: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyProtectionRequest {
    pub target_symbol: String,
    pub now_ms: u64,
    pub initial_equity_minor: u64,
    pub policy: StrategyProtectionPolicy,
    pub closed_trades: Vec<ClosedTradeObservation>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionScope {
    Global,
    Symbol,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionTrigger {
    pub code: String,
    pub scope: ProtectionScope,
    pub locked_until_ms: u64,
    pub observed: String,
    pub threshold: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyProtectionDecision {
    pub policy_id: String,
    pub target_symbol: String,
    pub evaluated_at_ms: u64,
    pub can_open_new_position: bool,
    pub global_lock_until_ms: Option<u64>,
    pub symbol_lock_until_ms: Option<u64>,
    pub triggers: Vec<ProtectionTrigger>,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtectionAction {
    Open,
    Reduce,
    Cancel,
}

pub fn protection_action_allowed(
    decision: &StrategyProtectionDecision,
    action: ProtectionAction,
) -> bool {
    action != ProtectionAction::Open || decision.can_open_new_position
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtectionAlertSyncReceipt {
    pub decisions_scanned: usize,
    pub alerts_created: usize,
    pub duplicate_alerts_skipped: usize,
    pub live_order_allowed: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(crate) fn validate_strategy_protection_policy(
    policy: &StrategyProtectionPolicy,
) -> Result<(), String> {
    if !valid_id(&policy.policy_id)
        || policy.lookback_ms == 0
        || policy.lock_duration_ms == 0
        || policy.maximum_stop_loss_count == 0
        || policy.maximum_consecutive_loss_count == 0
        || !(1..=10_000).contains(&policy.maximum_drawdown_bps)
        || policy.minimum_symbol_trade_count == 0
    {
        return Err("전략 보호 정책의 식별자·기간·표본·손실 한도를 확인해 주세요.".to_owned());
    }
    Ok(())
}

fn validate(request: &StrategyProtectionRequest) -> Result<(), String> {
    let policy = &request.policy;
    validate_strategy_protection_policy(policy)?;
    if request.target_symbol.trim().is_empty()
        || request.target_symbol.len() > 32
        || request.now_ms == 0
        || request.initial_equity_minor == 0
        || request.closed_trades.len() > 100_000
    {
        return Err("전략 보호 정책의 식별자·기간·표본·손실 한도를 확인해 주세요.".to_owned());
    }
    let mut previous_closed_at = 0;
    for trade in &request.closed_trades {
        if !valid_id(&trade.trade_id)
            || trade.symbol.trim().is_empty()
            || trade.symbol.len() > 32
            || trade.closed_at_ms == 0
            || trade.closed_at_ms > request.now_ms
            || trade.closed_at_ms < previous_closed_at
        {
            return Err(
                "종료 거래는 식별자와 시각 순서가 유효해야 하며 미래 거래를 포함할 수 없습니다."
                    .to_owned(),
            );
        }
        previous_closed_at = trade.closed_at_ms;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProtectionDecision {
    pub decision_id: i64,
    pub decision: StrategyProtectionDecision,
    pub created_at_ms: u64,
}

fn checked_pnl(credit: u64, cost: u64) -> Result<i64, String> {
    i64::try_from(i128::from(credit) - i128::from(cost))
        .map_err(|_| "모의원장 종료 거래 손익이 지원 범위를 초과했습니다.".to_owned())
}

pub(crate) fn closed_trades_from_ledger(
    events: &[LedgerEvent],
) -> Result<(u64, Vec<ClosedTradeObservation>), String> {
    let initial_equity_minor = match events.first() {
        Some(LedgerEvent::AccountOpened {
            initial_cash_minor, ..
        }) => *initial_cash_minor,
        _ => return Err("전략 보호 검사를 위한 모의계좌 개설 사건이 없습니다.".to_owned()),
    };
    let mut positions = BTreeMap::<String, (u64, u64)>::new();
    let mut closed = Vec::new();
    for event in &events[1..] {
        let LedgerEvent::OrderFilled {
            order_id,
            symbol,
            side,
            quantity,
            notional_minor,
            fee_minor,
            tax_minor,
            exit_reason,
            occurred_at_ms,
            ..
        } = event
        else {
            continue;
        };
        match side {
            TradeSide::Buy => {
                let debit = notional_minor
                    .checked_add(*fee_minor)
                    .and_then(|value| value.checked_add(*tax_minor))
                    .ok_or_else(|| "모의원장 매수 원가가 지원 범위를 초과했습니다.".to_owned())?;
                let position = positions.entry(symbol.clone()).or_default();
                position.0 = position
                    .0
                    .checked_add(*quantity)
                    .ok_or_else(|| "모의원장 보유 수량이 지원 범위를 초과했습니다.".to_owned())?;
                position.1 = position
                    .1
                    .checked_add(debit)
                    .ok_or_else(|| "모의원장 매수 원가가 지원 범위를 초과했습니다.".to_owned())?;
            }
            TradeSide::Sell => {
                let position = positions
                    .get(symbol)
                    .copied()
                    .ok_or_else(|| "모의원장 종료 거래에 대응하는 포지션이 없습니다.".to_owned())?;
                if position.0 < *quantity || position.0 == 0 {
                    return Err("모의원장 종료 거래 수량이 보유 수량을 초과했습니다.".to_owned());
                }
                let released_cost = u64::try_from(
                    u128::from(position.1) * u128::from(*quantity) / u128::from(position.0),
                )
                .map_err(|_| "모의원장 배분 원가가 지원 범위를 초과했습니다.".to_owned())?;
                let credit = notional_minor
                    .checked_sub(*fee_minor)
                    .and_then(|value| value.checked_sub(*tax_minor))
                    .ok_or_else(|| "모의원장 매도 비용이 명목 금액을 초과했습니다.".to_owned())?;
                closed.push(ClosedTradeObservation {
                    trade_id: order_id.clone(),
                    symbol: symbol.clone(),
                    closed_at_ms: *occurred_at_ms,
                    net_pnl_minor: checked_pnl(credit, released_cost)?,
                    exit_kind: match exit_reason.as_deref() {
                        Some("stop_loss") => TradeExitKind::StopLoss,
                        Some("take_profit") => TradeExitKind::TakeProfit,
                        Some("strategy_signal") | Some("period_end") => TradeExitKind::Signal,
                        _ => TradeExitKind::Manual,
                    },
                });
                if position.0 == *quantity {
                    positions.remove(symbol);
                } else {
                    positions.insert(
                        symbol.clone(),
                        (position.0 - *quantity, position.1 - released_cost),
                    );
                }
            }
        }
    }
    Ok((initial_equity_minor, closed))
}

fn store_decision(
    bridge: &PersistenceBridge,
    decision: &StrategyProtectionDecision,
) -> Result<(), String> {
    let json = serde_json::to_string(decision)
        .map_err(|_| "전략 보호 결정을 직렬화하지 못했습니다.".to_owned())?;
    let created_at_ms = persistence::now_ms()?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 보호 결정 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "INSERT INTO strategy_protection_decisions
             (policy_id, target_symbol, can_open_new_position, decision_json, evaluated_at_ms, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                decision.policy_id,
                decision.target_symbol,
                decision.can_open_new_position,
                json,
                decision.evaluated_at_ms,
                created_at_ms
            ],
        )
        .map_err(|error| format!("전략 보호 결정을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

pub(crate) fn evaluate_runtime_protection(
    bridge: &PersistenceBridge,
    risk_policy: &RiskPolicy,
    target_symbol: &str,
    currency: &str,
    now_ms: u64,
) -> Result<Option<StrategyProtectionDecision>, String> {
    let Some(policy) = risk_policy.protection.clone() else {
        return Ok(None);
    };
    let ledger_id = crate::paper_trading::ledger_id_for_currency(currency)?;
    let ledger = bridge.paper_ledger(ledger_id)?;
    let (initial_equity_minor, closed_trades) = closed_trades_from_ledger(ledger.events())?;
    let decision = evaluate_strategy_protection(StrategyProtectionRequest {
        target_symbol: target_symbol.to_owned(),
        now_ms,
        initial_equity_minor,
        policy,
        closed_trades,
    })?;
    store_decision(bridge, &decision)?;
    Ok(Some(decision))
}

#[tauri::command]
pub fn strategy_protection_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<StoredProtectionDecision>, String> {
    let limit = limit.clamp(1, 200);
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 보호 결정 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT decision_id, decision_json, created_at_ms
             FROM strategy_protection_decisions
             ORDER BY evaluated_at_ms DESC, decision_id DESC LIMIT ?1",
        )
        .map_err(|error| format!("전략 보호 결정 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })
        .map_err(|error| format!("전략 보호 결정 이력을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        let (decision_id, json, created_at_ms) =
            row.map_err(|error| format!("전략 보호 결정 이력을 읽지 못했습니다: {error}"))?;
        let decision = serde_json::from_str(&json)
            .map_err(|_| "저장된 전략 보호 결정을 해석하지 못했습니다.".to_owned())?;
        Ok(StoredProtectionDecision {
            decision_id,
            decision,
            created_at_ms,
        })
    })
    .collect()
}

#[tauri::command]
pub fn strategy_protection_alerts_sync(
    now_ms: u64,
    bridge: State<'_, PersistenceBridge>,
) -> Result<ProtectionAlertSyncReceipt, String> {
    if now_ms == 0 {
        return Err("전략 보호 알림 동기화 시각을 확인해 주세요.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 보호 알림 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare(
        "SELECT decision_id,decision_json FROM strategy_protection_decisions ORDER BY decision_id"
    ).map_err(|error| format!("전략 보호 결정을 준비하지 못했습니다: {error}"))?;
    let decisions = statement
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("전략 보호 결정을 조회하지 못했습니다: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("전략 보호 결정을 읽지 못했습니다: {error}"))?;
    drop(statement);
    let mut created = 0;
    let mut skipped = 0;
    for (decision_id, json) in &decisions {
        let decision: StrategyProtectionDecision = serde_json::from_str(json)
            .map_err(|_| "저장된 전략 보호 결정을 해석하지 못했습니다.".to_owned())?;
        for (index, trigger) in decision.triggers.iter().enumerate() {
            let (phase, seen_at, severity, message) = if trigger.locked_until_ms > now_ms {
                (
                    "started",
                    decision.evaluated_at_ms,
                    "warning",
                    format!(
                        "{} 전략 잠금 시작: {} ({}까지)",
                        decision.target_symbol, trigger.code, trigger.locked_until_ms
                    ),
                )
            } else {
                (
                    "released",
                    trigger.locked_until_ms,
                    "info",
                    format!(
                        "{} 전략 잠금 해제: {}",
                        decision.target_symbol, trigger.code
                    ),
                )
            };
            let alert_id = format!("strategy-{decision_id}-{index}-{phase}");
            let key = format!("strategy-protection:{decision_id}:{index}:{phase}");
            let changed = connection.execute(
                "INSERT OR IGNORE INTO operational_alerts(alert_id,deduplication_key,severity,message,first_seen_at_ms,last_seen_at_ms,occurrence_count,acknowledged_at_ms,response) VALUES(?1,?2,?3,?4,?5,?5,1,NULL,NULL)",
                params![alert_id,key,severity,message,seen_at.max(1)],
            ).map_err(|error| format!("전략 보호 알림을 저장하지 못했습니다: {error}"))?;
            if changed == 1 {
                created += 1;
            } else {
                skipped += 1;
            }
        }
    }
    Ok(ProtectionAlertSyncReceipt {
        decisions_scanned: decisions.len(),
        alerts_created: created,
        duplicate_alerts_skipped: skipped,
        live_order_allowed: false,
    })
}

fn push_trigger(
    triggers: &mut Vec<ProtectionTrigger>,
    code: &str,
    scope: ProtectionScope,
    locked_until_ms: u64,
    observed: String,
    threshold: String,
) {
    triggers.push(ProtectionTrigger {
        code: code.to_owned(),
        scope,
        locked_until_ms,
        observed,
        threshold,
    });
}

pub fn evaluate_strategy_protection(
    request: StrategyProtectionRequest,
) -> Result<StrategyProtectionDecision, String> {
    validate(&request)?;
    let policy = &request.policy;
    let lookback_start = request.now_ms.saturating_sub(policy.lookback_ms);
    let recent = request
        .closed_trades
        .iter()
        .filter(|trade| trade.closed_at_ms >= lookback_start)
        .collect::<Vec<_>>();
    let mut triggers = Vec::new();

    if let Some(last_symbol_trade) = request
        .closed_trades
        .iter()
        .rev()
        .find(|trade| trade.symbol == request.target_symbol)
    {
        let lock_until = last_symbol_trade
            .closed_at_ms
            .saturating_add(policy.cooldown_ms);
        if lock_until > request.now_ms {
            push_trigger(
                &mut triggers,
                "cooldown",
                ProtectionScope::Symbol,
                lock_until,
                format!("마지막 종료 {}ms", last_symbol_trade.closed_at_ms),
                format!("쿨다운 {}ms", policy.cooldown_ms),
            );
        }
    }

    let recent_stop_losses = recent
        .iter()
        .filter(|trade| trade.exit_kind == TradeExitKind::StopLoss)
        .count();
    if recent_stop_losses >= policy.maximum_stop_loss_count {
        let lock_until = recent
            .last()
            .map_or(request.now_ms, |trade| trade.closed_at_ms)
            .saturating_add(policy.lock_duration_ms);
        if lock_until > request.now_ms {
            push_trigger(
                &mut triggers,
                "stop_loss_guard",
                ProtectionScope::Global,
                lock_until,
                format!("최근 손절 {recent_stop_losses}건"),
                format!("{}건 이상", policy.maximum_stop_loss_count),
            );
        }
    }

    let consecutive_losses = request
        .closed_trades
        .iter()
        .rev()
        .take_while(|trade| trade.net_pnl_minor < 0)
        .count();
    if consecutive_losses >= policy.maximum_consecutive_loss_count {
        let lock_until = request
            .closed_trades
            .last()
            .map_or(request.now_ms, |trade| trade.closed_at_ms)
            .saturating_add(policy.lock_duration_ms);
        if lock_until > request.now_ms {
            push_trigger(
                &mut triggers,
                "consecutive_loss_guard",
                ProtectionScope::Global,
                lock_until,
                format!("연속 손실 {consecutive_losses}건"),
                format!("{}건 이상", policy.maximum_consecutive_loss_count),
            );
        }
    }

    let initial = i128::from(request.initial_equity_minor);
    let mut equity = initial;
    let mut peak = initial;
    let mut maximum_drawdown_bps = 0u64;
    for trade in &request.closed_trades {
        equity = equity.saturating_add(i128::from(trade.net_pnl_minor));
        peak = peak.max(equity);
        if peak > 0 && equity < peak {
            let drawdown = ((peak - equity).saturating_mul(10_000) / peak).clamp(0, 10_000);
            maximum_drawdown_bps = maximum_drawdown_bps.max(drawdown as u64);
        }
    }
    if maximum_drawdown_bps >= policy.maximum_drawdown_bps {
        let lock_until = request.now_ms.saturating_add(policy.lock_duration_ms);
        push_trigger(
            &mut triggers,
            "maximum_drawdown_guard",
            ProtectionScope::Global,
            lock_until,
            format!("최대 낙폭 {maximum_drawdown_bps}bp"),
            format!("{}bp 이상", policy.maximum_drawdown_bps),
        );
    }

    let recent_symbol = recent
        .iter()
        .filter(|trade| trade.symbol == request.target_symbol)
        .collect::<Vec<_>>();
    if recent_symbol.len() >= policy.minimum_symbol_trade_count {
        let net_pnl = recent_symbol
            .iter()
            .try_fold(0i64, |sum, trade| sum.checked_add(trade.net_pnl_minor))
            .ok_or_else(|| "종목별 손익 합계가 범위를 초과했습니다.".to_owned())?;
        if net_pnl <= policy.minimum_symbol_net_pnl_minor {
            let lock_until = recent_symbol
                .last()
                .map_or(request.now_ms, |trade| trade.closed_at_ms)
                .saturating_add(policy.lock_duration_ms);
            if lock_until > request.now_ms {
                push_trigger(
                    &mut triggers,
                    "low_profit_symbol_guard",
                    ProtectionScope::Symbol,
                    lock_until,
                    format!("{}건 순손익 {net_pnl}", recent_symbol.len()),
                    format!("{} 이하", policy.minimum_symbol_net_pnl_minor),
                );
            }
        }
    }

    let global_lock_until_ms = triggers
        .iter()
        .filter(|trigger| trigger.scope == ProtectionScope::Global)
        .map(|trigger| trigger.locked_until_ms)
        .max();
    let symbol_lock_until_ms = triggers
        .iter()
        .filter(|trigger| trigger.scope == ProtectionScope::Symbol)
        .map(|trigger| trigger.locked_until_ms)
        .max();

    Ok(StrategyProtectionDecision {
        policy_id: policy.policy_id.clone(),
        target_symbol: request.target_symbol,
        evaluated_at_ms: request.now_ms,
        can_open_new_position: global_lock_until_ms.is_none() && symbol_lock_until_ms.is_none(),
        global_lock_until_ms,
        symbol_lock_until_ms,
        triggers,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn strategy_protection_evaluate(
    request: StrategyProtectionRequest,
) -> Result<StrategyProtectionDecision, String> {
    evaluate_strategy_protection(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        paper_account::{execute_shadow_order, open_paper_account, ShadowOrderRequest},
        paper_trading::{PAPER_ACCOUNT_ID, PAPER_LEDGER_ID},
        simulation::TradingCosts,
    };

    fn policy() -> StrategyProtectionPolicy {
        StrategyProtectionPolicy {
            policy_id: "protection-v1".to_owned(),
            lookback_ms: 1_000,
            lock_duration_ms: 500,
            cooldown_ms: 100,
            maximum_stop_loss_count: 2,
            maximum_consecutive_loss_count: 3,
            maximum_drawdown_bps: 1_000,
            minimum_symbol_trade_count: 2,
            minimum_symbol_net_pnl_minor: 0,
        }
    }

    fn trade(
        id: &str,
        symbol: &str,
        closed_at_ms: u64,
        pnl: i64,
        exit_kind: TradeExitKind,
    ) -> ClosedTradeObservation {
        ClosedTradeObservation {
            trade_id: id.to_owned(),
            symbol: symbol.to_owned(),
            closed_at_ms,
            net_pnl_minor: pnl,
            exit_kind,
        }
    }

    #[test]
    fn protection_blocks_new_positions_after_repeated_stop_losses() {
        let decision = evaluate_strategy_protection(StrategyProtectionRequest {
            target_symbol: "BTCUSDT".to_owned(),
            now_ms: 1_000,
            initial_equity_minor: 10_000,
            policy: policy(),
            closed_trades: vec![
                trade("t-1", "BTCUSDT", 700, -100, TradeExitKind::StopLoss),
                trade("t-2", "ETHUSDT", 900, -100, TradeExitKind::StopLoss),
            ],
        })
        .expect("protection decision");
        assert!(!decision.can_open_new_position);
        assert!(decision
            .triggers
            .iter()
            .any(|trigger| trigger.code == "stop_loss_guard"));
        assert!(!decision.live_order_allowed);
        assert!(!protection_action_allowed(
            &decision,
            ProtectionAction::Open
        ));
        assert!(protection_action_allowed(
            &decision,
            ProtectionAction::Reduce
        ));
        assert!(protection_action_allowed(
            &decision,
            ProtectionAction::Cancel
        ));
    }

    #[test]
    fn protection_rejects_future_or_unsorted_trade_events() {
        let error = evaluate_strategy_protection(StrategyProtectionRequest {
            target_symbol: "AAPL".to_owned(),
            now_ms: 1_000,
            initial_equity_minor: 10_000,
            policy: policy(),
            closed_trades: vec![trade("future", "AAPL", 1_001, 10, TradeExitKind::Signal)],
        })
        .unwrap_err();
        assert!(error.contains("미래"));
    }

    #[test]
    fn reconstructs_realized_trade_pnl_from_the_append_only_ledger() {
        let costs = TradingCosts {
            buy_fee_bps: 0.0,
            sell_fee_bps: 0.0,
            sell_tax_bps: 0.0,
            slippage_bps: 0.0,
        };
        let events = vec![
            LedgerEvent::AccountOpened {
                account_id: "paper".to_owned(),
                currency: "KRW".to_owned(),
                initial_cash_minor: 1_000,
                occurred_at_ms: 1,
            },
            LedgerEvent::OrderFilled {
                account_id: "paper".to_owned(),
                order_id: "buy-1".to_owned(),
                idempotency_key: "buy-key".to_owned(),
                symbol: "005930".to_owned(),
                side: TradeSide::Buy,
                quantity: 2,
                quantity_scale: 1,
                reference_price_minor: 50,
                execution_price_minor: 50,
                notional_minor: 100,
                fee_minor: 1,
                tax_minor: 0,
                costs,
                exit_reason: None,
                cause_event_id: None,
                occurred_at_ms: 2,
            },
            LedgerEvent::OrderFilled {
                account_id: "paper".to_owned(),
                order_id: "sell-1".to_owned(),
                idempotency_key: "sell-key".to_owned(),
                symbol: "005930".to_owned(),
                side: TradeSide::Sell,
                quantity: 2,
                quantity_scale: 1,
                reference_price_minor: 60,
                execution_price_minor: 60,
                notional_minor: 120,
                fee_minor: 1,
                tax_minor: 0,
                costs,
                exit_reason: Some("stop_loss".to_owned()),
                cause_event_id: None,
                occurred_at_ms: 3,
            },
        ];
        let (initial, trades) = closed_trades_from_ledger(&events).expect("closed trades");
        assert_eq!(initial, 1_000);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].net_pnl_minor, 18);
        assert_eq!(trades[0].exit_kind, TradeExitKind::StopLoss);
    }

    #[test]
    fn active_runtime_policy_blocks_after_realized_consecutive_losses_and_persists_decision() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let costs = TradingCosts {
            buy_fee_bps: 0.0,
            sell_fee_bps: 0.0,
            sell_tax_bps: 0.0,
            slippage_bps: 0.0,
        };
        let mut ledger = bridge.paper_ledger(PAPER_LEDGER_ID).expect("ledger");
        open_paper_account(
            &mut ledger,
            PAPER_ACCOUNT_ID.to_owned(),
            "KRW".to_owned(),
            100_000_000,
            1,
        )
        .expect("account");
        let mut occurred_at_ms = 1;
        for index in 0..3 {
            occurred_at_ms += 1;
            execute_shadow_order(
                &mut ledger,
                ShadowOrderRequest {
                    account_id: PAPER_ACCOUNT_ID.to_owned(),
                    order_id: format!("buy-{index}"),
                    idempotency_key: format!("buy-key-{index}"),
                    symbol: "005930".to_owned(),
                    currency: "KRW".to_owned(),
                    side: TradeSide::Buy,
                    quantity: 1,
                    quantity_scale: 1,
                    reference_price_minor: 100,
                    occurred_at_ms,
                },
                costs,
            )
            .expect("buy");
            occurred_at_ms += 1;
            execute_shadow_order(
                &mut ledger,
                ShadowOrderRequest {
                    account_id: PAPER_ACCOUNT_ID.to_owned(),
                    order_id: format!("sell-{index}"),
                    idempotency_key: format!("sell-key-{index}"),
                    symbol: "005930".to_owned(),
                    currency: "KRW".to_owned(),
                    side: TradeSide::Sell,
                    quantity: 1,
                    quantity_scale: 1,
                    reference_price_minor: 90,
                    occurred_at_ms,
                },
                costs,
            )
            .expect("sell");
        }
        drop(ledger);
        let risk_policy = RiskPolicy {
            policy_id: "risk-runtime".to_owned(),
            max_order_notional_minor: 1_000_000,
            max_backtest_drawdown_bps: 2_000,
            stop_loss_bps: 500,
            take_profit_bps: 1_000,
            daily_loss_limit_minor: 500_000,
            protection: Some(StrategyProtectionPolicy {
                policy_id: "protect-runtime".to_owned(),
                lookback_ms: 10_000,
                lock_duration_ms: 1_000,
                cooldown_ms: 1,
                maximum_stop_loss_count: 10,
                maximum_consecutive_loss_count: 3,
                maximum_drawdown_bps: 9_000,
                minimum_symbol_trade_count: 10,
                minimum_symbol_net_pnl_minor: -1_000,
            }),
        };
        let decision =
            evaluate_runtime_protection(&bridge, &risk_policy, "005930", "KRW", occurred_at_ms + 1)
                .expect("runtime protection")
                .expect("configured decision");
        assert!(!decision.can_open_new_position);
        assert!(decision
            .triggers
            .iter()
            .any(|trigger| trigger.code == "consecutive_loss_guard"));
        let connection = bridge.connection.lock().expect("connection");
        let count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM strategy_protection_decisions",
                [],
                |row| row.get(0),
            )
            .expect("stored decisions");
        assert_eq!(count, 1);
    }
}
