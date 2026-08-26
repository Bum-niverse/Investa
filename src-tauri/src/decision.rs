use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::trading::TradeSide;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceClaim {
    pub claim_id: String,
    pub statement: String,
    pub evidence_ids: Vec<String>,
    pub counter_evidence_ids: Vec<String>,
    pub confidence_bps: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistReport {
    pub report_id: String,
    pub role_id: String,
    pub trace_id: String,
    pub data_as_of_ms: u64,
    pub claims: Vec<EvidenceClaim>,
    pub incomplete_reasons: Vec<String>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

pub fn validate_specialist_report(report: &SpecialistReport) -> Result<(), String> {
    if !valid_id(&report.report_id)
        || !valid_id(&report.role_id)
        || !valid_id(&report.trace_id)
        || report.data_as_of_ms == 0
        || report.claims.is_empty()
    {
        return Err("전문 분석 보고서의 ID·기준 시각·주장이 필요합니다.".to_owned());
    }
    let mut claim_ids = BTreeSet::new();
    for claim in &report.claims {
        if !valid_id(&claim.claim_id)
            || claim.statement.trim().is_empty()
            || claim.evidence_ids.is_empty()
            || claim.confidence_bps > 10_000
            || !claim_ids.insert(claim.claim_id.as_str())
        {
            return Err("모든 주장은 고유 ID·근거·0~100% 신뢰도를 가져야 합니다.".to_owned());
        }
        if claim.evidence_ids.iter().any(|id| !valid_id(id))
            || claim.counter_evidence_ids.iter().any(|id| !valid_id(id))
        {
            return Err("주장 근거 ID 형식이 올바르지 않습니다.".to_owned());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebateTurn {
    pub side: String,
    pub addressed_claim_ids: Vec<String>,
    pub response_claims: Vec<EvidenceClaim>,
    pub token_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebateOutcome {
    pub valid: bool,
    pub observe_only: bool,
    pub uncertainty_bps: u64,
    pub issues: Vec<String>,
}

pub fn review_debate(
    bullish: &SpecialistReport,
    bearish: &SpecialistReport,
    turns: &[DebateTurn],
    maximum_rounds: usize,
    maximum_tokens: usize,
) -> DebateOutcome {
    let mut issues = Vec::new();
    if validate_specialist_report(bullish).is_err() || validate_specialist_report(bearish).is_err()
    {
        issues.push("양측 전문 분석 보고서가 유효하지 않습니다.".to_owned());
    }
    if maximum_rounds == 0 || turns.len() > maximum_rounds.saturating_mul(2) {
        issues.push("토론 횟수 상한을 초과했습니다.".to_owned());
    }
    if turns.iter().map(|turn| turn.token_count).sum::<usize>() > maximum_tokens {
        issues.push("토론 토큰 예산을 초과했습니다.".to_owned());
    }
    let bullish_ids = bullish
        .claims
        .iter()
        .map(|claim| claim.claim_id.as_str())
        .collect::<BTreeSet<_>>();
    let bearish_ids = bearish
        .claims
        .iter()
        .map(|claim| claim.claim_id.as_str())
        .collect::<BTreeSet<_>>();
    for turn in turns {
        let opposite = if turn.side == "bull" {
            &bearish_ids
        } else if turn.side == "bear" {
            &bullish_ids
        } else {
            issues.push("토론 측은 bull 또는 bear여야 합니다.".to_owned());
            continue;
        };
        if turn.addressed_claim_ids.is_empty()
            || turn
                .addressed_claim_ids
                .iter()
                .any(|id| !opposite.contains(id.as_str()))
        {
            issues.push("상대방의 구체적인 주장 ID에 답해야 합니다.".to_owned());
        }
    }
    let valid = issues.is_empty();
    DebateOutcome {
        valid,
        observe_only: !valid || turns.is_empty(),
        uncertainty_bps: if valid && !turns.is_empty() {
            5_000
        } else {
            10_000
        },
        issues,
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredTradePlan {
    pub schema_version: u32,
    pub plan_id: String,
    pub symbol: String,
    pub side: TradeSide,
    pub entry_price_minor: u64,
    pub stop_price_minor: u64,
    pub target_price_minor: u64,
    pub valid_until_ms: u64,
    pub suggested_quantity: u64,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TradePlanReview {
    pub valid: bool,
    pub risk_minor_per_unit: Option<u64>,
    pub reward_minor_per_unit: Option<u64>,
    pub reward_to_risk_milli: Option<u64>,
    pub maximum_quantity_by_loss: Option<u64>,
    pub issues: Vec<String>,
}

pub fn review_trade_plan(
    plan: &StructuredTradePlan,
    now_ms: u64,
    maximum_loss_minor: u64,
) -> TradePlanReview {
    let mut issues = Vec::new();
    if plan.schema_version != 1
        || !valid_id(&plan.plan_id)
        || plan.symbol.trim().is_empty()
        || plan.entry_price_minor == 0
        || plan.stop_price_minor == 0
        || plan.target_price_minor == 0
        || plan.evidence_ids.is_empty()
        || plan.valid_until_ms <= now_ms
    {
        issues.push("거래 계획 JSON 계약·가격·근거·유효기간이 올바르지 않습니다.".to_owned());
    }
    let direction_valid = match plan.side {
        TradeSide::Buy => {
            plan.stop_price_minor < plan.entry_price_minor
                && plan.target_price_minor > plan.entry_price_minor
        }
        TradeSide::Sell => {
            plan.stop_price_minor > plan.entry_price_minor
                && plan.target_price_minor < plan.entry_price_minor
        }
    };
    if !direction_valid {
        issues.push("매수·매도 방향에 맞는 손절가와 목표가가 필요합니다.".to_owned());
    }
    let risk = direction_valid.then(|| plan.entry_price_minor.abs_diff(plan.stop_price_minor));
    let reward = direction_valid.then(|| plan.entry_price_minor.abs_diff(plan.target_price_minor));
    let ratio = risk.zip(reward).and_then(|(risk, reward)| {
        (risk > 0)
            .then(|| u64::try_from(u128::from(reward) * 1_000 / u128::from(risk)).ok())
            .flatten()
    });
    let max_quantity = risk.and_then(|risk| (risk > 0).then(|| maximum_loss_minor / risk));
    TradePlanReview {
        valid: issues.is_empty(),
        risk_minor_per_unit: risk,
        reward_minor_per_unit: reward,
        reward_to_risk_milli: ratio,
        maximum_quantity_by_loss: max_quantity.map(|limit| limit.min(plan.suggested_quantity)),
        issues,
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskPanelDecision {
    Approve,
    Reduce,
    Observe,
    Reject,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPanelVote {
    pub perspective: String,
    pub decision: RiskPanelDecision,
    pub measured_value: i64,
    pub limit_value: i64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioProposal {
    pub approved: bool,
    pub final_decision: RiskPanelDecision,
    pub analysis_ids: Vec<String>,
    pub strategy_version: String,
    pub risk_policy_version: String,
    pub reasons: Vec<String>,
    pub broker_credentials_available_to_model: bool,
}

pub fn decide_portfolio_proposal(
    analysis_ids: &[String],
    strategy_version: &str,
    risk_policy_version: &str,
    votes: &[RiskPanelVote],
) -> PortfolioProposal {
    let mut reasons = votes
        .iter()
        .map(|vote| {
            format!(
                "{}: {} (측정 {}, 한도 {})",
                vote.perspective, vote.reason, vote.measured_value, vote.limit_value
            )
        })
        .collect::<Vec<_>>();
    let malformed = analysis_ids.is_empty()
        || analysis_ids.iter().any(|id| !valid_id(id))
        || !valid_id(strategy_version)
        || !valid_id(risk_policy_version)
        || votes.len() < 3
        || votes.iter().any(|vote| vote.reason.trim().is_empty());
    let conservative_reject = votes.iter().any(|vote| {
        vote.perspective == "conservative" && vote.decision == RiskPanelDecision::Reject
    });
    let failed = malformed
        || votes
            .iter()
            .any(|vote| vote.decision == RiskPanelDecision::Failed);
    let final_decision = if failed || conservative_reject {
        RiskPanelDecision::Reject
    } else if votes
        .iter()
        .any(|vote| vote.decision == RiskPanelDecision::Observe)
    {
        RiskPanelDecision::Observe
    } else if votes
        .iter()
        .any(|vote| vote.decision == RiskPanelDecision::Reduce)
    {
        RiskPanelDecision::Reduce
    } else {
        RiskPanelDecision::Approve
    };
    if malformed {
        reasons.push("분석·전략·위험 정책 버전 또는 심의 계약이 불완전합니다.".to_owned());
    }
    if conservative_reject {
        reasons.push("보수 관점의 치명적 위험은 다수결로 무시하지 않습니다.".to_owned());
    }
    PortfolioProposal {
        approved: final_decision == RiskPanelDecision::Approve,
        final_decision,
        analysis_ids: analysis_ids.to_vec(),
        strategy_version: strategy_version.to_owned(),
        risk_policy_version: risk_policy_version.to_owned(),
        reasons,
        broker_credentials_available_to_model: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(role: &str, claim: &str) -> SpecialistReport {
        SpecialistReport {
            report_id: format!("report-{role}"),
            role_id: role.to_owned(),
            trace_id: "trace-1".to_owned(),
            data_as_of_ms: 100,
            claims: vec![EvidenceClaim {
                claim_id: claim.to_owned(),
                statement: "근거가 있는 주장".to_owned(),
                evidence_ids: vec!["evidence-1".to_owned()],
                counter_evidence_ids: vec![],
                confidence_bps: 6_000,
            }],
            incomplete_reasons: vec![],
        }
    }

    #[test]
    fn debate_must_address_the_opposite_claim_and_respect_budgets() {
        let outcome = review_debate(
            &report("bull", "bull-1"),
            &report("bear", "bear-1"),
            &[DebateTurn {
                side: "bull".to_owned(),
                addressed_claim_ids: vec!["bear-1".to_owned()],
                response_claims: vec![],
                token_count: 100,
            }],
            2,
            200,
        );
        assert!(outcome.valid);
        assert!(!outcome.observe_only);
    }

    #[test]
    fn trade_plan_uses_the_more_conservative_loss_based_quantity() {
        let review = review_trade_plan(
            &StructuredTradePlan {
                schema_version: 1,
                plan_id: "plan-1".to_owned(),
                symbol: "005930".to_owned(),
                side: TradeSide::Buy,
                entry_price_minor: 100,
                stop_price_minor: 90,
                target_price_minor: 130,
                valid_until_ms: 1_000,
                suggested_quantity: 100,
                evidence_ids: vec!["evidence-1".to_owned()],
            },
            100,
            250,
        );
        assert!(review.valid);
        assert_eq!(review.reward_to_risk_milli, Some(3_000));
        assert_eq!(review.maximum_quantity_by_loss, Some(25));
    }

    #[test]
    fn conservative_rejection_cannot_be_outvoted() {
        let votes = vec![
            RiskPanelVote {
                perspective: "aggressive".to_owned(),
                decision: RiskPanelDecision::Approve,
                measured_value: 1,
                limit_value: 2,
                reason: "통과".to_owned(),
            },
            RiskPanelVote {
                perspective: "neutral".to_owned(),
                decision: RiskPanelDecision::Approve,
                measured_value: 1,
                limit_value: 2,
                reason: "통과".to_owned(),
            },
            RiskPanelVote {
                perspective: "conservative".to_owned(),
                decision: RiskPanelDecision::Reject,
                measured_value: 3,
                limit_value: 2,
                reason: "치명 위험".to_owned(),
            },
        ];
        let proposal =
            decide_portfolio_proposal(&["analysis-1".to_owned()], "strategy-v1", "risk-v1", &votes);
        assert!(!proposal.approved);
        assert!(!proposal.broker_credentials_available_to_model);
    }
}
