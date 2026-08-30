use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use keyring::{Entry, Error as KeyringError};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    time::{Duration, Instant},
};
use uuid::Uuid;

const SERVICE: &str = "Investa.SocialAuth";
const GOOGLE_CLIENT_ACCOUNT: &str = "google-desktop-client-id";
const GOOGLE_CLIENT_SECRET_ACCOUNT: &str = "google-desktop-client-secret";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const GOOGLE_CALLBACK_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const GOOGLE_CALLBACK_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialAuthStatus {
    google_configured: bool,
    google_secret_configured: bool,
    apple_configured: bool,
    google_message: String,
    apple_message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialUser {
    provider: &'static str,
    name: Option<String>,
    email: Option<String>,
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleClientRequest {
    client_id: String,
    client_secret: String,
}

#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct GoogleTokenError {
    error: Option<String>,
}

#[derive(Deserialize)]
struct GoogleUserInfo {
    sub: String,
    email: Option<String>,
    email_verified: Option<bool>,
    name: Option<String>,
    picture: Option<String>,
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account)
        .map_err(|_| "소셜 로그인 보안 저장소를 열지 못했습니다.".to_owned())
}

fn optional_value(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("소셜 로그인 설정을 읽지 못했습니다.".to_owned()),
    }
}

fn validate_google_client_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !(20..=256).contains(&value.len())
        || !value.ends_with(".apps.googleusercontent.com")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(
            "Google Cloud에서 발급한 데스크톱 앱 OAuth Client ID를 입력해 주세요.".to_owned(),
        );
    }
    Ok(value.to_owned())
}

fn validate_google_client_secret(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !(8..=512).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\'' | b'`'))
    {
        return Err("Google Cloud에서 발급한 OAuth Client Secret을 입력해 주세요.".to_owned());
    }
    Ok(value.to_owned())
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                result.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .map_err(|_| "OAuth 응답 인코딩이 올바르지 않습니다.".to_owned())?;
                result.push(
                    u8::from_str_radix(hex, 16)
                        .map_err(|_| "OAuth 응답 인코딩이 올바르지 않습니다.".to_owned())?,
                );
                index += 3;
            }
            byte if byte.is_ascii() => {
                result.push(byte);
                index += 1;
            }
            _ => return Err("OAuth 응답 인코딩이 올바르지 않습니다.".to_owned()),
        }
    }
    String::from_utf8(result).map_err(|_| "OAuth 응답 문자열이 올바르지 않습니다.".to_owned())
}

fn callback_values(stream: &mut TcpStream, expected_state: &str) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| "OAuth 콜백 제한 시간을 설정하지 못했습니다.".to_owned())?;
    let mut request_line = String::new();
    BufReader::new(&mut *stream)
        .read_line(&mut request_line)
        .map_err(|_| "OAuth 콜백을 읽지 못했습니다.".to_owned())?;
    if request_line.len() > 8_192 || !request_line.starts_with("GET /?") {
        return Err("OAuth 콜백 요청 형식이 올바르지 않습니다.".to_owned());
    }
    let target = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "OAuth 콜백 경로가 없습니다.".to_owned())?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in target.trim_start_matches("/?").split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or_default();
        let value = percent_decode(parts.next().unwrap_or_default())?;
        match key {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }
    let success = error.is_none() && state.as_deref() == Some(expected_state) && code.is_some();
    let body = if success {
        "Investa Google 로그인이 완료되었습니다. 이 창을 닫고 앱으로 돌아가세요."
    } else {
        "Investa Google 로그인을 완료하지 못했습니다. 앱으로 돌아가 다시 시도하세요."
    };
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.as_bytes().len());
    let _ = stream.write_all(response.as_bytes());
    if error.is_some() {
        return Err("Google 로그인이 취소되거나 거부되었습니다.".to_owned());
    }
    if state.as_deref() != Some(expected_state) {
        return Err("Google OAuth state가 일치하지 않습니다.".to_owned());
    }
    code.ok_or_else(|| "Google OAuth 인증 코드가 없습니다.".to_owned())
}

fn wait_for_google_callback(
    listener: TcpListener,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, String> {
    listener
        .set_nonblocking(true)
        .map_err(|_| "OAuth 콜백 대기 설정에 실패했습니다.".to_owned())?;
    let started_at = Instant::now();
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => return callback_values(&mut stream, expected_state),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started_at.elapsed() >= timeout {
                    return Err(
                        "Google 로그인 대기 시간이 만료되었습니다. 브라우저의 이전 127.0.0.1 페이지를 새로고침하지 말고, Investa에서 Google 로그인 버튼을 다시 눌러 주세요."
                            .to_owned(),
                    );
                }
                std::thread::sleep(GOOGLE_CALLBACK_POLL_INTERVAL);
            }
            Err(_) => return Err("OAuth 콜백을 받지 못했습니다.".to_owned()),
        }
    }
}

