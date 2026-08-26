use std::path::Path;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    pub event_id: String,
    pub actor: String,
    pub action: String,
    pub target_id: String,
    pub previous_hash: Option<String>,
    pub next_hash: Option<String>,
    pub correlation_id: String,
    pub occurred_at_ms: u64,
    pub detail: String,
}

fn contains_secret_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "client_secret",
        "api_secret",
        "access_token",
        "authorization:",
        "password=",
        "private_key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub fn validate_audit_event(event: &AuditEvent) -> Result<(), String> {
    if event.event_id.trim().is_empty()
        || event.actor.trim().is_empty()
        || event.action.trim().is_empty()
        || event.target_id.trim().is_empty()
        || event.correlation_id.trim().is_empty()
        || event.occurred_at_ms == 0
    {
        return Err("감사 사건에는 행위자·시각·대상·상관 ID가 필요합니다.".to_owned());
    }
    if contains_secret_marker(&event.detail) {
        return Err("감사 로그에는 API 키·토큰·비밀번호를 기록할 수 없습니다.".to_owned());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalAlert {
    pub alert_id: String,
    pub deduplication_key: String,
    pub severity: AlertSeverity,
    pub message: String,
    pub first_seen_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub occurrence_count: u64,
    pub acknowledged_at_ms: Option<u64>,
    pub response: Option<String>,
}

pub fn merge_alert(
    existing: Option<&OperationalAlert>,
    mut incoming: OperationalAlert,
    deduplication_window_ms: u64,
) -> Result<OperationalAlert, String> {
    if incoming.alert_id.trim().is_empty()
        || incoming.deduplication_key.trim().is_empty()
        || incoming.message.trim().is_empty()
        || incoming.first_seen_at_ms == 0
        || incoming.last_seen_at_ms < incoming.first_seen_at_ms
    {
        return Err("운영 알림 계약이 올바르지 않습니다.".to_owned());
    }
    if let Some(previous) = existing {
        if previous.deduplication_key == incoming.deduplication_key
            && incoming
                .first_seen_at_ms
                .saturating_sub(previous.last_seen_at_ms)
                <= deduplication_window_ms
        {
            incoming.first_seen_at_ms = previous.first_seen_at_ms;
            incoming.occurrence_count = previous.occurrence_count.saturating_add(1);
            incoming.acknowledged_at_ms = None;
            incoming.response = None;
        }
    }
    Ok(incoming)
}

pub fn acknowledge_alert(
    alert: &mut OperationalAlert,
    acknowledged_at_ms: u64,
    response: &str,
) -> Result<(), String> {
    if alert.severity == AlertSeverity::Critical && response.trim().is_empty() {
        return Err("치명 알림은 대응 내용을 기록해야 합니다.".to_owned());
    }
    if acknowledged_at_ms < alert.last_seen_at_ms {
        return Err("알림 발생 이전 시각으로 확인 처리할 수 없습니다.".to_owned());
    }
    alert.acknowledged_at_ms = Some(acknowledged_at_ms);
    alert.response = Some(response.to_owned());
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthComponent {
    pub component_id: String,
    pub critical: bool,
    pub healthy: bool,
    pub last_success_at_ms: Option<u64>,
    pub retry_action: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub generated_at_ms: u64,
    pub automated_trading_ready: bool,
    pub components: Vec<HealthComponent>,
}

pub fn build_health_report(
    generated_at_ms: u64,
    components: &[HealthComponent],
) -> Result<HealthReport, String> {
    if generated_at_ms == 0
        || components.is_empty()
        || components.iter().any(|component| {
            component.component_id.trim().is_empty()
                || component.retry_action.trim().is_empty()
                || contains_secret_marker(&component.detail)
        })
    {
        return Err("비밀정보를 제거한 유효한 상태 구성요소가 필요합니다.".to_owned());
    }
    Ok(HealthReport {
        generated_at_ms,
        automated_trading_ready: components
            .iter()
            .all(|component| !component.critical || component.healthy),
        components: components.to_vec(),
    })
}

pub fn create_consistent_sqlite_backup(
    source: &Connection,
    destination: &Path,
) -> Result<(), String> {
    if destination.extension().and_then(|value| value.to_str()) != Some("sqlite3") {
        return Err("백업 파일은 .sqlite3 확장자를 사용해야 합니다.".to_owned());
    }
    if destination.exists() {
        return Err("기존 백업 파일을 덮어쓰지 않습니다.".to_owned());
    }
    let parent = destination
        .parent()
        .ok_or_else(|| "백업 대상 폴더를 확인하지 못했습니다.".to_owned())?;
    if !parent.is_dir() {
        return Err("존재하는 백업 대상 폴더가 필요합니다.".to_owned());
    }
    let path = destination
        .to_str()
        .ok_or_else(|| "백업 경로를 UTF-8로 표현할 수 없습니다.".to_owned())?;
    source
        .execute("VACUUM INTO ?1", params![path])
        .map_err(|error| format!("SQLite 일관 백업을 생성하지 못했습니다: {error}"))?;
    let backup = Connection::open(destination)
        .map_err(|error| format!("생성한 백업을 열지 못했습니다: {error}"))?;
    let check: String = backup
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|error| format!("백업 무결성을 검사하지 못했습니다: {error}"))?;
    if check != "ok" {
        return Err(format!(
            "생성한 백업의 quick_check 결과가 올바르지 않습니다: {check}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_rejects_secret_material() {
        let event = AuditEvent {
            event_id: "audit-1".to_owned(),
            actor: "local_user".to_owned(),
            action: "update".to_owned(),
            target_id: "risk-v1".to_owned(),
            previous_hash: None,
            next_hash: Some("hash".to_owned()),
            correlation_id: "trace-1".to_owned(),
            occurred_at_ms: 1,
            detail: "client_secret=plain".to_owned(),
        };
        assert!(validate_audit_event(&event).is_err());
    }

    #[test]
    fn critical_health_failure_disables_automated_trading_readiness() {
        let report = build_health_report(
            10,
            &[HealthComponent {
                component_id: "ledger".to_owned(),
                critical: true,
                healthy: false,
                last_success_at_ms: Some(1),
                retry_action: "원장 대사 재실행".to_owned(),
                detail: "원장 불일치".to_owned(),
            }],
        )
        .expect("health");
        assert!(!report.automated_trading_ready);
    }

    #[test]
    fn vacuum_into_creates_a_checked_snapshot_without_overwrite() {
        let source = Connection::open_in_memory().expect("source");
        source
            .execute_batch("CREATE TABLE sample(id INTEGER PRIMARY KEY, value TEXT); INSERT INTO sample(value) VALUES ('kept');")
            .expect("fixture");
        let path = std::env::temp_dir().join(format!(
            "investa-backup-test-{}.sqlite3",
            std::process::id()
        ));
        if path.exists() {
            std::fs::remove_file(&path).expect("remove stale test backup");
        }
        create_consistent_sqlite_backup(&source, &path).expect("backup");
        let backup = Connection::open(&path).expect("open backup");
        let value: String = backup
            .query_row("SELECT value FROM sample", [], |row| row.get(0))
            .expect("read backup");
        assert_eq!(value, "kept");
        assert!(create_consistent_sqlite_backup(&source, &path).is_err());
        drop(backup);
        std::fs::remove_file(path).expect("remove test backup");
    }
}
