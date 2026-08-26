use serde::{Deserialize, Serialize};

use crate::backtest::PriceBar;

const MIN_PUBLISHED_SAMPLE: usize = 20;
const DEFAULT_SCAN_BARS: usize = 1_500;
const MAX_SEQUENCE: usize = 20;
const BB_WINDOW: usize = 20;
const BB_STD_MULTIPLIER: f64 = 2.0;
const HORIZONS: [usize; 4] = [1, 3, 5, 20];

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandleDirection {
    Bullish,
    Bearish,
    Doji,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfidenceIntervalBps {
    pub low: u64,
    pub high: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectionDistribution {
    pub sample_size: usize,
    pub bullish_count: usize,
    pub bearish_count: usize,
    pub doji_count: usize,
    pub bullish_probability_bps: Option<u64>,
    pub bearish_probability_bps: Option<u64>,
    pub doji_probability_bps: Option<u64>,
    pub bullish_confidence_interval_bps: Option<ConfidenceIntervalBps>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HorizonDistribution {
    pub horizon_bars: usize,
    pub sample_size: usize,
    pub positive_count: usize,
    pub negative_count: usize,
    pub flat_count: usize,
    pub positive_probability_bps: Option<u64>,
    pub negative_probability_bps: Option<u64>,
    pub flat_probability_bps: Option<u64>,
    pub positive_confidence_interval_bps: Option<ConfidenceIntervalBps>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BollingerBehavior {
    pub current_position: String,
    pub upper_sample_size: usize,
    pub upper_breakout_probability_bps: Option<u64>,
    pub upper_reversal_probability_bps: Option<u64>,
    pub lower_sample_size: usize,
    pub lower_bounce_probability_bps: Option<u64>,
    pub lower_breakdown_probability_bps: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternProbabilityReport {
    pub source_revision: String,
    pub minimum_published_sample: usize,
    pub scanned_bar_count: usize,
    pub current_sequence_direction: CandleDirection,
    pub current_sequence_count: usize,
    pub next_candle: DirectionDistribution,
    pub horizon_outcomes: Vec<HorizonDistribution>,
    pub bollinger: BollingerBehavior,
    pub warnings: Vec<String>,
}

fn direction(bar: &PriceBar) -> CandleDirection {
    match bar.close_minor.cmp(&bar.open_minor) {
        std::cmp::Ordering::Greater => CandleDirection::Bullish,
        std::cmp::Ordering::Less => CandleDirection::Bearish,
        std::cmp::Ordering::Equal => CandleDirection::Doji,
    }
}

fn probability(count: usize, sample_size: usize) -> Option<u64> {
    (sample_size >= MIN_PUBLISHED_SAMPLE)
        .then(|| ((count as u128 * 10_000) / sample_size as u128) as u64)
}

fn wilson_interval(successes: usize, sample_size: usize) -> Option<ConfidenceIntervalBps> {
    if sample_size < MIN_PUBLISHED_SAMPLE {
        return None;
    }
    let n = sample_size as f64;
    let p = successes as f64 / n;
    let z = 1.959_963_984_540_054_f64;
    let denominator = 1.0 + z * z / n;
    let center = (p + z * z / (2.0 * n)) / denominator;
    let margin = z * ((p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt()) / denominator;
    Some(ConfidenceIntervalBps {
        low: ((center - margin).clamp(0.0, 1.0) * 10_000.0).round() as u64,
        high: ((center + margin).clamp(0.0, 1.0) * 10_000.0).round() as u64,
    })
}

fn distribution(directions: impl Iterator<Item = CandleDirection>) -> DirectionDistribution {
    let mut bullish = 0;
    let mut bearish = 0;
    let mut doji = 0;
    for value in directions {
        match value {
            CandleDirection::Bullish => bullish += 1,
            CandleDirection::Bearish => bearish += 1,
            CandleDirection::Doji => doji += 1,
        }
    }
    let sample_size = bullish + bearish + doji;
    DirectionDistribution {
        sample_size,
        bullish_count: bullish,
        bearish_count: bearish,
        doji_count: doji,
        bullish_probability_bps: probability(bullish, sample_size),
        bearish_probability_bps: probability(bearish, sample_size),
        doji_probability_bps: probability(doji, sample_size),
        bullish_confidence_interval_bps: wilson_interval(bullish, sample_size),
    }
}

fn exact_sequence_matches(
    directions: &[CandleDirection],
    current: CandleDirection,
    count: usize,
) -> Vec<usize> {
    if count == 0 || directions.len() <= count {
        return Vec::new();
    }
    let historical_end = directions.len() - 1;
    (count + 1..historical_end)
        .filter(|&next_index| {
            directions[next_index - count..next_index]
                .iter()
                .all(|value| *value == current)
                && directions[next_index - count - 1] != current
        })
        .collect()
}

fn mean(values: &[u64]) -> f64 {
    values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64
}

fn standard_deviation(values: &[u64], average: f64) -> f64 {
    let variance = values
        .iter()
        .map(|value| {
            let difference = *value as f64 - average;
            difference * difference
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

pub fn analyze_pattern_probabilities(bars: &[PriceBar]) -> Option<PatternProbabilityReport> {
    if bars.len() < BB_WINDOW + 21 {
        return None;
    }
    let start = bars.len().saturating_sub(DEFAULT_SCAN_BARS);
    let bars = &bars[start..];
    let directions = bars.iter().map(direction).collect::<Vec<_>>();
    let current_direction = *directions.last()?;
    let current_sequence_count = directions
        .iter()
        .rev()
        .take(MAX_SEQUENCE)
        .take_while(|value| **value == current_direction)
        .count();
    let matches = exact_sequence_matches(&directions, current_direction, current_sequence_count);
    let next_candle = distribution(matches.iter().map(|index| directions[*index]));

    let mut horizon_outcomes = Vec::new();
    for horizon in HORIZONS {
        let mut positive = 0;
        let mut negative = 0;
        let mut flat = 0;
        for &next_index in &matches {
            let outcome_index = next_index + horizon - 1;
            if outcome_index >= bars.len() - 1 {
                continue;
            }
            match bars[outcome_index]
                .close_minor
                .cmp(&bars[next_index - 1].close_minor)
            {
                std::cmp::Ordering::Greater => positive += 1,
                std::cmp::Ordering::Less => negative += 1,
                std::cmp::Ordering::Equal => flat += 1,
            }
        }
        let sample_size = positive + negative + flat;
        horizon_outcomes.push(HorizonDistribution {
            horizon_bars: horizon,
            sample_size,
            positive_count: positive,
            negative_count: negative,
            flat_count: flat,
            positive_probability_bps: probability(positive, sample_size),
            negative_probability_bps: probability(negative, sample_size),
            flat_probability_bps: probability(flat, sample_size),
            positive_confidence_interval_bps: wilson_interval(positive, sample_size),
        });
    }

    let mut upper_breakout = 0;
    let mut upper_reversal = 0;
    let mut lower_bounce = 0;
    let mut lower_breakdown = 0;
    let mut latest_upper = 0.0;
    let mut latest_lower = 0.0;
    let mut latest_basis = 0.0;
    for index in BB_WINDOW - 1..bars.len() {
        let closes = bars[index + 1 - BB_WINDOW..=index]
            .iter()
            .map(|bar| bar.close_minor)
            .collect::<Vec<_>>();
        let basis = mean(&closes);
        let deviation = standard_deviation(&closes, basis);
        let upper = basis + BB_STD_MULTIPLIER * deviation;
        let lower = basis - BB_STD_MULTIPLIER * deviation;
        if index == bars.len() - 1 {
            latest_upper = upper;
            latest_lower = lower;
            latest_basis = basis;
        }
        if index + 1 >= bars.len() {
            continue;
        }
        let next = &bars[index + 1];
        if bars[index].high_minor as f64 >= upper || bars[index].close_minor as f64 >= upper {
            if next.close_minor as f64 > upper || next.high_minor > bars[index].high_minor {
                upper_breakout += 1;
            } else if (next.close_minor as f64) < basis
                || next.close_minor < bars[index].close_minor
            {
                upper_reversal += 1;
            }
        }
        if bars[index].low_minor as f64 <= lower || bars[index].close_minor as f64 <= lower {
            if next.close_minor as f64 > basis || next.close_minor > bars[index].close_minor {
                lower_bounce += 1;
            } else if (next.close_minor as f64) < lower || next.low_minor < bars[index].low_minor {
                lower_breakdown += 1;
            }
        }
    }
    let latest = bars.last()?;
    let current_position = if latest.high_minor as f64 >= latest_upper
        || latest.close_minor as f64 >= latest_upper
    {
        "upper".to_owned()
    } else if latest.low_minor as f64 <= latest_lower || latest.close_minor as f64 <= latest_lower {
        "lower".to_owned()
    } else if latest.close_minor as f64 >= latest_basis {
        "middle_upper".to_owned()
    } else {
        "middle_lower".to_owned()
    };
    let upper_sample_size = upper_breakout + upper_reversal;
    let lower_sample_size = lower_bounce + lower_breakdown;
    let mut warnings = Vec::new();
    if next_candle.sample_size < MIN_PUBLISHED_SAMPLE {
        warnings
            .push("동일 연속 봉 표본이 20회 미만이어서 방향 확률을 공개하지 않습니다.".to_owned());
    }
    if upper_sample_size < MIN_PUBLISHED_SAMPLE || lower_sample_size < MIN_PUBLISHED_SAMPLE {
        warnings.push("볼린저 행동 표본이 부족한 구간의 확률은 공개하지 않습니다.".to_owned());
    }

    Some(PatternProbabilityReport {
        source_revision:
            "Bum-niverse/FinPilot@dca1e4b4ca280a28b3b6c40380c1e06adad5532d-reimplemented".to_owned(),
        minimum_published_sample: MIN_PUBLISHED_SAMPLE,
        scanned_bar_count: bars.len(),
        current_sequence_direction: current_direction,
        current_sequence_count,
        next_candle,
        horizon_outcomes,
        bollinger: BollingerBehavior {
            current_position,
            upper_sample_size,
            upper_breakout_probability_bps: probability(upper_breakout, upper_sample_size),
            upper_reversal_probability_bps: probability(upper_reversal, upper_sample_size),
            lower_sample_size,
            lower_bounce_probability_bps: probability(lower_bounce, lower_sample_size),
            lower_breakdown_probability_bps: probability(lower_breakdown, lower_sample_size),
        },
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(count: usize) -> Vec<PriceBar> {
        (0..count)
            .map(|index| {
                let open = 10_000 + ((index % 7) as u64 * 100);
                let close = match index % 5 {
                    0 | 1 => open + 80,
                    2 | 3 => open.saturating_sub(60),
                    _ => open,
                };
                PriceBar {
                    symbol: "005930".to_owned(),
                    currency: "KRW".to_owned(),
                    source: "fixture".to_owned(),
                    period_start_ms: index as u64 * 86_400_000,
                    period_end_ms: index as u64 * 86_400_000 + 80_000_000,
                    available_at_ms: index as u64 * 86_400_000 + 81_000_000,
                    ingested_at_ms: index as u64 * 86_400_000 + 82_000_000,
                    open_minor: open,
                    high_minor: open.max(close) + 50,
                    low_minor: open.min(close) - 50,
                    close_minor: close,
                    volume: 1_000,
                }
            })
            .collect()
    }

    #[test]
    fn does_not_publish_probabilities_below_the_minimum_sample() {
        let report = analyze_pattern_probabilities(&bars(70)).expect("report");
        if report.next_candle.sample_size < MIN_PUBLISHED_SAMPLE {
            assert_eq!(report.next_candle.bullish_probability_bps, None);
            assert_eq!(report.next_candle.bullish_confidence_interval_bps, None);
        }
    }

    #[test]
    fn future_bar_changes_do_not_change_the_previous_as_of_report() {
        let fixture = bars(220);
        let before = analyze_pattern_probabilities(&fixture[..200]).expect("before");
        let mut changed_future = fixture.clone();
        for bar in &mut changed_future[200..] {
            bar.close_minor = bar.open_minor + 5_000;
            bar.high_minor = bar.close_minor + 10;
        }
        let after = analyze_pattern_probabilities(&changed_future[..200]).expect("after");
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
    }

    #[test]
    fn exposes_all_configured_horizons_and_three_way_counts() {
        let report = analyze_pattern_probabilities(&bars(500)).expect("report");
        assert_eq!(
            report
                .horizon_outcomes
                .iter()
                .map(|item| item.horizon_bars)
                .collect::<Vec<_>>(),
            HORIZONS
        );
        assert_eq!(
            report.next_candle.sample_size,
            report.next_candle.bullish_count
                + report.next_candle.bearish_count
                + report.next_candle.doji_count
        );
        if report.next_candle.sample_size >= MIN_PUBLISHED_SAMPLE {
            assert!(report.next_candle.bullish_probability_bps.is_some());
            assert!(report.next_candle.bullish_confidence_interval_bps.is_some());
        }
    }
}
