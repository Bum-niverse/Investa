use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

use crate::{
    backtest::{run_backtest, BacktestConfig, BacktestResult, BacktestRiskLimits, PriceBar},
    persistence::{now_ms, PersistBacktest, PersistenceBridge},
    research::{review_strategy_spec, CrossDirection, ResearchReport, SignalSpec},
    simulation::TradingCosts,
};

const MINIMUM_OOS_TRADE_COUNT: usize = 200;
const REGIME_LOOKBACK_BARS: usize = 20;
const PROMOTION_POLICY_VERSION: &str = "paper-review-v1";
const MINIMUM_WIN_RATE_BPS: u64 = 5_500;
const MINIMUM_PROFIT_FACTOR_MILLI: u64 = 1_300;
const MAXIMUM_DRAWDOWN_BPS: u64 = 1_200;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneExperimentRequest {
    pub source_experiment_id: String,
    pub fast_window: usize,
    pub slow_window: usize,
    pub initial_cash_minor: u64,
    pub order_quantity: u64,
    pub close_open_position_at_end: bool,
    pub buy_fee_bps: f64,
    pub sell_fee_bps: f64,
    pub sell_tax_bps: f64,
    pub slippage_bps: f64,
    pub stop_loss_bps: Option<u64>,
    pub take_profit_bps: Option<u64>,
    pub daily_loss_limit_minor: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentComparison {
    pub source_experiment_id: String,
    pub cloned_experiment_id: String,
    pub source_config: BacktestConfig,
    pub cloned_config: BacktestConfig,
    pub source_result: BacktestResult,
    pub cloned_result: BacktestResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardRequest {
    pub source_experiment_id: String,
    pub fold_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardMetrics {
    pub total_return_bps: i64,
    pub max_drawdown_bps: u64,
    pub completed_trade_count: usize,
    pub win_rate_bps: Option<u64>,
    pub profit_factor_milli: Option<u64>,
    pub expected_trade_pnl_minor: Option<i64>,
    #[serde(default)]
    pub realized_pnl_minor: i64,
    #[serde(default)]
    pub gross_profit_minor: u64,
    #[serde(default)]
    pub gross_loss_minor: u64,
    #[serde(default)]
    pub alpha_vs_price_benchmark_bps: Option<i64>,
    #[serde(default)]
    pub periods_per_year: Option<u64>,
    #[serde(default)]
    pub period_returns_ppm: Vec<i64>,
    pub turnover_bps: u64,
    pub exposure_bps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardFold {
    pub fold_number: usize,
    pub training_bar_count: usize,
    pub oos_bar_count: usize,
    pub training_end_ms: u64,
    pub oos_start_ms: u64,
    pub oos_end_ms: u64,
    pub training: WalkForwardMetrics,
    pub out_of_sample: WalkForwardMetrics,
    pub regimes: Vec<RegimePerformance>,
    pub unclassified_trade_count: usize,
    #[serde(default)]
    pub state_model: StateModelDiagnostic,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateModelDiagnostic {
    pub model_id: String,
    pub training_transition_count: usize,
    pub oos_transition_count: usize,
    pub low_volatility_persistence_bps: Option<u64>,
    pub high_volatility_persistence_bps: Option<u64>,
    pub maximum_transition_uncertainty_bps: Option<u64>,
    pub transition_model_log_loss_milli: Option<u64>,
    pub independent_state_baseline_log_loss_milli: Option<u64>,
    pub beats_independent_baseline: bool,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedRegime {
    Bullish,
    Bearish,
    Sideways,
    HighVolatility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegimePerformance {
    pub regime: ObservedRegime,
    pub completed_trade_count: usize,
    pub winning_trade_count: usize,
    pub realized_pnl_minor: i64,
    pub win_rate_bps: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardReport {
    #[serde(default)]
    pub validation_run_id: String,
    #[serde(default)]
    pub created_at_ms: u64,
    pub source_experiment_id: String,
    #[serde(default)]
    pub strategy_trial_count: usize,
    pub fold_count: usize,
    pub initial_training_bar_count: usize,
    pub positive_oos_fold_count: usize,
    pub largest_absolute_oos_return_share_bps: u64,
    pub oos_return_spread_bps: u64,
    pub total_oos_trade_count: usize,
    pub minimum_oos_trade_count: usize,
    pub meets_research_sample_minimum: bool,
    pub promotion_blockers: Vec<String>,
    #[serde(default)]
    pub promotion_evaluation: PromotionEvaluation,
    #[serde(default)]
    pub overfit_diagnostics: OverfitDiagnostics,
    pub folds: Vec<WalkForwardFold>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionEvaluation {
    pub policy_version: String,
    pub eligible_for_paper_review: bool,
    pub checks: Vec<PromotionCheck>,
    pub warning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotionCheck {
    pub check_id: String,
    pub label: String,
    pub passed: bool,
    pub observed: String,
    pub required: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverfitDiagnostics {
    pub comparable_strategy_count: usize,
    pub evaluated_partition_count: usize,
    pub probability_of_backtest_overfitting_bps: Option<u64>,
    pub deflated_sharpe_ratio_milli: Option<i64>,
    pub minimum_track_record_length: Option<usize>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentBiasAudit {
    pub experiment_id: String,
    pub dataset_id: String,
    pub local_strategy_trial_count: usize,
    pub walk_forward_validation_count: usize,
    pub oos_fold_count: usize,
    pub oos_trade_count: usize,
    pub data_snooping_status: String,
    pub survivorship_bias_status: String,
    pub catalog_completeness: String,
    pub universe_membership_evidence: String,
    pub details: Vec<String>,
}

struct StoredExperiment {
    report: ResearchReport,
    config: BacktestConfig,
    result: BacktestResult,
    stored_bars: Vec<PriceBar>,
    backtest_bars: Vec<PriceBar>,
    provider: String,
    interval: String,
    adjusted: bool,
    warnings: Vec<String>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn requested_risk(request: &CloneExperimentRequest) -> Result<Option<BacktestRiskLimits>, String> {
    match (
        request.stop_loss_bps,
        request.take_profit_bps,
        request.daily_loss_limit_minor,
    ) {
        (None, None, None) => Ok(None),
        (Some(stop_loss_bps), Some(take_profit_bps), Some(daily_loss_limit_minor)) => {
            Ok(Some(BacktestRiskLimits {
                stop_loss_bps,
                take_profit_bps,
                daily_loss_limit_minor,
            }))
        }
        _ => Err("손절·익절·일일손실 한도는 모두 입력하거나 모두 비워야 합니다.".to_owned()),
    }
}

fn validate_request(
    request: &CloneExperimentRequest,
) -> Result<Option<BacktestRiskLimits>, String> {
    if !valid_id(&request.source_experiment_id) {
        return Err("복제할 실험 ID가 올바르지 않습니다.".to_owned());
    }
    if request.fast_window == 0
        || request.slow_window <= request.fast_window
        || request.slow_window > 10_000
        || request.initial_cash_minor == 0
        || request.order_quantity == 0
    {
        return Err("이동평균 기간, 초기자금과 주문 수량을 확인해 주세요.".to_owned());
    }
    requested_risk(request)
}

fn load_experiment(
    bridge: &PersistenceBridge,
    experiment_id: &str,
) -> Result<StoredExperiment, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "저장된 실험 조회 잠금을 획득하지 못했습니다.".to_owned())?;
    let stored: (String, String) = connection
        .query_row(
            "SELECT b.record_json, d.bars_json FROM backtest_runs b JOIN datasets d ON d.dataset_id = b.dataset_id WHERE b.experiment_id = ?1",
            params![experiment_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("저장된 실험을 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "복제할 저장 실험을 찾지 못했습니다.".to_owned())?;
    drop(connection);

    let record: Value = serde_json::from_str(&stored.0)
        .map_err(|error| format!("저장된 실험을 해석하지 못했습니다: {error}"))?;
    let dataset: Value = serde_json::from_str(&stored.1)
        .map_err(|error| format!("저장된 데이터셋을 해석하지 못했습니다: {error}"))?;
    let field = |name: &str| {
        record
            .get(name)
            .cloned()
            .ok_or_else(|| format!("저장된 실험에 {name} 항목이 없습니다."))
    };
    let stored_bars: Vec<PriceBar> = serde_json::from_value(
        dataset
            .get("bars")
            .cloned()
            .ok_or_else(|| "저장된 데이터셋에 가격봉이 없습니다.".to_owned())?,
    )
    .map_err(|error| format!("저장된 가격봉을 해석하지 못했습니다: {error}"))?;
    let mut backtest_bars = stored_bars.clone();
    for bar in &mut backtest_bars {
        if bar.high_minor == 0 {
            bar.high_minor = bar.open_minor.max(bar.close_minor);
        }
        if bar.low_minor == 0 {
            bar.low_minor = bar.open_minor.min(bar.close_minor);
        }
    }
    Ok(StoredExperiment {
        report: serde_json::from_value(field("report")?)
            .map_err(|error| format!("연구 보고서를 해석하지 못했습니다: {error}"))?,
        config: serde_json::from_value(field("config")?)
            .map_err(|error| format!("백테스트 설정을 해석하지 못했습니다: {error}"))?,
        result: serde_json::from_value(field("result")?)
            .map_err(|error| format!("백테스트 결과를 해석하지 못했습니다: {error}"))?,
        stored_bars,
        backtest_bars,
        provider: record
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("STORED")
            .to_owned(),
        interval: record
            .get("interval")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned(),
        adjusted: record
            .get("adjusted")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        warnings: record
            .get("warnings")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| format!("저장된 경고를 해석하지 못했습니다: {error}"))?
            .unwrap_or_default(),
    })
}

fn execute_clone(
    bridge: &PersistenceBridge,
    request: CloneExperimentRequest,
) -> Result<ExperimentComparison, String> {
    let risk_limits = validate_request(&request)?;
    let source = load_experiment(bridge, &request.source_experiment_id)?;
    let suffix = Uuid::new_v4().simple().to_string();
    let cloned_experiment_id = format!("experiment-clone-{}", &suffix[..16]);
    let mut report = source.report.clone();
    report.trace_id = format!("trace-clone-{}", &suffix[..16]);
    report.strategy_candidate.strategy_id = format!("strategy-clone-{}", &suffix[..16]);
    report.strategy_candidate.entry_signal = SignalSpec::MovingAverageCross {
        fast_window: request.fast_window,
        slow_window: request.slow_window,
        direction: CrossDirection::Above,
    };
    report.strategy_candidate.exit_signal = SignalSpec::MovingAverageCross {
        fast_window: request.fast_window,
        slow_window: request.slow_window,
        direction: CrossDirection::Below,
    };
    if report.strategy_candidate.limitations.len() < 64 {
        report.strategy_candidate.limitations.push(format!(
            "저장 실험 {}의 동일 데이터셋을 재사용한 파라미터 비교 실험입니다.",
            request.source_experiment_id
        ));
    }
    let review = review_strategy_spec(&report.strategy_candidate);
    if !review.executable {
        return Err("복제한 전략 계약이 실행 가능 상태가 아닙니다.".to_owned());
    }
    let cloned_config = BacktestConfig {
        experiment_id: cloned_experiment_id.clone(),
        dataset_id: source.config.dataset_id.clone(),
        code_version: env!("CARGO_PKG_VERSION").to_owned(),
        initial_cash_minor: request.initial_cash_minor,
        order_quantity: request.order_quantity,
        quantity_scale: source.config.quantity_scale,
        close_open_position_at_end: request.close_open_position_at_end,
        costs: TradingCosts {
            buy_fee_bps: request.buy_fee_bps,
            sell_fee_bps: request.sell_fee_bps,
            sell_tax_bps: request.sell_tax_bps,
            slippage_bps: request.slippage_bps,
        },
        risk_limits,
    };
    let cloned_result = run_backtest(
        &report.strategy_candidate,
        &source.backtest_bars,
        &cloned_config,
    )
    .map_err(|error| format!("복제 실험 실행에 실패했습니다: {}", error.message))?;
    let mut warnings = source.warnings.clone();
    warnings.push(format!(
        "원본 실험 {}은 변경하지 않았으며 복제본은 연구 실험으로 저장했습니다.",
        request.source_experiment_id
    ));
    bridge.persist_backtest(PersistBacktest {
        report: &report,
        review: &review,
        // 동일 dataset ID는 저장된 원문과 byte-equivalent JSON이어야 한다. 구버전
        // OHLC 보정본은 계산에만 사용하고 데이터셋 원문은 절대 덮어쓰지 않는다.
        bars: &source.stored_bars,
        config: &cloned_config,
        result: &cloned_result,
        provider: &source.provider,
        interval: &source.interval,
        adjusted: source.adjusted,
        warnings: &warnings,
        requested_at_ms: Some(now_ms()?),
        classification: "research_experiment",
    })?;
    Ok(ExperimentComparison {
        source_experiment_id: request.source_experiment_id,
        cloned_experiment_id,
        source_config: source.config,
        cloned_config,
        source_result: source.result,
        cloned_result,
    })
}

fn result_metrics(
    result: &BacktestResult,
    bars: &[PriceBar],
    config: &BacktestConfig,
) -> WalkForwardMetrics {
    let turnover_notional = result.fills.iter().fold(0_u128, |total, fill| {
        total.saturating_add(
            u128::from(fill.execution_price_minor) * u128::from(fill.quantity)
                / u128::from(config.quantity_scale),
        )
    });
    let turnover_bps = u64::try_from(
        turnover_notional.saturating_mul(10_000) / u128::from(config.initial_cash_minor),
    )
    .unwrap_or(u64::MAX);
    let total_duration = bars
        .last()
        .zip(bars.first())
        .map(|(last, first)| last.period_end_ms.saturating_sub(first.period_start_ms))
        .unwrap_or(0);
    let exposed_duration = result.trades.iter().fold(0_u128, |total, trade| {
        total.saturating_add(u128::from(
            trade.closed_at_ms.saturating_sub(trade.opened_at_ms),
        ))
    });
    let exposure_bps = if total_duration == 0 {
        0
    } else {
        u64::try_from(
            exposed_duration
                .saturating_mul(10_000)
                .checked_div(u128::from(total_duration))
                .unwrap_or_default()
                .min(10_000),
        )
        .unwrap_or(10_000)
    };
    let gross_profit_minor = result
        .trades
        .iter()
        .filter(|trade| trade.pnl_minor > 0)
        .fold(0_u64, |total, trade| {
            total.saturating_add(trade.pnl_minor.unsigned_abs())
        });
    let gross_loss_minor = result
        .trades
        .iter()
        .filter(|trade| trade.pnl_minor < 0)
        .fold(0_u64, |total, trade| {
            total.saturating_add(trade.pnl_minor.unsigned_abs())
        });
    WalkForwardMetrics {
        total_return_bps: result.total_return_bps,
        max_drawdown_bps: result.max_drawdown_bps,
        completed_trade_count: result.completed_trade_count,
        win_rate_bps: result.win_rate_bps,
        profit_factor_milli: result.profit_factor_milli,
        expected_trade_pnl_minor: result.average_trade_pnl_minor,
        realized_pnl_minor: result.realized_pnl_minor,
        gross_profit_minor,
        gross_loss_minor,
        alpha_vs_price_benchmark_bps: result
            .performance
            .as_ref()
            .map(|performance| performance.alpha_vs_price_benchmark_bps),
        periods_per_year: result
            .performance
            .as_ref()
            .map(|performance| performance.periods_per_year),
        period_returns_ppm: result
            .performance
            .as_ref()
            .map(|performance| performance.period_returns_ppm.clone())
            .unwrap_or_default(),
        turnover_bps,
        exposure_bps,
    }
}

fn promotion_evaluation(folds: &[WalkForwardFold]) -> PromotionEvaluation {
    let total_trades = folds
        .iter()
        .map(|fold| fold.out_of_sample.completed_trade_count)
        .sum::<usize>();
    let winning_trades = folds
        .iter()
        .flat_map(|fold| &fold.regimes)
        .map(|regime| regime.winning_trade_count)
        .sum::<usize>();
    let realized_pnl = folds
        .iter()
        .map(|fold| i128::from(fold.out_of_sample.realized_pnl_minor))
        .sum::<i128>();
    let gross_profit = folds
        .iter()
        .map(|fold| u128::from(fold.out_of_sample.gross_profit_minor))
        .sum::<u128>();
    let gross_loss = folds
        .iter()
        .map(|fold| u128::from(fold.out_of_sample.gross_loss_minor))
        .sum::<u128>();
    let win_rate_bps = (total_trades > 0)
        .then(|| winning_trades.saturating_mul(10_000) / total_trades)
        .and_then(|value| u64::try_from(value).ok());
    let expected_pnl = (total_trades > 0)
        .then(|| realized_pnl / i128::try_from(total_trades).unwrap_or(i128::MAX));
    let profit_factor_milli = (gross_loss > 0)
        .then(|| gross_profit.saturating_mul(1_000) / gross_loss)
        .and_then(|value| u64::try_from(value).ok());
    let maximum_drawdown_bps = folds
        .iter()
        .map(|fold| fold.out_of_sample.max_drawdown_bps)
        .max()
        .unwrap_or_default();
    let (benchmark_alpha_weighted, benchmark_bar_count) = folds
        .iter()
        .filter_map(|fold| {
            fold.out_of_sample
                .alpha_vs_price_benchmark_bps
                .map(|alpha| {
                    (
                        i128::from(alpha) * fold.oos_bar_count as i128,
                        fold.oos_bar_count,
                    )
                })
        })
        .fold(
            (0_i128, 0_usize),
            |(alpha_total, bar_total), (alpha, bars)| {
                (
                    alpha_total.saturating_add(alpha),
                    bar_total.saturating_add(bars),
                )
            },
        );
    let benchmark_alpha = (benchmark_bar_count > 0)
        .then(|| benchmark_alpha_weighted / benchmark_bar_count as i128)
        .and_then(|value| i64::try_from(value).ok());
    let populated_regimes = [
        ObservedRegime::Bullish,
        ObservedRegime::Bearish,
        ObservedRegime::Sideways,
        ObservedRegime::HighVolatility,
    ]
    .into_iter()
    .filter_map(|target| {
        let (count, pnl) = folds
            .iter()
            .flat_map(|fold| &fold.regimes)
            .filter(|regime| regime.regime == target)
            .fold((0_usize, 0_i128), |(count, pnl), regime| {
                (
                    count.saturating_add(regime.completed_trade_count),
                    pnl.saturating_add(i128::from(regime.realized_pnl_minor)),
                )
            });
        (count > 0).then_some((target, pnl))
    })
    .collect::<Vec<_>>();
    let regime_stable =
        populated_regimes.len() >= 2 && populated_regimes.iter().all(|(_, pnl)| *pnl >= 0);
    let mut checks = vec![
        PromotionCheck {
            check_id: "oos_sample".to_owned(),
            label: "OOS 표본".to_owned(),
            passed: total_trades >= MINIMUM_OOS_TRADE_COUNT,
            observed: format!("{total_trades}건"),
            required: format!("{}건 이상", MINIMUM_OOS_TRADE_COUNT),
        },
        PromotionCheck {
            check_id: "win_rate".to_owned(),
            label: "OOS 승률".to_owned(),
            passed: win_rate_bps.is_some_and(|value| value >= MINIMUM_WIN_RATE_BPS),
            observed: win_rate_bps
                .map(|value| format!("{:.2}%", value as f64 / 100.0))
                .unwrap_or_else(|| "계산 불가".to_owned()),
            required: "55.00% 이상".to_owned(),
        },
        PromotionCheck {
            check_id: "expected_pnl".to_owned(),
            label: "비용 차감 기대손익".to_owned(),
            passed: expected_pnl.is_some_and(|value| value > 0),
            observed: expected_pnl
                .map(|value| value.to_string())
                .unwrap_or_else(|| "계산 불가".to_owned()),
            required: "0 초과".to_owned(),
        },
        PromotionCheck {
            check_id: "profit_factor".to_owned(),
            label: "Profit Factor".to_owned(),
            passed: profit_factor_milli.is_some_and(|value| value >= MINIMUM_PROFIT_FACTOR_MILLI),
            observed: profit_factor_milli
                .map(|value| format!("{:.3}", value as f64 / 1_000.0))
                .unwrap_or_else(|| "계산 불가".to_owned()),
            required: "1.300 이상".to_owned(),
        },
        PromotionCheck {
            check_id: "maximum_drawdown".to_owned(),
            label: "OOS 최대낙폭".to_owned(),
            passed: maximum_drawdown_bps <= MAXIMUM_DRAWDOWN_BPS,
            observed: format!("{:.2}%", maximum_drawdown_bps as f64 / 100.0),
            required: "12.00% 이하".to_owned(),
        },
        PromotionCheck {
            check_id: "price_benchmark".to_owned(),
            label: "동일 종목 가격 대비".to_owned(),
            passed: benchmark_alpha.is_some_and(|value| value > 0),
            observed: benchmark_alpha
                .map(|value| format!("가중 평균 알파 {:.2}%p", value as f64 / 100.0))
                .unwrap_or_else(|| "계산 불가".to_owned()),
            required: "0%p 초과".to_owned(),
        },
        PromotionCheck {
            check_id: "regime_stability".to_owned(),
            label: "관측 레짐 안정성".to_owned(),
            passed: regime_stable,
            observed: format!("거래 발생 레짐 {}개", populated_regimes.len()),
            required: "2개 이상 레짐·각 레짐 손익 0 이상".to_owned(),
        },
    ];
    let eligible_for_paper_review = checks.iter().all(|check| check.passed);
    if folds.is_empty() {
        checks.clear();
    }
    PromotionEvaluation {
        policy_version: PROMOTION_POLICY_VERSION.to_owned(),
        eligible_for_paper_review,
        checks,
        warning: "통과는 내부 모의운영 검토 자격일 뿐 자동 주문·실전 승격·수익 보장이 아닙니다. 데이터·전략·비용·기준이 바뀌면 새 검증 실행이 필요합니다.".to_owned(),
    }
}

fn percentile(mut values: Vec<u64>, percentile: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    values
        .get((values.len() - 1).saturating_mul(percentile) / 100)
        .copied()
}

fn trailing_observation(bars: &[PriceBar]) -> Option<(i64, u64)> {
    let first = bars.first()?.close_minor;
    let last = bars.last()?.close_minor;
    if bars.len() < 3 || first == 0 {
        return None;
    }
    let trend_bps =
        i64::try_from((i128::from(last) - i128::from(first)) * 10_000 / i128::from(first)).ok()?;
    let returns = bars
        .windows(2)
        .map(|pair| pair[1].close_minor as f64 / pair[0].close_minor as f64 - 1.0)
        .collect::<Vec<_>>();
    let average = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - average).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    let volatility_bps = (variance.sqrt() * 10_000.0).round();
    volatility_bps
        .is_finite()
        .then_some((trend_bps, volatility_bps.max(0.0) as u64))
}

fn regime_thresholds(training_bars: &[PriceBar]) -> Option<(u64, u64)> {
    let observations = training_bars
        .windows(REGIME_LOOKBACK_BARS)
        .filter_map(trailing_observation)
        .collect::<Vec<_>>();
    Some((
        percentile(
            observations
                .iter()
                .map(|(trend, _)| trend.unsigned_abs())
                .collect(),
            50,
        )?
        .max(1),
        percentile(
            observations
                .iter()
                .map(|(_, volatility)| *volatility)
                .collect(),
            75,
        )?
        .max(1),
    ))
}

fn classify_regime(
    history: &[PriceBar],
    trend_threshold: u64,
    volatility_threshold: u64,
) -> Option<ObservedRegime> {
    let start = history.len().checked_sub(REGIME_LOOKBACK_BARS)?;
    let (trend_bps, volatility_bps) = trailing_observation(&history[start..])?;
    let trend_threshold = i64::try_from(trend_threshold).ok()?;
    Some(if volatility_bps > volatility_threshold {
        ObservedRegime::HighVolatility
    } else if trend_bps > trend_threshold {
        ObservedRegime::Bullish
    } else if trend_bps < -trend_threshold {
        ObservedRegime::Bearish
    } else {
        ObservedRegime::Sideways
    })
}

fn regime_performance(
    training_bars: &[PriceBar],
    oos_bars: &[PriceBar],
    result: &BacktestResult,
) -> (Vec<RegimePerformance>, usize) {
    let Some((trend_threshold, volatility_threshold)) = regime_thresholds(training_bars) else {
        return (Vec::new(), result.completed_trade_count);
    };
    let mut history = training_bars.to_vec();
    history.extend_from_slice(oos_bars);
    let mut aggregates = [
        (ObservedRegime::Bullish, 0_usize, 0_usize, 0_i128),
        (ObservedRegime::Bearish, 0, 0, 0),
        (ObservedRegime::Sideways, 0, 0, 0),
        (ObservedRegime::HighVolatility, 0, 0, 0),
    ];
    let mut unclassified = 0_usize;
    for trade in &result.trades {
        let Some(index) = oos_bars
            .iter()
            .position(|bar| bar.period_start_ms == trade.opened_at_ms)
        else {
            unclassified += 1;
            continue;
        };
        let Some(regime) = classify_regime(
            &history[..training_bars.len() + index],
            trend_threshold,
            volatility_threshold,
        ) else {
            unclassified += 1;
            continue;
        };
        let aggregate = aggregates
            .iter_mut()
            .find(|(candidate, ..)| *candidate == regime)
            .expect("all observed regimes are initialized");
        aggregate.1 += 1;
        aggregate.2 += usize::from(trade.pnl_minor > 0);
        aggregate.3 += i128::from(trade.pnl_minor);
    }
    let reports = aggregates
        .into_iter()
        .map(
            |(regime, completed_trade_count, winning_trade_count, pnl)| RegimePerformance {
                regime,
                completed_trade_count,
                winning_trade_count,
                realized_pnl_minor: i64::try_from(pnl).unwrap_or(if pnl.is_negative() {
                    i64::MIN
                } else {
                    i64::MAX
                }),
                win_rate_bps: (completed_trade_count > 0).then(|| {
                    u64::try_from(winning_trade_count * 10_000 / completed_trade_count)
                        .unwrap_or_default()
                }),
            },
        )
        .collect();
    (reports, unclassified)
}

fn absolute_return_bps(bars: &[PriceBar]) -> Vec<u64> {
    bars.windows(2)
        .filter(|pair| pair[0].close_minor > 0)
        .map(|pair| {
            u64::try_from(
                u128::from(pair[1].close_minor.abs_diff(pair[0].close_minor)) * 10_000
                    / u128::from(pair[0].close_minor),
            )
            .unwrap_or(u64::MAX)
        })
        .collect()
}

fn volatility_states(bars: &[PriceBar], threshold_bps: u64) -> Vec<usize> {
    absolute_return_bps(bars)
        .into_iter()
        .map(|value| usize::from(value > threshold_bps))
        .collect()
}

fn state_model_diagnostic(
    training_bars: &[PriceBar],
    oos_bars: &[PriceBar],
) -> StateModelDiagnostic {
    let mut diagnostic = StateModelDiagnostic {
        model_id: "two-state-volatility-markov-v1".to_owned(),
        ..StateModelDiagnostic::default()
    };
    let training_returns = absolute_return_bps(training_bars);
    if training_returns.len() < 30 || oos_bars.len() < 10 {
        diagnostic.blockers.push(
            "상태 전이모형은 학습 수익률 30개와 OOS 가격봉 10개 이상이 필요합니다.".to_owned(),
        );
        return diagnostic;
    }
    let threshold = percentile(training_returns, 50).unwrap_or_default();
    let training_states = volatility_states(training_bars, threshold);
    let mut joined = training_bars
        .last()
        .cloned()
        .into_iter()
        .chain(oos_bars.iter().cloned())
        .collect::<Vec<_>>();
    if joined.len() < 2 {
        diagnostic
            .blockers
            .push("OOS 상태 전이를 만들 수 없습니다.".to_owned());
        return diagnostic;
    }
    let oos_states = volatility_states(&joined, threshold);
    joined.clear();
    let mut transitions = [[1_u64; 2]; 2];
    for pair in training_states.windows(2) {
        transitions[pair[0]][pair[1]] += 1;
    }
    diagnostic.training_transition_count = training_states.len().saturating_sub(1);
    diagnostic.oos_transition_count = oos_states.len().saturating_sub(1);
    if diagnostic.oos_transition_count == 0 {
        diagnostic
            .blockers
            .push("OOS 상태 전이가 없습니다.".to_owned());
        return diagnostic;
    }
    let probability = |from: usize, to: usize| {
        transitions[from][to] as f64
            / (transitions[from][0].saturating_add(transitions[from][1])) as f64
    };
    diagnostic.low_volatility_persistence_bps = Some((probability(0, 0) * 10_000.0).round() as u64);
    diagnostic.high_volatility_persistence_bps =
        Some((probability(1, 1) * 10_000.0).round() as u64);
    diagnostic.maximum_transition_uncertainty_bps = (0..2)
        .map(|from| {
            let count = transitions[from][0].saturating_add(transitions[from][1]) as f64;
            let p = probability(from, 1);
            (p * (1.0 - p) / count).sqrt() * 10_000.0
        })
        .map(|value| value.round() as u64)
        .max();
    let high_count = training_states.iter().filter(|state| **state == 1).count() + 1;
    let baseline_high = high_count as f64 / (training_states.len() + 2) as f64;
    let mut transition_loss = 0.0;
    let mut baseline_loss = 0.0;
    for pair in oos_states.windows(2) {
        transition_loss -= probability(pair[0], pair[1]).clamp(1e-9, 1.0).ln();
        let baseline = if pair[1] == 1 {
            baseline_high
        } else {
            1.0 - baseline_high
        };
        baseline_loss -= baseline.clamp(1e-9, 1.0).ln();
    }
    let count = diagnostic.oos_transition_count as f64;
    let transition_milli = (transition_loss / count * 1_000.0).round() as u64;
    let baseline_milli = (baseline_loss / count * 1_000.0).round() as u64;
    diagnostic.transition_model_log_loss_milli = Some(transition_milli);
    diagnostic.independent_state_baseline_log_loss_milli = Some(baseline_milli);
    diagnostic.beats_independent_baseline = transition_milli < baseline_milli;
    if !diagnostic.beats_independent_baseline {
        diagnostic
            .blockers
            .push("OOS에서 상태 전이모형이 독립상태 기준보다 낫지 않습니다.".to_owned());
    }
    diagnostic
}

fn partition_masks(fold_count: usize) -> Vec<u64> {
    if !(4..=20).contains(&fold_count) {
        return Vec::new();
    }
    let training_size = fold_count / 2;
    let full_mask = (1_u64 << fold_count) - 1;
    (1..full_mask)
        .filter(|mask| mask.count_ones() as usize == training_size)
        .filter(|mask| fold_count % 2 == 1 || *mask < (full_mask ^ *mask))
        .collect()
}

fn average_selected(values: &[i64], mask: u64, selected: bool) -> f64 {
    let mut total = 0_f64;
    let mut count = 0_usize;
    for (index, value) in values.iter().enumerate() {
        if ((mask >> index) & 1 == 1) == selected {
            total += *value as f64;
            count += 1;
        }
    }
    total / count.max(1) as f64
}

fn overfit_diagnostics(reports: &[WalkForwardReport]) -> OverfitDiagnostics {
    let Some(reference) = reports.last() else {
        return OverfitDiagnostics::default();
    };
    let fold_count = reference.fold_count;
    let comparable = reports
        .iter()
        .filter(|report| {
            report.fold_count == fold_count
                && report.folds.len() == fold_count
                && report
                    .folds
                    .iter()
                    .zip(&reference.folds)
                    .all(|(left, right)| {
                        left.oos_start_ms == right.oos_start_ms
                            && left.oos_end_ms == right.oos_end_ms
                    })
        })
        .collect::<Vec<_>>();
    let masks = partition_masks(fold_count);
    let mut blockers = Vec::new();
    if comparable.len() < 3 {
        blockers.push(format!(
            "동일 데이터·동일 OOS 경계의 전략이 최소 3개 필요하지만 현재 {}개입니다.",
            comparable.len()
        ));
    }
    if masks.is_empty() {
        blockers.push(format!(
            "CSCV 분할에는 OOS 구간이 최소 4개 필요하지만 현재 {fold_count}개입니다."
        ));
    }
    let pbo = if comparable.len() >= 3 && !masks.is_empty() {
        let returns = comparable
            .iter()
            .map(|report| {
                report
                    .folds
                    .iter()
                    .map(|fold| fold.out_of_sample.total_return_bps)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let overfit_count = masks
            .iter()
            .filter(|mask| {
                let best_index = returns
                    .iter()
                    .enumerate()
                    .max_by(|(_, left), (_, right)| {
                        average_selected(left, **mask, true)
                            .total_cmp(&average_selected(right, **mask, true))
                    })
                    .map(|(index, _)| index)
                    .unwrap_or_default();
                let selected_oos = average_selected(&returns[best_index], **mask, false);
                let below = returns
                    .iter()
                    .filter(|values| average_selected(values, **mask, false) < selected_oos)
                    .count();
                let equal = returns
                    .iter()
                    .filter(|values| average_selected(values, **mask, false) == selected_oos)
                    .count();
                (below as f64 + equal as f64 * 0.5) / returns.len() as f64 <= 0.5
            })
            .count();
        Some(u64::try_from(overfit_count * 10_000 / masks.len()).unwrap_or(10_000))
    } else {
        None
    };
    let current_returns = reference
        .folds
        .iter()
        .flat_map(|fold| &fold.out_of_sample.period_returns_ppm)
        .map(|value| *value as f64 / 1_000_000.0)
        .collect::<Vec<_>>();
    let minimum_track_record_length = minimum_track_record_length(&current_returns);
    if minimum_track_record_length.is_none() {
        blockers.push(format!(
            "MinTRL은 양의 Sharpe와 OOS 기간 수익률 30개 이상이 필요하지만 현재 {}개입니다.",
            current_returns.len()
        ));
    }
    let complete_trial_catalog = reference.strategy_trial_count == comparable.len();
    let deflated_sharpe_ratio_milli = if complete_trial_catalog {
        deflated_sharpe_ratio(&comparable, &current_returns)
    } else {
        blockers.push(format!(
            "Deflated Sharpe에는 전체 시도 {}개의 OOS 원시 수익률이 필요하지만 현재 {}개만 비교 가능합니다.",
            reference.strategy_trial_count,
            comparable.len()
        ));
        None
    };
    if complete_trial_catalog && deflated_sharpe_ratio_milli.is_none() {
        blockers.push(
            "Deflated Sharpe는 비교 전략 3개, 전략별 OOS 수익률 30개와 비상수 Sharpe 분산이 필요합니다."
                .to_owned(),
        );
    }
    OverfitDiagnostics {
        comparable_strategy_count: comparable.len(),
        evaluated_partition_count: if pbo.is_some() { masks.len() } else { 0 },
        probability_of_backtest_overfitting_bps: pbo,
        deflated_sharpe_ratio_milli,
        minimum_track_record_length,
        blockers,
    }
}

fn return_moments(returns: &[f64]) -> Option<(f64, f64, f64, f64)> {
    if returns.len() < 2 {
        return None;
    }
    let count = returns.len() as f64;
    let average = returns.iter().sum::<f64>() / count;
    let centered = returns
        .iter()
        .map(|value| value - average)
        .collect::<Vec<_>>();
    let deviation =
        (centered.iter().map(|value| value.powi(2)).sum::<f64>() / (count - 1.0)).sqrt();
    if !deviation.is_finite() || deviation <= 0.0 {
        return None;
    }
    let skewness =
        centered.iter().map(|value| value.powi(3)).sum::<f64>() / count / deviation.powi(3);
    let kurtosis =
        centered.iter().map(|value| value.powi(4)).sum::<f64>() / count / deviation.powi(4);
    Some((average / deviation, deviation, skewness, kurtosis))
}

fn correlation(left: &[f64], right: &[f64]) -> Option<f64> {
    let count = left.len().min(right.len());
    if count < 30 {
        return None;
    }
    let left = &left[..count];
    let right = &right[..count];
    let left_mean = left.iter().sum::<f64>() / count as f64;
    let right_mean = right.iter().sum::<f64>() / count as f64;
    let covariance = left
        .iter()
        .zip(right)
        .map(|(l, r)| (l - left_mean) * (r - right_mean))
        .sum::<f64>();
    let left_scale = left
        .iter()
        .map(|value| (value - left_mean).powi(2))
        .sum::<f64>();
    let right_scale = right
        .iter()
        .map(|value| (value - right_mean).powi(2))
        .sum::<f64>();
    let denominator = (left_scale * right_scale).sqrt();
    (denominator > 0.0).then(|| (covariance / denominator).clamp(-1.0, 1.0))
}

fn standard_normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / 2_f64.sqrt();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let polynomial =
        (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t
            + 0.254_829_592)
            * t;
    let erf = sign * (1.0 - polynomial * (-x * x).exp());
    (0.5 * (1.0 + erf)).clamp(0.0, 1.0)
}

fn inverse_standard_normal_cdf(probability: f64) -> Option<f64> {
    if !(0.0..1.0).contains(&probability) {
        return None;
    }
    const A: [f64; 6] = [
        -39.69683028665376,
        220.9460984245205,
        -275.9285104469687,
        138.357_751_867_269,
        -30.66479806614716,
        2.506628277459239,
    ];
    const B: [f64; 5] = [
        -54.47609879822406,
        161.5858368580409,
        -155.6989798598866,
        66.80131188771972,
        -13.28068155288572,
    ];
    const C: [f64; 6] = [
        -0.007784894002430293,
        -0.3223964580411365,
        -2.400758277161838,
        -2.549732539343734,
        4.374664141464968,
        2.938163982698783,
    ];
    const D: [f64; 4] = [
        0.007784695709041462,
        0.3224671290700398,
        2.445134137142996,
        3.754408661907416,
    ];
    let low = 0.02425;
    let high = 1.0 - low;
    let value = if probability < low {
        let q = (-2.0 * probability.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if probability <= high {
        let q = probability - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - probability).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    value.is_finite().then_some(value)
}

fn deflated_sharpe_ratio(reports: &[&WalkForwardReport], current_returns: &[f64]) -> Option<i64> {
    if reports.len() < 3 || current_returns.len() < 30 {
        return None;
    }
    let return_series = reports
        .iter()
        .map(|report| {
            report
                .folds
                .iter()
                .flat_map(|fold| &fold.out_of_sample.period_returns_ppm)
                .map(|value| *value as f64 / 1_000_000.0)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if return_series.iter().any(|returns| returns.len() < 30) {
        return None;
    }
    let sharpes = return_series
        .iter()
        .map(|returns| return_moments(returns).map(|moments| moments.0))
        .collect::<Option<Vec<_>>>()?;
    let sharpe_average = sharpes.iter().sum::<f64>() / sharpes.len() as f64;
    let sharpe_variance = sharpes
        .iter()
        .map(|value| (value - sharpe_average).powi(2))
        .sum::<f64>()
        / (sharpes.len() - 1) as f64;
    if sharpe_variance <= 0.0 {
        return None;
    }
    let correlations = (0..return_series.len())
        .flat_map(|left| ((left + 1)..return_series.len()).map(move |right| (left, right)))
        .filter_map(|(left, right)| correlation(&return_series[left], &return_series[right]))
        .collect::<Vec<_>>();
    if correlations.is_empty() {
        return None;
    }
    let average_positive_correlation =
        (correlations.iter().sum::<f64>() / correlations.len() as f64).clamp(0.0, 0.99);
    let effective_trials =
        1.0 + (reports.len() as f64 - 1.0) * (1.0 - average_positive_correlation);
    if effective_trials <= 1.0 {
        return None;
    }
    let euler_gamma = 0.577_215_664_901_532_9;
    let first = inverse_standard_normal_cdf(1.0 - 1.0 / effective_trials)?;
    let second = inverse_standard_normal_cdf(1.0 - 1.0 / (effective_trials * std::f64::consts::E))?;
    let expected_maximum_sharpe =
        sharpe_variance.sqrt() * ((1.0 - euler_gamma) * first + euler_gamma * second);
    let (observed_sharpe, _, skewness, kurtosis) = return_moments(current_returns)?;
    let variance_adjustment =
        1.0 - skewness * observed_sharpe + (kurtosis - 1.0) * observed_sharpe.powi(2) / 4.0;
    if variance_adjustment <= 0.0 {
        return None;
    }
    let statistic = (observed_sharpe - expected_maximum_sharpe)
        * ((current_returns.len() - 1) as f64).sqrt()
        / variance_adjustment.sqrt();
    Some((standard_normal_cdf(statistic) * 1_000.0).round() as i64)
}

fn minimum_track_record_length(returns: &[f64]) -> Option<usize> {
    if returns.len() < 30 {
        return None;
    }
    let count = returns.len() as f64;
    let average = returns.iter().sum::<f64>() / count;
    let centered = returns
        .iter()
        .map(|value| value - average)
        .collect::<Vec<_>>();
    let variance = centered.iter().map(|value| value.powi(2)).sum::<f64>() / (count - 1.0);
    let deviation = variance.sqrt();
    if !deviation.is_finite() || deviation <= 0.0 || average <= 0.0 {
        return None;
    }
    let sharpe = average / deviation;
    let skewness =
        centered.iter().map(|value| value.powi(3)).sum::<f64>() / count / deviation.powi(3);
    let kurtosis =
        centered.iter().map(|value| value.powi(4)).sum::<f64>() / count / deviation.powi(4);
    let adjustment = 1.0 - skewness * sharpe + (kurtosis - 1.0) * sharpe.powi(2) / 4.0;
    let required = 1.0 + adjustment.max(0.0) * (1.644_853_626_951_472_2 / sharpe).powi(2);
    (required.is_finite() && required > 0.0).then(|| required.ceil() as usize)
}

fn comparable_walk_forward_reports(
    bridge: &PersistenceBridge,
    source_experiment_id: &str,
) -> Result<Vec<WalkForwardReport>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "비교 OOS 검증 조회 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT w.source_experiment_id, w.report_json
             FROM walk_forward_runs w
             JOIN backtest_runs candidate ON candidate.experiment_id = w.source_experiment_id
             WHERE candidate.dataset_id = (
                 SELECT dataset_id FROM backtest_runs WHERE experiment_id = ?1
             )
             ORDER BY w.created_at_ms DESC, w.validation_run_id DESC",
        )
        .map_err(|error| format!("비교 OOS 검증 쿼리를 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![source_experiment_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("비교 OOS 검증을 조회하지 못했습니다: {error}"))?;
    let mut latest_by_experiment = HashMap::new();
    for row in rows {
        let (experiment_id, serialized) =
            row.map_err(|error| format!("비교 OOS 검증 행을 읽지 못했습니다: {error}"))?;
        if latest_by_experiment.contains_key(&experiment_id) {
            continue;
        }
        let report = serde_json::from_str(&serialized)
            .map_err(|error| format!("비교 OOS 검증 결과를 해석하지 못했습니다: {error}"))?;
        latest_by_experiment.insert(experiment_id, report);
    }
    Ok(latest_by_experiment.into_values().collect())
}

fn execute_walk_forward(
    bridge: &PersistenceBridge,
    request: WalkForwardRequest,
) -> Result<WalkForwardReport, String> {
    if !valid_id(&request.source_experiment_id) || !(2..=5).contains(&request.fold_count) {
        return Err("저장 실험 ID와 OOS 구간 수(2~5)를 확인해 주세요.".to_owned());
    }
    let source = load_experiment(bridge, &request.source_experiment_id)?;
    let (strategy_trial_count, created_at_ms) = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "전략 실험 횟수 조회 잠금을 획득하지 못했습니다.".to_owned())?;
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM backtest_runs WHERE dataset_id = ?1",
                params![source.config.dataset_id],
                |row| row.get::<_, u64>(0),
            )
            .map_err(|error| format!("전략 실험 횟수를 조회하지 못했습니다: {error}"))?;
        (usize::try_from(count).unwrap_or(usize::MAX), now_ms()?)
    };
    let slow_window = match source.report.strategy_candidate.entry_signal {
        SignalSpec::MovingAverageCross { slow_window, .. } => slow_window,
    };
    let minimum_segment = slow_window.saturating_add(2);
    let initial_training_bar_count = source.backtest_bars.len() / 2;
    let remaining = source
        .backtest_bars
        .len()
        .saturating_sub(initial_training_bar_count);
    if initial_training_bar_count < minimum_segment
        || remaining < minimum_segment.saturating_mul(request.fold_count)
    {
        return Err(format!(
            "{}개 OOS 구간을 만들 가격봉이 부족합니다. 현재 {}봉, 느린 이평 {}봉 기준 최소 {}봉이 필요합니다.",
            request.fold_count,
            source.backtest_bars.len(),
            slow_window,
            minimum_segment.saturating_mul(request.fold_count + 1)
        ));
    }

    let base_oos_size = remaining / request.fold_count;
    let mut folds = Vec::with_capacity(request.fold_count);
    let run_suffix = Uuid::new_v4().simple().to_string();
    for index in 0..request.fold_count {
        let training_end = initial_training_bar_count + base_oos_size * index;
        let oos_end = if index + 1 == request.fold_count {
            source.backtest_bars.len()
        } else {
            training_end + base_oos_size
        };
        let training_bars = &source.backtest_bars[..training_end];
        let oos_bars = &source.backtest_bars[training_end..oos_end];
        let mut training_config = source.config.clone();
        training_config.experiment_id = format!("wf-train-{}-{}", index + 1, &run_suffix[..12]);
        let mut oos_config = source.config.clone();
        oos_config.experiment_id = format!("wf-oos-{}-{}", index + 1, &run_suffix[..12]);
        let training = run_backtest(
            &source.report.strategy_candidate,
            training_bars,
            &training_config,
        )
        .map_err(|error| format!("{}번 학습 구간 계산 실패: {}", index + 1, error.message))?;
        let out_of_sample = run_backtest(&source.report.strategy_candidate, oos_bars, &oos_config)
            .map_err(|error| format!("{}번 OOS 구간 계산 실패: {}", index + 1, error.message))?;
        let (regimes, unclassified_trade_count) =
            regime_performance(training_bars, oos_bars, &out_of_sample);
        let state_model = state_model_diagnostic(training_bars, oos_bars);
        folds.push(WalkForwardFold {
            fold_number: index + 1,
            training_bar_count: training_bars.len(),
            oos_bar_count: oos_bars.len(),
            training_end_ms: training_bars
                .last()
                .expect("validated training segment")
                .period_end_ms,
            oos_start_ms: oos_bars
                .first()
                .expect("validated OOS segment")
                .period_start_ms,
            oos_end_ms: oos_bars
                .last()
                .expect("validated OOS segment")
                .period_end_ms,
            training: result_metrics(&training, training_bars, &training_config),
            out_of_sample: result_metrics(&out_of_sample, oos_bars, &oos_config),
            regimes,
            unclassified_trade_count,
            state_model,
        });
    }
    let positive_oos_fold_count = folds
        .iter()
        .filter(|fold| fold.out_of_sample.total_return_bps > 0)
        .count();
    let absolute_returns = folds
        .iter()
        .map(|fold| fold.out_of_sample.total_return_bps.unsigned_abs())
        .collect::<Vec<_>>();
    let absolute_return_sum = absolute_returns
        .iter()
        .map(|value| u128::from(*value))
        .sum::<u128>();
    let largest_absolute_oos_return_share_bps =
        u128::from(absolute_returns.iter().copied().max().unwrap_or_default())
            .saturating_mul(10_000)
            .checked_div(absolute_return_sum)
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or_default();
    let oos_returns = folds
        .iter()
        .map(|fold| fold.out_of_sample.total_return_bps)
        .collect::<Vec<_>>();
    let oos_return_spread_bps = oos_returns
        .iter()
        .max()
        .zip(oos_returns.iter().min())
        .map(|(maximum, minimum)| i128::from(*maximum).saturating_sub(i128::from(*minimum)))
        .and_then(|spread| u64::try_from(spread).ok())
        .unwrap_or_default();
    let total_oos_trade_count = folds
        .iter()
        .map(|fold| fold.out_of_sample.completed_trade_count)
        .sum::<usize>();
    let meets_research_sample_minimum = total_oos_trade_count >= MINIMUM_OOS_TRADE_COUNT;
    let promotion_evaluation = promotion_evaluation(&folds);
    let promotion_blockers = promotion_evaluation
        .checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| {
            format!(
                "{}: {} 필요, 현재 {}",
                check.label, check.required, check.observed
            )
        })
        .collect();
    let mut report = WalkForwardReport {
        validation_run_id: format!("walk-forward-{}", &run_suffix[..16]),
        created_at_ms,
        source_experiment_id: request.source_experiment_id,
        strategy_trial_count,
        fold_count: request.fold_count,
        initial_training_bar_count,
        positive_oos_fold_count,
        largest_absolute_oos_return_share_bps,
        oos_return_spread_bps,
        total_oos_trade_count,
        minimum_oos_trade_count: MINIMUM_OOS_TRADE_COUNT,
        meets_research_sample_minimum,
        promotion_blockers,
        promotion_evaluation,
        overfit_diagnostics: OverfitDiagnostics::default(),
        folds,
        warnings: vec![
            "고정된 원본 파라미터를 시간순 OOS 구간에 반복 적용한 1차 검증이며 자동 파라미터 최적화가 아닙니다.".to_owned(),
            "각 OOS 구간은 이전 구간의 포지션과 자금을 이월하지 않고 독립 초기화하며, 느린 이동평균만큼의 초기 구간은 신호 준비에 사용됩니다.".to_owned(),
            "OOS 수익 구간 수는 전략 합격선이나 주문 승인 조건이 아닙니다.".to_owned(),
            "관측 레짐 v1은 학습 구간의 20봉 절대추세 중앙값과 변동성 75분위수를 사용하는 결정론적 분류이며 Markov 상태모형이나 미래 예측이 아닙니다.".to_owned(),
            "2상태 변동성 전이모형은 각 fold 학습 구간에서만 추정하고 별도 OOS에서 독립상태 기준과 log loss를 비교합니다. 성과 우위나 미래 예측을 뜻하지 않습니다.".to_owned(),
        ],
    };
    let mut comparable = comparable_walk_forward_reports(bridge, &report.source_experiment_id)?;
    comparable.push(report.clone());
    report.overfit_diagnostics = overfit_diagnostics(&comparable);
    let serialized = serde_json::to_string(&report)
        .map_err(|error| format!("OOS 검증 결과를 직렬화하지 못했습니다: {error}"))?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "OOS 검증 결과 저장 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "INSERT INTO walk_forward_runs
             (validation_run_id, source_experiment_id, fold_count, strategy_trial_count, report_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                report.validation_run_id,
                report.source_experiment_id,
                report.fold_count,
                report.strategy_trial_count,
                serialized,
                report.created_at_ms
            ],
        )
        .map_err(|error| format!("OOS 검증 결과를 저장하지 못했습니다: {error}"))?;
    Ok(report)
}

fn latest_walk_forward(
    bridge: &PersistenceBridge,
    source_experiment_id: &str,
) -> Result<Option<WalkForwardReport>, String> {
    if !valid_id(source_experiment_id) {
        return Err("조회할 저장 실험 ID가 올바르지 않습니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "OOS 검증 결과 조회 잠금을 획득하지 못했습니다.".to_owned())?;
    let serialized = connection
        .query_row(
            "SELECT report_json FROM walk_forward_runs
             WHERE source_experiment_id = ?1
             ORDER BY created_at_ms DESC, validation_run_id DESC LIMIT 1",
            params![source_experiment_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("저장된 OOS 검증 결과를 조회하지 못했습니다: {error}"))?;
    serialized
        .map(|value| {
            serde_json::from_str(&value)
                .map_err(|error| format!("저장된 OOS 검증 결과를 해석하지 못했습니다: {error}"))
        })
        .transpose()
}

fn walk_forward_history(
    bridge: &PersistenceBridge,
    source_experiment_id: &str,
) -> Result<Vec<WalkForwardReport>, String> {
    if !valid_id(source_experiment_id) {
        return Err("조회할 저장 실험 ID가 올바르지 않습니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "OOS 검증 이력 조회 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT report_json FROM walk_forward_runs
             WHERE source_experiment_id = ?1
             ORDER BY created_at_ms DESC, validation_run_id DESC LIMIT 20",
        )
        .map_err(|error| format!("OOS 검증 이력 쿼리를 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![source_experiment_id], |row| row.get::<_, String>(0))
        .map_err(|error| format!("OOS 검증 이력을 조회하지 못했습니다: {error}"))?;
    let mut reports = Vec::new();
    for row in rows {
        let serialized =
            row.map_err(|error| format!("OOS 검증 이력 행을 읽지 못했습니다: {error}"))?;
        reports.push(
            serde_json::from_str(&serialized)
                .map_err(|error| format!("저장된 OOS 검증 이력을 해석하지 못했습니다: {error}"))?,
        );
    }
    Ok(reports)
}

fn experiment_bias_audit(
    bridge: &PersistenceBridge,
    experiment_id: &str,
) -> Result<ExperimentBiasAudit, String> {
    if !valid_id(experiment_id) {
        return Err("감사할 저장 실험 ID가 올바르지 않습니다.".to_owned());
    }
    let (dataset_id, local_strategy_trial_count, serialized_reports) = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "실험 편향 감사 조회 잠금을 획득하지 못했습니다.".to_owned())?;
        let dataset_id: String = connection
            .query_row(
                "SELECT dataset_id FROM backtest_runs WHERE experiment_id=?1",
                params![experiment_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("감사할 저장 실험을 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| "감사할 저장 실험을 찾지 못했습니다.".to_owned())?;
        let local_strategy_trial_count: u64 = connection
            .query_row(
                "SELECT COUNT(*) FROM backtest_runs WHERE dataset_id=?1",
                params![dataset_id],
                |row| row.get(0),
            )
            .map_err(|error| {
                format!("동일 데이터셋의 로컬 실험 수를 조회하지 못했습니다: {error}")
            })?;
        let mut statement = connection
            .prepare("SELECT report_json FROM walk_forward_runs WHERE source_experiment_id=?1 ORDER BY created_at_ms,validation_run_id")
            .map_err(|error| format!("OOS 감사 쿼리를 준비하지 못했습니다: {error}"))?;
        let serialized_reports = statement
            .query_map(params![experiment_id], |row| row.get::<_, String>(0))
            .map_err(|error| format!("OOS 감사 기록을 조회하지 못했습니다: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("OOS 감사 기록을 읽지 못했습니다: {error}"))?;
        (
            dataset_id,
            usize::try_from(local_strategy_trial_count).unwrap_or(usize::MAX),
            serialized_reports,
        )
    };
    let reports = serialized_reports
        .iter()
        .map(|serialized| {
            serde_json::from_str::<WalkForwardReport>(serialized)
                .map_err(|error| format!("OOS 감사 기록을 해석하지 못했습니다: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let oos_fold_count = reports.iter().map(|report| report.folds.len()).sum();
    let oos_trade_count = reports
        .iter()
        .map(|report| report.total_oos_trade_count)
        .sum();
    Ok(ExperimentBiasAudit {
        experiment_id: experiment_id.to_owned(),
        dataset_id,
        local_strategy_trial_count,
        walk_forward_validation_count: reports.len(),
        oos_fold_count,
        oos_trade_count,
        data_snooping_status: "needs_review".to_owned(),
        survivorship_bias_status: "needs_review".to_owned(),
        catalog_completeness: "local_only".to_owned(),
        universe_membership_evidence: "missing".to_owned(),
        details: vec![
            "동일 데이터셋으로 저장된 로컬 전략 시도만 집계하며 외부·삭제 실험까지 완전하다고 증명하지 않습니다.".to_owned(),
            "시간순 OOS 기록은 집계하지만 과거 시점의 종목군 편입·상장폐지 이력이 없어 생존편향 통과로 판정하지 않습니다.".to_owned(),
            "이 감사 결과는 주문 승인이나 미래 성과 보증에 사용하지 않습니다.".to_owned(),
        ],
    })
}

#[tauri::command]
pub fn backtest_experiment_clone_run(
    request: CloneExperimentRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<ExperimentComparison, String> {
    execute_clone(&bridge, request)
}

#[tauri::command]
pub fn backtest_experiment_walk_forward(
    request: WalkForwardRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<WalkForwardReport, String> {
    execute_walk_forward(&bridge, request)
}

#[tauri::command]
pub fn backtest_experiment_walk_forward_latest(
    source_experiment_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Option<WalkForwardReport>, String> {
    latest_walk_forward(&bridge, &source_experiment_id)
}

#[tauri::command]
pub fn backtest_experiment_walk_forward_history(
    source_experiment_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<WalkForwardReport>, String> {
    walk_forward_history(&bridge, &source_experiment_id)
}

#[tauri::command]
pub fn backtest_experiment_bias_audit(
    experiment_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<ExperimentBiasAudit, String> {
    experiment_bias_audit(&bridge, &experiment_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::{EvidenceKind, Market, ReferenceEvidence, StrategySpec};

    fn request() -> CloneExperimentRequest {
        CloneExperimentRequest {
            source_experiment_id: "source-1".to_owned(),
            fast_window: 5,
            slow_window: 20,
            initial_cash_minor: 100_000,
            order_quantity: 1,
            close_open_position_at_end: true,
            buy_fee_bps: 1.5,
            sell_fee_bps: 1.5,
            sell_tax_bps: 20.0,
            slippage_bps: 0.0,
            stop_loss_bps: None,
            take_profit_bps: None,
            daily_loss_limit_minor: None,
        }
    }

    #[test]
    fn risk_limits_are_all_or_none() {
        let mut input = request();
        input.stop_loss_bps = Some(500);
        assert!(validate_request(&input).is_err());
        input.take_profit_bps = Some(1_000);
        input.daily_loss_limit_minor = Some(10_000);
        assert_eq!(
            validate_request(&input).unwrap().unwrap().stop_loss_bps,
            500
        );
    }

    #[test]
    fn moving_average_windows_must_be_ordered() {
        let mut input = request();
        input.fast_window = 20;
        input.slow_window = 5;
        assert!(validate_request(&input).is_err());
    }

    #[test]
    fn minimum_track_record_length_requires_enough_positive_non_constant_returns() {
        let positive = (0..40)
            .map(|index| if index % 3 == 0 { -0.002 } else { 0.004 })
            .collect::<Vec<_>>();
        assert!(minimum_track_record_length(&positive).is_some());
        assert!(minimum_track_record_length(&positive[..20]).is_none());
        assert!(minimum_track_record_length(&vec![-0.001; 40]).is_none());
    }

    #[test]
    fn clone_is_saved_as_a_new_research_experiment_without_mutating_source() {
        let bridge = PersistenceBridge::in_memory().unwrap();
        let report = ResearchReport {
            trace_id: "trace-source-1".to_owned(),
            request: "저장 실험 복제 테스트".to_owned(),
            evidence: vec![ReferenceEvidence {
                evidence_id: "evidence-1".to_owned(),
                kind: EvidenceKind::Documentation,
                source_url: "https://example.com/source".to_owned(),
                revision: None,
                license: None,
                summary: "테스트 근거".to_owned(),
                claimed_result: None,
            }],
            strategy_candidate: StrategySpec {
                schema_version: "1".to_owned(),
                strategy_id: "strategy-source-1".to_owned(),
                name: "이동평균 교차".to_owned(),
                market: Market::Korea,
                symbol: "005930".to_owned(),
                currency: "KRW".to_owned(),
                hypothesis: "단기 평균이 장기 평균을 상향 돌파하면 진입한다.".to_owned(),
                source_evidence_ids: vec!["evidence-1".to_owned()],
                entry_signal: SignalSpec::MovingAverageCross {
                    fast_window: 2,
                    slow_window: 4,
                    direction: CrossDirection::Above,
                },
                exit_signal: SignalSpec::MovingAverageCross {
                    fast_window: 2,
                    slow_window: 4,
                    direction: CrossDirection::Below,
                },
                limitations: vec!["합성 데이터 테스트".to_owned()],
                unknowns: vec![],
            },
        };
        let bars = (0..60_u64)
            .map(|index| {
                let close = if (index / 5) % 2 == 0 {
                    10_000 + index % 5 * 500
                } else {
                    12_000 - index % 5 * 500
                };
                PriceBar {
                    symbol: "005930".to_owned(),
                    currency: "KRW".to_owned(),
                    source: "TEST".to_owned(),
                    period_start_ms: index * 86_400_000,
                    period_end_ms: index * 86_400_000 + 86_399_999,
                    available_at_ms: index * 86_400_000 + 86_399_999,
                    ingested_at_ms: index * 86_400_000 + 86_399_999,
                    open_minor: close,
                    high_minor: close,
                    low_minor: close,
                    close_minor: close,
                    volume: 1_000,
                }
            })
            .collect::<Vec<_>>();
        let config = BacktestConfig {
            experiment_id: "source-1".to_owned(),
            dataset_id: "dataset-source-1".to_owned(),
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
        let review = review_strategy_spec(&report.strategy_candidate);
        let result = run_backtest(&report.strategy_candidate, &bars, &config).unwrap();
        bridge
            .persist_backtest(PersistBacktest {
                report: &report,
                review: &review,
                bars: &bars,
                config: &config,
                result: &result,
                provider: "TEST",
                interval: "1d",
                adjusted: false,
                warnings: &[],
                requested_at_ms: None,
                classification: "system_check",
            })
            .unwrap();

        let cloned = execute_clone(&bridge, request()).unwrap();
        assert_ne!(cloned.source_experiment_id, cloned.cloned_experiment_id);
        assert_eq!(cloned.cloned_config.dataset_id, config.dataset_id);
        let connection = bridge.connection.lock().unwrap();
        let rows: Vec<(String, String)> = connection
            .prepare(
                "SELECT experiment_id, classification FROM backtest_runs ORDER BY experiment_id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(rows.contains(&("source-1".to_owned(), "system_check".to_owned())));
        assert!(rows.contains(&(
            cloned.cloned_experiment_id,
            "research_experiment".to_owned()
        )));
        drop(connection);

        let walk_forward = execute_walk_forward(
            &bridge,
            WalkForwardRequest {
                source_experiment_id: "source-1".to_owned(),
                fold_count: 4,
            },
        )
        .unwrap();
        assert_eq!(walk_forward.folds.len(), 4);
        assert!(walk_forward
            .folds
            .windows(2)
            .all(|pair| pair[0].oos_end_ms <= pair[1].oos_start_ms));
        assert!(walk_forward
            .folds
            .iter()
            .all(|fold| fold.training_end_ms <= fold.oos_start_ms));
        assert!(walk_forward.largest_absolute_oos_return_share_bps <= 10_000);
        assert!(walk_forward.folds.iter().all(|fold| {
            fold.out_of_sample.exposure_bps <= 10_000 && fold.training.exposure_bps <= 10_000
        }));
        assert!(!walk_forward.meets_research_sample_minimum);
        assert_eq!(walk_forward.minimum_oos_trade_count, 200);
        assert!(!walk_forward.promotion_evaluation.eligible_for_paper_review);
        assert_eq!(
            walk_forward.promotion_blockers.len(),
            walk_forward
                .promotion_evaluation
                .checks
                .iter()
                .filter(|check| !check.passed)
                .count()
        );
        assert!(walk_forward.folds.iter().all(|fold| {
            fold.regimes.len() == 4
                && fold
                    .regimes
                    .iter()
                    .map(|regime| regime.completed_trade_count)
                    .sum::<usize>()
                    + fold.unclassified_trade_count
                    == fold.out_of_sample.completed_trade_count
        }));
        assert!(walk_forward
            .folds
            .iter()
            .all(|fold| fold.state_model.model_id == "two-state-volatility-markov-v1"));
        let state_model = state_model_diagnostic(&bars[..40], &bars[40..]);
        assert!(state_model.training_transition_count > 0);
        assert!(state_model.oos_transition_count > 0);
        assert!(state_model.transition_model_log_loss_milli.is_some());
        let persisted = latest_walk_forward(&bridge, "source-1").unwrap().unwrap();
        assert_eq!(persisted.validation_run_id, walk_forward.validation_run_id);
        assert_eq!(persisted.source_experiment_id, "source-1");
        assert_eq!(persisted.strategy_trial_count, 2);
        let connection = bridge.connection.lock().unwrap();
        let stored_count: u64 = connection
            .query_row("SELECT COUNT(*) FROM walk_forward_runs", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stored_count, 1);
        drop(connection);
        let history = walk_forward_history(&bridge, "source-1").unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].validation_run_id, persisted.validation_run_id);
        let bias_audit = experiment_bias_audit(&bridge, "source-1").unwrap();
        assert_eq!(bias_audit.local_strategy_trial_count, 2);
        assert_eq!(bias_audit.walk_forward_validation_count, 1);
        assert_eq!(bias_audit.oos_fold_count, 4);
        assert_eq!(bias_audit.data_snooping_status, "needs_review");
        assert_eq!(bias_audit.survivorship_bias_status, "needs_review");
        assert_eq!(bias_audit.catalog_completeness, "local_only");
        assert_eq!(bias_audit.universe_membership_evidence, "missing");

        let mut candidates = vec![walk_forward.clone(), walk_forward.clone(), walk_forward];
        for (strategy_index, candidate) in candidates.iter_mut().enumerate() {
            candidate.validation_run_id = format!("candidate-{strategy_index}");
            for (fold_index, fold) in candidate.folds.iter_mut().enumerate() {
                fold.out_of_sample.total_return_bps =
                    (strategy_index as i64 - 1) * 100 + fold_index as i64 * 25;
            }
        }
        let diagnostics = overfit_diagnostics(&candidates);
        assert_eq!(diagnostics.comparable_strategy_count, 3);
        assert!(diagnostics.evaluated_partition_count > 0);
        assert!(diagnostics
            .probability_of_backtest_overfitting_bps
            .is_some());
        assert!(diagnostics.deflated_sharpe_ratio_milli.is_none());

        let strategy_trial_count = candidates.len();
        for (strategy_index, candidate) in candidates.iter_mut().enumerate() {
            candidate.strategy_trial_count = strategy_trial_count;
            for (fold_index, fold) in candidate.folds.iter_mut().enumerate() {
                fold.out_of_sample.period_returns_ppm = (0..10)
                    .map(|period_index| {
                        let alternating = if period_index % 3 == 0 { -1 } else { 1 };
                        1_000
                            + strategy_index as i64 * 700
                            + fold_index as i64 * 90
                            + alternating * (period_index as i64 + 1) * 120
                    })
                    .collect();
            }
        }
        let diagnostics_with_complete_trials = overfit_diagnostics(&candidates);
        assert_eq!(
            diagnostics_with_complete_trials.comparable_strategy_count,
            3
        );
        assert!(diagnostics_with_complete_trials
            .deflated_sharpe_ratio_milli
            .is_some());
    }
}