fn google_token_error_message(error_code: Option<&str>) -> String {
    match error_code {
        Some("invalid_client") => "Google OAuth 클라이언트 확인에 실패했습니다. Google Cloud에서 '데스크톱 앱' 유형으로 발급한 Client ID인지 확인해 주세요.",
        Some("invalid_grant") => "Google 인증 코드가 만료되었거나 이미 사용되었습니다. 이전 Google 탭을 닫고 Investa에서 Google 로그인을 다시 시작해 주세요.",
        Some("redirect_uri_mismatch") => "Google OAuth 반환 주소가 일치하지 않습니다. '데스크톱 앱' 유형 Client ID를 사용해야 합니다.",
        Some("access_denied") => "Google 계정이 접근을 허용하지 않았습니다. OAuth 테스트 사용자와 동의 화면 상태를 확인해 주세요.",
        _ => "Google OAuth 토큰 교환이 거부되었습니다. Google Cloud의 OAuth 앱 상태와 Client ID 유형을 확인해 주세요.",
    }
    .to_owned()
}

fn browser_command(url: &str) -> Command {
    // explorer.exe can hand the request to an existing shell process and exit
    // without forwarding the URL. FileProtocolHandler invokes the registered
    // HTTPS handler directly while keeping the filtered child environment.
    let mut command = Command::new("rundll32.exe");
    command.env_clear();
    for name in [
        "SYSTEMROOT",
        "WINDIR",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "PATH",
        "PATHEXT",
        "TEMP",
        "TMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.args(["url.dll,FileProtocolHandler", url]);
    command
}

#[tauri::command]
pub fn social_auth_status() -> Result<SocialAuthStatus, String> {
    let google_configured = optional_value(GOOGLE_CLIENT_ACCOUNT)?.is_some();
    let google_secret_configured = optional_value(GOOGLE_CLIENT_SECRET_ACCOUNT)?.is_some();
    Ok(SocialAuthStatus {
        google_configured,
        google_secret_configured,
        apple_configured: false,
        google_message: if google_configured && google_secret_configured {
            "Google 데스크톱 OAuth Client ID·Secret과 PKCE 준비됨"
        } else if google_configured {
            "Client ID는 저장됐지만 이 Google 클라이언트의 Client Secret이 필요합니다."
        } else {
            "설정에서 데스크톱 OAuth Client ID와 Client Secret이 필요합니다."
        }
        .to_owned(),
        apple_message: "Apple Developer Services ID와 HTTPS callback 준비 후 활성화됩니다."
            .to_owned(),
    })
}

#[tauri::command]
pub fn social_auth_save_google_client(
    request: GoogleClientRequest,
) -> Result<SocialAuthStatus, String> {
    let client_id = validate_google_client_id(&request.client_id)?;
    let client_secret = validate_google_client_secret(&request.client_secret)?;
    entry(GOOGLE_CLIENT_ACCOUNT)?
        .set_password(&client_id)
        .map_err(|_| "Google OAuth Client ID를 저장하지 못했습니다.".to_owned())?;
    entry(GOOGLE_CLIENT_SECRET_ACCOUNT)?
        .set_password(&client_secret)
        .map_err(|_| "Google OAuth Client Secret을 저장하지 못했습니다.".to_owned())?;
    social_auth_status()
}

#[tauri::command]
pub fn social_auth_delete_google_client() -> Result<SocialAuthStatus, String> {
    match entry(GOOGLE_CLIENT_ACCOUNT)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => {}
        Err(_) => return Err("Google OAuth 설정을 삭제하지 못했습니다.".to_owned()),
    }
    match entry(GOOGLE_CLIENT_SECRET_ACCOUNT)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => {}
        Err(_) => return Err("Google OAuth Client Secret을 삭제하지 못했습니다.".to_owned()),
    }
    social_auth_status()
}

