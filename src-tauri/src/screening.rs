use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::research::Market;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentStatus {
    Tradable,
    Halted,
    Managed,
    Delisted,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniverseEntry {
    pub symbol: String,
    pub market: Market,
    pub status: InstrumentStatus,
    pub effective_from_ms: u64,
    pub effective_to_ms: Option<u64>,
    pub spread_bps: Option<u64>,
    pub abnormal_spread_bps: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UniverseVersion {
    pub universe_id: String,
    pub version: u32,
    pub active_markets: Vec<Market>,
    pub entries: Vec<UniverseEntry>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleOperator {
    GreaterOrEqual,
    LessOrEqual,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreeningRule {
    pub rule_id: String,
    pub metric: String,
    pub operator: RuleOperator,
    pub threshold: i64,
    pub score_weight_bps: u64,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreeningStrategy {
    pub strategy_id: String,
    pub version: u32,
    pub rules: Vec<ScreeningRule>,
    pub maximum_candidates_per_market: usize,
    pub analysis_budget: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreeningObservation {
    pub symbol: String,
    pub market: Market,
    pub observed_at_ms: u64,
    pub metrics: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreeningReason {
    pub rule_id: String,
    pub observed_value: Option<i64>,
    pub threshold: Option<i64>,
    pub passed: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenedCandidate {
    pub symbol: String,
    pub market: Market,
    pub score_bps: u64,
    pub included: bool,
    pub reasons: Vec<ScreeningReason>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreeningResult {
    pub universe_id: String,
    pub universe_version: u32,
    pub strategy_id: String,
    pub strategy_version: u32,
    pub as_of_ms: u64,
    pub candidates: Vec<ScreenedCandidate>,
    pub excluded: Vec<ScreenedCandidate>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

pub fn validate_screening_contract(
    universe: &UniverseVersion,
    strategy: &ScreeningStrategy,
) -> Result<(), String> {
    if !valid_id(&universe.universe_id)
        || universe.version == 0
        || universe.active_markets.is_empty()
        || !valid_id(&strategy.strategy_id)
        || strategy.version == 0
        || strategy.maximum_candidates_per_market == 0
        || strategy.analysis_budget == 0
    {
        return Err("유니버스와 전략에는 불변 ID·버전·실행 상한이 필요합니다.".to_owned());
    }
    let mut markets = BTreeSet::new();
    for market in &universe.active_markets {
        if !markets.insert(format!("{market:?}")) {
            return Err("활성 시장은 중복될 수 없습니다.".to_owned());
        }
    }
    let mut entries = BTreeSet::new();
    for entry in &universe.entries {
        if entry.symbol.trim().is_empty()
            || entry.effective_from_ms == 0
            || entry
                .effective_to_ms
                .is_some_and(|end| end <= entry.effective_from_ms)
            || entry.abnormal_spread_bps == 0
            || !entries.insert((format!("{:?}", entry.market), entry.symbol.as_str()))
        {
            return Err(
                "유니버스 종목의 적용 기간·종목·스프레드 계약이 올바르지 않습니다.".to_owned(),
            );
        }
    }
    let mut rule_ids = BTreeSet::new();
    let mut bounds: BTreeMap<&str, (Option<i64>, Option<i64>)> = BTreeMap::new();
    for rule in &strategy.rules {
        if !valid_id(&rule.rule_id)
            || rule.metric.trim().is_empty()
            || rule.description.trim().is_empty()
            || rule.score_weight_bps > 10_000
            || !rule_ids.insert(rule.rule_id.as_str())
        {
            return Err("스크리닝 규칙의 ID·설명·가중치가 올바르지 않습니다.".to_owned());
        }
        let bound = bounds.entry(rule.metric.as_str()).or_default();
        match rule.operator {
            RuleOperator::GreaterOrEqual => {
                bound.0 = Some(
                    bound
                        .0
                        .map_or(rule.threshold, |value| value.max(rule.threshold)),
                )
            }
            RuleOperator::LessOrEqual => {
                bound.1 = Some(
                    bound
                        .1
                        .map_or(rule.threshold, |value| value.min(rule.threshold)),
                )
            }
        }
    }
    if bounds
        .values()
        .any(|(minimum, maximum)| minimum.zip(*maximum).is_some_and(|(min, max)| min > max))
    {
        return Err("같은 지표의 최소값이 최대값보다 큰 모순된 규칙입니다.".to_owned());
    }
    Ok(())
}

pub fn screen_candidates(
    universe: &UniverseVersion,
    strategy: &ScreeningStrategy,
    as_of_ms: u64,
    observations: &[ScreeningObservation],
) -> Result<ScreeningResult, String> {
    validate_screening_contract(universe, strategy)?;
    if as_of_ms == 0 {
        return Err("스크리닝 기준 시각이 필요합니다.".to_owned());
    }
    let observation_map = observations
        .iter()
        .filter(|item| item.observed_at_ms <= as_of_ms)
        .map(|item| ((format!("{:?}", item.market), item.symbol.as_str()), item))
        .collect::<BTreeMap<_, _>>();
    let active_markets = universe
        .active_markets
        .iter()
        .map(|market| format!("{market:?}"))
        .collect::<BTreeSet<_>>();
    let mut included_by_market: BTreeMap<String, Vec<ScreenedCandidate>> = BTreeMap::new();
    let mut excluded = Vec::new();

    for entry in &universe.entries {
        let market_key = format!("{:?}", entry.market);
        if !active_markets.contains(&market_key)
            || entry.effective_from_ms > as_of_ms
            || entry.effective_to_ms.is_some_and(|end| as_of_ms >= end)
        {
            continue;
        }
        let mut reasons = Vec::new();
        if entry.status != InstrumentStatus::Tradable {
            reasons.push(ScreeningReason {
                rule_id: "universe.status".to_owned(),
                observed_value: None,
                threshold: None,
                passed: false,
                message: "거래정지·관리·상장폐지 종목은 제외합니다.".to_owned(),
            });
        }
        if entry
            .spread_bps
            .is_none_or(|spread| spread > entry.abnormal_spread_bps)
        {
            reasons.push(ScreeningReason {
                rule_id: "universe.spread".to_owned(),
                observed_value: entry.spread_bps.map(|value| value as i64),
                threshold: Some(entry.abnormal_spread_bps as i64),
                passed: false,
                message: "스프레드가 없거나 비정상 범위를 넘었습니다.".to_owned(),
            });
        }
        let observation = observation_map.get(&(market_key.clone(), entry.symbol.as_str()));
        let mut score = 0u64;
        for rule in &strategy.rules {
            let observed = observation
                .and_then(|item| item.metrics.get(&rule.metric))
                .copied();
            let passed = observed.is_some_and(|value| match rule.operator {
                RuleOperator::GreaterOrEqual => value >= rule.threshold,
                RuleOperator::LessOrEqual => value <= rule.threshold,
            });
            if passed {
                score = score.saturating_add(rule.score_weight_bps);
            }
            reasons.push(ScreeningReason {
                rule_id: rule.rule_id.clone(),
                observed_value: observed,
                threshold: Some(rule.threshold),
                passed,
                message: if passed {
                    rule.description.clone()
                } else {
                    format!("규칙 실패: {}", rule.description)
                },
            });
        }
        let included = reasons.iter().all(|reason| reason.passed);
        let candidate = ScreenedCandidate {
            symbol: entry.symbol.clone(),
            market: entry.market,
            score_bps: score.min(10_000),
            included,
            reasons,
        };
        if included {
            included_by_market
                .entry(market_key)
                .or_default()
                .push(candidate);
        } else {
            excluded.push(candidate);
        }
    }
    let mut candidates = Vec::new();
    for values in included_by_market.values_mut() {
        values.sort_by(|left, right| {
            right
                .score_bps
                .cmp(&left.score_bps)
                .then_with(|| left.symbol.cmp(&right.symbol))
        });
        let keep = strategy
            .maximum_candidates_per_market
            .min(strategy.analysis_budget.saturating_sub(candidates.len()));
        let overflow = values.split_off(keep.min(values.len()));
        for mut item in overflow {
            item.included = false;
            item.reasons.push(ScreeningReason {
                rule_id: "budget.limit".to_owned(),
                observed_value: None,
                threshold: Some(keep as i64),
                passed: false,
                message: "시장별 후보 또는 전체 분석 예산 상한을 초과했습니다.".to_owned(),
            });
            excluded.push(item);
        }
        candidates.append(values);
    }
    candidates.sort_by(|left, right| {
        format!("{:?}", left.market)
            .cmp(&format!("{:?}", right.market))
            .then_with(|| right.score_bps.cmp(&left.score_bps))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    excluded.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    Ok(ScreeningResult {
        universe_id: universe.universe_id.clone(),
        universe_version: universe.version,
        strategy_id: strategy.strategy_id.clone(),
        strategy_version: strategy.version,
        as_of_ms,
        candidates,
        excluded,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn universe() -> UniverseVersion {
        UniverseVersion {
            universe_id: "kr-us-v1".to_owned(),
            version: 1,
            active_markets: vec![Market::Korea, Market::UnitedStates],
            entries: vec![
                UniverseEntry {
                    symbol: "005930".to_owned(),
                    market: Market::Korea,
                    status: InstrumentStatus::Tradable,
                    effective_from_ms: 1,
                    effective_to_ms: None,
                    spread_bps: Some(5),
                    abnormal_spread_bps: 100,
                },
                UniverseEntry {
                    symbol: "000660".to_owned(),
                    market: Market::Korea,
                    status: InstrumentStatus::Halted,
                    effective_from_ms: 1,
                    effective_to_ms: None,
                    spread_bps: Some(5),
                    abnormal_spread_bps: 100,
                },
            ],
        }
    }

    fn strategy() -> ScreeningStrategy {
        ScreeningStrategy {
            strategy_id: "liquid-v1".to_owned(),
            version: 1,
            rules: vec![ScreeningRule {
                rule_id: "volume.minimum".to_owned(),
                metric: "average_volume".to_owned(),
                operator: RuleOperator::GreaterOrEqual,
                threshold: 100,
                score_weight_bps: 10_000,
                description: "평균 거래량이 최소값 이상입니다.".to_owned(),
            }],
            maximum_candidates_per_market: 10,
            analysis_budget: 10,
        }
    }

    #[test]
    fn screening_is_deterministic_and_explains_exclusions() {
        let observations = vec![ScreeningObservation {
            symbol: "005930".to_owned(),
            market: Market::Korea,
            observed_at_ms: 50,
            metrics: BTreeMap::from([("average_volume".to_owned(), 200)]),
        }];
        let first =
            screen_candidates(&universe(), &strategy(), 100, &observations).expect("screen");
        let second =
            screen_candidates(&universe(), &strategy(), 100, &observations).expect("screen");
        assert_eq!(first.candidates[0].symbol, second.candidates[0].symbol);
        assert_eq!(first.excluded[0].symbol, "000660");
        assert!(first.excluded[0]
            .reasons
            .iter()
            .any(|reason| !reason.passed));
    }

    #[test]
    fn rejects_contradictory_rule_bounds() {
        let mut strategy = strategy();
        strategy.rules.extend([
            ScreeningRule {
                rule_id: "price.minimum".to_owned(),
                metric: "price".to_owned(),
                operator: RuleOperator::GreaterOrEqual,
                threshold: 200,
                score_weight_bps: 0,
                description: "최소 가격".to_owned(),
            },
            ScreeningRule {
                rule_id: "price.maximum".to_owned(),
                metric: "price".to_owned(),
                operator: RuleOperator::LessOrEqual,
                threshold: 100,
                score_weight_bps: 0,
                description: "최대 가격".to_owned(),
            },
        ]);
        assert!(validate_screening_contract(&universe(), &strategy).is_err());
    }
}
