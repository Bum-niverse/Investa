use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceObservation {
    pub trade_id: String,
    pub decision_id: String,
    pub strategy_id: String,
    pub regime: String,
    pub gross_pnl_minor: i64,
    pub costs_minor: u64,
    pub realized: bool,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceSummary {
    pub realized_net_pnl_minor: i64,
    pub unrealized_net_pnl_minor: i64,
    pub gross_pnl_minor: i64,
    pub total_costs_minor: u64,
    pub net_pnl_minor: i64,
    pub trade_count: usize,
    pub profitable_trade_count: usize,
    pub win_rate_bps: Option<u64>,
    pub uncertainty: String,
}

pub fn summarize_performance(
    observations: &[PerformanceObservation],
) -> Result<PerformanceSummary, String> {
    let mut realized = 0i128;
    let mut unrealized = 0i128;
    let mut gross = 0i128;
    let mut costs = 0u128;
    let mut wins = 0usize;
    for item in observations {
        if item.trade_id.trim().is_empty()
            || item.decision_id.trim().is_empty()
            || item.strategy_id.trim().is_empty()
            || item.regime.trim().is_empty()
            || item.observed_at_ms == 0
        {
            return Err("성과 관측값에는 거래·판단·전략·레짐·시각이 필요합니다.".to_owned());
        }
        let net = i128::from(item.gross_pnl_minor) - i128::from(item.costs_minor);
        gross = gross
            .checked_add(i128::from(item.gross_pnl_minor))
            .ok_or_else(|| "성과 합계가 지원 범위를 초과했습니다.".to_owned())?;
        costs = costs
            .checked_add(u128::from(item.costs_minor))
            .ok_or_else(|| "비용 합계가 지원 범위를 초과했습니다.".to_owned())?;
        if item.realized {
            realized = realized
                .checked_add(net)
                .ok_or_else(|| "실현손익 합계가 지원 범위를 초과했습니다.".to_owned())?;
        } else {
            unrealized = unrealized
                .checked_add(net)
                .ok_or_else(|| "미실현손익 합계가 지원 범위를 초과했습니다.".to_owned())?;
        }
        wins += usize::from(net > 0);
    }
    let trade_count = observations.len();
    let win_rate_bps = (trade_count > 0).then(|| (wins as u64 * 10_000) / trade_count as u64);
    let uncertainty = if trade_count < 30 {
        "표본이 30건 미만이므로 승률과 기대값의 불확실성이 큽니다."
    } else {
        "표본 수만으로 통계적 유의성이나 미래 성과를 보장하지 않습니다."
    };
    Ok(PerformanceSummary {
        realized_net_pnl_minor: i64::try_from(realized)
            .map_err(|_| "실현손익을 표시할 수 없습니다.".to_owned())?,
        unrealized_net_pnl_minor: i64::try_from(unrealized)
            .map_err(|_| "미실현손익을 표시할 수 없습니다.".to_owned())?,
        gross_pnl_minor: i64::try_from(gross)
            .map_err(|_| "총손익을 표시할 수 없습니다.".to_owned())?,
        total_costs_minor: u64::try_from(costs)
            .map_err(|_| "총비용을 표시할 수 없습니다.".to_owned())?,
        net_pnl_minor: i64::try_from(realized + unrealized)
            .map_err(|_| "순손익을 표시할 수 없습니다.".to_owned())?,
        trade_count,
        profitable_trade_count: wins,
        win_rate_bps,
        uncertainty: uncertainty.to_owned(),
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttributionBucket {
    pub key: String,
    pub sample_count: usize,
    pub net_pnl_minor: i64,
}

pub fn attribute_without_rewriting_history(
    observations: &[PerformanceObservation],
) -> Result<Vec<AttributionBucket>, String> {
    summarize_performance(observations)?;
    let mut buckets: BTreeMap<String, (usize, i128)> = BTreeMap::new();
    for item in observations {
        let key = format!("{}::{}", item.strategy_id, item.regime);
        let entry = buckets.entry(key).or_default();
        entry.0 += 1;
        entry.1 += i128::from(item.gross_pnl_minor) - i128::from(item.costs_minor);
    }
    buckets
        .into_iter()
        .map(|(key, (sample_count, pnl))| {
            Ok(AttributionBucket {
                key,
                sample_count,
                net_pnl_minor: i64::try_from(pnl)
                    .map_err(|_| "기여도 손익을 표시할 수 없습니다.".to_owned())?,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyPerformanceObservation {
    pub currency: String,
    pub trade_id: String,
    pub decision_id: String,
    pub strategy_id: String,
    pub regime: String,
    pub gross_pnl_minor: i64,
    pub costs_minor: u64,
    pub realized: bool,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyPerformanceBook {
    pub by_currency: BTreeMap<String, PerformanceSummary>,
    pub base_currency_total: Option<i64>,
    pub conversion_message: String,
}

pub fn summarize_performance_by_currency(
    observations: &[CurrencyPerformanceObservation],
) -> Result<CurrencyPerformanceBook, String> {
    let mut grouped: BTreeMap<String, Vec<PerformanceObservation>> = BTreeMap::new();
    for item in observations {
        let currency = item.currency.trim().to_ascii_uppercase();
        if currency.len() < 3
            || currency.len() > 8
            || !currency.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            return Err("성과 통화는 3~8자의 영문·숫자 코드여야 합니다.".to_owned());
        }
        grouped
            .entry(currency)
            .or_default()
            .push(PerformanceObservation {
                trade_id: item.trade_id.clone(),
                decision_id: item.decision_id.clone(),
                strategy_id: item.strategy_id.clone(),
                regime: item.regime.clone(),
                gross_pnl_minor: item.gross_pnl_minor,
                costs_minor: item.costs_minor,
                realized: item.realized,
                observed_at_ms: item.observed_at_ms,
            });
    }
    let by_currency = grouped
        .into_iter()
        .map(|(currency, values)| summarize_performance(&values).map(|summary| (currency, summary)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(CurrencyPerformanceBook {
        by_currency,
        base_currency_total: None,
        conversion_message:
            "명시적인 시점 정합 환율이 없어 통화별 손익을 임의 합산하지 않았습니다.".to_owned(),
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionOutcomeObservation {
    pub decision_id: String,
    pub strategy_id: String,
    pub agent_id: String,
    pub predicted_up_bps: u64,
    pub expected_value_minor: i64,
    pub realized_up: bool,
    pub realized_net_pnl_minor: i64,
    pub evidence_complete: bool,
    pub observed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionQualityBucket {
    pub key: String,
    pub sample_count: usize,
    pub evidence_complete_count: usize,
    pub positive_expected_value_count: usize,
    pub mean_brier_score_bps: u64,
    pub realized_net_pnl_minor: i64,
}

pub fn summarize_decision_quality(
    observations: &[DecisionOutcomeObservation],
) -> Result<Vec<DecisionQualityBucket>, String> {
    let mut buckets: BTreeMap<String, (usize, usize, usize, u128, i128)> = BTreeMap::new();
    for item in observations {
        if item.decision_id.trim().is_empty()
            || item.strategy_id.trim().is_empty()
            || item.agent_id.trim().is_empty()
            || item.predicted_up_bps > 10_000
            || item.observed_at_ms == 0
        {
            return Err("판단 성과에는 식별자·확률·관측 시각이 필요합니다.".to_owned());
        }
        let target = if item.realized_up { 10_000i128 } else { 0i128 };
        let error = i128::from(item.predicted_up_bps) - target;
        let squared_bps = u128::try_from(error.saturating_mul(error) / 10_000)
            .map_err(|_| "확률 보정 오차를 계산할 수 없습니다.".to_owned())?;
        let entry = buckets
            .entry(format!("{}::{}", item.agent_id, item.strategy_id))
            .or_default();
        entry.0 += 1;
        entry.1 += usize::from(item.evidence_complete);
        entry.2 += usize::from(item.expected_value_minor > 0);
        entry.3 = entry.3.saturating_add(squared_bps);
        entry.4 = entry
            .4
            .saturating_add(i128::from(item.realized_net_pnl_minor));
    }
    buckets
        .into_iter()
        .map(|(key, value)| {
            Ok(DecisionQualityBucket {
                key,
                sample_count: value.0,
                evidence_complete_count: value.1,
                positive_expected_value_count: value.2,
                mean_brier_score_bps: u64::try_from(value.3 / value.0.max(1) as u128)
                    .map_err(|_| "평균 확률 보정 오차를 표시할 수 없습니다.".to_owned())?,
                realized_net_pnl_minor: i64::try_from(value.4)
                    .map_err(|_| "판단별 실현손익을 표시할 수 없습니다.".to_owned())?,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImprovementProposal {
    pub proposal_id: String,
    pub evidence_ids: Vec<String>,
    pub expected_effect: String,
    pub risks: Vec<String>,
    pub affected_components: Vec<String>,
    pub automatically_applied: bool,
    pub requires_full_revalidation: bool,
}

pub fn validate_improvement_proposal(proposal: &ImprovementProposal) -> Result<(), String> {
    if proposal.proposal_id.trim().is_empty()
        || proposal.evidence_ids.is_empty()
        || proposal.expected_effect.trim().is_empty()
        || proposal.risks.is_empty()
        || proposal.affected_components.is_empty()
        || proposal.automatically_applied
        || !proposal.requires_full_revalidation
    {
        return Err(
            "개선안은 근거·효과·위험·영향 범위와 전체 재검증 요구를 가져야 합니다.".to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_keeps_gross_cost_realized_and_unrealized_separate() {
        let summary = summarize_performance(&[
            PerformanceObservation {
                trade_id: "trade-1".to_owned(),
                decision_id: "decision-1".to_owned(),
                strategy_id: "strategy-1".to_owned(),
                regime: "bull".to_owned(),
                gross_pnl_minor: 100,
                costs_minor: 10,
                realized: true,
                observed_at_ms: 1,
            },
            PerformanceObservation {
                trade_id: "trade-2".to_owned(),
                decision_id: "decision-2".to_owned(),
                strategy_id: "strategy-1".to_owned(),
                regime: "bear".to_owned(),
                gross_pnl_minor: -20,
                costs_minor: 5,
                realized: false,
                observed_at_ms: 2,
            },
        ])
        .expect("summary");
        assert_eq!(summary.gross_pnl_minor, 80);
        assert_eq!(summary.total_costs_minor, 15);
        assert_eq!(summary.realized_net_pnl_minor, 90);
        assert_eq!(summary.unrealized_net_pnl_minor, -25);
        assert_eq!(summary.net_pnl_minor, 65);
    }

    #[test]
    fn improvement_proposals_can_never_apply_themselves() {
        let proposal = ImprovementProposal {
            proposal_id: "proposal-1".to_owned(),
            evidence_ids: vec!["evidence-1".to_owned()],
            expected_effect: "지연 감소".to_owned(),
            risks: vec!["과최적화".to_owned()],
            affected_components: vec!["strategy".to_owned()],
            automatically_applied: false,
            requires_full_revalidation: true,
        };
        validate_improvement_proposal(&proposal).expect("proposal");
    }

    #[test]
    fn multi_currency_performance_is_never_implicitly_converted() {
        let book = summarize_performance_by_currency(&[
            CurrencyPerformanceObservation {
                currency: "KRW".to_owned(),
                trade_id: "kr-1".to_owned(),
                decision_id: "d-1".to_owned(),
                strategy_id: "s-1".to_owned(),
                regime: "bull".to_owned(),
                gross_pnl_minor: 100,
                costs_minor: 10,
                realized: true,
                observed_at_ms: 1,
            },
            CurrencyPerformanceObservation {
                currency: "USD".to_owned(),
                trade_id: "us-1".to_owned(),
                decision_id: "d-2".to_owned(),
                strategy_id: "s-1".to_owned(),
                regime: "bull".to_owned(),
                gross_pnl_minor: 200,
                costs_minor: 20,
                realized: true,
                observed_at_ms: 2,
            },
        ])
        .expect("currency book");
        assert_eq!(book.by_currency.len(), 2);
        assert_eq!(book.base_currency_total, None);
    }

    #[test]
    fn decision_quality_uses_calibration_and_expected_value_not_raw_accuracy() {
        let buckets = summarize_decision_quality(&[DecisionOutcomeObservation {
            decision_id: "d-1".to_owned(),
            strategy_id: "s-1".to_owned(),
            agent_id: "bull-researcher".to_owned(),
            predicted_up_bps: 7_000,
            expected_value_minor: 50,
            realized_up: true,
            realized_net_pnl_minor: 40,
            evidence_complete: true,
            observed_at_ms: 1,
        }])
        .expect("decision quality");
        assert_eq!(buckets[0].mean_brier_score_bps, 900);
        assert_eq!(buckets[0].positive_expected_value_count, 1);
        assert_eq!(buckets[0].realized_net_pnl_minor, 40);
    }
}
