use std::collections::BTreeSet;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use crate::persistence::{now_ms, PersistenceBridge};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionLevel {
    Idea,
    Code,
    UnitTested,
    AccountVerified,
    Deployed,
    RepeatedOperation,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicEvidenceArtifact {
    pub evidence_id: String,
    pub title: String,
    pub source_kind: String,
    pub source_reference: String,
    pub observed_completion: CompletionLevel,
    pub claimed_completion: CompletionLevel,
    pub verified_at_ms: u64,
    pub contains_personal_data: bool,
    pub contains_private_strategy: bool,
    pub license: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidencePack {
    pub pack_id: String,
    pub artifacts: Vec<PublicEvidenceArtifact>,
    pub excluded_evidence_ids: Vec<String>,
    pub confirmation_required: Vec<String>,
}

fn unsafe_reference(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.contains(":\\")
        || value.starts_with('/')
        || ["api_key", "client_secret", "access_token", "account_number"]
            .iter()
            .any(|marker| lower.contains(marker))
}

pub fn build_evidence_pack(
    pack_id: &str,
    artifacts: &[PublicEvidenceArtifact],
) -> Result<EvidencePack, String> {
    if pack_id.trim().is_empty() {
        return Err("근거 묶음 ID가 필요합니다.".to_owned());
    }
    let mut seen = BTreeSet::new();
    let mut included = Vec::new();
    let mut excluded = Vec::new();
    let mut confirmation = Vec::new();
    for artifact in artifacts {
        if artifact.evidence_id.trim().is_empty()
            || artifact.title.trim().is_empty()
            || artifact.verified_at_ms == 0
            || !seen.insert(artifact.evidence_id.as_str())
        {
            return Err("공개 근거의 ID·제목·검증 시각이 올바르지 않습니다.".to_owned());
        }
        if artifact.claimed_completion > artifact.observed_completion {
            confirmation.push(format!(
                "{}: 관측된 완료 수준보다 높은 주장을 확인해야 합니다.",
                artifact.evidence_id
            ));
            continue;
        }
        if artifact.contains_personal_data
            || artifact.contains_private_strategy
            || unsafe_reference(&artifact.source_reference)
            || artifact.license.as_deref().is_none_or(str::is_empty)
        {
            excluded.push(artifact.evidence_id.clone());
            continue;
        }
        included.push(artifact.clone());
    }
    Ok(EvidencePack {
        pack_id: pack_id.to_owned(),
        artifacts: included,
        excluded_evidence_ids: excluded,
        confirmation_required: confirmation,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentEvent {
    pub event_id: String,
    pub happened_at_ms: u64,
    pub action: String,
    pub outcome: String,
    pub blocker: Option<String>,
    pub evidence_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftArticle {
    pub recommended_title: String,
    pub alternative_titles: Vec<String>,
    pub sections: Vec<String>,
    pub body_markdown: String,
    pub confirmation_required: Vec<String>,
}

pub fn draft_development_article(
    project_name: &str,
    events: &[DevelopmentEvent],
) -> Result<DraftArticle, String> {
    if project_name.trim().is_empty() || events.is_empty() {
        return Err("프로젝트명과 실제 개발 사건이 필요합니다.".to_owned());
    }
    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| event.happened_at_ms);
    if ordered.iter().any(|event| {
        event.event_id.trim().is_empty()
            || event.happened_at_ms == 0
            || event.action.trim().is_empty()
            || event.outcome.trim().is_empty()
            || event.evidence_id.trim().is_empty()
    }) {
        return Err("개발 사건의 ID·시각·행동·결과·근거가 필요합니다.".to_owned());
    }
    let sections = vec![
        "무엇을 만들려고 했나".to_owned(),
        "실제로 시도한 것".to_owned(),
        "막힌 지점과 수정".to_owned(),
        "현재 검증된 범위".to_owned(),
    ];
    let mut body = format!("# {project_name} 개발 기록\n\n## 실제로 시도한 것\n\n");
    for event in &ordered {
        body.push_str(&format!(
            "- {} — {} (`{}`)\n",
            event.action, event.outcome, event.evidence_id
        ));
        if let Some(blocker) = &event.blocker {
            body.push_str(&format!("  - 막힌 지점: {blocker}\n"));
        }
    }
    body.push_str("\n## 현재 검증된 범위\n\n근거 묶음에서 확인된 내용만 공개합니다.\n");
    Ok(DraftArticle {
        recommended_title: format!("{project_name}, 아이디어에서 검증 가능한 프로그램까지"),
        alternative_titles: vec![
            format!("{project_name}를 만들며 실제로 막힌 것들"),
            format!("자동화를 믿기 전에 {project_name}에 넣은 안전장치"),
        ],
        sections,
        body_markdown: body,
        confirmation_required: Vec::new(),
    })
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCandidate {
    pub media_id: String,
    pub evidence_id: String,
    pub caption: String,
    pub alt_text: String,
    pub license: Option<String>,
    pub synthetic: bool,
    pub presented_as_real_execution: bool,
    pub width_px: u32,
    pub mobile_overflow: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaReview {
    pub approved_media_ids: Vec<String>,
    pub rejected: Vec<String>,
}

pub fn review_media(candidates: &[MediaCandidate]) -> Result<MediaReview, String> {
    let mut approved = Vec::new();
    let mut rejected = Vec::new();
    for item in candidates {
        if item.media_id.trim().is_empty()
            || item.evidence_id.trim().is_empty()
            || item.caption.trim().is_empty()
            || item.alt_text.trim().is_empty()
            || item.width_px == 0
        {
            return Err("미디어에는 ID·근거·캡션·대체 텍스트·크기가 필요합니다.".to_owned());
        }
        if item.license.as_deref().is_none_or(str::is_empty)
            || item.mobile_overflow
            || (item.synthetic && item.presented_as_real_execution)
        {
            rejected.push(item.media_id.clone());
        } else {
            approved.push(item.media_id.clone());
        }
    }
    Ok(MediaReview {
        approved_media_ids: approved,
        rejected,
    })
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArticleStatus {
    Draft,
    Approved,
    PrivateSaved,
    PublishFailed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArticleRevision {
    pub article_id: String,
    pub revision: u32,
    pub content_hash: String,
    pub media_hash: String,
    pub status: ArticleStatus,
    pub approved_revision: Option<u32>,
    pub publish_idempotency_key: Option<String>,
}

pub fn approve_article(revision: &mut ArticleRevision) -> Result<(), String> {
    if revision.article_id.trim().is_empty()
        || revision.revision == 0
        || revision.content_hash.len() < 16
        || revision.media_hash.len() < 16
    {
        return Err("승인할 원고 리비전과 콘텐츠·미디어 해시가 필요합니다.".to_owned());
    }
    revision.status = ArticleStatus::Approved;
    revision.approved_revision = Some(revision.revision);
    Ok(())
}

pub fn revise_article(
    article: &mut ArticleRevision,
    new_content_hash: &str,
    new_media_hash: &str,
) -> Result<(), String> {
    if new_content_hash.len() < 16 || new_media_hash.len() < 16 {
        return Err("새 콘텐츠와 미디어 해시가 필요합니다.".to_owned());
    }
    article.revision = article
        .revision
        .checked_add(1)
        .ok_or_else(|| "원고 리비전이 지원 범위를 초과했습니다.".to_owned())?;
    article.content_hash = new_content_hash.to_owned();
    article.media_hash = new_media_hash.to_owned();
    article.status = ArticleStatus::Draft;
    article.approved_revision = None;
    article.publish_idempotency_key = None;
    Ok(())
}

pub fn save_private_for_publish(
    article: &mut ArticleRevision,
    idempotency_key: &str,
) -> Result<(), String> {
    if article.status != ArticleStatus::Approved
        || article.approved_revision != Some(article.revision)
        || idempotency_key.trim().is_empty()
    {
        return Err("현재 리비전의 대표 승인과 게시 멱등성 키가 필요합니다.".to_owned());
    }
    if article
        .publish_idempotency_key
        .as_deref()
        .is_some_and(|existing| existing != idempotency_key)
    {
        return Err("같은 원고 리비전에 다른 게시 키를 사용할 수 없습니다.".to_owned());
    }
    article.publish_idempotency_key = Some(idempotency_key.to_owned());
    article.status = ArticleStatus::PrivateSaved;
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PersistedArticleStatus {
    Draft,
    Rejected,
    Approved,
    PrivateSaved,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveArticleDraftRequest {
    pub article_id: String,
    pub title: String,
    pub body_markdown: String,
    pub media_ids: Vec<String>,
    pub links: Vec<String>,
    pub masking_confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewArticleRequest {
    pub article_id: String,
    pub revision: u32,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PersistedArticleRevision {
    pub article_id: String,
    pub revision: u32,
    pub title: String,
    pub body_markdown: String,
    pub media_ids: Vec<String>,
    pub links: Vec<String>,
    pub masking_confirmed: bool,
    pub status: PersistedArticleStatus,
    pub review_note: Option<String>,
    pub created_at_ms: u64,
}

fn valid_article_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_article_draft(request: &SaveArticleDraftRequest) -> Result<(), String> {
    if !valid_article_id(request.article_id.trim())
        || request.title.trim().is_empty()
        || request.title.chars().count() > 300
        || request.body_markdown.trim().is_empty()
        || request.body_markdown.len() > 200_000
        || request.media_ids.len() > 100
        || request.links.len() > 100
        || request
            .media_ids
            .iter()
            .any(|value| !valid_article_id(value))
        || request.links.iter().any(|value| {
            !(value.starts_with("https://") || value.starts_with("http://"))
                || unsafe_reference(value)
        })
    {
        return Err("원고 ID·제목·본문·미디어·링크 형식을 확인해 주세요.".to_owned());
    }
    if unsafe_reference(&request.body_markdown) {
        return Err("원고에 비밀정보 또는 절대 로컬 경로로 의심되는 문자열이 있습니다.".to_owned());
    }
    Ok(())
}

fn persisted_status_db(status: &PersistedArticleStatus) -> &'static str {
    match status {
        PersistedArticleStatus::Draft => "draft",
        PersistedArticleStatus::Rejected => "rejected",
        PersistedArticleStatus::Approved => "approved",
        PersistedArticleStatus::PrivateSaved => "private_saved",
    }
}

fn article_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PersistedArticleRevision> {
    let status: String = row.get(7)?;
    let media_json: String = row.get(4)?;
    let links_json: String = row.get(5)?;
    let media_ids = serde_json::from_str(&media_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let links = serde_json::from_str(&links_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(PersistedArticleRevision {
        article_id: row.get(0)?,
        revision: row.get(1)?,
        title: row.get(2)?,
        body_markdown: row.get(3)?,
        media_ids,
        links,
        masking_confirmed: row.get(6)?,
        status: match status.as_str() {
            "rejected" => PersistedArticleStatus::Rejected,
            "approved" => PersistedArticleStatus::Approved,
            "private_saved" => PersistedArticleStatus::PrivateSaved,
            _ => PersistedArticleStatus::Draft,
        },
        review_note: row.get(8)?,
        created_at_ms: row.get(9)?,
    })
}

fn latest_article(
    connection: &rusqlite::Connection,
    article_id: &str,
) -> Result<Option<PersistedArticleRevision>, String> {
    connection.query_row(
        "SELECT article_id,revision,title,body_markdown,media_json,links_json,masking_confirmed,status,review_note,created_at_ms FROM publicity_article_revisions WHERE article_id=?1 ORDER BY revision DESC LIMIT 1",
        params![article_id], article_row,
    ).optional().map_err(|error| format!("원고 리비전을 조회하지 못했습니다: {error}"))
}

fn save_article_draft(
    request: SaveArticleDraftRequest,
    bridge: &PersistenceBridge,
) -> Result<PersistedArticleRevision, String> {
    validate_article_draft(&request)?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "홍보 원고 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let latest = latest_article(&connection, &request.article_id)?;
    let media_json = serde_json::to_string(&request.media_ids)
        .map_err(|_| "미디어 목록을 직렬화하지 못했습니다.".to_owned())?;
    let links_json = serde_json::to_string(&request.links)
        .map_err(|_| "링크 목록을 직렬화하지 못했습니다.".to_owned())?;
    if let Some(latest) = latest.as_ref() {
        if latest.title == request.title.trim()
            && latest.body_markdown == request.body_markdown.trim()
            && latest.media_ids == request.media_ids
            && latest.links == request.links
            && latest.masking_confirmed == request.masking_confirmed
        {
            return Ok(latest.clone());
        }
    }
    let revision = match latest {
        Some(item) => item
            .revision
            .checked_add(1)
            .ok_or_else(|| "원고 리비전 한도를 초과했습니다.".to_owned())?,
        None => 1,
    };
    let created_at_ms = now_ms()?;
    connection.execute(
        "INSERT INTO publicity_article_revisions(article_id,revision,title,body_markdown,media_json,links_json,masking_confirmed,status,review_note,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,'draft',NULL,?8)",
        params![request.article_id,revision,request.title.trim(),request.body_markdown.trim(),media_json,links_json,request.masking_confirmed,created_at_ms],
    ).map_err(|error| format!("새 원고 리비전을 저장하지 못했습니다: {error}"))?;
    latest_article(&connection, &request.article_id)?
        .ok_or_else(|| "저장된 원고 리비전을 찾지 못했습니다.".to_owned())
}

#[tauri::command]
pub fn publicity_article_draft_save(
    request: SaveArticleDraftRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<PersistedArticleRevision, String> {
    save_article_draft(request, bridge.inner())
}

fn review_article(
    request: ReviewArticleRequest,
    approved: bool,
    bridge: &PersistenceBridge,
) -> Result<PersistedArticleRevision, String> {
    if !valid_article_id(&request.article_id)
        || request.revision == 0
        || request.note.trim().len() < 2
        || request.note.len() > 1_000
    {
        return Err("검토할 원고 리비전과 검토 의견을 확인해 주세요.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "홍보 원고 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let latest = latest_article(&connection, &request.article_id)?
        .ok_or_else(|| "검토할 원고가 없습니다.".to_owned())?;
    if latest.revision != request.revision {
        return Err("최신 원고 리비전만 검토할 수 있습니다.".to_owned());
    }
    if approved && !latest.masking_confirmed {
        return Err("마스킹 확인 전에는 원고를 승인할 수 없습니다.".to_owned());
    }
    let status = if approved {
        PersistedArticleStatus::Approved
    } else {
        PersistedArticleStatus::Rejected
    };
    connection.execute(
        "UPDATE publicity_article_revisions SET status=?3,review_note=?4 WHERE article_id=?1 AND revision=?2 AND status IN ('draft','rejected','approved')",
        params![request.article_id,request.revision,persisted_status_db(&status),request.note.trim()],
    ).map_err(|error| format!("원고 검토 결과를 저장하지 못했습니다: {error}"))?;
    latest_article(&connection, &request.article_id)?
        .ok_or_else(|| "검토된 원고를 찾지 못했습니다.".to_owned())
}

#[tauri::command]
pub fn publicity_article_approve(
    request: ReviewArticleRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<PersistedArticleRevision, String> {
    review_article(request, true, bridge.inner())
}

#[tauri::command]
pub fn publicity_article_reject(
    request: ReviewArticleRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<PersistedArticleRevision, String> {
    review_article(request, false, bridge.inner())
}

#[tauri::command]
pub fn publicity_article_latest(
    article_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Option<PersistedArticleRevision>, String> {
    if !valid_article_id(&article_id) {
        return Err("원고 ID를 확인해 주세요.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "홍보 원고 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    latest_article(&connection, &article_id)
}

#[tauri::command]
pub fn publicity_evidence_pack_preview(
    pack_id: String,
    artifacts: Vec<PublicEvidenceArtifact>,
) -> Result<EvidencePack, String> {
    build_evidence_pack(&pack_id, &artifacts)
}

#[tauri::command]
pub fn publicity_draft_preview(
    project_name: String,
    events: Vec<DevelopmentEvent>,
) -> Result<DraftArticle, String> {
    draft_development_article(&project_name, &events)
}

#[tauri::command]
pub fn publicity_media_review(candidates: Vec<MediaCandidate>) -> Result<MediaReview, String> {
    review_media(&candidates)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualPublishPackageRequest {
    pub package_id: String,
    pub article_id: String,
    pub revision: u32,
    pub evidence_pack: EvidencePack,
    pub media_review: MediaReview,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualPublishPackageReceipt {
    pub package_id: String,
    pub directory_name: String,
    pub included_evidence_count: usize,
    pub approved_media_count: usize,
    pub external_publish_performed: bool,
}

fn safe_package_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[tauri::command]
pub fn publicity_manual_package_export(
    app: tauri::AppHandle,
    request: ManualPublishPackageRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<ManualPublishPackageReceipt, String> {
    let article = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "홍보 원고 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        latest_article(&connection, &request.article_id)?
            .ok_or_else(|| "내보낼 원고가 없습니다.".to_owned())?
    };
    let evidence_pack = build_evidence_pack(
        &request.evidence_pack.pack_id,
        &request.evidence_pack.artifacts,
    )?;
    let media_review = &request.media_review;
    let media_ids_valid = media_review.approved_media_ids.iter().all(|media_id| {
        valid_article_id(media_id) && article.media_ids.iter().any(|saved| saved == media_id)
    }) && media_review
        .rejected
        .iter()
        .all(|media_id| valid_article_id(media_id));
    if !safe_package_id(&request.package_id)
        || article.revision != request.revision
        || article.status != PersistedArticleStatus::Approved
        || !article.masking_confirmed
        || !evidence_pack.confirmation_required.is_empty()
        || !media_ids_valid
    {
        return Err("최신 대표 승인 리비전과 확인이 끝난 공개 근거 묶음이 필요합니다.".to_owned());
    }
    if unsafe_reference(&article.body_markdown)
        || article
            .body_markdown
            .to_ascii_lowercase()
            .contains("client secret")
        || article
            .body_markdown
            .to_ascii_lowercase()
            .contains("private strategy")
    {
        return Err("원고에 비밀정보·절대 경로·비공개 전략 의심 문자열이 있습니다.".to_owned());
    }
    let parent = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("앱 데이터 경로를 확인하지 못했습니다: {error}"))?
        .join("publish-packages");
    std::fs::create_dir_all(&parent)
        .map_err(|error| format!("수동 게시 패키지 상위 폴더를 만들지 못했습니다: {error}"))?;
    let root = parent.join(&request.package_id);
    std::fs::create_dir(&root).map_err(|error| {
        format!("같은 ID의 수동 게시 패키지가 이미 있거나 폴더를 만들 수 없습니다: {error}")
    })?;
    let article = format!(
        "# {}\n\n{}\n",
        article.title.trim(),
        article.body_markdown.trim()
    );
    let included_evidence_count = evidence_pack.artifacts.len();
    let approved_media_count = media_review.approved_media_ids.len();
    let manifest = serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "packageId": request.package_id,
        "manualPublishOnly": true,
        "evidence": evidence_pack.artifacts,
        "excludedEvidenceIds": evidence_pack.excluded_evidence_ids,
        "approvedMediaIds": media_review.approved_media_ids,
        "rejectedMedia": media_review.rejected,
    }))
    .map_err(|error| format!("패키지 목록을 만들지 못했습니다: {error}"))?;
    std::fs::write(root.join("article.md"), article)
        .map_err(|error| format!("원고를 내보내지 못했습니다: {error}"))?;
    std::fs::write(root.join("manifest.json"), manifest)
        .map_err(|error| format!("패키지 목록을 내보내지 못했습니다: {error}"))?;
    std::fs::write(root.join("README.txt"), "이 폴더는 티스토리 자동 게시를 수행하지 않습니다. article.md를 검토한 뒤 사용자가 직접 복사하고 승인된 미디어만 업로드하세요.\n")
        .map_err(|error| format!("수동 게시 안내를 내보내지 못했습니다: {error}"))?;
    Ok(ManualPublishPackageReceipt {
        package_id: request.package_id,
        directory_name: root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_owned(),
        included_evidence_count,
        approved_media_count,
        external_publish_performed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_id_rejects_path_traversal() {
        assert!(!safe_package_id("../escape"));
        assert!(safe_package_id("investa-release_1"));
    }

    #[test]
    fn evidence_pack_rejects_overclaiming_secrets_and_unknown_licenses() {
        let pack = build_evidence_pack(
            "pack-1",
            &[
                PublicEvidenceArtifact {
                    evidence_id: "evidence-1".to_owned(),
                    title: "단위 테스트".to_owned(),
                    source_kind: "test".to_owned(),
                    source_reference: "cargo-test-output".to_owned(),
                    observed_completion: CompletionLevel::UnitTested,
                    claimed_completion: CompletionLevel::UnitTested,
                    verified_at_ms: 1,
                    contains_personal_data: false,
                    contains_private_strategy: false,
                    license: Some("project-owned".to_owned()),
                },
                PublicEvidenceArtifact {
                    evidence_id: "evidence-2".to_owned(),
                    title: "로컬 코드".to_owned(),
                    source_kind: "code".to_owned(),
                    source_reference: "C:\\private\\file".to_owned(),
                    observed_completion: CompletionLevel::Code,
                    claimed_completion: CompletionLevel::Deployed,
                    verified_at_ms: 1,
                    contains_personal_data: false,
                    contains_private_strategy: false,
                    license: None,
                },
            ],
        )
        .expect("pack");
        assert_eq!(pack.artifacts.len(), 1);
        assert_eq!(pack.confirmation_required.len(), 1);
    }

    #[test]
    fn article_changes_invalidate_approval_and_publish_key() {
        let mut article = ArticleRevision {
            article_id: "article-1".to_owned(),
            revision: 1,
            content_hash: "0123456789abcdef".to_owned(),
            media_hash: "fedcba9876543210".to_owned(),
            status: ArticleStatus::Draft,
            approved_revision: None,
            publish_idempotency_key: None,
        };
        approve_article(&mut article).expect("approve");
        save_private_for_publish(&mut article, "publish-1").expect("save private");
        revise_article(&mut article, "1111111111111111", "2222222222222222").expect("revise");
        assert_eq!(article.status, ArticleStatus::Draft);
        assert_eq!(article.approved_revision, None);
        assert_eq!(article.publish_idempotency_key, None);
    }

    #[test]
    fn persisted_article_edit_creates_revision_and_invalidates_approval() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let draft = |body: &str| SaveArticleDraftRequest {
            article_id: "investa-devlog".into(),
            title: "Investa 개발기".into(),
            body_markdown: body.into(),
            media_ids: vec!["screen-1".into()],
            links: vec!["https://example.com/evidence".into()],
            masking_confirmed: true,
        };
        let first = save_article_draft(draft("첫 번째 검증 원고"), &bridge).expect("draft");
        let approved = review_article(
            ReviewArticleRequest {
                article_id: first.article_id.clone(),
                revision: first.revision,
                note: "공개 근거 확인".into(),
            },
            true,
            &bridge,
        )
        .expect("approve");
        assert_eq!(approved.status, PersistedArticleStatus::Approved);
        let second = save_article_draft(draft("수정된 검증 원고"), &bridge).expect("revision");
        assert_eq!(second.revision, 2);
        assert_eq!(second.status, PersistedArticleStatus::Draft);
        assert!(review_article(
            ReviewArticleRequest {
                article_id: second.article_id.clone(),
                revision: 1,
                note: "오래된 승인".into()
            },
            true,
            &bridge
        )
        .is_err());
    }
}
