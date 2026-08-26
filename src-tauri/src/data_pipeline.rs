use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::data_quality::{CommunityObservation, PointInTimeRecord, QualityFlag, TemporalMetadata};

const MAX_BATCH_ITEMS: usize = 500;
const MAX_CONTENT_CHARS: usize = 50_000;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceChannel {
    Market,
    Financial,
    News,
    Community,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawEvidenceItem {
    pub item_id: String,
    pub symbol: String,
    pub provider: String,
    pub provider_revision: String,
    pub channel: EvidenceChannel,
    pub event_time_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub content: String,
    pub sentiment_bps: Option<i32>,
    pub suspected_bot: bool,
    pub promotional: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeEvidenceRequest {
    pub snapshot_id: String,
    pub items: Vec<RawEvidenceItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizeEvidenceResult {
    pub records: Vec<PointInTimeRecord>,
    pub community_observations: Vec<CommunityObservation>,
    pub duplicate_items: usize,
    pub factual_record_count: usize,
    pub community_record_count: usize,
    pub raw_content_persisted: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn normalize_evidence(
    request: NormalizeEvidenceRequest,
) -> Result<NormalizeEvidenceResult, String> {
    if !valid_id(&request.snapshot_id)
        || request.items.is_empty()
        || request.items.len() > MAX_BATCH_ITEMS
    {
        return Err("스냅샷 ID와 1~500개의 근거 항목이 필요합니다.".to_owned());
    }
    let mut seen = BTreeSet::new();
    let mut duplicates = 0;
    let mut records = Vec::new();
    let mut communities = Vec::new();
    for item in request.items {
        if !valid_id(&item.item_id)
            || item.symbol.trim().is_empty()
            || item.provider.trim().is_empty()
            || item.provider_revision.trim().is_empty()
            || item.content.trim().is_empty()
            || item.content.chars().count() > MAX_CONTENT_CHARS
            || item.event_time_ms == 0
            || item.event_time_ms > item.available_at_ms
            || item.available_at_ms > item.ingested_at_ms
        {
            return Err("근거 항목의 ID·본문·출처·시각 계약이 올바르지 않습니다.".to_owned());
        }
        let content_hash = hash(item.content.trim());
        if !seen.insert((
            item.provider.clone(),
            item.item_id.clone(),
            content_hash.clone(),
        )) {
            duplicates += 1;
            continue;
        }
        if item.channel == EvidenceChannel::Community {
            let sentiment_bps = item.sentiment_bps.ok_or_else(|| {
                "커뮤니티 항목에는 -10000~10000 심리 점수가 필요합니다.".to_owned()
            })?;
            if sentiment_bps.unsigned_abs() > 10_000 {
                return Err("커뮤니티 심리 점수는 -10000~10000이어야 합니다.".to_owned());
            }
            communities.push(CommunityObservation {
                provider: item.provider,
                item_id: item.item_id,
                text_fingerprint: content_hash,
                published_at_ms: item.event_time_ms,
                available_at_ms: item.available_at_ms,
                sentiment_bps,
                suspected_bot: item.suspected_bot,
                promotional: item.promotional,
            });
            continue;
        }
        if item.sentiment_bps.is_some() {
            return Err(
                "사실 데이터와 뉴스에는 커뮤니티 심리 점수를 혼합할 수 없습니다.".to_owned(),
            );
        }
        let mut quality_flags = Vec::new();
        if item.suspected_bot {
            quality_flags.push(QualityFlag::SuspectedBot);
        }
        if item.promotional {
            quality_flags.push(QualityFlag::Promotional);
        }
        records.push(PointInTimeRecord {
            record_id: item.item_id,
            snapshot_id: request.snapshot_id.clone(),
            symbol: item.symbol,
            metadata: TemporalMetadata {
                event_time_ms: item.event_time_ms,
                available_at_ms: item.available_at_ms,
                ingested_at_ms: item.ingested_at_ms,
                source: item.provider,
                source_revision: item.provider_revision,
            },
            quality_flags,
            payload_hash: content_hash,
        });
    }
    let factual_record_count = records.len();
    let community_record_count = communities.len();
    Ok(NormalizeEvidenceResult {
        records,
        community_observations: communities,
        duplicate_items: duplicates,
        factual_record_count,
        community_record_count,
        raw_content_persisted: false,
    })
}

#[tauri::command]
pub fn data_normalize_preview(
    request: NormalizeEvidenceRequest,
) -> Result<NormalizeEvidenceResult, String> {
    normalize_evidence(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, channel: EvidenceChannel) -> RawEvidenceItem {
        RawEvidenceItem {
            item_id: id.to_owned(),
            symbol: "005930".to_owned(),
            provider: "provider".to_owned(),
            provider_revision: "rev-1".to_owned(),
            channel,
            event_time_ms: 10,
            available_at_ms: 20,
            ingested_at_ms: 30,
            content: format!("{id} normalized content"),
            sentiment_bps: (channel == EvidenceChannel::Community).then_some(500),
            suspected_bot: false,
            promotional: false,
        }
    }

    #[test]
    fn separates_factual_evidence_from_community_sentiment() {
        let result = normalize_evidence(NormalizeEvidenceRequest {
            snapshot_id: "snapshot-1".to_owned(),
            items: vec![
                item("news-1", EvidenceChannel::News),
                item("post-1", EvidenceChannel::Community),
            ],
        })
        .expect("normalize");
        assert_eq!(result.factual_record_count, 1);
        assert_eq!(result.community_record_count, 1);
        assert!(!result.raw_content_persisted);
        assert_eq!(result.records[0].payload_hash.len(), 64);
    }

    #[test]
    fn rejects_sentiment_mixed_into_factual_news() {
        let mut news = item("news-1", EvidenceChannel::News);
        news.sentiment_bps = Some(100);
        assert!(normalize_evidence(NormalizeEvidenceRequest {
            snapshot_id: "snapshot-2".to_owned(),
            items: vec![news],
        })
        .is_err());
    }

    #[test]
    fn deduplicates_identical_provider_revision_payload() {
        let original = item("post-1", EvidenceChannel::Community);
        let duplicate = item("post-1", EvidenceChannel::Community);
        let result = normalize_evidence(NormalizeEvidenceRequest {
            snapshot_id: "snapshot-3".to_owned(),
            items: vec![original, duplicate],
        })
        .expect("normalize");
        assert_eq!(result.duplicate_items, 1);
        assert_eq!(result.community_record_count, 1);
    }
}
