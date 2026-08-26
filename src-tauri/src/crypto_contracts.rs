use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CryptoInstrumentKind {
    Spot,
    LinearPerpetual,
    InversePerpetual,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoInstrumentSpec {
    pub specification_id: String,
    pub exchange: String,
    pub symbol: String,
    pub base_asset: String,
    pub quote_asset: String,
    pub kind: CryptoInstrumentKind,
    pub contract_size_numerator: u64,
    pub contract_size_scale: u64,
    pub price_tick_minor: u64,
    pub quantity_step_base_units: u64,
    pub minimum_notional_minor: u64,
    pub effective_from_ms: u64,
    pub effective_to_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoMarketPoint {
    pub specification_id: String,
    pub event_time_ms: u64,
    pub exchange_time_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub last_price_minor: u64,
    pub mark_price_minor: Option<u64>,
    pub index_price_minor: Option<u64>,
    pub funding_rate_ppm: Option<i64>,
    pub open_interest_base_units: Option<u64>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

pub fn validate_crypto_market_point(
    spec: &CryptoInstrumentSpec,
    point: &CryptoMarketPoint,
    as_of_ms: u64,
) -> Result<(), String> {
    if !valid_id(&spec.specification_id)
        || spec.exchange.trim().is_empty()
        || spec.symbol.trim().is_empty()
        || spec.base_asset.trim().is_empty()
        || spec.quote_asset.trim().is_empty()
        || spec.contract_size_numerator == 0
        || spec.contract_size_scale == 0
        || spec.price_tick_minor == 0
        || spec.quantity_step_base_units == 0
        || spec.minimum_notional_minor == 0
        || spec.effective_from_ms == 0
        || spec
            .effective_to_ms
            .is_some_and(|end| end <= spec.effective_from_ms)
    {
        return Err("코인 상품 명세의 식별자·단위·유효기간이 올바르지 않습니다.".to_owned());
    }
    if point.specification_id != spec.specification_id
        || point.event_time_ms > point.available_at_ms
        || point.exchange_time_ms > point.available_at_ms
        || point.available_at_ms > point.ingested_at_ms
        || point.available_at_ms > as_of_ms
        || point.last_price_minor == 0
        || as_of_ms < spec.effective_from_ms
        || spec.effective_to_ms.is_some_and(|end| as_of_ms >= end)
    {
        return Err("기준 시각에 유효한 상품 명세와 시장 데이터가 필요합니다.".to_owned());
    }
    match spec.kind {
        CryptoInstrumentKind::Spot => {
            if point.mark_price_minor.is_some()
                || point.index_price_minor.is_some()
                || point.funding_rate_ppm.is_some()
            {
                return Err(
                    "현물 데이터에 파생상품 mark/index/funding을 혼합할 수 없습니다.".to_owned(),
                );
            }
        }
        CryptoInstrumentKind::LinearPerpetual | CryptoInstrumentKind::InversePerpetual => {
            if point.mark_price_minor.is_none_or(|price| price == 0)
                || point.index_price_minor.is_none_or(|price| price == 0)
                || point.funding_rate_ppm.is_none()
            {
                return Err(
                    "무기한 선물에는 mark/index/funding 시점 데이터가 필요합니다.".to_owned(),
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoSpotPlan {
    pub quantity_base_units: u64,
    pub quantity_scale: u64,
    pub entry_price_minor: u64,
    pub stop_price_minor: u64,
    pub target_price_minor: u64,
    pub available_quote_minor: u64,
    pub fee_bps: u64,
    pub slippage_bps: u64,
    pub current_crypto_exposure_minor: u64,
    pub maximum_crypto_exposure_minor: u64,
    pub holding_period_ms: u64,
    pub invalidation: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoSpotDecision {
    pub approved: bool,
    pub notional_minor: u64,
    pub estimated_entry_cost_minor: u64,
    pub issues: Vec<String>,
}

pub fn evaluate_crypto_spot_plan(
    spec: &CryptoInstrumentSpec,
    plan: &CryptoSpotPlan,
) -> CryptoSpotDecision {
    let mut issues = Vec::new();
    if spec.kind != CryptoInstrumentKind::Spot {
        issues.push("현물 계획에는 현물 상품 명세만 사용할 수 있습니다.".to_owned());
    }
    if plan.quantity_base_units == 0
        || plan.quantity_scale == 0
        || !plan
            .quantity_base_units
            .is_multiple_of(spec.quantity_step_base_units)
    {
        issues.push("수량은 거래소 quantity step과 고정소수점 단위를 만족해야 합니다.".to_owned());
    }
    if plan.entry_price_minor == 0
        || plan.stop_price_minor >= plan.entry_price_minor
        || plan.target_price_minor <= plan.entry_price_minor
        || plan.holding_period_ms == 0
        || plan.invalidation.trim().is_empty()
    {
        issues.push("진입·손절·목표·보유기간·무효화 조건이 필요합니다.".to_owned());
    }
    let notional = u64::try_from(
        u128::from(plan.entry_price_minor) * u128::from(plan.quantity_base_units)
            / u128::from(plan.quantity_scale.max(1)),
    )
    .unwrap_or(u64::MAX);
    if notional < spec.minimum_notional_minor {
        issues.push("최소 주문금액을 충족하지 못했습니다.".to_owned());
    }
    let fee_and_slippage_bps = plan.fee_bps.saturating_add(plan.slippage_bps);
    let costs =
        u64::try_from((u128::from(notional) * u128::from(fee_and_slippage_bps)).div_ceil(10_000))
            .unwrap_or(u64::MAX);
    if notional.saturating_add(costs) > plan.available_quote_minor {
        issues.push("수수료와 슬리피지를 포함한 가용 잔고가 부족합니다.".to_owned());
    }
    if plan.current_crypto_exposure_minor.saturating_add(notional)
        > plan.maximum_crypto_exposure_minor
    {
        issues.push("코인 자산군 최대 노출을 초과합니다.".to_owned());
    }
    CryptoSpotDecision {
        approved: issues.is_empty(),
        notional_minor: notional,
        estimated_entry_cost_minor: costs,
        issues,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IsolatedLeveragePlan {
    pub side: String,
    pub leverage_milli: u64,
    pub notional_minor: u64,
    pub isolated_margin_minor: u64,
    pub entry_price_minor: u64,
    pub mark_price_minor: u64,
    pub liquidation_price_minor: u64,
    pub maintenance_margin_minor: u64,
    pub estimated_funding_minor: i64,
    pub stop_loss_minor: u64,
    pub daily_loss_minor: u64,
    pub total_derivative_notional_minor: u64,
    pub volatility_bps: u64,
    pub reduce_only_on_exit: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoLeveragePolicy {
    pub maximum_leverage_milli: u64,
    pub maximum_position_loss_minor: u64,
    pub minimum_liquidation_buffer_bps: u64,
    pub daily_loss_limit_minor: u64,
    pub maximum_total_notional_minor: u64,
    pub maximum_volatility_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoLeverageDecision {
    pub approved: bool,
    pub fail_closed: bool,
    pub issues: Vec<String>,
}

pub fn evaluate_isolated_leverage(
    plan: &IsolatedLeveragePlan,
    policy: &CryptoLeveragePolicy,
) -> CryptoLeverageDecision {
    let mut issues = Vec::new();
    if plan.side != "long" && plan.side != "short" {
        issues.push("파생 포지션 방향은 long 또는 short여야 합니다.".to_owned());
    }
    if plan.leverage_milli == 0
        || plan.leverage_milli > policy.maximum_leverage_milli.min(2_000)
        || plan.notional_minor == 0
        || plan.isolated_margin_minor == 0
        || plan.mark_price_minor == 0
        || plan.liquidation_price_minor == 0
        || plan.maintenance_margin_minor > plan.isolated_margin_minor
    {
        issues.push("최대 2배 격리증거금·가격·유지증거금 계약을 만족하지 못했습니다.".to_owned());
    }
    let liquidation_buffer_bps = if plan.mark_price_minor > 0 {
        u64::try_from(
            u128::from(plan.mark_price_minor.abs_diff(plan.liquidation_price_minor)) * 10_000
                / u128::from(plan.mark_price_minor),
        )
        .unwrap_or(0)
    } else {
        0
    };
    if liquidation_buffer_bps < policy.minimum_liquidation_buffer_bps {
        issues.push("청산가 완충거리가 최소 한도보다 작습니다.".to_owned());
    }
    if plan.stop_loss_minor > policy.maximum_position_loss_minor
        || plan.daily_loss_minor >= policy.daily_loss_limit_minor
        || plan.total_derivative_notional_minor > policy.maximum_total_notional_minor
        || plan.volatility_bps > policy.maximum_volatility_bps
    {
        issues.push("포지션 손실·일일 손실·총 명목 노출·변동성 한도를 초과했습니다.".to_owned());
    }
    if !plan.reduce_only_on_exit {
        issues.push("종료·감축 주문은 reduce-only여야 합니다.".to_owned());
    }
    CryptoLeverageDecision {
        approved: issues.is_empty(),
        fail_closed: !issues.is_empty(),
        issues,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoOperationsSnapshot {
    pub heartbeat_age_ms: u64,
    pub websocket_connected: bool,
    pub rest_consistent: bool,
    pub exchange_in_maintenance: bool,
    pub ledger_consistent: bool,
    pub balance_consistent: bool,
    pub positions_consistent: bool,
    pub open_orders_consistent: bool,
    pub fills_consistent: bool,
    pub funding_consistent: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoOperationsDecision {
    pub new_orders_allowed: bool,
    pub reconnect_required: bool,
    pub user_attention_required: bool,
    pub issues: Vec<String>,
}

pub fn evaluate_crypto_operations(
    snapshot: &CryptoOperationsSnapshot,
    maximum_heartbeat_age_ms: u64,
) -> CryptoOperationsDecision {
    let mut issues = Vec::new();
    if snapshot.heartbeat_age_ms > maximum_heartbeat_age_ms {
        issues.push("heartbeat 지연".to_owned());
    }
    if !snapshot.websocket_connected {
        issues.push("websocket 단절".to_owned());
    }
    if !snapshot.rest_consistent {
        issues.push("REST 상태 불일치".to_owned());
    }
    if snapshot.exchange_in_maintenance {
        issues.push("거래소 점검".to_owned());
    }
    if !snapshot.ledger_consistent
        || !snapshot.balance_consistent
        || !snapshot.positions_consistent
        || !snapshot.open_orders_consistent
        || !snapshot.fills_consistent
        || !snapshot.funding_consistent
    {
        issues.push("거래소와 내부 원장 대사 미완료".to_owned());
    }
    CryptoOperationsDecision {
        new_orders_allowed: issues.is_empty(),
        reconnect_required: !snapshot.websocket_connected
            || snapshot.heartbeat_age_ms > maximum_heartbeat_age_ms,
        user_attention_required: snapshot.exchange_in_maintenance
            || !snapshot.rest_consistent
            || !snapshot.ledger_consistent,
        issues,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoPromotionStressInput {
    pub experiment_id: String,
    pub out_of_sample_trade_count: u64,
    pub minimum_trade_count: u64,
    pub cost_adjusted_expected_pnl_minor: i64,
    pub profit_factor_milli: u64,
    pub minimum_profit_factor_milli: u64,
    pub maximum_drawdown_bps: u64,
    pub maximum_allowed_drawdown_bps: u64,
    pub liquidation_count: u64,
    pub stressed_funding_pnl_minor: i64,
    pub volatility_shock_passed: bool,
    pub api_delay_recovery_passed: bool,
    pub restart_reconciliation_passed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoPromotionStressDecision {
    pub eligible_for_shadow: bool,
    pub live_order_enabled: bool,
    pub issues: Vec<String>,
}

pub fn evaluate_crypto_promotion_stress(
    input: &CryptoPromotionStressInput,
) -> CryptoPromotionStressDecision {
    let mut issues = Vec::new();
    if !valid_id(&input.experiment_id)
        || input.minimum_trade_count == 0
        || input.maximum_allowed_drawdown_bps == 0
        || input.minimum_profit_factor_milli < 1_000
    {
        issues.push("실험 ID와 최소 표본·낙폭 기준이 필요합니다.".to_owned());
    }
    if input.out_of_sample_trade_count < input.minimum_trade_count {
        issues.push("OOS 거래 표본이 최소 기준보다 적습니다.".to_owned());
    }
    if input.cost_adjusted_expected_pnl_minor <= 0 {
        issues.push("수수료·슬리피지 반영 후 기대손익이 양수가 아닙니다.".to_owned());
    }
    if input.profit_factor_milli < input.minimum_profit_factor_milli {
        issues.push("Profit Factor가 승격 기준보다 낮습니다.".to_owned());
    }
    if input.maximum_drawdown_bps > input.maximum_allowed_drawdown_bps {
        issues.push("최대 낙폭이 승격 한도를 초과했습니다.".to_owned());
    }
    if input.liquidation_count > 0 {
        issues.push("청산이 한 번이라도 발생한 전략은 승격할 수 없습니다.".to_owned());
    }
    if input.stressed_funding_pnl_minor >= 0 {
        issues.push("불리한 펀딩비 스트레스 손실이 입력되지 않았습니다.".to_owned());
    }
    if !input.volatility_shock_passed {
        issues.push("변동성 급등 스트레스를 통과하지 못했습니다.".to_owned());
    }
    if !input.api_delay_recovery_passed {
        issues.push("API 지연·단절 복구 검증을 통과하지 못했습니다.".to_owned());
    }
    if !input.restart_reconciliation_passed {
        issues.push("재시작 후 원장 대사를 통과하지 못했습니다.".to_owned());
    }
    CryptoPromotionStressDecision {
        eligible_for_shadow: issues.is_empty(),
        live_order_enabled: false,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perpetual_spec() -> CryptoInstrumentSpec {
        CryptoInstrumentSpec {
            specification_id: "binance-btcusdt-v1".to_owned(),
            exchange: "binance".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            base_asset: "BTC".to_owned(),
            quote_asset: "USDT".to_owned(),
            kind: CryptoInstrumentKind::LinearPerpetual,
            contract_size_numerator: 1,
            contract_size_scale: 1,
            price_tick_minor: 1,
            quantity_step_base_units: 1,
            minimum_notional_minor: 5,
            effective_from_ms: 1,
            effective_to_ms: None,
        }
    }

    #[test]
    fn perpetual_market_data_requires_point_in_time_mark_index_and_funding() {
        let spec = perpetual_spec();
        let mut point = CryptoMarketPoint {
            specification_id: spec.specification_id.clone(),
            event_time_ms: 10,
            exchange_time_ms: 10,
            available_at_ms: 11,
            ingested_at_ms: 12,
            last_price_minor: 100,
            mark_price_minor: Some(100),
            index_price_minor: Some(99),
            funding_rate_ppm: Some(10),
            open_interest_base_units: Some(1_000),
        };
        validate_crypto_market_point(&spec, &point, 20).expect("point");
        point.available_at_ms = 21;
        assert!(validate_crypto_market_point(&spec, &point, 20).is_err());
    }

    #[test]
    fn leverage_is_fail_closed_above_two_times_or_without_reduce_only() {
        let decision = evaluate_isolated_leverage(
            &IsolatedLeveragePlan {
                side: "long".to_owned(),
                leverage_milli: 2_500,
                notional_minor: 10_000,
                isolated_margin_minor: 5_000,
                entry_price_minor: 100,
                mark_price_minor: 100,
                liquidation_price_minor: 50,
                maintenance_margin_minor: 1_000,
                estimated_funding_minor: 10,
                stop_loss_minor: 100,
                daily_loss_minor: 0,
                total_derivative_notional_minor: 10_000,
                volatility_bps: 100,
                reduce_only_on_exit: false,
            },
            &CryptoLeveragePolicy {
                maximum_leverage_milli: 3_000,
                maximum_position_loss_minor: 500,
                minimum_liquidation_buffer_bps: 2_000,
                daily_loss_limit_minor: 1_000,
                maximum_total_notional_minor: 20_000,
                maximum_volatility_bps: 500,
            },
        );
        assert!(!decision.approved);
        assert!(decision.fail_closed);
    }

    #[test]
    fn operations_block_orders_until_every_ledger_view_is_reconciled() {
        let decision = evaluate_crypto_operations(
            &CryptoOperationsSnapshot {
                heartbeat_age_ms: 10,
                websocket_connected: true,
                rest_consistent: true,
                exchange_in_maintenance: false,
                ledger_consistent: true,
                balance_consistent: true,
                positions_consistent: true,
                open_orders_consistent: false,
                fills_consistent: true,
                funding_consistent: true,
            },
            100,
        );
        assert!(!decision.new_orders_allowed);
    }

    #[test]
    fn crypto_promotion_requires_oos_cost_and_operational_stress_evidence() {
        let approved = evaluate_crypto_promotion_stress(&CryptoPromotionStressInput {
            experiment_id: "btc-perp-oos-1".to_owned(),
            out_of_sample_trade_count: 80,
            minimum_trade_count: 50,
            cost_adjusted_expected_pnl_minor: 10,
            profit_factor_milli: 1_300,
            minimum_profit_factor_milli: 1_200,
            maximum_drawdown_bps: 800,
            maximum_allowed_drawdown_bps: 1_000,
            liquidation_count: 0,
            stressed_funding_pnl_minor: -100,
            volatility_shock_passed: true,
            api_delay_recovery_passed: true,
            restart_reconciliation_passed: true,
        });
        assert!(approved.eligible_for_shadow);
        assert!(!approved.live_order_enabled);

        let rejected = evaluate_crypto_promotion_stress(&CryptoPromotionStressInput {
            liquidation_count: 1,
            ..CryptoPromotionStressInput {
                experiment_id: "btc-perp-oos-2".to_owned(),
                out_of_sample_trade_count: 80,
                minimum_trade_count: 50,
                cost_adjusted_expected_pnl_minor: 10,
                profit_factor_milli: 1_300,
                minimum_profit_factor_milli: 1_200,
                maximum_drawdown_bps: 800,
                maximum_allowed_drawdown_bps: 1_000,
                liquidation_count: 0,
                stressed_funding_pnl_minor: -100,
                volatility_shock_passed: true,
                api_delay_recovery_passed: true,
                restart_reconciliation_passed: true,
            }
        });
        assert!(!rejected.eligible_for_shadow);
        assert!(rejected.issues.iter().any(|issue| issue.contains("청산")));
    }
}
