use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

const STRATEGY_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Market {
    Korea,
    UnitedStates,
    Crypto,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Repository,
    Paper,
    Documentation,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceEvidence {
    pub evidence_id: String,
    pub kind: EvidenceKind,
    pub source_url: String,
    pub revision: Option<String>,
    pub license: Option<String>,
    pub summary: String,
    pub claimed_result: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossDirection {
    Above,
    Below,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SignalSpec {
    MovingAverageCross {
        fast_window: usize,
        slow_window: usize,
        direction: CrossDirection,
    },
    PriceChannelBreakout {
        lookback: usize,
        direction: CrossDirection,
    },
    MeanReversion {
        window: usize,
        deviation_bps: u64,
        direction: CrossDirection,
    },
    VolatilityExpansion {
        atr_window: usize,
        breakout_window: usize,
        minimum_expansion_bps: u64,
        direction: CrossDirection,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategySpec {
    pub schema_version: String,
    pub strategy_id: String,
    pub name: String,
    pub market: Market,
    pub symbol: String,
    pub currency: String,
    pub hypothesis: String,
    pub source_evidence_ids: Vec<String>,
    pub entry_signal: SignalSpec,
    pub exit_signal: SignalSpec,
    pub limitations: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub trace_id: String,
    pub request: String,
    pub evidence: Vec<ReferenceEvidence>,
    pub strategy_candidate: StrategySpec,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchIssueCode {
    InvalidIdentifier,
    InvalidText,
    InvalidSourceUrl,
    MissingRepositoryRevision,
    DuplicateEvidence,
    MissingEvidence,
    UnsupportedSchemaVersion,
    InvalidSymbol,
    InvalidCurrency,
    InvalidSignal,
    UnresolvedUnknowns,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchIssue {
    pub code: ResearchIssueCode,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyReview {
    pub valid: bool,
    pub executable: bool,
    pub issues: Vec<ResearchIssue>,
}

fn issue(issues: &mut Vec<ResearchIssue>, code: ResearchIssueCode, field: &str, message: &str) {
    issues.push(ResearchIssue {
        code,
        field: field.to_owned(),
        message: message.to_owned(),
    });
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 24
        && value.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
}

fn valid_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn valid_text(value: &str, max_len: usize) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= max_len
        && !trimmed.chars().any(char::is_control)
}

fn validate_signal(
    signal: &SignalSpec,
    field: &str,
    expected_direction: CrossDirection,
    issues: &mut Vec<ResearchIssue>,
) {
    let valid = match signal {
        SignalSpec::MovingAverageCross {
            fast_window,
            slow_window,
            direction,
        } if *fast_window > 0
            && *slow_window > *fast_window
            && *slow_window <= 10_000
            && *direction == expected_direction =>
        {
            true
        }
        SignalSpec::PriceChannelBreakout {
            lookback,
            direction,
        } if (2..=10_000).contains(lookback) && *direction == expected_direction => true,
        SignalSpec::MeanReversion {
            window,
            deviation_bps,
            direction,
        } if (2..=10_000).contains(window)
            && (1..10_000).contains(deviation_bps)
            && *direction
                == match expected_direction {
                    CrossDirection::Above => CrossDirection::Below,
                    CrossDirection::Below => CrossDirection::Above,
                } =>
        {
            true
        }
        SignalSpec::VolatilityExpansion {
            atr_window,
            breakout_window,
            minimum_expansion_bps,
            direction,
        } if (2..=10_000).contains(atr_window)
            && (2..=10_000).contains(breakout_window)
            && (1..=100_000).contains(minimum_expansion_bps)
            && *direction == expected_direction =>
        {
            true
        }
        _ => false,
    };
    if !valid {
        issue(
            issues,
            ResearchIssueCode::InvalidSignal,
            field,
            "전략 파라미터는 지원 범위 안이어야 하며 플러그인별 진입·청산 방향이 명확해야 합니다.",
        );
    }
}

fn signal_family(signal: &SignalSpec) -> &'static str {
    match signal {
        SignalSpec::MovingAverageCross { .. } => "trend.moving_average_cross",
        SignalSpec::PriceChannelBreakout { .. } => "breakout.price_channel",
        SignalSpec::MeanReversion { .. } => "mean_reversion.distance_from_mean",
        SignalSpec::VolatilityExpansion { .. } => "volatility.atr_expansion",
    }
}

pub fn review_strategy_spec(spec: &StrategySpec) -> StrategyReview {
    let mut issues = Vec::new();

    if spec.schema_version != STRATEGY_SCHEMA_VERSION {
        issue(
            &mut issues,
            ResearchIssueCode::UnsupportedSchemaVersion,
            "schemaVersion",
            "지원하는 StrategySpec schemaVersion은 1입니다.",
        );
    }
    if !valid_identifier(&spec.strategy_id) {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidIdentifier,
            "strategyId",
            "strategyId는 1~64자의 영문·숫자·하이픈·밑줄이어야 합니다.",
        );
    }
    if !valid_text(&spec.name, 120) || !valid_text(&spec.hypothesis, 2_000) {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidText,
            "nameOrHypothesis",
            "전략 이름과 가설은 비어 있지 않은 제한 길이 텍스트여야 합니다.",
        );
    }
    if !valid_symbol(&spec.symbol) {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidSymbol,
            "symbol",
            "종목 코드는 영문 대문자·숫자·점·하이픈만 사용할 수 있습니다.",
        );
    }
    if !valid_currency(&spec.currency) {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidCurrency,
            "currency",
            "통화는 KRW·USD처럼 영문 대문자 3자로 기록해야 합니다.",
        );
    }
    if spec.market == Market::Crypto && (spec.currency != "KRW" || !spec.symbol.starts_with("KRW-"))
    {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidSymbol,
            "marketSymbolCurrency",
            "현재 코인 전략은 업비트 KRW 마켓 코드와 KRW 통화만 지원합니다.",
        );
    }

    let unique_evidence: BTreeSet<&str> = spec
        .source_evidence_ids
        .iter()
        .map(String::as_str)
        .collect();
    if spec.source_evidence_ids.is_empty()
        || spec.source_evidence_ids.len() > 64
        || unique_evidence.len() != spec.source_evidence_ids.len()
        || spec
            .source_evidence_ids
            .iter()
            .any(|id| !valid_identifier(id))
    {
        issue(
            &mut issues,
            ResearchIssueCode::MissingEvidence,
            "sourceEvidenceIds",
            "전략은 중복되지 않은 유효한 근거 ID를 하나 이상 참조해야 합니다.",
        );
    }

    validate_signal(
        &spec.entry_signal,
        "entrySignal",
        CrossDirection::Above,
        &mut issues,
    );

    if spec.limitations.len() > 64
        || spec
            .limitations
            .iter()
            .any(|value| !valid_text(value, 2_000))
        || spec.unknowns.len() > 64
        || spec.unknowns.iter().any(|value| !valid_text(value, 2_000))
    {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidText,
            "limitationsOrUnknowns",
            "한계와 미해결 항목은 각각 최대 64개의 제한 길이 텍스트여야 합니다.",
        );
    }
    validate_signal(
        &spec.exit_signal,
        "exitSignal",
        CrossDirection::Below,
        &mut issues,
    );
    if signal_family(&spec.entry_signal) != signal_family(&spec.exit_signal) {
        issue(
            &mut issues,
            ResearchIssueCode::InvalidSignal,
            "entrySignalOrExitSignal",
            "진입과 청산 신호는 동일한 버전형 전략 플러그인 계열이어야 합니다.",
        );
    }

    if !spec.unknowns.is_empty() {
        issue(
            &mut issues,
            ResearchIssueCode::UnresolvedUnknowns,
            "unknowns",
            "해결되지 않은 항목이 있는 전략은 백테스트 실행 후보로 승격할 수 없습니다.",
        );
    }

    let valid = !issues
        .iter()
        .any(|item| item.code != ResearchIssueCode::UnresolvedUnknowns);
    StrategyReview {
        valid,
        executable: valid && spec.unknowns.is_empty(),
        issues,
    }
}

