use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    backtest::PriceBar,
    research::{CrossDirection, Market, SignalSpec, StrategySpec},
};

const SUPPORTED_INTERVAL_SECONDS: [u64; 8] = [60, 180, 300, 900, 1_800, 3_600, 14_400, 86_400];
const MIN_FIXED_EXECUTION_SECONDS: u64 = 15;
const MAX_FIXED_EXECUTION_SECONDS: u64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyDecisionCadence {
    #[serde(rename = "tick")]
    Tick,
    #[serde(rename = "1m")]
    OneMinute,
    #[serde(rename = "3m")]
    ThreeMinutes,
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "30m")]
    ThirtyMinutes,
    #[serde(rename = "1h")]
    OneHour,
    #[serde(rename = "4h")]
    FourHours,
    #[serde(rename = "1d")]
    OneDay,
}

impl StrategyDecisionCadence {
    fn interval_seconds(self) -> Option<u64> {
        match self {
            Self::Tick => None,
            Self::OneMinute => Some(60),
            Self::ThreeMinutes => Some(180),
            Self::FiveMinutes => Some(300),
            Self::FifteenMinutes => Some(900),
            Self::ThirtyMinutes => Some(1_800),
            Self::OneHour => Some(3_600),
            Self::FourHours => Some(14_400),
            Self::OneDay => Some(86_400),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum StrategyExecutionCadence {
    Tick,
    FixedSeconds { seconds: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyDataProviderDescriptor {
    pub provider_id: &'static str,
    pub label: &'static str,
    pub supported_markets: Vec<Market>,
    pub supported_decision_cadences: Vec<StrategyDecisionCadence>,
    pub completed_bar_only: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyCadenceValidationRequest {
    pub strategy: StrategySpec,
    pub provider_id: String,
    pub decision_cadence: StrategyDecisionCadence,
    pub execution_cadence: StrategyExecutionCadence,
    pub backtest_interval_seconds: Option<u64>,
    pub runtime_interval_seconds: Option<u64>,
    pub available_bar_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyCadenceContract {
    pub plugin: StrategyPluginDescriptor,
    pub provider: StrategyDataProviderDescriptor,
    pub decision_cadence: StrategyDecisionCadence,
    pub execution_cadence: StrategyExecutionCadence,
    pub decision_interval_seconds: Option<u64>,
    pub requires_completed_bar: bool,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyPluginDescriptor {
    pub plugin_id: &'static str,
    pub version: u32,
    pub category: &'static str,
    pub required_bar_fields: Vec<&'static str>,
    pub supported_markets: Vec<Market>,
    pub supported_interval_seconds: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyPluginValidationRequest {
    pub strategy: StrategySpec,
    pub interval_seconds: u64,
    pub available_bar_fields: Vec<String>,
}

pub(crate) fn descriptor(signal: &SignalSpec) -> StrategyPluginDescriptor {
    let (plugin_id, category, required_bar_fields) = match signal {
        SignalSpec::MovingAverageCross { .. } => (
            "trend.moving_average_cross",
            "trend",
            vec!["close", "availableAt", "ingestedAt"],
        ),
        SignalSpec::PriceChannelBreakout { .. } => (
            "breakout.price_channel",
            "breakout",
            vec!["high", "low", "close", "availableAt", "ingestedAt"],
        ),
        SignalSpec::MeanReversion { .. } => (
            "mean_reversion.distance_from_mean",
            "mean_reversion",
            vec!["close", "availableAt", "ingestedAt"],
        ),
        SignalSpec::VolatilityExpansion { .. } => (
            "volatility.atr_expansion",
            "volatility",
            vec!["high", "low", "close", "availableAt", "ingestedAt"],
        ),
    };
    StrategyPluginDescriptor {
        plugin_id,
        version: 1,
        category,
        required_bar_fields,
        supported_markets: vec![Market::Korea, Market::UnitedStates, Market::Crypto],
        supported_interval_seconds: SUPPORTED_INTERVAL_SECONDS.to_vec(),
    }
}

#[tauri::command]
pub fn strategy_plugin_catalog() -> Vec<StrategyPluginDescriptor> {
    [
        SignalSpec::MovingAverageCross {
            fast_window: 5,
            slow_window: 20,
            direction: CrossDirection::Above,
        },
        SignalSpec::PriceChannelBreakout {
            lookback: 20,
            direction: CrossDirection::Above,
        },
        SignalSpec::MeanReversion {
            window: 20,
            deviation_bps: 200,
            direction: CrossDirection::Below,
        },
        SignalSpec::VolatilityExpansion {
            atr_window: 14,
            breakout_window: 20,
            minimum_expansion_bps: 12_500,
            direction: CrossDirection::Above,
        },
    ]
    .iter()
    .map(descriptor)
    .collect()
}

fn all_bar_cadences() -> Vec<StrategyDecisionCadence> {
    vec![
        StrategyDecisionCadence::OneMinute,
        StrategyDecisionCadence::ThreeMinutes,
        StrategyDecisionCadence::FiveMinutes,
        StrategyDecisionCadence::FifteenMinutes,
        StrategyDecisionCadence::ThirtyMinutes,
        StrategyDecisionCadence::OneHour,
        StrategyDecisionCadence::FourHours,
        StrategyDecisionCadence::OneDay,
    ]
}

#[tauri::command]
pub fn strategy_cadence_catalog() -> Vec<StrategyDataProviderDescriptor> {
    vec![
        StrategyDataProviderDescriptor {
            provider_id: "stored_pit_dataset",
            label: "저장된 시점 정합 데이터셋",
            supported_markets: vec![Market::Korea, Market::UnitedStates, Market::Crypto],
            supported_decision_cadences: all_bar_cadences(),
            completed_bar_only: true,
        },
        StrategyDataProviderDescriptor {
            provider_id: "local_completed_bar_aggregation",
            label: "로컬 실시간 완료 봉 집계",
            supported_markets: vec![Market::Crypto],
            supported_decision_cadences: all_bar_cadences()
                .into_iter()
                .filter(|cadence| *cadence != StrategyDecisionCadence::OneDay)
                .collect(),
            completed_bar_only: true,
        },
        StrategyDataProviderDescriptor {
            provider_id: "normalized_trade_stream",
            label: "정규화 실시간 체결 스트림",
            supported_markets: vec![Market::Crypto],
            supported_decision_cadences: vec![StrategyDecisionCadence::Tick],
            completed_bar_only: false,
        },
    ]
}

#[tauri::command]
pub fn strategy_cadence_validate(
    request: StrategyCadenceValidationRequest,
) -> Result<StrategyCadenceContract, String> {
    let provider = strategy_cadence_catalog()
        .into_iter()
        .find(|item| item.provider_id == request.provider_id)
        .ok_or_else(|| "서버가 검증한 전략 데이터 공급자가 아닙니다.".to_owned())?;
    if !provider
        .supported_markets
        .contains(&request.strategy.market)
    {
        return Err(format!(
            "{} 공급자는 요청한 자산 시장을 지원하지 않습니다.",
            provider.label
        ));
    }
    if !provider
        .supported_decision_cadences
        .contains(&request.decision_cadence)
    {
        return Err(format!(
            "{} 공급자는 요청한 판단 주기를 지원하지 않습니다.",
            provider.label
        ));
    }

    let interval_seconds = request.decision_cadence.interval_seconds().ok_or_else(|| {
        "현재 전략 플러그인은 완료 봉 기반입니다. tick 판단에는 별도의 체결·호가 전략이 필요합니다."
            .to_owned()
    })?;
    if request.backtest_interval_seconds != Some(interval_seconds)
        || request.runtime_interval_seconds != Some(interval_seconds)
    {
        return Err(
            "판단 주기와 백테스트·런타임 interval이 모두 정확히 일치해야 합니다.".to_owned(),
        );
    }
    if let StrategyExecutionCadence::FixedSeconds { seconds } = &request.execution_cadence {
        if !(MIN_FIXED_EXECUTION_SECONDS..=MAX_FIXED_EXECUTION_SECONDS).contains(seconds) {
            return Err(format!(
                "고정 실행 관리 주기는 {MIN_FIXED_EXECUTION_SECONDS}초 이상 {MAX_FIXED_EXECUTION_SECONDS}초 이하여야 합니다."
            ));
        }
    }

    let plugin = strategy_plugin_validate(StrategyPluginValidationRequest {
        strategy: request.strategy,
        interval_seconds,
        available_bar_fields: request.available_bar_fields,
    })?;
    Ok(StrategyCadenceContract {
        plugin,
        provider,
        decision_cadence: request.decision_cadence,
        execution_cadence: request.execution_cadence,
        decision_interval_seconds: Some(interval_seconds),
        requires_completed_bar: true,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn strategy_plugin_validate(
    request: StrategyPluginValidationRequest,
) -> Result<StrategyPluginDescriptor, String> {
    let review = crate::research::review_strategy_spec(&request.strategy);
    if !review.executable {
        return Err("검증되지 않았거나 미해결 항목이 있는 전략은 실행할 수 없습니다.".to_owned());
    }
    if signal_family(&request.strategy.entry_signal) != signal_family(&request.strategy.exit_signal)
    {
        return Err("진입과 청산은 동일한 전략 플러그인 계열이어야 합니다.".to_owned());
    }
    let plugin = descriptor(&request.strategy.entry_signal);
    if !plugin.supported_markets.contains(&request.strategy.market)
        || !plugin
            .supported_interval_seconds
            .contains(&request.interval_seconds)
    {
        return Err(format!(
            "{} v{}가 지원하지 않는 시장 또는 봉 주기입니다.",
            plugin.plugin_id, plugin.version
        ));
    }
    let available = request
        .available_bar_fields
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if plugin
        .required_bar_fields
        .iter()
        .any(|field| !available.contains(field))
    {
        return Err("전략 플러그인에 필요한 시점 정합 가격봉 필드가 누락되었습니다.".to_owned());
    }
    Ok(plugin)
}

fn signal_family(signal: &SignalSpec) -> &'static str {
    descriptor(signal).plugin_id
}

pub fn minimum_history(signal: &SignalSpec) -> usize {
    match signal {
        SignalSpec::MovingAverageCross { slow_window, .. } => slow_window.saturating_add(1),
        SignalSpec::PriceChannelBreakout { lookback, .. }
        | SignalSpec::MeanReversion {
            window: lookback, ..
        } => lookback.saturating_add(1),
        SignalSpec::VolatilityExpansion {
            atr_window,
            breakout_window,
            ..
        } => atr_window.max(breakout_window).saturating_add(2),
    }
}

pub fn validate_runtime_contract(spec: &StrategySpec, bars: &[PriceBar]) -> Result<(), String> {
    if signal_family(&spec.entry_signal) != signal_family(&spec.exit_signal) {
        return Err("진입과 청산은 동일한 전략 플러그인 계열이어야 합니다.".to_owned());
    }
    let plugin = descriptor(&spec.entry_signal);
    if !plugin.supported_markets.contains(&spec.market) {
        return Err(format!(
            "{} v{}가 지원하지 않는 시장입니다.",
            plugin.plugin_id, plugin.version
        ));
    }
    let required = minimum_history(&spec.entry_signal).max(minimum_history(&spec.exit_signal));
    if bars.len() <= required {
        return Err(format!(
            "{} v{} 신호와 다음 봉 체결에 필요한 완료 봉이 부족합니다.",
            plugin.plugin_id, plugin.version
        ));
    }
    Ok(())
}

fn mean_close(bars: &[PriceBar]) -> Result<u128, String> {
    let total = bars.iter().try_fold(0_u128, |sum, bar| {
        sum.checked_add(u128::from(bar.close_minor))
            .ok_or_else(|| "전략 평균 계산이 범위를 초과했습니다.".to_owned())
    })?;
    Ok(total / bars.len() as u128)
}

fn price_channel(
    bars: &[PriceBar],
    index: usize,
    lookback: usize,
    direction: CrossDirection,
) -> Result<bool, String> {
    if index < lookback {
        return Ok(false);
    }
    let history = &bars[index - lookback..index];
    let current = bars[index].close_minor;
    Ok(match direction {
        CrossDirection::Above => {
            current > history.iter().map(|bar| bar.high_minor).max().unwrap_or(0)
        }
        CrossDirection::Below => {
            current
                < history
                    .iter()
                    .map(|bar| bar.low_minor)
                    .min()
                    .unwrap_or(u64::MAX)
        }
    })
}

fn true_range(bar: &PriceBar, previous_close: u64) -> u64 {
    let high_low = bar.high_minor.saturating_sub(bar.low_minor);
    let high_close = bar.high_minor.abs_diff(previous_close);
    let low_close = bar.low_minor.abs_diff(previous_close);
    high_low.max(high_close).max(low_close)
}

pub fn signal_matches(
    bars: &[PriceBar],
    index: usize,
    signal: &SignalSpec,
) -> Result<bool, String> {
    match signal {
        SignalSpec::MovingAverageCross {
            fast_window,
            slow_window,
            direction,
        } => {
            if index < *slow_window {
                return Ok(false);
            }
            let previous_fast = mean_close(&bars[index - fast_window..index])?;
            let previous_slow = mean_close(&bars[index - slow_window..index])?;
            let current_fast = mean_close(&bars[index + 1 - fast_window..=index])?;
            let current_slow = mean_close(&bars[index + 1 - slow_window..=index])?;
            Ok(match direction {
                CrossDirection::Above => {
                    previous_fast <= previous_slow && current_fast > current_slow
                }
                CrossDirection::Below => {
                    previous_fast >= previous_slow && current_fast < current_slow
                }
            })
        }
        SignalSpec::PriceChannelBreakout {
            lookback,
            direction,
        } => price_channel(bars, index, *lookback, *direction),
        SignalSpec::MeanReversion {
            window,
            deviation_bps,
            direction,
        } => {
            if index < *window {
                return Ok(false);
            }
            let average = mean_close(&bars[index + 1 - window..=index])?;
            let current = u128::from(bars[index].close_minor);
            let threshold = match direction {
                CrossDirection::Above => average
                    .checked_mul(u128::from(10_000 + deviation_bps))
                    .map(|value| value / 10_000),
                CrossDirection::Below => average
                    .checked_mul(u128::from(10_000 - deviation_bps))
                    .map(|value| value / 10_000),
            }
            .ok_or_else(|| "평균회귀 임계값 계산이 범위를 초과했습니다.".to_owned())?;
            Ok(match direction {
                CrossDirection::Above => current > threshold,
                CrossDirection::Below => current < threshold,
            })
        }
        SignalSpec::VolatilityExpansion {
            atr_window,
            breakout_window,
            minimum_expansion_bps,
            direction,
        } => {
            let required = atr_window.max(breakout_window).saturating_add(1);
            if index < required {
                return Ok(false);
            }
            let atr_start = index - atr_window;
            let total = (atr_start..index).try_fold(0_u128, |sum, cursor| {
                sum.checked_add(u128::from(true_range(
                    &bars[cursor],
                    bars[cursor - 1].close_minor,
                )))
                .ok_or_else(|| "ATR 합계가 범위를 초과했습니다.".to_owned())
            })?;
            let atr = total / *atr_window as u128;
            if atr == 0 {
                return Ok(false);
            }
            let current_range = u128::from(true_range(&bars[index], bars[index - 1].close_minor));
            let expansion_bps = current_range
                .checked_mul(10_000)
                .map(|value| value / atr)
                .ok_or_else(|| "변동성 확장 비율이 범위를 초과했습니다.".to_owned())?;
            Ok(expansion_bps >= u128::from(*minimum_expansion_bps)
                && price_channel(bars, index, *breakout_window, *direction)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(index: u64, close: u64) -> PriceBar {
        PriceBar {
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            source: "fixture".to_owned(),
            period_start_ms: index * 60_000,
            period_end_ms: (index + 1) * 60_000,
            available_at_ms: (index + 1) * 60_000,
            ingested_at_ms: (index + 1) * 60_000,
            open_minor: close,
            high_minor: close + 2,
            low_minor: close.saturating_sub(2).max(1),
            close_minor: close,
            volume: 100,
        }
    }

    fn spec(entry_signal: SignalSpec, exit_signal: SignalSpec) -> StrategySpec {
        StrategySpec {
            schema_version: "1".to_owned(),
            strategy_id: "plugin-test".to_owned(),
            name: "플러그인 테스트".to_owned(),
            market: Market::Korea,
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            hypothesis: "완료 봉만 사용한다.".to_owned(),
            source_evidence_ids: vec!["fixture".to_owned()],
            entry_signal,
            exit_signal,
            limitations: vec!["테스트 전용".to_owned()],
            unknowns: Vec::new(),
        }
    }

    #[test]
    fn catalog_exposes_four_versioned_deterministic_plugins() {
        let catalog = strategy_plugin_catalog();
        assert_eq!(catalog.len(), 4);
        assert!(catalog.iter().all(|item| item.version == 1));
        assert!(catalog
            .iter()
            .all(|item| item.supported_interval_seconds.contains(&60)));
    }

    #[test]
    fn rejects_mixed_plugins_and_unsupported_intervals() {
        let mixed = spec(
            SignalSpec::PriceChannelBreakout {
                lookback: 2,
                direction: CrossDirection::Above,
            },
            SignalSpec::MovingAverageCross {
                fast_window: 2,
                slow_window: 3,
                direction: CrossDirection::Below,
            },
        );
        let bars = (0..8)
            .map(|index| bar(index, 100 + index))
            .collect::<Vec<_>>();
        assert!(validate_runtime_contract(&mixed, &bars).is_err());

        let valid = spec(
            SignalSpec::PriceChannelBreakout {
                lookback: 2,
                direction: CrossDirection::Above,
            },
            SignalSpec::PriceChannelBreakout {
                lookback: 2,
                direction: CrossDirection::Below,
            },
        );
        assert!(strategy_plugin_validate(StrategyPluginValidationRequest {
            strategy: valid.clone(),
            interval_seconds: 1,
            available_bar_fields: vec![
                "high".to_owned(),
                "low".to_owned(),
                "close".to_owned(),
                "availableAt".to_owned(),
                "ingestedAt".to_owned(),
            ],
        })
        .is_err());
        assert!(strategy_plugin_validate(StrategyPluginValidationRequest {
            strategy: valid,
            interval_seconds: 60,
            available_bar_fields: vec![
                "close".to_owned(),
                "availableAt".to_owned(),
                "ingestedAt".to_owned(),
            ],
        })
        .is_err());
    }

    fn valid_breakout_spec() -> StrategySpec {
        spec(
            SignalSpec::PriceChannelBreakout {
                lookback: 2,
                direction: CrossDirection::Above,
            },
            SignalSpec::PriceChannelBreakout {
                lookback: 2,
                direction: CrossDirection::Below,
            },
        )
    }

    fn complete_bar_fields() -> Vec<String> {
        ["high", "low", "close", "availableAt", "ingestedAt"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn cadence_catalog_exposes_nine_closed_decision_cadences() {
        let providers = strategy_cadence_catalog();
        let mut cadences = providers
            .iter()
            .flat_map(|provider| provider.supported_decision_cadences.iter().copied())
            .collect::<Vec<_>>();
        cadences.sort_by_key(|cadence| cadence.interval_seconds().unwrap_or(0));
        cadences.dedup();
        assert_eq!(cadences.len(), 9);
        assert_eq!(all_bar_cadences().len(), 8);
    }

    #[test]
    fn cadence_contract_separates_bar_decision_from_tick_execution() {
        let contract = strategy_cadence_validate(StrategyCadenceValidationRequest {
            strategy: valid_breakout_spec(),
            provider_id: "stored_pit_dataset".to_owned(),
            decision_cadence: StrategyDecisionCadence::FiveMinutes,
            execution_cadence: StrategyExecutionCadence::Tick,
            backtest_interval_seconds: Some(300),
            runtime_interval_seconds: Some(300),
            available_bar_fields: complete_bar_fields(),
        })
        .unwrap();
        assert_eq!(contract.decision_interval_seconds, Some(300));
        assert!(contract.requires_completed_bar);
        assert!(!contract.live_order_allowed);
        assert_eq!(contract.execution_cadence, StrategyExecutionCadence::Tick);
    }

    #[test]
    fn cadence_contract_rejects_tick_for_bar_plugins_and_interval_mismatch() {
        let mut crypto = valid_breakout_spec();
        crypto.market = Market::Crypto;
        assert!(strategy_cadence_validate(StrategyCadenceValidationRequest {
            strategy: crypto,
            provider_id: "normalized_trade_stream".to_owned(),
            decision_cadence: StrategyDecisionCadence::Tick,
            execution_cadence: StrategyExecutionCadence::Tick,
            backtest_interval_seconds: None,
            runtime_interval_seconds: None,
            available_bar_fields: complete_bar_fields(),
        })
        .unwrap_err()
        .contains("별도의 체결·호가 전략"));

        assert!(strategy_cadence_validate(StrategyCadenceValidationRequest {
            strategy: valid_breakout_spec(),
            provider_id: "stored_pit_dataset".to_owned(),
            decision_cadence: StrategyDecisionCadence::OneHour,
            execution_cadence: StrategyExecutionCadence::FixedSeconds { seconds: 60 },
            backtest_interval_seconds: Some(3_600),
            runtime_interval_seconds: Some(300),
            available_bar_fields: complete_bar_fields(),
        })
        .unwrap_err()
        .contains("정확히 일치"));
    }

    #[test]
    fn cadence_contract_rejects_unverified_provider_and_invalid_execution_range() {
        assert!(strategy_cadence_validate(StrategyCadenceValidationRequest {
            strategy: valid_breakout_spec(),
            provider_id: "client_claimed_provider".to_owned(),
            decision_cadence: StrategyDecisionCadence::OneMinute,
            execution_cadence: StrategyExecutionCadence::Tick,
            backtest_interval_seconds: Some(60),
            runtime_interval_seconds: Some(60),
            available_bar_fields: complete_bar_fields(),
        })
        .unwrap_err()
        .contains("서버가 검증"));

        for seconds in [0, 14, 86_401] {
            assert!(strategy_cadence_validate(StrategyCadenceValidationRequest {
                strategy: valid_breakout_spec(),
                provider_id: "stored_pit_dataset".to_owned(),
                decision_cadence: StrategyDecisionCadence::OneMinute,
                execution_cadence: StrategyExecutionCadence::FixedSeconds { seconds },
                backtest_interval_seconds: Some(60),
                runtime_interval_seconds: Some(60),
                available_bar_fields: complete_bar_fields(),
            })
            .unwrap_err()
            .contains("15초 이상"));
        }
    }

    #[test]
    fn evaluates_breakout_mean_reversion_and_volatility_without_future_bars() {
        let mut breakout = vec![bar(0, 100), bar(1, 101), bar(2, 102)];
        breakout.push(bar(3, 110));
        assert!(signal_matches(
            &breakout,
            3,
            &SignalSpec::PriceChannelBreakout {
                lookback: 3,
                direction: CrossDirection::Above,
            }
        )
        .unwrap());

        let mean_reversion = vec![bar(0, 100), bar(1, 100), bar(2, 100), bar(3, 80)];
        assert!(signal_matches(
            &mean_reversion,
            3,
            &SignalSpec::MeanReversion {
                window: 3,
                deviation_bps: 1_000,
                direction: CrossDirection::Below,
            }
        )
        .unwrap());

        let mut volatility = (0..6).map(|index| bar(index, 100)).collect::<Vec<_>>();
        volatility.push(bar(6, 130));
        assert!(signal_matches(
            &volatility,
            6,
            &SignalSpec::VolatilityExpansion {
                atr_window: 3,
                breakout_window: 3,
                minimum_expansion_bps: 15_000,
                direction: CrossDirection::Above,
            }
        )
        .unwrap());
    }
}
