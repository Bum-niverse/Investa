use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityFlag {
    Delayed,
    Missing,
    Duplicate,
    Correction,
    OutOfOrder,
    FallbackSource,
    SuspectedBot,
    Promotional,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporalMetadata {
    pub event_time_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub source: String,
    pub source_revision: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointInTimeRecord {
    pub record_id: String,
    pub snapshot_id: String,
    pub symbol: String,
    pub metadata: TemporalMetadata,
    pub quality_flags: Vec<QualityFlag>,
    pub payload_hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFreshnessPolicy {
    pub source: String,
    pub required: bool,
    pub maximum_age_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisDisposition {
    Ready,
    Degraded,
    ObserveOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataQualityIssue {
    pub code: String,
    pub record_id: Option<String>,
    pub source: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointInTimeSnapshot {
    pub snapshot_id: String,
    pub as_of_ms: u64,
    pub records: Vec<PointInTimeRecord>,
    pub disposition: AnalysisDisposition,
    pub order_allowed: bool,
    pub issues: Vec<DataQualityIssue>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

pub fn build_point_in_time_snapshot(
    snapshot_id: &str,
    as_of_ms: u64,
    records: &[PointInTimeRecord],
    policies: &[SourceFreshnessPolicy],
) -> Result<PointInTimeSnapshot, String> {
    if !valid_id(snapshot_id) || as_of_ms == 0 {
        return Err("유효한 스냅샷 ID와 기준 시각이 필요합니다.".to_owned());
    }
    let mut issues = Vec::new();
    let mut latest_by_record: BTreeMap<&str, &PointInTimeRecord> = BTreeMap::new();
    let mut seen_revisions = BTreeSet::new();

    for record in records {
        if !valid_id(&record.record_id)
            || record.snapshot_id != snapshot_id
            || record.symbol.trim().is_empty()
            || record.metadata.source.trim().is_empty()
            || record.metadata.source_revision.trim().is_empty()
            || record.payload_hash.len() < 16
        {
            return Err(
                "시점 정합 레코드의 식별자 또는 출처 메타데이터가 올바르지 않습니다.".to_owned(),
            );
        }
        if record.metadata.event_time_ms > record.metadata.available_at_ms
            || record.metadata.available_at_ms > record.metadata.ingested_at_ms
        {
            return Err("eventTime ≤ availableAt ≤ ingestedAt 순서를 만족해야 합니다.".to_owned());
        }
        if !seen_revisions.insert((
            record.record_id.as_str(),
            record.metadata.source_revision.as_str(),
        )) {
            issues.push(DataQualityIssue {
                code: "duplicate_revision".to_owned(),
                record_id: Some(record.record_id.clone()),
                source: Some(record.metadata.source.clone()),
                message: "같은 원천 리비전이 중복 수집되어 한 번만 사용했습니다.".to_owned(),
            });
            continue;
        }
        if record.metadata.available_at_ms > as_of_ms {
            continue;
        }
        match latest_by_record.get(record.record_id.as_str()) {
            Some(previous)
                if previous.metadata.available_at_ms > record.metadata.available_at_ms =>
            {
                issues.push(DataQualityIssue {
                    code: "out_of_order_correction".to_owned(),
                    record_id: Some(record.record_id.clone()),
                    source: Some(record.metadata.source.clone()),
                    message: "늦게 도착한 과거 리비전을 원시 기록으로만 보존했습니다.".to_owned(),
                });
            }
            Some(_) => {
                latest_by_record.insert(record.record_id.as_str(), record);
            }
            None => {
                latest_by_record.insert(record.record_id.as_str(), record);
            }
        }
    }

    let selected = latest_by_record.into_values().cloned().collect::<Vec<_>>();
    let latest_by_source = selected.iter().fold(BTreeMap::new(), |mut acc, record| {
        acc.entry(record.metadata.source.as_str())
            .and_modify(|value: &mut u64| *value = (*value).max(record.metadata.available_at_ms))
            .or_insert(record.metadata.available_at_ms);
        acc
    });
    let mut required_source_failed = false;
    let mut degraded = !issues.is_empty();
    for policy in policies {
        if policy.source.trim().is_empty() || policy.maximum_age_ms == 0 {
            return Err("출처별 신선도 정책이 올바르지 않습니다.".to_owned());
        }
        match latest_by_source.get(policy.source.as_str()) {
            None => {
                issues.push(DataQualityIssue {
                    code: "missing_source".to_owned(),
                    record_id: None,
                    source: Some(policy.source.clone()),
                    message: "분석에 필요한 출처가 없습니다.".to_owned(),
                });
                required_source_failed |= policy.required;
                degraded = true;
            }
            Some(latest) if as_of_ms.saturating_sub(*latest) > policy.maximum_age_ms => {
                issues.push(DataQualityIssue {
                    code: "stale_source".to_owned(),
                    record_id: None,
                    source: Some(policy.source.clone()),
                    message: "출처 데이터가 허용된 최대 지연을 초과했습니다.".to_owned(),
                });
                required_source_failed |= policy.required;
                degraded = true;
            }
            Some(_) => {}
        }
    }
    if selected.is_empty() {
        required_source_failed = true;
        issues.push(DataQualityIssue {
            code: "empty_snapshot".to_owned(),
            record_id: None,
            source: None,
            message: "기준 시각에 이용 가능한 레코드가 없습니다.".to_owned(),
        });
    }
    let disposition = if required_source_failed {
        AnalysisDisposition::ObserveOnly
    } else if degraded {
        AnalysisDisposition::Degraded
    } else {
        AnalysisDisposition::Ready
    };
    Ok(PointInTimeSnapshot {
        snapshot_id: snapshot_id.to_owned(),
        as_of_ms,
        records: selected,
        disposition,
        order_allowed: disposition == AnalysisDisposition::Ready,
        issues,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityObservation {
    pub provider: String,
    pub item_id: String,
    pub text_fingerprint: String,
    pub published_at_ms: u64,
    pub available_at_ms: u64,
    pub sentiment_bps: i32,
    pub suspected_bot: bool,
    pub promotional: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunitySentimentSummary {
    pub unique_sample_count: usize,
    pub excluded_duplicate_count: usize,
    pub excluded_low_quality_count: usize,
    pub mean_sentiment_bps: Option<i32>,
    pub uncertainty: String,
    pub is_factual_evidence: bool,
}

pub fn summarize_community_sentiment(
    as_of_ms: u64,
    observations: &[CommunityObservation],
) -> Result<CommunitySentimentSummary, String> {
    let mut fingerprints = BTreeSet::new();
    let mut values = Vec::new();
    let mut duplicates = 0;
    let mut low_quality = 0;
    for item in observations {
        if item.provider.trim().is_empty()
            || !valid_id(&item.item_id)
            || item.text_fingerprint.len() < 16
            || item.sentiment_bps.unsigned_abs() > 10_000
            || item.published_at_ms > item.available_at_ms
        {
            return Err("커뮤니티 관측값의 출처·시각·점수 계약이 올바르지 않습니다.".to_owned());
        }
        if item.available_at_ms > as_of_ms {
            continue;
        }
        if !fingerprints.insert(item.text_fingerprint.as_str()) {
            duplicates += 1;
            continue;
        }
        if item.suspected_bot || item.promotional {
            low_quality += 1;
            continue;
        }
        values.push(i64::from(item.sentiment_bps));
    }
    let mean = if values.len() >= 5 {
        Some((values.iter().sum::<i64>() / values.len() as i64) as i32)
    } else {
        None
    };
    let uncertainty = match values.len() {
        0..=4 => "표본 부족: 방향 점수를 공개하지 않습니다.",
        5..=19 => "높음: 소규모 표본으로 참고용 심리만 제공합니다.",
        _ => "중간: 사실 뉴스와 분리된 커뮤니티 심리입니다.",
    };
    Ok(CommunitySentimentSummary {
        unique_sample_count: values.len(),
        excluded_duplicate_count: duplicates,
        excluded_low_quality_count: low_quality,
        mean_sentiment_bps: mean,
        uncertainty: uncertainty.to_owned(),
        is_factual_evidence: false,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackSourceInput {
    pub required_source: String,
    pub fallback_source: String,
    pub symbol: String,
    pub as_of_ms: u64,
    pub maximum_age_ms: u64,
    pub primary_available_at_ms: Option<u64>,
    pub primary_value_minor: Option<u64>,
    pub fallback_available_at_ms: u64,
    pub fallback_value_minor: u64,
    pub fallback_revision: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackSourceDecision {
    pub used_fallback: bool,
    pub analysis_allowed: bool,
    pub order_allowed: bool,
    pub value_difference_bps: Option<i64>,
    pub message: String,
}

pub fn evaluate_fallback_source(
    input: &FallbackSourceInput,
) -> Result<FallbackSourceDecision, String> {
    if input.required_source.trim().is_empty()
        || input.fallback_source.trim().is_empty()
        || input.required_source == input.fallback_source
        || input.symbol.trim().is_empty()
        || input.as_of_ms == 0
        || input.maximum_age_ms == 0
        || input.fallback_available_at_ms > input.as_of_ms
        || input.fallback_value_minor == 0
        || input.fallback_revision.trim().is_empty()
        || input.reason.trim().is_empty()
        || input
            .primary_available_at_ms
            .is_some_and(|value| value > input.as_of_ms)
        || input.primary_available_at_ms.is_some() != input.primary_value_minor.is_some()
    {
        return Err("대체 출처 비교의 출처·시각·값·리비전 계약이 올바르지 않습니다.".to_owned());
    }
    let primary_is_fresh = input.primary_available_at_ms.is_some_and(|available_at| {
        input.as_of_ms.saturating_sub(available_at) <= input.maximum_age_ms
    });
    if primary_is_fresh {
        return Ok(FallbackSourceDecision {
            used_fallback: false,
            analysis_allowed: true,
            order_allowed: true,
            value_difference_bps: input.primary_value_minor.map(|primary| {
                (i128::from(input.fallback_value_minor) - i128::from(primary))
                    .saturating_mul(10_000)
                    .checked_div(i128::from(primary.max(1)))
                    .and_then(|value| i64::try_from(value).ok())
                    .unwrap_or(if input.fallback_value_minor >= primary {
                        i64::MAX
                    } else {
                        i64::MIN
                    })
            }),
            message: "필수 출처가 신선하므로 대체 출처를 사용하지 않았습니다.".to_owned(),
        });
    }
    let fallback_is_fresh = input
        .as_of_ms
        .saturating_sub(input.fallback_available_at_ms)
        <= input.maximum_age_ms;
    let value_difference_bps = input.primary_value_minor.map(|primary| {
        (i128::from(input.fallback_value_minor) - i128::from(primary))
            .saturating_mul(10_000)
            .checked_div(i128::from(primary.max(1)))
            .and_then(|value| i64::try_from(value).ok())
            .unwrap_or(if input.fallback_value_minor >= primary {
                i64::MAX
            } else {
                i64::MIN
            })
    });
    Ok(FallbackSourceDecision {
        used_fallback: fallback_is_fresh,
        analysis_allowed: fallback_is_fresh,
        order_allowed: false,
        value_difference_bps,
        message: if fallback_is_fresh {
            format!(
                "{} 출처를 {} 사유로 분석에만 사용했습니다. 주문 승격은 차단됩니다.",
                input.fallback_source, input.reason
            )
        } else {
            "필수 출처와 대체 출처가 모두 신선도 기준을 충족하지 못했습니다.".to_owned()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(revision: &str, available_at_ms: u64) -> PointInTimeRecord {
        PointInTimeRecord {
            record_id: "news-1".to_owned(),
            snapshot_id: "snap-1".to_owned(),
            symbol: "005930".to_owned(),
            metadata: TemporalMetadata {
                event_time_ms: available_at_ms - 10,
                available_at_ms,
                ingested_at_ms: available_at_ms + 10,
                source: "official-news".to_owned(),
                source_revision: revision.to_owned(),
            },
            quality_flags: vec![],
            payload_hash: "0123456789abcdef".to_owned(),
        }
    }

    #[test]
    fn snapshot_excludes_future_corrections_and_is_reproducible() {
        let records = vec![record("v1", 100), record("v2", 300)];
        let first = build_point_in_time_snapshot(
            "snap-1",
            200,
            &records,
            &[SourceFreshnessPolicy {
                source: "official-news".to_owned(),
                required: true,
                maximum_age_ms: 200,
            }],
        )
        .expect("snapshot");
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.records[0].metadata.source_revision, "v1");
        assert!(first.order_allowed);
    }

    #[test]
    fn stale_required_source_fails_closed() {
        let snapshot = build_point_in_time_snapshot(
            "snap-1",
            1_000,
            &[record("v1", 100)],
            &[SourceFreshnessPolicy {
                source: "official-news".to_owned(),
                required: true,
                maximum_age_ms: 100,
            }],
        )
        .expect("snapshot");
        assert_eq!(snapshot.disposition, AnalysisDisposition::ObserveOnly);
        assert!(!snapshot.order_allowed);
    }

    #[test]
    fn community_summary_deduplicates_and_never_becomes_factual_evidence() {
        let observations = vec![
            CommunityObservation {
                provider: "provider".to_owned(),
                item_id: "item-1".to_owned(),
                text_fingerprint: "aaaaaaaaaaaaaaaa".to_owned(),
                published_at_ms: 10,
                available_at_ms: 11,
                sentiment_bps: 1_000,
                suspected_bot: false,
                promotional: false,
            },
            CommunityObservation {
                provider: "provider".to_owned(),
                item_id: "item-2".to_owned(),
                text_fingerprint: "aaaaaaaaaaaaaaaa".to_owned(),
                published_at_ms: 10,
                available_at_ms: 12,
                sentiment_bps: 9_000,
                suspected_bot: false,
                promotional: false,
            },
        ];
        let summary = summarize_community_sentiment(20, &observations).expect("summary");
        assert_eq!(summary.unique_sample_count, 1);
        assert_eq!(summary.excluded_duplicate_count, 1);
        assert_eq!(summary.mean_sentiment_bps, None);
        assert!(!summary.is_factual_evidence);
    }

    #[test]
    fn fallback_source_is_recorded_and_never_authorizes_orders() {
        let decision = evaluate_fallback_source(&FallbackSourceInput {
            required_source: "official".to_owned(),
            fallback_source: "secondary".to_owned(),
            symbol: "005930".to_owned(),
            as_of_ms: 1_000,
            maximum_age_ms: 100,
            primary_available_at_ms: Some(100),
            primary_value_minor: Some(10_000),
            fallback_available_at_ms: 950,
            fallback_value_minor: 10_100,
            fallback_revision: "secondary-v1".to_owned(),
            reason: "필수 출처 지연".to_owned(),
        })
        .expect("fallback decision");
        assert!(decision.used_fallback);
        assert!(decision.analysis_allowed);
        assert!(!decision.order_allowed);
        assert_eq!(decision.value_difference_bps, Some(100));
    }

    #[test]
    fn fallback_source_rejects_the_same_provider() {
        let result = evaluate_fallback_source(&FallbackSourceInput {
            required_source: "official".to_owned(),
            fallback_source: "official".to_owned(),
            symbol: "005930".to_owned(),
            as_of_ms: 1_000,
            maximum_age_ms: 100,
            primary_available_at_ms: None,
            primary_value_minor: None,
            fallback_available_at_ms: 950,
            fallback_value_minor: 10_100,
            fallback_revision: "v1".to_owned(),
            reason: "누락".to_owned(),
        });
        assert!(result.is_err());
    }
}
