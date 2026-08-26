use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketSnapshot {
    pub symbol: String,
    pub price_minor: u64,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowTradePlan {
    pub symbol: String,
    pub side: TradeSide,
    pub quantity: u64,
    pub limit_price_minor: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPolicy {
    pub trading_enabled: bool,
    pub max_order_notional_minor: u64,
    pub max_gross_exposure_minor: u64,
    pub max_daily_loss_minor: u64,
    pub max_market_data_age_ms: u64,
    pub max_price_deviation_bps: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskContext {
    pub now_ms: u64,
    pub current_gross_exposure_minor: u64,
    pub realized_pnl_minor: i64,
    pub kill_switch_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskViolationCode {
    TradingDisabled,
    KillSwitchActive,
    InvalidPlan,
    InvalidMarketSnapshot,
    StaleMarketData,
    PriceDeviationExceeded,
    OrderNotionalExceeded,
    GrossExposureExceeded,
    DailyLossExceeded,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskViolation {
    pub code: RiskViolationCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskDecision {
    pub approved: bool,
    pub order_notional_minor: Option<u64>,
    pub projected_gross_exposure_minor: Option<u64>,
    pub violations: Vec<RiskViolation>,
}

fn reject(violations: &mut Vec<RiskViolation>, code: RiskViolationCode, message: &str) {
    violations.push(RiskViolation {
        code,
        message: message.to_owned(),
    });
}

pub fn evaluate_shadow_risk(
    plan: ShadowTradePlan,
    snapshot: MarketSnapshot,
    policy: RiskPolicy,
    context: RiskContext,
) -> RiskDecision {
    let mut violations = Vec::new();

    if !policy.trading_enabled {
        reject(
            &mut violations,
            RiskViolationCode::TradingDisabled,
            "모의 거래 정책이 비활성화되어 있습니다.",
        );
    }
    if context.kill_switch_active {
        reject(
            &mut violations,
            RiskViolationCode::KillSwitchActive,
            "킬 스위치가 활성화되어 있습니다.",
        );
    }
    if plan.symbol.trim().is_empty() || plan.quantity == 0 || plan.limit_price_minor == 0 {
        reject(
            &mut violations,
            RiskViolationCode::InvalidPlan,
            "종목, 수량과 지정 가격은 모두 유효해야 합니다.",
        );
    }
    if snapshot.symbol != plan.symbol
        || snapshot.price_minor == 0
        || snapshot.observed_at_ms > context.now_ms
    {
        reject(
            &mut violations,
            RiskViolationCode::InvalidMarketSnapshot,
            "거래 계획과 일치하는 과거 또는 현재 시점의 유효한 시세가 필요합니다.",
        );
    } else if context.now_ms - snapshot.observed_at_ms > policy.max_market_data_age_ms {
        reject(
            &mut violations,
            RiskViolationCode::StaleMarketData,
            "허용된 지연 시간을 넘긴 시세로는 거래할 수 없습니다.",
        );
    }

    let price_difference = plan.limit_price_minor.abs_diff(snapshot.price_minor);
    let price_deviation_bps = price_difference
        .checked_mul(10_000)
        .and_then(|difference| difference.checked_div(snapshot.price_minor));
    if snapshot.price_minor > 0
        && price_deviation_bps.is_none_or(|deviation| deviation > policy.max_price_deviation_bps)
    {
        reject(
            &mut violations,
            RiskViolationCode::PriceDeviationExceeded,
            "지정 가격이 최신 시세의 허용 범위를 벗어났습니다.",
        );
    }

    let order_notional = plan.quantity.checked_mul(plan.limit_price_minor);
    if order_notional.is_none() {
        reject(
            &mut violations,
            RiskViolationCode::InvalidPlan,
            "주문 금액을 안전하게 계산할 수 없습니다.",
        );
    }
    if order_notional.is_some_and(|notional| notional > policy.max_order_notional_minor) {
        reject(
            &mut violations,
            RiskViolationCode::OrderNotionalExceeded,
            "주문 금액이 건별 한도를 초과했습니다.",
        );
    }

    let projected_gross_exposure = order_notional
        .and_then(|notional| context.current_gross_exposure_minor.checked_add(notional));
    if projected_gross_exposure.is_none()
        || projected_gross_exposure
            .is_some_and(|exposure| exposure > policy.max_gross_exposure_minor)
    {
        reject(
            &mut violations,
            RiskViolationCode::GrossExposureExceeded,
            "예상 총 익스포저가 허용 한도를 초과했습니다.",
        );
    }

    if context.realized_pnl_minor < 0
        && context.realized_pnl_minor.unsigned_abs() >= policy.max_daily_loss_minor
    {
        reject(
            &mut violations,
            RiskViolationCode::DailyLossExceeded,
            "일일 손실 한도에 도달했습니다.",
        );
    }

    RiskDecision {
        approved: violations.is_empty(),
        order_notional_minor: order_notional,
        projected_gross_exposure_minor: projected_gross_exposure,
        violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_inputs() -> (ShadowTradePlan, MarketSnapshot, RiskPolicy, RiskContext) {
        (
            ShadowTradePlan {
                symbol: "005930".to_owned(),
                side: TradeSide::Buy,
                quantity: 10,
                limit_price_minor: 70_000,
            },
            MarketSnapshot {
                symbol: "005930".to_owned(),
                price_minor: 69_900,
                observed_at_ms: 995_000,
            },
            RiskPolicy {
                trading_enabled: true,
                max_order_notional_minor: 1_000_000,
                max_gross_exposure_minor: 5_000_000,
                max_daily_loss_minor: 100_000,
                max_market_data_age_ms: 10_000,
                max_price_deviation_bps: 100,
            },
            RiskContext {
                now_ms: 1_000_000,
                current_gross_exposure_minor: 1_000_000,
                realized_pnl_minor: 0,
                kill_switch_active: false,
            },
        )
    }

    #[test]
    fn approves_a_valid_shadow_plan() {
        let (plan, snapshot, policy, context) = valid_inputs();

        let decision = evaluate_shadow_risk(plan, snapshot, policy, context);

        assert!(decision.approved);
        assert_eq!(decision.order_notional_minor, Some(700_000));
        assert!(decision.violations.is_empty());
    }

    #[test]
    fn rejects_when_trading_is_disabled_or_kill_switch_is_active() {
        let (plan, snapshot, mut policy, mut context) = valid_inputs();
        policy.trading_enabled = false;
        context.kill_switch_active = true;

        let decision = evaluate_shadow_risk(plan, snapshot, policy, context);

        assert!(!decision.approved);
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::TradingDisabled));
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::KillSwitchActive));
    }

    #[test]
    fn rejects_stale_market_data_and_limit_breaches() {
        let (mut plan, mut snapshot, mut policy, mut context) = valid_inputs();
        plan.quantity = 20;
        snapshot.observed_at_ms = 900_000;
        policy.max_gross_exposure_minor = 2_000_000;
        context.realized_pnl_minor = -100_000;

        let decision = evaluate_shadow_risk(plan, snapshot, policy, context);

        assert!(!decision.approved);
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::StaleMarketData));
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::OrderNotionalExceeded));
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::GrossExposureExceeded));
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::DailyLossExceeded));
    }

    #[test]
    fn rejects_a_future_or_mismatched_snapshot() {
        let (plan, mut snapshot, policy, context) = valid_inputs();
        snapshot.symbol = "AAPL".to_owned();
        snapshot.observed_at_ms = context.now_ms + 1;

        let decision = evaluate_shadow_risk(plan, snapshot, policy, context);

        assert!(!decision.approved);
        assert!(decision
            .violations
            .iter()
            .any(|violation| violation.code == RiskViolationCode::InvalidMarketSnapshot));
    }
}
