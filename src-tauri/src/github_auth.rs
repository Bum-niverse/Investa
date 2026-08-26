use serde::Serialize;
use std::process::Command;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubUser {
    id: u64,
    login: String,
    name: Option<String>,
    avatar_url: String,
}

#[tauri::command]
pub async fn github_session() -> Result<GithubUser, String> {
    let auth = Command::new("gh")
        .args(["auth", "status", "--hostname", "github.com"])
        .output()
        .map_err(|_| "GitHub CLI를 찾지 못했습니다. 먼저 GitHub CLI를 설치해 주세요.".to_owned())?;
    if !auth.status.success() {
        return Err("GitHub CLI 로그인이 필요합니다.".to_owned());
    }

    let output = Command::new("gh")
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
pub async fn github_login_start() -> Result<(), String> {
    let script = "Start-Process -FilePath 'gh' -ArgumentList @('auth','login','--hostname','github.com','--web','--git-protocol','https')";
    Command::new("powershell")
        .args(["-NoProfile", "-Command", script])
        .spawn()
        .map_err(|_| "GitHub CLI 로그인 창을 열지 못했습니다.".to_owned())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::GithubUser;

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
