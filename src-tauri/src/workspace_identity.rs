use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use uuid::Uuid;

const SERVICE: &str = "Investa.WorkspaceIdentity";
const OWNER_ACCOUNT: &str = "workspace-owner-v1";
const LEGACY_GITHUB_SERVICE: &str = "Investa.GitHubGate";
const LEGACY_GITHUB_ACCOUNT: &str = "workspace-owner-id";
const LEGACY_GOOGLE_SERVICE: &str = "Investa.SocialAuth";
const LEGACY_GOOGLE_ACCOUNT: &str = "google-owner-subject";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkedIdentity {
    provider: String,
    subject: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceOwnerRecord {
    version: u8,
    workspace_id: String,
    primary: LinkedIdentity,
    linked_identities: Vec<LinkedIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthenticatedIdentity(LinkedIdentity);

#[derive(Default)]
pub struct WorkspaceIdentityBridge {
    authenticated: Mutex<Option<AuthenticatedIdentity>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIdentityStatus {
    initialized: bool,
    session_authenticated: bool,
    primary_provider: Option<String>,
    linked_providers: Vec<String>,
    linked_account_count: usize,
}

fn credential(service: &str, account: &str) -> Result<Entry, String> {
    Entry::new(service, account)
        .map_err(|_| "작업공간 소유자 보안 저장소를 열지 못했습니다.".to_owned())
}

fn optional_credential(service: &str, account: &str) -> Result<Option<String>, String> {
    match credential(service, account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("작업공간 소유자 정보를 읽지 못했습니다.".to_owned()),
    }
}

fn validate_identity(provider: &str, subject: &str) -> Result<LinkedIdentity, String> {
    if !matches!(provider, "github" | "google" | "apple") {
        return Err("지원하지 않는 로그인 공급자입니다.".to_owned());
    }
    let subject = subject.trim();
    if subject.is_empty() || subject.len() > 256 || subject.chars().any(char::is_control) {
        return Err("로그인 계정 식별자가 올바르지 않습니다.".to_owned());
    }
    Ok(LinkedIdentity {
        provider: provider.to_owned(),
        subject: subject.to_owned(),
    })
}

fn load_record() -> Result<Option<WorkspaceOwnerRecord>, String> {
    let Some(raw) = optional_credential(SERVICE, OWNER_ACCOUNT)? else {
        return Ok(None);
    };
    let record: WorkspaceOwnerRecord = serde_json::from_str(&raw)
        .map_err(|_| "작업공간 소유자 정보가 손상되었습니다.".to_owned())?;
    if record.version != 1
        || record.workspace_id.is_empty()
        || record.linked_identities.is_empty()
        || !record.linked_identities.contains(&record.primary)
    {
        return Err("작업공간 소유자 정보가 올바르지 않습니다.".to_owned());
    }
    Ok(Some(record))
}

fn save_record(record: &WorkspaceOwnerRecord) -> Result<(), String> {
    let serialized = serde_json::to_string(record)
        .map_err(|_| "작업공간 소유자 정보를 직렬화하지 못했습니다.".to_owned())?;
    credential(SERVICE, OWNER_ACCOUNT)?
        .set_password(&serialized)
        .map_err(|_| "작업공간 소유자 정보를 저장하지 못했습니다.".to_owned())
}

fn record_for_primary(identity: LinkedIdentity) -> WorkspaceOwnerRecord {
    WorkspaceOwnerRecord {
        version: 1,
        workspace_id: Uuid::new_v4().to_string(),
        primary: identity.clone(),
        linked_identities: vec![identity],
    }
}

fn migrate_legacy_record() -> Result<Option<WorkspaceOwnerRecord>, String> {
    // 기존 버전은 GitHub와 Google을 서로 독립적으로 소유자 처리했다. 두 값이 모두
    // 있으면 오래전부터 기본 게이트였던 GitHub만 승계해 자동 계정 병합을 막는다.
    let primary = if let Some(subject) =
        optional_credential(LEGACY_GITHUB_SERVICE, LEGACY_GITHUB_ACCOUNT)?
    {
        Some(validate_identity("github", &subject)?)
    } else if let Some(subject) = optional_credential(LEGACY_GOOGLE_SERVICE, LEGACY_GOOGLE_ACCOUNT)?
    {
        Some(validate_identity("google", &subject)?)
    } else {
        None
    };
    let Some(primary) = primary else {
        return Ok(None);
    };
    let record = record_for_primary(primary);
    save_record(&record)?;
    Ok(Some(record))
}

fn load_or_migrate_record() -> Result<Option<WorkspaceOwnerRecord>, String> {
    load_record()?.map_or_else(migrate_legacy_record, |record| Ok(Some(record)))
}

fn authorize_record(
    record: Option<WorkspaceOwnerRecord>,
    identity: LinkedIdentity,
) -> Result<(WorkspaceOwnerRecord, bool), String> {
    match record {
        Some(record) if record.linked_identities.contains(&identity) => Ok((record, false)),
        Some(_) => Err(format!(
            "이 {} 계정은 현재 Investa 작업공간에 연결되어 있지 않습니다. 먼저 소유자 계정으로 로그인한 뒤 설정 > 로그인 공급자에서 연결해 주세요.",
            provider_label(&identity.provider)
        )),
        None => Ok((record_for_primary(identity), true)),
    }
}

fn link_record(mut record: WorkspaceOwnerRecord, identity: LinkedIdentity) -> WorkspaceOwnerRecord {
    if !record.linked_identities.contains(&identity) {
        record.linked_identities.push(identity);
    }
    record
}

fn provider_label(provider: &str) -> &str {
    match provider {
        "github" => "GitHub",
        "google" => "Google",
        "apple" => "Apple",
        _ => "외부",
    }
}

impl WorkspaceIdentityBridge {
    pub fn authenticate(&self, provider: &str, subject: &str) -> Result<(), String> {
        let identity = validate_identity(provider, subject)?;
        let (record, created) = authorize_record(load_or_migrate_record()?, identity.clone())?;
        if created {
            save_record(&record)?;
        }
        *self
            .authenticated
            .lock()
            .map_err(|_| "로그인 세션 잠금을 열지 못했습니다.".to_owned())? =
            Some(AuthenticatedIdentity(identity));
        Ok(())
    }

    pub fn link(&self, provider: &str, subject: &str) -> Result<(), String> {
        let identity = validate_identity(provider, subject)?;
        let authenticated = self
            .authenticated
            .lock()
            .map_err(|_| "로그인 세션 잠금을 열지 못했습니다.".to_owned())?
            .clone()
            .ok_or_else(|| {
                "소유자 계정으로 로그인한 세션에서만 계정을 연결할 수 있습니다.".to_owned()
            })?;
        let record = load_or_migrate_record()?
            .ok_or_else(|| "먼저 소유자 계정으로 작업공간을 생성해 주세요.".to_owned())?;
        if !record.linked_identities.contains(&authenticated.0) {
            return Err("현재 로그인 세션은 이 작업공간의 소유자가 아닙니다.".to_owned());
        }
        save_record(&link_record(record, identity))
    }

    fn status(&self) -> Result<WorkspaceIdentityStatus, String> {
        let record = load_or_migrate_record()?;
        let session_authenticated = self
            .authenticated
            .lock()
            .map_err(|_| "로그인 세션 잠금을 열지 못했습니다.".to_owned())?
            .is_some();
        let Some(record) = record else {
            return Ok(WorkspaceIdentityStatus {
                initialized: false,
                session_authenticated,
                primary_provider: None,
                linked_providers: Vec::new(),
                linked_account_count: 0,
            });
        };
        let mut linked_providers = record
            .linked_identities
            .iter()
            .map(|identity| identity.provider.clone())
            .collect::<Vec<_>>();
        linked_providers.sort();
        linked_providers.dedup();
        Ok(WorkspaceIdentityStatus {
            initialized: true,
            session_authenticated,
            primary_provider: Some(record.primary.provider),
            linked_account_count: record.linked_identities.len(),
            linked_providers,
        })
    }
}

#[tauri::command]
pub fn workspace_identity_status(
    bridge: tauri::State<'_, WorkspaceIdentityBridge>,
) -> Result<WorkspaceIdentityStatus, String> {
    bridge.status()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(provider: &str, subject: &str) -> LinkedIdentity {
        validate_identity(provider, subject).unwrap()
    }

    #[test]
    fn first_verified_login_creates_the_only_owner_identity() {
        let google = identity("google", "google-subject");
        let (record, created) = authorize_record(None, google.clone()).unwrap();
        assert!(created);
        assert_eq!(record.primary, google);
        assert_eq!(record.linked_identities.len(), 1);
    }

    #[test]
    fn unrelated_provider_identity_cannot_open_an_existing_workspace() {
        let github = identity("github", "1001");
        let record = record_for_primary(github);
        let error = authorize_record(Some(record), identity("google", "other-google"))
            .expect_err("an unlinked account must be rejected");
        assert!(error.contains("연결되어 있지 않습니다"));
    }

    #[test]
    fn linked_provider_identity_can_open_the_same_workspace() {
        let github = identity("github", "1001");
        let google = identity("google", "google-subject");
        let record = link_record(record_for_primary(github), google.clone());
        let (_, created) = authorize_record(Some(record), google).unwrap();
        assert!(!created);
    }

    #[test]
    fn linking_is_idempotent_and_does_not_replace_the_primary_identity() {
        let github = identity("github", "1001");
        let google = identity("google", "google-subject");
        let record = link_record(record_for_primary(github.clone()), google.clone());
        let record = link_record(record, google);
        assert_eq!(record.primary, github);
        assert_eq!(record.linked_identities.len(), 2);
    }
}