pub fn review_research_report(report: &ResearchReport) -> StrategyReview {
    let mut review = review_strategy_spec(&report.strategy_candidate);

    if !valid_identifier(&report.trace_id) {
        issue(
            &mut review.issues,
            ResearchIssueCode::InvalidIdentifier,
            "traceId",
            "traceId는 재현 가능한 제한 길이 식별자여야 합니다.",
        );
    }
    if !valid_text(&report.request, 4_000) {
        issue(
            &mut review.issues,
            ResearchIssueCode::InvalidText,
            "request",
            "연구 요청은 비어 있지 않은 제한 길이 텍스트여야 합니다.",
        );
    }

    let mut evidence_ids = BTreeSet::new();
    if report.evidence.is_empty() || report.evidence.len() > 128 {
        issue(
            &mut review.issues,
            ResearchIssueCode::MissingEvidence,
            "evidence",
            "연구 보고서는 1~128개의 근거를 포함해야 합니다.",
        );
    }
    for (index, evidence) in report.evidence.iter().enumerate() {
        let field = format!("evidence[{index}]");
        if !valid_identifier(&evidence.evidence_id) {
            issue(
                &mut review.issues,
                ResearchIssueCode::InvalidIdentifier,
                &format!("{field}.evidenceId"),
                "근거 ID 형식이 올바르지 않습니다.",
            );
        }
        if !evidence_ids.insert(evidence.evidence_id.as_str()) {
            issue(
                &mut review.issues,
                ResearchIssueCode::DuplicateEvidence,
                &format!("{field}.evidenceId"),
                "같은 근거 ID가 두 번 포함되었습니다.",
            );
        }
        if !evidence.source_url.starts_with("https://") || !valid_text(&evidence.source_url, 2_048)
        {
            issue(
                &mut review.issues,
                ResearchIssueCode::InvalidSourceUrl,
                &format!("{field}.sourceUrl"),
                "근거 URL은 길이가 제한된 HTTPS 주소여야 합니다.",
            );
        }
        if evidence.kind == EvidenceKind::Repository
            && evidence
                .revision
                .as_deref()
                .is_none_or(|revision| !valid_text(revision, 128))
        {
            issue(
                &mut review.issues,
                ResearchIssueCode::MissingRepositoryRevision,
                &format!("{field}.revision"),
                "저장소 근거에는 재현 가능한 commit 또는 tag가 필요합니다.",
            );
        }
        if !valid_text(&evidence.summary, 4_000) {
            issue(
                &mut review.issues,
                ResearchIssueCode::InvalidText,
                &format!("{field}.summary"),
                "근거 요약은 비어 있지 않은 제한 길이 텍스트여야 합니다.",
            );
        }
        if evidence
            .license
            .as_deref()
            .is_some_and(|value| !valid_text(value, 256))
            || evidence
                .claimed_result
                .as_deref()
                .is_some_and(|value| !valid_text(value, 2_000))
        {
            issue(
                &mut review.issues,
                ResearchIssueCode::InvalidText,
                &format!("{field}.metadata"),
                "라이선스와 주장 성과는 제한 길이 텍스트여야 합니다.",
            );
        }
    }

    for evidence_id in &report.strategy_candidate.source_evidence_ids {
        if !evidence_ids.contains(evidence_id.as_str()) {
            issue(
                &mut review.issues,
                ResearchIssueCode::MissingEvidence,
                "strategyCandidate.sourceEvidenceIds",
                "전략이 보고서에 없는 근거 ID를 참조합니다.",
            );
        }
    }

    review.valid = !review
        .issues
        .iter()
        .any(|item| item.code != ResearchIssueCode::UnresolvedUnknowns);
    review.executable = review.valid && report.strategy_candidate.unknowns.is_empty();
    review
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn valid_strategy() -> StrategySpec {
        StrategySpec {
            schema_version: "1".to_owned(),
            strategy_id: "ma-cross-fixture".to_owned(),
            name: "이동평균 교차 재현 전략".to_owned(),
            market: Market::Korea,
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            hypothesis: "단기 평균이 장기 평균을 상향 돌파한 뒤 반대 교차에서 청산한다.".to_owned(),
            source_evidence_ids: vec!["repo-1".to_owned()],
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
            limitations: vec!["고정 fixture 검증 전용".to_owned()],
            unknowns: Vec::new(),
        }
    }

    fn valid_report() -> ResearchReport {
        ResearchReport {
            trace_id: "trace-001".to_owned(),
            request: "고정된 저장소 revision에서 전략 규칙만 구조화해줘.".to_owned(),
            evidence: vec![ReferenceEvidence {
                evidence_id: "repo-1".to_owned(),
                kind: EvidenceKind::Repository,
                source_url: "https://github.com/example/strategy".to_owned(),
                revision: Some("0123456789abcdef".to_owned()),
                license: Some("MIT".to_owned()),
                summary: "이동평균 교차 규칙을 설명한다.".to_owned(),
                claimed_result: None,
            }],
            strategy_candidate: valid_strategy(),
        }
    }

    #[test]
    fn accepts_a_reproducible_research_report_without_setting_performance_thresholds() {
        let review = review_research_report(&valid_report());

        assert!(review.valid);
        assert!(review.executable);
        assert!(review.issues.is_empty());
    }

    #[test]
    fn keeps_a_strategy_non_executable_while_unknowns_remain() {
        let mut report = valid_report();
        report
            .strategy_candidate
            .unknowns
            .push("거래 비용 가정 미확인".to_owned());

        let review = review_research_report(&report);

        assert!(review.valid);
        assert!(!review.executable);
        assert_eq!(review.issues[0].code, ResearchIssueCode::UnresolvedUnknowns);
    }

    #[test]
    fn rejects_missing_revision_evidence_and_invalid_signal_windows() {
        let mut report = valid_report();
        report.evidence[0].revision = None;
        report.strategy_candidate.entry_signal = SignalSpec::MovingAverageCross {
            fast_window: 5,
            slow_window: 3,
            direction: CrossDirection::Above,
        };

        let review = review_research_report(&report);

        assert!(!review.valid);
        assert!(!review.executable);
        assert!(review
            .issues
            .iter()
            .any(|item| item.code == ResearchIssueCode::MissingRepositoryRevision));
        assert!(review
            .issues
            .iter()
            .any(|item| item.code == ResearchIssueCode::InvalidSignal));
    }

    #[test]
    fn rejects_evidence_references_that_are_not_in_the_report() {
        let mut report = valid_report();
        report.strategy_candidate.source_evidence_ids = vec!["missing-evidence".to_owned()];

        let review = review_research_report(&report);

        assert!(!review.valid);
        assert!(review
            .issues
            .iter()
            .any(|item| item.code == ResearchIssueCode::MissingEvidence));
    }

    #[test]
    fn accepts_each_supported_strategy_plugin_and_rejects_mixed_families() {
        let signal_pairs = [
            (
                SignalSpec::PriceChannelBreakout {
                    lookback: 20,
                    direction: CrossDirection::Above,
                },
                SignalSpec::PriceChannelBreakout {
                    lookback: 20,
                    direction: CrossDirection::Below,
                },
            ),
            (
                SignalSpec::MeanReversion {
                    window: 20,
                    deviation_bps: 200,
                    direction: CrossDirection::Below,
                },
                SignalSpec::MeanReversion {
                    window: 20,
                    deviation_bps: 200,
                    direction: CrossDirection::Above,
                },
            ),
            (
                SignalSpec::VolatilityExpansion {
                    atr_window: 14,
                    breakout_window: 20,
                    minimum_expansion_bps: 12_500,
                    direction: CrossDirection::Above,
                },
                SignalSpec::VolatilityExpansion {
                    atr_window: 14,
                    breakout_window: 20,
                    minimum_expansion_bps: 12_500,
                    direction: CrossDirection::Below,
                },
            ),
        ];
        for (entry, exit) in signal_pairs {
            let mut report = valid_report();
            report.strategy_candidate.entry_signal = entry;
            report.strategy_candidate.exit_signal = exit;
            assert!(review_research_report(&report).executable);
        }

        let mut mixed = valid_report();
        mixed.strategy_candidate.entry_signal = SignalSpec::PriceChannelBreakout {
            lookback: 20,
            direction: CrossDirection::Above,
        };
        assert!(!review_research_report(&mixed).executable);
    }
}
