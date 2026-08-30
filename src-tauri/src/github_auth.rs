use serde::Serialize;
use std::{env, path::PathBuf, process::Command};

const GH_ENV_ALLOWLIST: &[&str] = &[
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "LOCALAPPDATA",
    "APPDATA",
    "PROGRAMDATA",
    "TEMP",
    "TMP",
    "PATH",
    "PATHEXT",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

fn gh_executable() -> Result<PathBuf, String> {
    for root in [env::var_os("ProgramFiles"), env::var_os("ProgramW6432")]
        .into_iter()
        .flatten()
    {
        let candidate = PathBuf::from(root).join("GitHub CLI").join("gh.exe");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let output = Command::new("where.exe")
        .arg("gh.exe")
        .output()
        .map_err(|_| "GitHub CLI를 찾지 못했습니다. 먼저 GitHub CLI를 설치해 주세요.".to_owned())?;
    let candidate = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            "GitHub CLI를 찾지 못했습니다. 먼저 GitHub CLI를 설치해 주세요.".to_owned()
        })?;
    candidate
        .canonicalize()
        .map_err(|_| "GitHub CLI 실행 경로를 확인하지 못했습니다.".to_owned())
}

fn gh_command(executable: &PathBuf) -> Command {
    let mut command = Command::new(executable);
    let allowed = GH_ENV_ALLOWLIST
        .iter()
        .filter_map(|name| env::var_os(name).map(|value| (*name, value)))
        .collect::<Vec<_>>();
    command.env_clear();
    for (name, value) in allowed {
        command.env(name, value);
    }
    command
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubUser {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: String,
}

fn current_github_user() -> Result<GithubUser, String> {
    let executable = gh_executable()?;
    let auth = gh_command(&executable)
        .args(["auth", "status", "--hostname", "github.com"])
        .output()
        .map_err(|_| "GitHub CLI를 찾지 못했습니다. 먼저 GitHub CLI를 설치해 주세요.".to_owned())?;
    if !auth.status.success() {
        return Err("GitHub CLI 로그인이 필요합니다.".to_owned());
    }

    let output = gh_command(&executable)
        .args(["api", "user"])
        .output()
        .map_err(|_| "GitHub 사용자 정보를 확인하지 못했습니다.".to_owned())?;
    if !output.status.success() {
        return Err(
            "GitHub 사용자 정보를 확인하지 못했습니다. 네트워크와 CLI 세션을 확인해 주세요."
                .to_owned(),
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "GitHub 사용자 응답 형식이 올바르지 않습니다.".to_owned())?;
    let id = value["id"]
        .as_u64()
        .ok_or_else(|| "GitHub 사용자 ID를 확인하지 못했습니다.".to_owned())?;
    let login = value["login"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "GitHub 로그인 이름을 확인하지 못했습니다.".to_owned())?
        .to_owned();
    Ok(GithubUser {
        id,
        login,
        name: value["name"].as_str().map(str::to_owned),
        avatar_url: value["avatar_url"].as_str().unwrap_or_default().to_owned(),
    })
}

#[tauri::command]
pub async fn github_session(
    identity: tauri::State<'_, crate::workspace_identity::WorkspaceIdentityBridge>,
) -> Result<GithubUser, String> {
    let user = current_github_user()?;
    identity.authenticate("github", &user.id.to_string())?;
    Ok(user)
}

#[tauri::command]
pub async fn github_link_current_session(
    identity: tauri::State<'_, crate::workspace_identity::WorkspaceIdentityBridge>,
) -> Result<GithubUser, String> {
    let user = current_github_user()?;
    identity.link("github", &user.id.to_string())?;
    Ok(user)
}

#[tauri::command]
pub async fn github_login_start() -> Result<(), String> {
    let executable = gh_executable()?;
    let mut command = gh_command(&executable);
    command.args([
        "auth",
        "login",
        "--hostname",
        "github.com",
        "--web",
        "--git-protocol",
        "https",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0000_0010);
    }
    command
        .spawn()
        .map_err(|_| "GitHub CLI 로그인 창을 열지 못했습니다.".to_owned())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GithubUser, GH_ENV_ALLOWLIST};

    #[test]
    fn github_environment_allowlist_excludes_tokens_and_financial_secrets() {
        for name in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "TELEGRAM_BOT_TOKEN",
            "BINANCE_SECRET_KEY",
        ] {
            assert!(!GH_ENV_ALLOWLIST.contains(&name));
        }
    }

    #[test]
    fn github_user_serialization_never_contains_a_token() {
        let user = GithubUser {
            id: 1,
            login: "tester".to_owned(),
            name: None,
            avatar_url: "https://avatars.githubusercontent.com/u/1".to_owned(),
        };
        let serialized = serde_json::to_string(&user).unwrap();
        assert!(!serialized.contains("token"));
        assert!(serialized.contains("tester"));
    }
}
