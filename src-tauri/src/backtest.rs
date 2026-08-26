use serde::{Deserialize, Serialize};

use crate::{
    pattern_probability::{analyze_pattern_probabilities, PatternProbabilityReport},
    research::{review_strategy_spec, CrossDirection, SignalSpec, StrategySpec},
    simulation::{quote_execution_scaled, CostError, TradingCosts},
    trading::TradeSide,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceBar {
    pub symbol: String,
    pub currency: String,
    pub source: String,
    pub period_start_ms: u64,
    pub period_end_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub open_minor: u64,
    #[serde(default)]
    pub high_minor: u64,
    #[serde(default)]
    pub low_minor: u64,
    pub close_minor: u64,
    pub volume: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRiskLimits {
    pub stop_loss_bps: u64,
    pub take_profit_bps: u64,
    pub daily_loss_limit_minor: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestConfig {
    pub experiment_id: String,
    pub dataset_id: String,
    pub code_version: String,
    pub initial_cash_minor: u64,
    pub order_quantity: u64,
    pub quantity_scale: u64,
    pub close_open_position_at_end: bool,
    pub costs: TradingCosts,
    #[serde(default)]
    pub risk_limits: Option<BacktestRiskLimits>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BacktestErrorCode {
    InvalidStrategy,
    InvalidConfig,
    InsufficientData,
    MixedSymbol,
    InvalidBar,
    DuplicateOrUnsortedBar,
    LookAheadRisk,
    InsufficientCash,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestError {
    pub code: BacktestErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestFill {
    pub side: TradeSide,
    pub period_start_ms: u64,
    pub reference_price_minor: u64,
    pub execution_price_minor: u64,
    pub quantity: u64,
    pub fee_minor: u64,
    pub tax_minor: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BacktestExitKind {
    Signal,
    StopLoss,
    TakeProfit,
    EndOfPeriod,
}

fn default_backtest_exit_kind() -> BacktestExitKind {
    BacktestExitKind::Signal
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestTrade {
    pub opened_at_ms: u64,
    pub closed_at_ms: u64,
    pub quantity: u64,
    pub entry_price_minor: u64,
    pub exit_price_minor: u64,
    pub pnl_minor: i64,
    #[serde(default = "default_backtest_exit_kind")]
    pub exit_kind: BacktestExitKind,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceMetrics {
    pub periods_per_year: u64,
    pub observed_return_count: usize,
    pub annualized_volatility_bps: Option<u64>,
    pub sharpe_ratio_milli: Option<i64>,
    pub sortino_ratio_milli: Option<i64>,
    pub price_benchmark_return_bps: i64,
    pub alpha_vs_price_benchmark_bps: i64,
    #[serde(default)]
    pub period_returns_ppm: Vec<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Passed,
    NeedsReview,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiasCheck {
    pub status: ValidationStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparableBaseline {
    pub name: String,
    pub symbol: String,
    pub currency: String,
    pub first_period_start_ms: u64,
    pub last_period_end_ms: u64,
    pub input_bar_count: usize,
    pub initial_cash_minor: u64,
    pub order_quantity: u64,
    pub quantity_scale: u64,
    pub final_equity_minor: Option<u64>,
    pub total_return_bps: Option<i64>,
    pub total_cost_minor: Option<u64>,
    pub same_period: bool,
    pub same_symbol: bool,
    pub same_currency: bool,
    pub same_cost_model: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestValidationReport {
    pub look_ahead: BiasCheck,
    pub survivorship_bias: BiasCheck,
    pub data_snooping: BiasCheck,
    pub comparable_baseline: ComparableBaseline,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResult {
    pub experiment_id: String,
    pub dataset_id: String,
    pub strategy_id: String,
    pub code_version: String,
    pub input_bar_count: usize,
    pub first_period_start_ms: u64,
    pub last_available_at_ms: u64,
    pub last_ingested_at_ms: u64,
    pub initial_cash_minor: u64,
    pub final_cash_minor: u64,
    pub final_equity_minor: u64,
    pub realized_pnl_minor: i64,
    pub total_return_bps: i64,
    pub max_drawdown_bps: u64,
    pub completed_trade_count: usize,
    pub win_rate_bps: Option<u64>,
    pub average_trade_pnl_minor: Option<i64>,
    pub profit_factor_milli: Option<u64>,
    pub open_position_quantity: u64,
    #[serde(default)]
    pub performance: Option<PerformanceMetrics>,
    #[serde(default)]
    pub pattern_probability: Option<PatternProbabilityReport>,
    #[serde(default)]
    pub robustness: Option<crate::quant_risk::BacktestRobustnessReport>,
    pub validation: BacktestValidationReport,
    pub fills: Vec<BacktestFill>,
    pub trades: Vec<BacktestTrade>,
}

#[derive(Debug, Clone, Copy)]
enum PendingSignal {
    Enter,
    Exit,
}

#[derive(Debug, Clone, Copy)]
struct OpenPosition {
    quantity: u64,
    opened_at_ms: u64,
    entry_price_minor: u64,
    cost_basis_minor: u64,
}

fn error(code: BacktestErrorCode, message: &str) -> BacktestError {
    BacktestError {
        code,
        message: message.to_owned(),
    }
}

fn map_cost_error(cost_error: CostError) -> BacktestError {
    let code = match cost_error {
        CostError::InvalidCosts | CostError::InvalidOrder => BacktestErrorCode::InvalidConfig,
        CostError::Overflow => BacktestErrorCode::ArithmeticOverflow,
    };
    error(
        code,
        "거래 비용 또는 체결 금액을 안전하게 계산할 수 없습니다.",
    )
}

fn signal_parts(signal: &SignalSpec) -> (usize, usize, CrossDirection) {
    match signal {
        SignalSpec::MovingAverageCross {
            fast_window,
            slow_window,
            direction,
        } => (*fast_window, *slow_window, *direction),
    }
}

fn mean(values: &[PriceBar]) -> Result<u128, BacktestError> {
    let total = values.iter().try_fold(0_u128, |sum, bar| {
        sum.checked_add(u128::from(bar.close_minor)).ok_or_else(|| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "이동평균 합계가 범위를 초과했습니다.",
            )
        })
    })?;
    Ok(total / values.len() as u128)
}

fn crossed(bars: &[PriceBar], index: usize, signal: &SignalSpec) -> Result<bool, BacktestError> {
    let (fast, slow, direction) = signal_parts(signal);
    if index < slow {
        return Ok(false);
    }

    let previous_fast = mean(&bars[index - fast..index])?;
    let previous_slow = mean(&bars[index - slow..index])?;
    let current_fast = mean(&bars[index + 1 - fast..=index])?;
    let current_slow = mean(&bars[index + 1 - slow..=index])?;

    Ok(match direction {
        CrossDirection::Above => previous_fast <= previous_slow && current_fast > current_slow,
        CrossDirection::Below => previous_fast >= previous_slow && current_fast < current_slow,
    })
}

/// 저장된 시점 정합 가격봉의 마지막 완료 봉에서만 신호를 평가한다.
/// 섀도우 감시 엔진은 이 순수 함수를 사용하며 외부 주문을 만들지 않는다.
pub fn latest_signal(
    spec: &StrategySpec,
    bars: &[PriceBar],
) -> Result<Option<TradeSide>, BacktestError> {
    let review = review_strategy_spec(spec);
    if !review.executable {
        return Err(error(
            BacktestErrorCode::InvalidStrategy,
            "검증되지 않은 전략의 최신 신호는 평가할 수 없습니다.",
        ));
    }
    let Some(index) = bars.len().checked_sub(1) else {
        return Ok(None);
    };
    if crossed(bars, index, &spec.entry_signal)? {
        Ok(Some(TradeSide::Buy))
    } else if crossed(bars, index, &spec.exit_signal)? {
        Ok(Some(TradeSide::Sell))
    } else {
        Ok(None)
    }
}

fn signed_difference(left: u64, right: u64) -> Result<i64, BacktestError> {
    let difference = i128::from(left) - i128::from(right);
    i64::try_from(difference).map_err(|_| {
        error(
            BacktestErrorCode::ArithmeticOverflow,
            "손익 값이 지원 범위를 초과했습니다.",
        )
    })
}

fn ratio_bps(numerator: i128, denominator: u64) -> Result<i64, BacktestError> {
    if denominator == 0 {
        return Err(error(
            BacktestErrorCode::InvalidConfig,
            "비율 계산의 분모가 0입니다.",
        ));
    }
    let value = numerator.checked_mul(10_000).ok_or_else(|| {
        error(
            BacktestErrorCode::ArithmeticOverflow,
            "수익률 계산이 범위를 초과했습니다.",
        )
    })? / i128::from(denominator);
    i64::try_from(value).map_err(|_| {
        error(
            BacktestErrorCode::ArithmeticOverflow,
            "수익률이 지원 범위를 초과했습니다.",
        )
    })
}

fn ratio_milli(value: f64) -> Option<i64> {
    value
        .is_finite()
        .then(|| (value * 1_000.0).round())
        .and_then(|scaled| i64::try_from(scaled as i128).ok())
}

fn periods_per_year(spec: &StrategySpec, bars: &[PriceBar]) -> Option<u64> {
    let mut gaps = bars
        .windows(2)
        .map(|pair| {
            pair[1]
                .period_start_ms
                .saturating_sub(pair[0].period_start_ms)
        })
        .filter(|gap| *gap > 0)
        .collect::<Vec<_>>();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_unstable();
    let median = gaps[gaps.len() / 2];
    (median >= 20 * 60 * 60 * 1_000).then_some(match spec.market {
        crate::research::Market::Crypto => 365,
        crate::research::Market::Korea | crate::research::Market::UnitedStates => 252,
    })
}

fn performance_metrics(
    spec: &StrategySpec,
    bars: &[PriceBar],
    equity_curve: &[u64],
    total_return_bps: i64,
) -> Result<Option<PerformanceMetrics>, BacktestError> {
    let Some(periods_per_year) = periods_per_year(spec, bars) else {
        return Ok(None);
    };
    let returns = equity_curve
        .windows(2)
        .filter_map(|pair| (pair[0] > 0).then(|| pair[1] as f64 / pair[0] as f64 - 1.0))
        .collect::<Vec<_>>();
    let annualization = (periods_per_year as f64).sqrt();
    let (annualized_volatility_bps, sharpe_ratio_milli, sortino_ratio_milli) = if returns.len() >= 2
    {
        let average = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns
            .iter()
            .map(|value| (value - average).powi(2))
            .sum::<f64>()
            / (returns.len() - 1) as f64;
        let deviation = variance.sqrt();
        let downside = returns
            .iter()
            .map(|value| value.min(0.0).powi(2))
            .sum::<f64>();
        let downside = (downside / returns.len() as f64).sqrt();
        (
            (deviation.is_finite()).then(|| (deviation * annualization * 10_000.0).round() as u64),
            (deviation > 0.0)
                .then(|| ratio_milli(average / deviation * annualization))
                .flatten(),
            (downside > 0.0)
                .then(|| ratio_milli(average / downside * annualization))
                .flatten(),
        )
    } else {
        (None, None, None)
    };
    let first = bars.first().expect("validated non-empty bars").close_minor;
    let last = bars.last().expect("validated non-empty bars").close_minor;
    let price_benchmark_return_bps = ratio_bps(i128::from(last) - i128::from(first), first)?;
    Ok(Some(PerformanceMetrics {
        periods_per_year,
        observed_return_count: returns.len(),
        annualized_volatility_bps,
        sharpe_ratio_milli,
        sortino_ratio_milli,
        price_benchmark_return_bps,
        alpha_vs_price_benchmark_bps: total_return_bps - price_benchmark_return_bps,
        period_returns_ppm: returns
            .iter()
            .filter_map(|value| {
                let scaled = (value * 1_000_000.0).round();
                scaled
                    .is_finite()
                    .then(|| i64::try_from(scaled as i128).ok())
                    .flatten()
            })
            .collect(),
    }))
}

fn valid_metadata(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.len() <= 128 && !trimmed.chars().any(char::is_control)
}

fn validate_inputs(
    spec: &StrategySpec,
    bars: &[PriceBar],
    config: &BacktestConfig,
) -> Result<(), BacktestError> {
    let review = review_strategy_spec(spec);
    if !review.executable {
        return Err(error(
            BacktestErrorCode::InvalidStrategy,
            "검증되지 않았거나 해결되지 않은 항목이 있는 StrategySpec은 실행할 수 없습니다.",
        ));
    }
    if !valid_metadata(&config.experiment_id)
        || !valid_metadata(&config.dataset_id)
        || !valid_metadata(&config.code_version)
        || config.initial_cash_minor == 0
        || config.order_quantity == 0
        || config.quantity_scale == 0
    {
        return Err(error(
            BacktestErrorCode::InvalidConfig,
            "실험 식별자, 버전, 자금과 수량이 필요합니다.",
        ));
    }
    crate::simulation::validate_costs(config.costs).map_err(map_cost_error)?;

    let (_, slow_window, _) = signal_parts(&spec.entry_signal);
    if bars.len() <= slow_window + 1 || bars.len() > 5_000_000 {
        return Err(error(
            BacktestErrorCode::InsufficientData,
            "신호와 다음 시점 체결을 계산할 가격 봉이 부족합니다.",
        ));
    }

    let mut previous_start = None;
    let mut previous_end = None;
    for bar in bars {
        if bar.symbol != spec.symbol {
            return Err(error(
                BacktestErrorCode::MixedSymbol,
                "StrategySpec과 다른 종목의 가격 봉이 포함되었습니다.",
            ));
        }
        if bar.currency != spec.currency {
            return Err(error(
                BacktestErrorCode::InvalidBar,
                "StrategySpec과 가격 봉의 통화 단위가 일치하지 않습니다.",
            ));
        }
        if bar.period_start_ms >= bar.period_end_ms
            || bar.available_at_ms < bar.period_end_ms
            || bar.ingested_at_ms < bar.available_at_ms
            || bar.open_minor == 0
            || bar.high_minor == 0
            || bar.low_minor == 0
            || bar.close_minor == 0
            || bar.low_minor > bar.open_minor
            || bar.low_minor > bar.close_minor
            || bar.high_minor < bar.open_minor
            || bar.high_minor < bar.close_minor
            || bar.low_minor > bar.high_minor
            || bar.source.trim().is_empty()
            || bar.source.len() > 128
        {
            return Err(error(
                BacktestErrorCode::InvalidBar,
                "가격 봉의 시각 또는 가격이 유효하지 않습니다.",
            ));
        }
        if previous_start.is_some_and(|start| bar.period_start_ms <= start)
            || previous_end.is_some_and(|end| bar.period_start_ms < end)
        {
            return Err(error(
                BacktestErrorCode::DuplicateOrUnsortedBar,
                "가격 봉은 중복 없이 시간순으로 정렬되고 서로 겹치지 않아야 합니다.",
            ));
        }
        previous_start = Some(bar.period_start_ms);
        previous_end = Some(bar.period_end_ms);
    }
    Ok(())
}

fn comparable_buy_and_hold_baseline(
    bars: &[PriceBar],
    config: &BacktestConfig,
) -> Result<ComparableBaseline, BacktestError> {
    let first = bars.first().expect("validated non-empty bars");
    let last = bars.last().expect("validated non-empty bars");
    let buy = quote_execution_scaled(
        TradeSide::Buy,
        first.open_minor,
        config.order_quantity,
        config.quantity_scale,
        config.costs,
    )
    .map_err(map_cost_error)?;
    let sell = quote_execution_scaled(
        TradeSide::Sell,
        last.close_minor,
        config.order_quantity,
        config.quantity_scale,
        config.costs,
    )
    .map_err(map_cost_error)?;
    let debit = buy
        .notional_minor
        .checked_add(buy.fee_minor)
        .and_then(|value| value.checked_add(buy.tax_minor))
        .ok_or_else(|| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "기준선 매수 비용이 범위를 초과했습니다.",
            )
        })?;
    let credit = sell
        .notional_minor
        .checked_sub(sell.fee_minor)
        .and_then(|value| value.checked_sub(sell.tax_minor))
        .ok_or_else(|| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "기준선 매도 비용이 체결 금액을 초과했습니다.",
            )
        })?;
    let total_cost_minor = buy
        .fee_minor
        .checked_add(buy.tax_minor)
        .and_then(|value| value.checked_add(sell.fee_minor))
        .and_then(|value| value.checked_add(sell.tax_minor))
        .ok_or_else(|| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "기준선 비용 합계가 범위를 초과했습니다.",
            )
        })?;
    let (final_equity_minor, total_return_bps, detail) = if debit <= config.initial_cash_minor {
        let final_equity = config
            .initial_cash_minor
            .checked_sub(debit)
            .and_then(|value| value.checked_add(credit))
            .ok_or_else(|| {
                error(
                    BacktestErrorCode::ArithmeticOverflow,
                    "기준선 평가자산이 범위를 초과했습니다.",
                )
            })?;
        (
            Some(final_equity),
            Some(ratio_bps(
                i128::from(final_equity) - i128::from(config.initial_cash_minor),
                config.initial_cash_minor,
            )?),
            "전략과 동일한 종목·기간·통화·주문수량·비용 모델로 최초 시가 매수 후 최종 종가 매도했습니다.".to_owned(),
        )
    } else {
        (
            None,
            None,
            "동일 주문수량의 최초 시가 매수 금액이 초기자금을 초과해 기준선 수익률을 계산하지 못했습니다.".to_owned(),
        )
    };
    Ok(ComparableBaseline {
        name: "cost_aligned_buy_and_hold".to_owned(),
        symbol: first.symbol.clone(),
        currency: first.currency.clone(),
        first_period_start_ms: first.period_start_ms,
        last_period_end_ms: last.period_end_ms,
        input_bar_count: bars.len(),
        initial_cash_minor: config.initial_cash_minor,
        order_quantity: config.order_quantity,
        quantity_scale: config.quantity_scale,
        final_equity_minor,
        total_return_bps,
        total_cost_minor: Some(total_cost_minor),
        same_period: true,
        same_symbol: true,
        same_currency: true,
        same_cost_model: true,
        detail,
    })
}

fn validation_report(
    bars: &[PriceBar],
    config: &BacktestConfig,
) -> Result<BacktestValidationReport, BacktestError> {
    let baseline = comparable_buy_and_hold_baseline(bars, config)?;
    Ok(BacktestValidationReport {
        look_ahead: BiasCheck {
            status: ValidationStatus::Passed,
            detail: "봉의 availableAt/ingestedAt 순서를 검증하고 신호 다음 봉 시가 이전 가용성까지 강제했습니다.".to_owned(),
        },
        survivorship_bias: BiasCheck {
            status: ValidationStatus::NeedsReview,
            detail: "단일 종목 데이터만으로는 당시 투자 유니버스 편입·상장폐지 이력을 증명할 수 없습니다. 다종목 비교 전에 시점별 유니버스 스냅샷이 필요합니다.".to_owned(),
        },
        data_snooping: BiasCheck {
            status: ValidationStatus::NeedsReview,
            detail: "이 단일 실행은 탐색한 전략·파라미터 총횟수를 알 수 없습니다. 저장 실험 카탈로그와 OOS/워크포워드 결과를 함께 검토해야 합니다.".to_owned(),
        },
        comparable_baseline: baseline,
    })
}

pub fn run_backtest(
    spec: &StrategySpec,
    bars: &[PriceBar],
    config: &BacktestConfig,
) -> Result<BacktestResult, BacktestError> {
    run_backtest_with_risk(spec, bars, config, config.risk_limits)
}

pub fn run_backtest_with_risk(
    spec: &StrategySpec,
    bars: &[PriceBar],
    config: &BacktestConfig,
    risk: Option<BacktestRiskLimits>,
) -> Result<BacktestResult, BacktestError> {
    validate_inputs(spec, bars, config)?;
    if risk.is_some_and(|limits| {
        limits.stop_loss_bps == 0
            || limits.stop_loss_bps >= 10_000
            || limits.take_profit_bps == 0
            || limits.daily_loss_limit_minor == 0
    }) {
        return Err(error(
            BacktestErrorCode::InvalidConfig,
            "손절·익절·일일손실 위험 한도가 유효하지 않습니다.",
        ));
    }
    let last_bar = bars.last().ok_or_else(|| {
        error(
            BacktestErrorCode::InsufficientData,
            "백테스트에 사용할 가격 봉이 없습니다.",
        )
    })?;

    let mut cash = config.initial_cash_minor;
    let mut position: Option<OpenPosition> = None;
    let mut pending: Option<(PendingSignal, u64)> = None;
    let mut fills = Vec::new();
    let mut trades = Vec::new();
    let mut peak_equity = config.initial_cash_minor;
    let mut equity_curve = vec![config.initial_cash_minor];
    let mut max_drawdown_bps = 0_u64;
    let mut loss_day = None;
    let mut daily_realized_loss_minor = 0_u64;

    for (index, bar) in bars.iter().enumerate() {
        let current_day = bar.period_start_ms / 86_400_000;
        if loss_day != Some(current_day) {
            loss_day = Some(current_day);
            daily_realized_loss_minor = 0;
        }
        if let Some((signal, signal_available_at)) = pending.take() {
            if signal_available_at > bar.period_start_ms {
                return Err(error(
                    BacktestErrorCode::LookAheadRisk,
                    "신호가 실제로 이용 가능해지기 전에 다음 가격 봉 시가로 체결할 수 없습니다.",
                ));
            }

            match signal {
                PendingSignal::Enter if position.is_none() => {
                    let quote = quote_execution_scaled(
                        TradeSide::Buy,
                        bar.open_minor,
                        config.order_quantity,
                        config.quantity_scale,
                        config.costs,
                    )
                    .map_err(map_cost_error)?;
                    let debit = quote
                        .notional_minor
                        .checked_add(quote.fee_minor)
                        .and_then(|value| value.checked_add(quote.tax_minor))
                        .ok_or_else(|| {
                            error(
                                BacktestErrorCode::ArithmeticOverflow,
                                "매수 결제 금액이 범위를 초과했습니다.",
                            )
                        })?;
                    cash = cash.checked_sub(debit).ok_or_else(|| {
                        error(
                            BacktestErrorCode::InsufficientCash,
                            "명시된 주문 수량을 체결할 가상 예수금이 부족합니다.",
                        )
                    })?;
                    position = Some(OpenPosition {
                        quantity: config.order_quantity,
                        opened_at_ms: bar.period_start_ms,
                        entry_price_minor: quote.execution_price_minor,
                        cost_basis_minor: debit,
                    });
                    fills.push(BacktestFill {
                        side: TradeSide::Buy,
                        period_start_ms: bar.period_start_ms,
                        reference_price_minor: bar.open_minor,
                        execution_price_minor: quote.execution_price_minor,
                        quantity: config.order_quantity,
                        fee_minor: quote.fee_minor,
                        tax_minor: quote.tax_minor,
                    });
                }
                PendingSignal::Exit if position.is_some() => {
                    let open = position.take().ok_or_else(|| {
                        error(
                            BacktestErrorCode::InvalidConfig,
                            "청산할 내부 포지션 상태를 찾지 못했습니다.",
                        )
                    })?;
                    let quote = quote_execution_scaled(
                        TradeSide::Sell,
                        bar.open_minor,
                        open.quantity,
                        config.quantity_scale,
                        config.costs,
                    )
                    .map_err(map_cost_error)?;
                    let credit = quote
                        .notional_minor
                        .checked_sub(quote.fee_minor)
                        .and_then(|value| value.checked_sub(quote.tax_minor))
                        .ok_or_else(|| {
                            error(
                                BacktestErrorCode::ArithmeticOverflow,
                                "매도 결제 금액이 범위를 초과했습니다.",
                            )
                        })?;
                    cash = cash.checked_add(credit).ok_or_else(|| {
                        error(
                            BacktestErrorCode::ArithmeticOverflow,
                            "가상 예수금이 범위를 초과했습니다.",
                        )
                    })?;
                    trades.push(BacktestTrade {
                        opened_at_ms: open.opened_at_ms,
                        closed_at_ms: bar.period_start_ms,
                        quantity: open.quantity,
                        entry_price_minor: open.entry_price_minor,
                        exit_price_minor: quote.execution_price_minor,
                        pnl_minor: signed_difference(credit, open.cost_basis_minor)?,
                        exit_kind: BacktestExitKind::Signal,
                    });
                    fills.push(BacktestFill {
                        side: TradeSide::Sell,
                        period_start_ms: bar.period_start_ms,
                        reference_price_minor: bar.open_minor,
                        execution_price_minor: quote.execution_price_minor,
                        quantity: open.quantity,
                        fee_minor: quote.fee_minor,
                        tax_minor: quote.tax_minor,
                    });
                }
                _ => {}
            }
        }

        if let (Some(open), Some(limits)) = (position, risk) {
            let stop_price = u64::try_from(
                u128::from(open.entry_price_minor) * u128::from(10_000 - limits.stop_loss_bps)
                    / 10_000,
            )
            .map_err(|_| {
                error(
                    BacktestErrorCode::ArithmeticOverflow,
                    "손절 가격 계산이 범위를 초과했습니다.",
                )
            })?;
            let take_price = u64::try_from(
                u128::from(open.entry_price_minor) * u128::from(10_000 + limits.take_profit_bps)
                    / 10_000,
            )
            .map_err(|_| {
                error(
                    BacktestErrorCode::ArithmeticOverflow,
                    "익절 가격 계산이 범위를 초과했습니다.",
                )
            })?;
            let trigger = if bar.low_minor <= stop_price {
                Some((stop_price.max(1), BacktestExitKind::StopLoss))
            } else if bar.high_minor >= take_price {
                Some((take_price, BacktestExitKind::TakeProfit))
            } else {
                None
            };
            if let Some((reference_price, exit_kind)) = trigger {
                let quote = quote_execution_scaled(
                    TradeSide::Sell,
                    reference_price,
                    open.quantity,
                    config.quantity_scale,
                    config.costs,
                )
                .map_err(map_cost_error)?;
                let credit = quote
                    .notional_minor
                    .checked_sub(quote.fee_minor)
                    .and_then(|value| value.checked_sub(quote.tax_minor))
                    .ok_or_else(|| {
                        error(
                            BacktestErrorCode::ArithmeticOverflow,
                            "위험 청산 금액이 범위를 초과했습니다.",
                        )
                    })?;
                cash = cash.checked_add(credit).ok_or_else(|| {
                    error(
                        BacktestErrorCode::ArithmeticOverflow,
                        "위험 청산 후 예수금이 범위를 초과했습니다.",
                    )
                })?;
                let pnl = signed_difference(credit, open.cost_basis_minor)?;
                if pnl < 0 {
                    daily_realized_loss_minor =
                        daily_realized_loss_minor.saturating_add(pnl.unsigned_abs());
                }
                trades.push(BacktestTrade {
                    opened_at_ms: open.opened_at_ms,
                    closed_at_ms: bar.period_start_ms,
                    quantity: open.quantity,
                    entry_price_minor: open.entry_price_minor,
                    exit_price_minor: quote.execution_price_minor,
                    pnl_minor: pnl,
                    exit_kind,
                });
                fills.push(BacktestFill {
                    side: TradeSide::Sell,
                    period_start_ms: bar.period_start_ms,
                    reference_price_minor: reference_price,
                    execution_price_minor: quote.execution_price_minor,
                    quantity: open.quantity,
                    fee_minor: quote.fee_minor,
                    tax_minor: quote.tax_minor,
                });
                position = None;
                pending = None;
            }
        }

        let marked_position = match position {
            Some(open) => u64::try_from(
                u128::from(open.quantity) * u128::from(bar.close_minor)
                    / u128::from(config.quantity_scale),
            )
            .map_err(|_| {
                error(
                    BacktestErrorCode::ArithmeticOverflow,
                    "평가 금액이 범위를 초과했습니다.",
                )
            })?,
            None => 0,
        };
        let equity = cash.checked_add(marked_position).ok_or_else(|| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "평가 자산이 범위를 초과했습니다.",
            )
        })?;
        equity_curve.push(equity);
        peak_equity = peak_equity.max(equity);
        let drawdown = peak_equity - equity;
        let drawdown_bps = u64::try_from(u128::from(drawdown) * 10_000 / u128::from(peak_equity))
            .map_err(|_| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "낙폭 계산이 범위를 초과했습니다.",
            )
        })?;
        max_drawdown_bps = max_drawdown_bps.max(drawdown_bps);

        if index + 1 < bars.len() {
            let daily_loss_blocked = risk
                .is_some_and(|limits| daily_realized_loss_minor >= limits.daily_loss_limit_minor);
            if position.is_none()
                && !daily_loss_blocked
                && crossed(bars, index, &spec.entry_signal)?
            {
                pending = Some((PendingSignal::Enter, bar.available_at_ms));
            } else if position.is_some() && crossed(bars, index, &spec.exit_signal)? {
                pending = Some((PendingSignal::Exit, bar.available_at_ms));
            }
        }
    }

    if config.close_open_position_at_end {
        if let Some(open) = position.take() {
            let quote = quote_execution_scaled(
                TradeSide::Sell,
                last_bar.close_minor,
                open.quantity,
                config.quantity_scale,
                config.costs,
            )
            .map_err(map_cost_error)?;
            let credit = quote
                .notional_minor
                .checked_sub(quote.fee_minor)
                .and_then(|value| value.checked_sub(quote.tax_minor))
                .ok_or_else(|| {
                    error(
                        BacktestErrorCode::ArithmeticOverflow,
                        "종료 청산 금액이 범위를 초과했습니다.",
                    )
                })?;
            cash = cash.checked_add(credit).ok_or_else(|| {
                error(
                    BacktestErrorCode::ArithmeticOverflow,
                    "종료 예수금이 범위를 초과했습니다.",
                )
            })?;
            trades.push(BacktestTrade {
                opened_at_ms: open.opened_at_ms,
                closed_at_ms: last_bar.available_at_ms,
                quantity: open.quantity,
                entry_price_minor: open.entry_price_minor,
                exit_price_minor: quote.execution_price_minor,
                pnl_minor: signed_difference(credit, open.cost_basis_minor)?,
                exit_kind: BacktestExitKind::EndOfPeriod,
            });
            fills.push(BacktestFill {
                side: TradeSide::Sell,
                period_start_ms: last_bar.available_at_ms,
                reference_price_minor: last_bar.close_minor,
                execution_price_minor: quote.execution_price_minor,
                quantity: open.quantity,
                fee_minor: quote.fee_minor,
                tax_minor: quote.tax_minor,
            });
        }
    }

    let final_mark = match position {
        Some(open) => u64::try_from(
            u128::from(open.quantity) * u128::from(last_bar.close_minor)
                / u128::from(config.quantity_scale),
        )
        .map_err(|_| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "최종 평가 금액이 범위를 초과했습니다.",
            )
        })?,
        None => 0,
    };
    let final_equity = cash.checked_add(final_mark).ok_or_else(|| {
        error(
            BacktestErrorCode::ArithmeticOverflow,
            "최종 평가 자산이 범위를 초과했습니다.",
        )
    })?;
    peak_equity = peak_equity.max(final_equity);
    let terminal_drawdown = peak_equity - final_equity;
    let terminal_drawdown_bps = u64::try_from(
        u128::from(terminal_drawdown) * 10_000 / u128::from(peak_equity),
    )
    .map_err(|_| {
        error(
            BacktestErrorCode::ArithmeticOverflow,
            "최종 낙폭 계산이 범위를 초과했습니다.",
        )
    })?;
    max_drawdown_bps = max_drawdown_bps.max(terminal_drawdown_bps);
    let realized_pnl = trades.iter().try_fold(0_i64, |total, trade| {
        total.checked_add(trade.pnl_minor).ok_or_else(|| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "누적 실현손익이 범위를 초과했습니다.",
            )
        })
    })?;
    let wins = trades.iter().filter(|trade| trade.pnl_minor > 0).count();
    let gross_profit =
        trades
            .iter()
            .filter(|trade| trade.pnl_minor > 0)
            .try_fold(0_u128, |total, trade| {
                total.checked_add(trade.pnl_minor as u128).ok_or_else(|| {
                    error(
                        BacktestErrorCode::ArithmeticOverflow,
                        "총이익 계산이 범위를 초과했습니다.",
                    )
                })
            })?;
    let gross_loss =
        trades
            .iter()
            .filter(|trade| trade.pnl_minor < 0)
            .try_fold(0_u128, |total, trade| {
                total
                    .checked_add(trade.pnl_minor.unsigned_abs() as u128)
                    .ok_or_else(|| {
                        error(
                            BacktestErrorCode::ArithmeticOverflow,
                            "총손실 계산이 범위를 초과했습니다.",
                        )
                    })
            })?;
    let profit_factor_milli = if gross_loss > 0 {
        let scaled = gross_profit
            .checked_mul(1_000)
            .and_then(|value| value.checked_div(gross_loss))
            .ok_or_else(|| {
                error(
                    BacktestErrorCode::ArithmeticOverflow,
                    "Profit Factor 계산이 범위를 초과했습니다.",
                )
            })?;
        Some(u64::try_from(scaled).map_err(|_| {
            error(
                BacktestErrorCode::ArithmeticOverflow,
                "Profit Factor가 지원 범위를 초과했습니다.",
            )
        })?)
    } else {
        None
    };

    if let Some(last_equity) = equity_curve.last_mut() {
        *last_equity = final_equity;
    }
    let total_return_bps = ratio_bps(
        i128::from(final_equity) - i128::from(config.initial_cash_minor),
        config.initial_cash_minor,
    )?;
    let performance = performance_metrics(spec, bars, &equity_curve, total_return_bps)?;
    let pattern_probability = analyze_pattern_probabilities(bars);
    let robustness = crate::quant_risk::backtest_robustness(
        &config.experiment_id,
        &config.dataset_id,
        config.initial_cash_minor,
        &trades,
    );
    let validation = validation_report(bars, config)?;

    Ok(BacktestResult {
        experiment_id: config.experiment_id.clone(),
        dataset_id: config.dataset_id.clone(),
        strategy_id: spec.strategy_id.clone(),
        code_version: config.code_version.clone(),
        input_bar_count: bars.len(),
        first_period_start_ms: bars[0].period_start_ms,
        last_available_at_ms: last_bar.available_at_ms,
        last_ingested_at_ms: last_bar.ingested_at_ms,
        initial_cash_minor: config.initial_cash_minor,
        final_cash_minor: cash,
        final_equity_minor: final_equity,
        realized_pnl_minor: realized_pnl,
        total_return_bps,
        max_drawdown_bps,
        completed_trade_count: trades.len(),
        win_rate_bps: (!trades.is_empty()).then(|| (wins as u64) * 10_000 / trades.len() as u64),
        average_trade_pnl_minor: (!trades.is_empty()).then(|| realized_pnl / trades.len() as i64),
        profit_factor_milli,
        open_position_quantity: position.map_or(0, |open| open.quantity),
        performance,
        pattern_probability,
        robustness: Some(robustness),
        validation,
        fills,
        trades,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::{Market, StrategySpec};

    fn strategy() -> StrategySpec {
        StrategySpec {
            schema_version: "1".to_owned(),
            strategy_id: "fixture-ma-cross".to_owned(),
            name: "fixture 이동평균".to_owned(),
            market: Market::Korea,
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            hypothesis: "고정 데이터에서 교차 신호의 시점 정합성을 검증한다.".to_owned(),
            source_evidence_ids: vec!["fixture-evidence".to_owned()],
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
            limitations: vec!["테스트 fixture".to_owned()],
            unknowns: Vec::new(),
        }
    }

    fn bars() -> Vec<PriceBar> {
        [10, 9, 8, 9, 10, 12, 15, 18, 17, 16, 15]
            .into_iter()
            .enumerate()
            .map(|(index, price)| {
                let start = index as u64 * 1_000;
                PriceBar {
                    symbol: "005930".to_owned(),
                    currency: "KRW".to_owned(),
                    source: "fixture".to_owned(),
                    period_start_ms: start,
                    period_end_ms: start + 900,
                    available_at_ms: start + 950,
                    ingested_at_ms: start + 960,
                    open_minor: price * 1_000,
                    high_minor: price * 1_000,
                    low_minor: price * 1_000,
                    close_minor: price * 1_000,
                    volume: 1_000,
                }
            })
            .collect()
    }

    fn config(costs: TradingCosts) -> BacktestConfig {
        BacktestConfig {
            experiment_id: "experiment-001".to_owned(),
            dataset_id: "fixture-v1".to_owned(),
            code_version: "test".to_owned(),
            initial_cash_minor: 1_000_000,
            order_quantity: 10,
            quantity_scale: 1,
            close_open_position_at_end: true,
            costs,
            risk_limits: None,
        }
    }

    fn zero_costs() -> TradingCosts {
        TradingCosts {
            buy_fee_bps: 0.0,
            sell_fee_bps: 0.0,
            sell_tax_bps: 0.0,
            slippage_bps: 0.0,
        }
    }

    #[test]
    fn produces_the_same_result_for_the_same_point_in_time_inputs() {
        let first = run_backtest(&strategy(), &bars(), &config(zero_costs())).unwrap();
        let second = run_backtest(&strategy(), &bars(), &config(zero_costs())).unwrap();

        assert_eq!(first.final_equity_minor, second.final_equity_minor);
        assert_eq!(first.realized_pnl_minor, second.realized_pnl_minor);
        assert_eq!(first.fills.len(), 2);
        assert_eq!(first.completed_trade_count, 1);
        assert_eq!(first.win_rate_bps, Some(10_000));
        assert!(first.realized_pnl_minor > 0);
    }

    #[test]
    fn explicit_costs_reduce_the_reproduced_result_without_defining_a_pass_line() {
        let without_costs = run_backtest(&strategy(), &bars(), &config(zero_costs())).unwrap();
        let with_costs = run_backtest(
            &strategy(),
            &bars(),
            &config(TradingCosts {
                buy_fee_bps: 5.0,
                sell_fee_bps: 5.0,
                sell_tax_bps: 15.0,
                slippage_bps: 10.0,
            }),
        )
        .unwrap();

        assert!(with_costs.final_equity_minor < without_costs.final_equity_minor);
        assert!(with_costs.realized_pnl_minor < without_costs.realized_pnl_minor);
    }

    #[test]
    fn reports_daily_risk_adjusted_metrics_and_price_benchmark_alpha() {
        let mut fixture = bars();
        for (index, bar) in fixture.iter_mut().enumerate() {
            let start = index as u64 * 86_400_000;
            bar.period_start_ms = start;
            bar.period_end_ms = start + 80_000_000;
            bar.available_at_ms = start + 81_000_000;
            bar.ingested_at_ms = start + 82_000_000;
        }
        let result = run_backtest(&strategy(), &fixture, &config(zero_costs())).unwrap();
        let performance = result.performance.expect("daily performance metrics");
        assert_eq!(performance.periods_per_year, 252);
        assert_eq!(performance.observed_return_count, fixture.len());
        assert_eq!(
            performance.period_returns_ppm.len(),
            performance.observed_return_count
        );
        assert_eq!(
            performance.alpha_vs_price_benchmark_bps,
            result.total_return_bps - performance.price_benchmark_return_bps
        );
        assert!(performance.annualized_volatility_bps.is_some());
    }

    #[test]
    fn reports_bias_limits_and_a_cost_aligned_same_period_baseline() {
        let costs = TradingCosts {
            buy_fee_bps: 5.0,
            sell_fee_bps: 5.0,
            sell_tax_bps: 15.0,
            slippage_bps: 10.0,
        };
        let result = run_backtest(&strategy(), &bars(), &config(costs)).expect("backtest");
        let audit = result.validation;
        assert!(matches!(audit.look_ahead.status, ValidationStatus::Passed));
        assert!(matches!(
            audit.survivorship_bias.status,
            ValidationStatus::NeedsReview
        ));
        assert!(matches!(
            audit.data_snooping.status,
            ValidationStatus::NeedsReview
        ));
        assert!(audit.comparable_baseline.same_period);
        assert!(audit.comparable_baseline.same_symbol);
        assert!(audit.comparable_baseline.same_currency);
        assert!(audit.comparable_baseline.same_cost_model);
        assert_eq!(audit.comparable_baseline.input_bar_count, bars().len());
        assert!(audit.comparable_baseline.total_cost_minor.unwrap() > 0);
        assert!(audit.comparable_baseline.total_return_bps.is_some());
    }

    #[test]
    fn ohlc_risk_overlay_closes_at_the_conservative_stop_price() {
        let mut fixture = bars();
        // 진입은 6번째 봉 시가에서 발생한다. 같은 봉에서 손절과 익절을 모두 건드리면
        // 일봉 내부 순서를 알 수 없으므로 보수적으로 손절을 먼저 적용한다.
        fixture[5].low_minor = 10_000;
        fixture[5].high_minor = 14_000;
        let result = run_backtest_with_risk(
            &strategy(),
            &fixture,
            &config(zero_costs()),
            Some(BacktestRiskLimits {
                stop_loss_bps: 500,
                take_profit_bps: 1_000,
                daily_loss_limit_minor: 1_000_000,
            }),
        )
        .expect("risk backtest");
        assert_eq!(result.completed_trade_count, 1);
        assert_eq!(result.trades[0].exit_price_minor, 11_400);
        assert!(result.trades[0].pnl_minor < 0);
    }

    #[test]
    fn rejects_a_signal_that_was_not_available_before_the_next_open() {
        let mut delayed = bars();
        delayed[4].available_at_ms = delayed[5].period_start_ms + 1;
        delayed[4].ingested_at_ms = delayed[4].available_at_ms;

        let result = run_backtest(&strategy(), &delayed, &config(zero_costs()));

        assert_eq!(result.unwrap_err().code, BacktestErrorCode::LookAheadRisk);
    }

    #[test]
    fn rejects_unsorted_duplicate_and_mixed_symbol_data() {
        let mut duplicate = bars();
        duplicate[4].period_start_ms = duplicate[3].period_start_ms;
        assert_eq!(
            run_backtest(&strategy(), &duplicate, &config(zero_costs()))
                .unwrap_err()
                .code,
            BacktestErrorCode::DuplicateOrUnsortedBar
        );

        let mut mixed = bars();
        mixed[4].symbol = "AAPL".to_owned();
        assert_eq!(
            run_backtest(&strategy(), &mixed, &config(zero_costs()))
                .unwrap_err()
                .code,
            BacktestErrorCode::MixedSymbol
        );

        let mut wrong_currency = bars();
        wrong_currency[4].currency = "USD".to_owned();
        assert_eq!(
            run_backtest(&strategy(), &wrong_currency, &config(zero_costs()))
                .unwrap_err()
                .code,
            BacktestErrorCode::InvalidBar
        );
    }
}