async fn google_oauth_user() -> Result<GoogleUserInfo, String> {
    let client_id = optional_value(GOOGLE_CLIENT_ACCOUNT)?.ok_or_else(|| {
        "설정에서 Google 데스크톱 OAuth Client ID를 먼저 저장해 주세요.".to_owned()
    })?;
    let client_secret = optional_value(GOOGLE_CLIENT_SECRET_ACCOUNT)?.ok_or_else(|| {
        "설정에서 Google 데스크톱 OAuth Client Secret을 함께 저장해 주세요.".to_owned()
    })?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| "Google 로그인용 로컬 콜백 포트를 열지 못했습니다.".to_owned())?;
    let port = listener
        .local_addr()
        .map_err(|_| "Google 로그인용 로컬 포트를 확인하지 못했습니다.".to_owned())?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = Uuid::new_v4().simple().to_string();
    let url = format!("{GOOGLE_AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=online&prompt=select_account",
        percent_encode(&client_id), percent_encode(&redirect_uri), percent_encode("openid email profile"), percent_encode(&challenge), percent_encode(&state));
    browser_command(&url)
        .spawn()
        .map_err(|_| "Google 로그인 브라우저를 열지 못했습니다.".to_owned())?;
    let expected_state = state.clone();
    let code = tauri::async_runtime::spawn_blocking(move || {
        wait_for_google_callback(listener, &expected_state, GOOGLE_CALLBACK_TIMEOUT)
    })
    .await
    .map_err(|_| "Google 로그인 콜백 작업이 중단되었습니다.".to_owned())??;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Google OAuth 클라이언트를 만들지 못했습니다.".to_owned())?;
    let token = client
        .post(GOOGLE_TOKEN_URL)
        .form(&[
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ])
        .send()
        .await
        .map_err(|_| "Google 인증 코드를 교환하지 못했습니다.".to_owned())?;
    if !token.status().is_success() {
        let error = token.json::<GoogleTokenError>().await.ok();
        return Err(google_token_error_message(
            error.as_ref().and_then(|value| value.error.as_deref()),
        ));
    }
    let token = token
        .json::<GoogleTokenResponse>()
        .await
        .map_err(|_| "Google OAuth 응답 형식이 올바르지 않습니다.".to_owned())?;
    let response = client
        .get(GOOGLE_USERINFO_URL)
        .bearer_auth(&token.access_token)
        .send()
        .await
        .map_err(|_| "Google 사용자 정보를 확인하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err("Google 사용자 확인이 거부되었습니다.".to_owned());
    }
    let user = response
        .json::<GoogleUserInfo>()
        .await
        .map_err(|_| "Google 사용자 응답 형식이 올바르지 않습니다.".to_owned())?;
    if user.email.is_some() && user.email_verified != Some(true) {
        return Err("검증되지 않은 Google 이메일은 사용할 수 없습니다.".to_owned());
    }
    Ok(user)
}

fn social_user(user: GoogleUserInfo) -> SocialUser {
    SocialUser {
        provider: "google",
        name: user.name,
        email: user.email,
        avatar_url: user.picture,
    }
}

#[tauri::command]
pub async fn google_login(
    identity: tauri::State<'_, crate::workspace_identity::WorkspaceIdentityBridge>,
) -> Result<SocialUser, String> {
    let user = google_oauth_user().await?;
    identity.authenticate("google", &user.sub)?;
    Ok(social_user(user))
}

#[tauri::command]
pub async fn google_link_account(
    identity: tauri::State<'_, crate::workspace_identity::WorkspaceIdentityBridge>,
) -> Result<SocialUser, String> {
    let user = google_oauth_user().await?;
    identity.link("google", &user.sub)?;
    Ok(social_user(user))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpStream, thread};
    #[test]
    fn validates_only_google_desktop_client_ids() {
        assert!(validate_google_client_id("1234567890-example.apps.googleusercontent.com").is_ok());
        assert!(validate_google_client_id("https://example.com").is_err());
    }
    #[test]
    fn validates_google_client_secrets_without_echoing_them() {
        assert!(validate_google_client_secret("GOCSPX-example_secret-123").is_ok());
        let error = validate_google_client_secret("bad secret").unwrap_err();
        assert!(!error.contains("bad secret"));
    }
    #[test]
    fn percent_encoding_round_trips_callback_values() {
        let value = "code/value + state";
        assert_eq!(percent_decode(&percent_encode(value)).unwrap(), value);
    }

    #[test]
    fn browser_command_uses_the_registered_https_handler() {
        let command = browser_command("https://accounts.google.com/example");
        assert_eq!(command.get_program(), "rundll32.exe");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                "url.dll,FileProtocolHandler",
                "https://accounts.google.com/example"
            ]
        );
    }

    #[test]
    fn loopback_callback_accepts_the_matching_state() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            stream
                .write_all(b"GET /?code=auth-code&state=expected-state HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
                .unwrap();
        });

        let code = wait_for_google_callback(listener, "expected-state", Duration::from_secs(1));
        client.join().unwrap();
        assert_eq!(code.unwrap(), "auth-code");
    }

    #[test]
    fn expired_loopback_callback_explains_how_to_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let error = wait_for_google_callback(listener, "expected-state", Duration::ZERO)
            .expect_err("an unused callback must time out");

        assert!(error.contains("Investa에서 Google 로그인 버튼을 다시 눌러"));
    }

    #[test]
    fn google_token_errors_are_mapped_without_provider_payloads() {
        assert!(google_token_error_message(Some("invalid_client")).contains("데스크톱 앱"));
        assert!(google_token_error_message(Some("invalid_grant")).contains("이미 사용"));
        assert!(!google_token_error_message(Some("unknown-provider-detail"))
            .contains("unknown-provider-detail"));
    }
}
