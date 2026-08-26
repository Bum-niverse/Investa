use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{simulation::TradingCosts, trading::TradeSide};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreTradePolicy {
    pub maximum_loss_minor: u64,
    pub maximum_order_notional_minor: u64,
    pub maximum_participation_bps: u64,
    pub maximum_quote_age_ms: u64,
    pub maximum_price_deviation_bps: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreTradeInput {
    pub symbol: String,
    pub side: TradeSide,
    pub suggested_quantity: u64,
    pub entry_price_minor: u64,
    pub stop_price_minor: u64,
    pub quote_price_minor: u64,
    pub quote_observed_at_ms: u64,
    pub average_period_volume: u64,
    pub current_gross_exposure_minor: u64,
    pub maximum_gross_exposure_minor: u64,
    pub costs: TradingCosts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeterministicRiskCheck {
    pub rule_id: String,
    pub passed: bool,
    pub measured: String,
    pub limit: String,
    pub evaluated_at_ms: u64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreTradeDecision {
    pub approved: bool,
    pub final_quantity: u64,
    pub estimated_notional_minor: u64,
    pub estimated_round_trip_cost_minor: u64,
    pub checks: Vec<DeterministicRiskCheck>,
}

fn check(
    checks: &mut Vec<DeterministicRiskCheck>,
    rule_id: &str,
    passed: bool,
    measured: impl ToString,
    limit: impl ToString,
    now_ms: u64,
    message: &str,
) {
    checks.push(DeterministicRiskCheck {
        rule_id: rule_id.to_owned(),
        passed,
        measured: measured.to_string(),
        limit: limit.to_string(),
        evaluated_at_ms: now_ms,
        message: message.to_owned(),
    });
}

fn ceil_bps(notional: u64, bps: f64) -> Option<u64> {
    if !bps.is_finite() || !(0.0..=10_000.0).contains(&bps) {
        return None;
    }
    let millionth_bps = (bps * 1_000_000.0).round() as u128;
    let numerator = u128::from(notional).checked_mul(millionth_bps)?;
    u64::try_from(numerator.div_ceil(10_000_000_000)).ok()
}

pub fn evaluate_pretrade(
    policy: &PreTradePolicy,
    input: &PreTradeInput,
    now_ms: u64,
) -> PreTradeDecision {
    let mut checks = Vec::new();
    let direction_valid = match input.side {
        TradeSide::Buy => input.stop_price_minor < input.entry_price_minor,
        TradeSide::Sell => input.stop_price_minor > input.entry_price_minor,
    };
    let stop_distance = input.entry_price_minor.abs_diff(input.stop_price_minor);
    check(
        &mut checks,
        "pretrade.stop_direction",
        direction_valid && stop_distance > 0,
        stop_distance,
        "> 0 and direction-valid",
        now_ms,
        "손절가 방향과 손절 거리를 확인했습니다.",
    );
    let loss_based_quantity = policy
        .maximum_loss_minor
        .checked_div(stop_distance)
        .unwrap_or_default();
    let final_quantity = input.suggested_quantity.min(loss_based_quantity);
    check(
        &mut checks,
        "pretrade.position_size",
        final_quantity > 0,
        final_quantity,
        loss_based_quantity,
        now_ms,
        "제안 수량과 허용 손실액 중 더 보수적인 수량을 사용합니다.",
    );
    let notional = final_quantity.saturating_mul(input.entry_price_minor);
    check(
        &mut checks,
        "pretrade.order_notional",
        notional <= policy.maximum_order_notional_minor,
        notional,
        policy.maximum_order_notional_minor,
        now_ms,
        "주문 금액 한도를 확인했습니다.",
    );
    let projected_exposure = input.current_gross_exposure_minor.saturating_add(notional);
    check(
        &mut checks,
        "pretrade.gross_exposure",
        projected_exposure <= input.maximum_gross_exposure_minor,
        projected_exposure,
        input.maximum_gross_exposure_minor,
        now_ms,
        "총 익스포저 한도를 확인했습니다.",
    );
    let participation_bps = if input.average_period_volume > 0 {
        u64::try_from(u128::from(final_quantity) * 10_000 / u128::from(input.average_period_volume))
            .unwrap_or(u64::MAX)
    } else {
        u64::MAX
    };
    check(
        &mut checks,
        "market.participation",
        participation_bps <= policy.maximum_participation_bps,
        participation_bps,
        policy.maximum_participation_bps,
        now_ms,
        "평균 거래량 대비 주문 크기를 확인했습니다.",
    );
    let quote_age = now_ms
        .checked_sub(input.quote_observed_at_ms)
        .unwrap_or(u64::MAX);
    check(
        &mut checks,
        "market.quote_freshness",
        input.quote_price_minor > 0 && quote_age <= policy.maximum_quote_age_ms,
        quote_age,
        policy.maximum_quote_age_ms,
        now_ms,
        "시세 지연을 확인했습니다.",
    );
    let deviation_bps = if input.quote_price_minor > 0 {
        u64::try_from(
            u128::from(input.entry_price_minor.abs_diff(input.quote_price_minor)) * 10_000
                / u128::from(input.quote_price_minor),
        )
        .unwrap_or(u64::MAX)
    } else {
        u64::MAX
    };
    check(
        &mut checks,
        "market.price_deviation",
        deviation_bps <= policy.maximum_price_deviation_bps,
        deviation_bps,
        policy.maximum_price_deviation_bps,
        now_ms,
        "최신 시세 대비 주문 가격 괴리를 확인했습니다.",
    );
    let round_trip_bps = input.costs.buy_fee_bps
        + input.costs.sell_fee_bps
        + input.costs.sell_tax_bps
        + input.costs.slippage_bps * 2.0;
    let estimated_round_trip_cost_minor = ceil_bps(notional, round_trip_bps).unwrap_or(u64::MAX);
    check(
        &mut checks,
        "market.cost_model",
        estimated_round_trip_cost_minor != u64::MAX,
        estimated_round_trip_cost_minor,
        "finite explicit market costs",
        now_ms,
        "시장별 수수료·세금·슬리피지 비용을 계산했습니다.",
    );
    PreTradeDecision {
        approved: checks.iter().all(|item| item.passed),
        final_quantity,
        estimated_notional_minor: notional,
        estimated_round_trip_cost_minor,
        checks,
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    CancelPending,
    ClosePositions,
    HoldAndObserve,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillSwitchState {
    pub active: bool,
    pub reason: Option<String>,
    pub activated_at_ms: Option<u64>,
    pub selected_recovery: Option<RecoveryAction>,
    pub cause_verified_resolved: bool,
    pub user_resume_approved: bool,
}

impl KillSwitchState {
    pub fn activate(&mut self, reason: &str, now_ms: u64) -> Result<(), String> {
        if reason.trim().is_empty() || now_ms == 0 {
            return Err("킬 스위치 원인과 시각이 필요합니다.".to_owned());
        }
        self.active = true;
        self.reason = Some(reason.to_owned());
        self.activated_at_ms = Some(now_ms);
        self.selected_recovery = None;
        self.cause_verified_resolved = false;
        self.user_resume_approved = false;
        Ok(())
    }

    pub fn select_recovery(&mut self, action: RecoveryAction) -> Result<(), String> {
        if !self.active {
            return Err("활성 킬 스위치에서만 복구 행동을 선택할 수 있습니다.".to_owned());
        }
        self.selected_recovery = Some(action);
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if !self.active
            || self.selected_recovery.is_none()
            || !self.cause_verified_resolved
            || !self.user_resume_approved
        {
            return Err(
                "원인 해소 확인, 복구 행동과 사용자 재개 승인이 모두 필요합니다.".to_owned(),
            );
        }
        self.active = false;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderLifecycleStatus {
    Created,
    Submitted,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderLifecycleEvent {
    pub event_id: String,
    pub client_order_id: String,
    pub broker_order_id: Option<String>,
    pub status: OrderLifecycleStatus,
    pub cumulative_filled_quantity: u64,
    pub average_fill_price_minor: Option<u64>,
    pub occurred_at_ms: u64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayedOrder {
    pub client_order_id: String,
    pub status: OrderLifecycleStatus,
    pub order_quantity: u64,
    pub cumulative_filled_quantity: u64,
    pub remaining_quantity: u64,
    pub average_fill_price_minor: Option<u64>,
}

fn order_transition_allowed(from: OrderLifecycleStatus, to: OrderLifecycleStatus) -> bool {
    matches!(
        (from, to),
        (
            OrderLifecycleStatus::Created,
            OrderLifecycleStatus::Submitted
        ) | (
            OrderLifecycleStatus::Created,
            OrderLifecycleStatus::Rejected
        ) | (
            OrderLifecycleStatus::Created,
            OrderLifecycleStatus::Cancelled
        ) | (
            OrderLifecycleStatus::Submitted,
            OrderLifecycleStatus::PartiallyFilled
        ) | (
            OrderLifecycleStatus::Submitted,
            OrderLifecycleStatus::Filled
        ) | (
            OrderLifecycleStatus::Submitted,
            OrderLifecycleStatus::Rejected
        ) | (
            OrderLifecycleStatus::Submitted,
            OrderLifecycleStatus::Cancelled
        ) | (
            OrderLifecycleStatus::PartiallyFilled,
            OrderLifecycleStatus::PartiallyFilled
        ) | (
            OrderLifecycleStatus::PartiallyFilled,
            OrderLifecycleStatus::Filled
        ) | (
            OrderLifecycleStatus::PartiallyFilled,
            OrderLifecycleStatus::Cancelled
        )
    )
}

pub fn replay_order_events(
    order_quantity: u64,
    events: &[OrderLifecycleEvent],
) -> Result<ReplayedOrder, String> {
    if order_quantity == 0 || events.is_empty() {
        return Err("주문 수량과 최소 하나의 주문 사건이 필요합니다.".to_owned());
    }
    let client_order_id = events[0].client_order_id.as_str();
    let mut seen = BTreeSet::new();
    let mut previous: Option<&OrderLifecycleEvent> = None;
    for event in events {
        if event.client_order_id != client_order_id
            || event.event_id.trim().is_empty()
            || event.reason.trim().is_empty()
            || event.cumulative_filled_quantity > order_quantity
            || !seen.insert(event.event_id.as_str())
        {
            return Err("주문 사건 ID·수량·원인이 올바르지 않습니다.".to_owned());
        }
        if let Some(before) = previous {
            if event.occurred_at_ms < before.occurred_at_ms
                || !order_transition_allowed(before.status, event.status)
                || event.cumulative_filled_quantity < before.cumulative_filled_quantity
            {
                return Err("허용되지 않은 주문 상태 역전이 또는 체결 수량 감소입니다.".to_owned());
            }
        } else if event.status != OrderLifecycleStatus::Created
            || event.cumulative_filled_quantity != 0
        {
            return Err("주문 사건은 미체결 Created 상태에서 시작해야 합니다.".to_owned());
        }
        match event.status {
            OrderLifecycleStatus::PartiallyFilled => {
                if event.cumulative_filled_quantity == 0
                    || event.cumulative_filled_quantity >= order_quantity
                    || event
                        .average_fill_price_minor
                        .is_none_or(|price| price == 0)
                {
                    return Err("부분 체결 수량과 평균 체결가가 일치하지 않습니다.".to_owned());
                }
            }
            OrderLifecycleStatus::Filled
                if event.cumulative_filled_quantity != order_quantity
                    || event
                        .average_fill_price_minor
                        .is_none_or(|price| price == 0) =>
            {
                return Err("완전 체결 수량과 평균 체결가가 일치하지 않습니다.".to_owned());
            }
            OrderLifecycleStatus::Filled => {}
            _ => {}
        }
        previous = Some(event);
    }
    let last = previous.expect("events checked non-empty");
    Ok(ReplayedOrder {
        client_order_id: client_order_id.to_owned(),
        status: last.status,
        order_quantity,
        cumulative_filled_quantity: last.cumulative_filled_quantity,
        remaining_quantity: order_quantity - last.cumulative_filled_quantity,
        average_fill_price_minor: last.average_fill_price_minor,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationDifference {
    pub difference_id: String,
    pub automatically_resolvable: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconciliationDecision {
    pub complete: bool,
    pub new_entries_allowed: bool,
    pub resend_allowed: bool,
    pub differences: Vec<ReconciliationDifference>,
}

pub fn reconcile_order(
    local: &ReplayedOrder,
    broker_status: Option<OrderLifecycleStatus>,
    broker_filled_quantity: Option<u64>,
) -> ReconciliationDecision {
    let mut differences = Vec::new();
    match broker_status {
        None => differences.push(ReconciliationDifference {
            difference_id: "broker.missing_order".to_owned(),
            automatically_resolvable: matches!(
                local.status,
                OrderLifecycleStatus::Created | OrderLifecycleStatus::Rejected
            ),
            message: "로컬 주문에 대응하는 브로커 주문을 찾지 못했습니다.".to_owned(),
        }),
        Some(status) if status != local.status => differences.push(ReconciliationDifference {
            difference_id: "broker.status_mismatch".to_owned(),
            automatically_resolvable: false,
            message: "로컬과 브로커 주문 상태가 다릅니다.".to_owned(),
        }),
        Some(_) => {}
    }
    if broker_filled_quantity.is_some_and(|value| value != local.cumulative_filled_quantity) {
        differences.push(ReconciliationDifference {
            difference_id: "broker.fill_mismatch".to_owned(),
            automatically_resolvable: false,
            message: "로컬과 브로커 누적 체결 수량이 다릅니다.".to_owned(),
        });
    }
    ReconciliationDecision {
        complete: differences.is_empty(),
        new_entries_allowed: differences.is_empty(),
        resend_allowed: differences.is_empty(),
        differences,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn costs() -> TradingCosts {
        TradingCosts {
            buy_fee_bps: 1.5,
            sell_fee_bps: 1.5,
            sell_tax_bps: 20.0,
            slippage_bps: 0.0,
        }
    }

    #[test]
    fn pretrade_uses_loss_based_quantity_and_reports_every_rule() {
        let decision = evaluate_pretrade(
            &PreTradePolicy {
                maximum_loss_minor: 1_000,
                maximum_order_notional_minor: 100_000,
                maximum_participation_bps: 100,
                maximum_quote_age_ms: 1_000,
                maximum_price_deviation_bps: 100,
            },
            &PreTradeInput {
                symbol: "005930".to_owned(),
                side: TradeSide::Buy,
                suggested_quantity: 100,
                entry_price_minor: 100,
                stop_price_minor: 90,
                quote_price_minor: 100,
                quote_observed_at_ms: 900,
                average_period_volume: 10_000,
                current_gross_exposure_minor: 0,
                maximum_gross_exposure_minor: 100_000,
                costs: costs(),
            },
            1_000,
        );
        assert!(decision.approved);
        assert_eq!(decision.final_quantity, 100);
        assert!(decision.checks.iter().all(|item| !item.rule_id.is_empty()));
    }

    #[test]
    fn kill_switch_requires_resolution_recovery_and_user_approval() {
        let mut state = KillSwitchState {
            active: false,
            reason: None,
            activated_at_ms: None,
            selected_recovery: None,
            cause_verified_resolved: false,
            user_resume_approved: false,
        };
        state.activate("daily_loss", 1).expect("activate");
        assert!(state.resume().is_err());
        state
            .select_recovery(RecoveryAction::CancelPending)
            .expect("recovery");
        state.cause_verified_resolved = true;
        state.user_resume_approved = true;
        state.resume().expect("resume");
        assert!(!state.active);
    }

    #[test]
    fn order_replay_validates_partial_fills_and_reconciliation_fails_closed() {
        let events = vec![
            OrderLifecycleEvent {
                event_id: "event-1".to_owned(),
                client_order_id: "order-1".to_owned(),
                broker_order_id: None,
                status: OrderLifecycleStatus::Created,
                cumulative_filled_quantity: 0,
                average_fill_price_minor: None,
                occurred_at_ms: 1,
                reason: "created".to_owned(),
            },
            OrderLifecycleEvent {
                event_id: "event-2".to_owned(),
                client_order_id: "order-1".to_owned(),
                broker_order_id: Some("broker-1".to_owned()),
                status: OrderLifecycleStatus::Submitted,
                cumulative_filled_quantity: 0,
                average_fill_price_minor: None,
                occurred_at_ms: 2,
                reason: "submitted".to_owned(),
            },
            OrderLifecycleEvent {
                event_id: "event-3".to_owned(),
                client_order_id: "order-1".to_owned(),
                broker_order_id: Some("broker-1".to_owned()),
                status: OrderLifecycleStatus::PartiallyFilled,
                cumulative_filled_quantity: 4,
                average_fill_price_minor: Some(100),
                occurred_at_ms: 3,
                reason: "partial".to_owned(),
            },
        ];
        let replayed = replay_order_events(10, &events).expect("replay");
        assert_eq!(replayed.remaining_quantity, 6);
        let reconciliation = reconcile_order(
            &replayed,
            Some(OrderLifecycleStatus::PartiallyFilled),
            Some(3),
        );
        assert!(!reconciliation.complete);
        assert!(!reconciliation.new_entries_allowed);
        assert!(!reconciliation.resend_allowed);
    }
}
