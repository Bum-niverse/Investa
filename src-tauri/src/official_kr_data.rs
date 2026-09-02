use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const KEYRING_SERVICE: &str = "Investa.OfficialKrData";
const OPENDART_ENDPOINT: &str = "https://opendart.fss.or.kr/api/list.json";
const NAVER_NEWS_ENDPOINT: &str = "https://openapi.naver.com/v1/search/news.json";
const REQUEST_TIMEOUT_SECONDS: u64 = 10;
const MAX_RESPONSE_BYTES: usize = 1_048_576;

pub struct OfficialKrDataBridge {
    client: Client,
}

impl Default for OfficialKrDataBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl OfficialKrDataBridge {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
                .build()
                .expect("official Korean data HTTP client"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialKrDataConfigRequest {
    opendart_api_key: Option<String>,
    naver_client_id: Option<String>,
    naver_client_secret: Option<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialKrDataStatus {
    opendart_configured: bool,
    naver_news_configured: bool,
    read_only: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDartDisclosureRequest {
    corp_code: String,
    begin_date: String,
    end_date: String,
    page_count: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenDartDisclosure {
    receipt_number: String,
    corporation_name: String,
    report_name: String,
    filer_name: String,
    receipt_date: String,
    remark: String,
    source_url: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenDartDisclosureSnapshot {
    provider: &'static str,
    corp_code: String,
    begin_date: String,
    end_date: String,
    fetched_at_ms: u64,
    items: Vec<OpenDartDisclosure>,
    read_only: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NaverNewsRequest {
    query: String,
    display: Option<u16>,
    start: Option<u16>,
    sort: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NaverNewsItem {
    title: String,
    original_link: String,
    link: String,
    description: String,
    published_at: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NaverNewsSnapshot {
    provider: &'static str,
    query: String,
    fetched_at_ms: u64,
    total: u64,
    start: u64,
    display: u64,
    items: Vec<NaverNewsItem>,
    read_only: bool,
}

#[derive(Debug, Deserialize)]
struct OpenDartEnvelope {
    status: String,
    message: String,
    #[serde(default)]
    list: Vec<OpenDartRawItem>,
}

#[derive(Debug, Deserialize)]
struct OpenDartRawItem {
    rcept_no: String,
    corp_name: String,
    report_nm: String,
    flr_nm: String,
    rcept_dt: String,
    #[serde(default)]
    rm: String,
}

#[derive(Debug, Deserialize)]
struct NaverNewsEnvelope {
    total: u64,
    start: u64,
    display: u64,
    #[serde(default)]
    items: Vec<NaverNewsRawItem>,
}

#[derive(Debug, Deserialize)]
struct NaverNewsRawItem {
    title: String,
    #[serde(default)]
    originallink: String,
    link: String,
    description: String,
    #[serde(rename = "pubDate")]
    pub_date: String,
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|_| "공식 국내 데이터 보안 저장소를 열지 못했습니다.".to_owned())
}

fn optional_secret(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("공식 국내 데이터 자격정보를 읽지 못했습니다.".to_owned()),
    }
}

fn validate_secret(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if !(8..=512).contains(&value.len())
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
        || value.contains(['\"', '\'', '`'])
    {
        return Err(format!("{label} 형식을 확인해 주세요."));
    }
    Ok(value.to_owned())
}

fn update_secret(account: &str, value: Option<String>, label: &str) -> Result<(), String> {
    let Some(value) = value else {
        return Ok(());
    };
    let credential = entry(account)?;
    match Some(value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => credential
            .set_password(&validate_secret(value, label)?)
            .map_err(|_| format!("{label}을 저장하지 못했습니다.")),
        None => match credential.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(format!("{label}을 삭제하지 못했습니다.")),
        },
    }
}

fn normalize_secret_change(
    value: Option<String>,
    label: &str,
) -> Result<Option<Option<String>>, String> {
    value
        .map(|value| {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                validate_secret(value, label).map(Some)
            }
        })
        .transpose()
}

fn replace_secret(account: &str, value: Option<&str>, label: &str) -> Result<(), String> {
    let credential = entry(account)?;
    match value {
        Some(value) => credential
            .set_password(value)
            .map_err(|_| format!("{label}을 저장하지 못했습니다.")),
        None => match credential.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err(format!("{label}을 삭제하지 못했습니다.")),
        },
    }
}

fn configuration_status() -> Result<OfficialKrDataStatus, String> {
    let opendart_configured = optional_secret("opendart-api-key")?.is_some();
    let naver_client_id = optional_secret("naver-client-id")?.is_some();
    let naver_client_secret = optional_secret("naver-client-secret")?.is_some();
    if naver_client_id != naver_client_secret {
        return Err("네이버 뉴스 자격정보가 불완전합니다. 삭제 후 다시 등록해 주세요.".to_owned());
    }
    Ok(OfficialKrDataStatus {
        opendart_configured,
        naver_news_configured: naver_client_id && naver_client_secret,
        read_only: true,
        message:
            "공시·뉴스는 읽기 전용이며 자격정보 원문을 SQLite·로그·분석 요청에 저장하지 않습니다."
                .to_owned(),
    })
}

fn valid_date(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn validate_disclosure_request(
    request: OpenDartDisclosureRequest,
) -> Result<OpenDartDisclosureRequest, String> {
    if request.corp_code.len() != 8
        || !request.corp_code.bytes().all(|byte| byte.is_ascii_digit())
        || !valid_date(&request.begin_date)
        || !valid_date(&request.end_date)
        || request.begin_date > request.end_date
        || !(1..=100).contains(&request.page_count.unwrap_or(50))
    {
        return Err("OpenDART 회사 고유번호·조회 기간·건수를 확인해 주세요.".to_owned());
    }
    Ok(request)
}

fn clean_html(value: &str) -> String {
    value
        .replace("<b>", "")
        .replace("</b>", "")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn bounded_text(value: String, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn safe_http_url(value: String) -> String {
    if value.len() > 2_048 {
        return String::new();
    }
    Url::parse(&value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.to_string())
        .unwrap_or_default()
}

async fn bounded_json<T: DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> Result<T, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(format!("{provider} 응답 크기가 허용 범위를 초과했습니다."));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| format!("{provider} 응답을 읽지 못했습니다."))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(format!("{provider} 응답 크기가 허용 범위를 초과했습니다."));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("{provider} 응답 형식을 확인하지 못했습니다."))
}

fn validate_news_request(request: NaverNewsRequest) -> Result<NaverNewsRequest, String> {
    let query = request.query.trim();
    if query.is_empty()
        || query.chars().count() > 100
        || query.chars().any(char::is_control)
        || !(1..=100).contains(&request.display.unwrap_or(20))
        || !(1..=1_000).contains(&request.start.unwrap_or(1))
        || !matches!(request.sort.as_deref().unwrap_or("date"), "date" | "sim")
    {
        return Err("네이버 뉴스 검색어·페이지·정렬 조건을 확인해 주세요.".to_owned());
    }
    Ok(NaverNewsRequest {
        query: query.to_owned(),
        ..request
    })
}

fn safe_http_error(provider: &str, status: StatusCode) -> String {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            format!("{provider} 자격정보 또는 애플리케이션 권한을 확인해 주세요.")
        }
        StatusCode::TOO_MANY_REQUESTS => format!("{provider} 호출 한도를 초과했습니다."),
        StatusCode::BAD_REQUEST => format!("{provider} 요청 조건이 거부되었습니다."),
        _ => format!("{provider} 서버가 요청을 처리하지 못했습니다."),
    }
}

fn parse_opendart(value: OpenDartEnvelope) -> Result<Vec<OpenDartDisclosure>, String> {
    if value.status == "013" {
        return Ok(Vec::new());
    }
    if value.status != "000" {
        let _ = value.message;
        return Err(
            "OpenDART가 조회를 거부했습니다. 회사 코드·기간·호출 한도를 확인해 주세요.".to_owned(),
        );
    }
    Ok(value
        .list
        .into_iter()
        .filter(|item| {
            item.rcept_no.len() == 14 && item.rcept_no.bytes().all(|b| b.is_ascii_digit())
        })
        .map(|item| OpenDartDisclosure {
            source_url: format!(
                "https://dart.fss.or.kr/dsaf001/main.do?rcpNo={}",
                item.rcept_no
            ),
            receipt_number: item.rcept_no,
            corporation_name: bounded_text(item.corp_name, 200),
            report_name: bounded_text(item.report_nm, 500),
            filer_name: bounded_text(item.flr_nm, 200),
            receipt_date: bounded_text(item.rcept_dt, 8),
            remark: bounded_text(item.rm, 200),
        })
        .collect())
}

fn parse_naver(value: NaverNewsEnvelope) -> NaverNewsSnapshot {
    NaverNewsSnapshot {
        provider: "NAVER_NEWS_SEARCH",
        query: String::new(),
        fetched_at_ms: 0,
        total: value.total,
        start: value.start,
        display: value.display,
        items: value
            .items
            .into_iter()
            .map(|item| NaverNewsItem {
                title: bounded_text(clean_html(&item.title), 500),
                original_link: safe_http_url(item.originallink),
                link: safe_http_url(item.link),
                description: bounded_text(clean_html(&item.description), 2_000),
                published_at: bounded_text(item.pub_date, 100),
            })
            .collect(),
        read_only: true,
    }
}

#[tauri::command]
pub fn official_kr_data_status() -> Result<OfficialKrDataStatus, String> {
    configuration_status()
}

#[tauri::command]
pub fn official_kr_data_save_config(
    request: OfficialKrDataConfigRequest,
) -> Result<OfficialKrDataStatus, String> {
    if request.naver_client_id.is_some() != request.naver_client_secret.is_some() {
        return Err("네이버 Client ID와 Secret은 함께 저장하거나 함께 삭제해야 합니다.".to_owned());
    }
    let naver_client_id = normalize_secret_change(request.naver_client_id, "네이버 Client ID")?;
    let naver_client_secret =
        normalize_secret_change(request.naver_client_secret, "네이버 Client Secret")?;
    update_secret(
        "opendart-api-key",
        request.opendart_api_key,
        "OpenDART API 키",
    )?;
    if let (Some(client_id), Some(client_secret)) = (naver_client_id, naver_client_secret) {
        let previous_id = optional_secret("naver-client-id")?;
        let previous_secret = optional_secret("naver-client-secret")?;
        replace_secret("naver-client-id", client_id.as_deref(), "네이버 Client ID")?;
        if let Err(error) = replace_secret(
            "naver-client-secret",
            client_secret.as_deref(),
            "네이버 Client Secret",
        ) {
            let rollback_id = replace_secret(
                "naver-client-id",
                previous_id.as_deref(),
                "네이버 Client ID",
            );
            let rollback_secret = replace_secret(
                "naver-client-secret",
                previous_secret.as_deref(),
                "네이버 Client Secret",
            );
            if rollback_id.is_err() || rollback_secret.is_err() {
                return Err(
                    "네이버 자격정보 저장과 복구에 실패했습니다. 두 값을 삭제한 뒤 다시 등록해 주세요."
                        .to_owned(),
                );
            }
            return Err(error);
        }
    }
    configuration_status()
}

#[tauri::command]
pub async fn opendart_disclosures(
    bridge: tauri::State<'_, OfficialKrDataBridge>,
    request: OpenDartDisclosureRequest,
) -> Result<OpenDartDisclosureSnapshot, String> {
    let request = validate_disclosure_request(request)?;
    let api_key = optional_secret("opendart-api-key")?
        .ok_or_else(|| "OpenDART API 키를 먼저 설정해 주세요.".to_owned())?;
    let page_count = request.page_count.unwrap_or(50);
    let response = bridge
        .client
        .get(OPENDART_ENDPOINT)
        .query(&[
            ("crtfc_key", api_key.as_str()),
            ("corp_code", request.corp_code.as_str()),
            ("bgn_de", request.begin_date.as_str()),
            ("end_de", request.end_date.as_str()),
            ("page_count", &page_count.to_string()),
        ])
        .send()
        .await
        .map_err(|_| "OpenDART에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(safe_http_error("OpenDART", response.status()));
    }
    let items = parse_opendart(bounded_json(response, "OpenDART").await?)?;
    Ok(OpenDartDisclosureSnapshot {
        provider: "OPENDART_DISCLOSURES",
        corp_code: request.corp_code,
        begin_date: request.begin_date,
        end_date: request.end_date,
        fetched_at_ms: crate::persistence::now_ms()?,
        items,
        read_only: true,
    })
}

#[tauri::command]
pub async fn naver_news_search(
    bridge: tauri::State<'_, OfficialKrDataBridge>,
    request: NaverNewsRequest,
) -> Result<NaverNewsSnapshot, String> {
    let request = validate_news_request(request)?;
    let client_id = optional_secret("naver-client-id")?
        .ok_or_else(|| "네이버 뉴스 Client ID를 먼저 설정해 주세요.".to_owned())?;
    let client_secret = optional_secret("naver-client-secret")?
        .ok_or_else(|| "네이버 뉴스 Client Secret을 먼저 설정해 주세요.".to_owned())?;
    let display = request.display.unwrap_or(20).to_string();
    let start = request.start.unwrap_or(1).to_string();
    let sort = request.sort.as_deref().unwrap_or("date");
    let response = bridge
        .client
        .get(NAVER_NEWS_ENDPOINT)
        .header("X-Naver-Client-Id", client_id)
        .header("X-Naver-Client-Secret", client_secret)
        .query(&[
            ("query", request.query.as_str()),
            ("display", display.as_str()),
            ("start", start.as_str()),
            ("sort", sort),
        ])
        .send()
        .await
        .map_err(|_| "네이버 뉴스 검색 API에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(safe_http_error("네이버 뉴스", response.status()));
    }
    let mut snapshot = parse_naver(bounded_json(response, "네이버 뉴스").await?);
    snapshot.query = request.query;
    snapshot.fetched_at_ms = crate::persistence::now_ms()?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_bounded_read_only_requests() {
        assert!(validate_disclosure_request(OpenDartDisclosureRequest {
            corp_code: "00126380".to_owned(),
            begin_date: "20260101".to_owned(),
            end_date: "20261231".to_owned(),
            page_count: Some(100),
        })
        .is_ok());
        assert!(validate_news_request(NaverNewsRequest {
            query: "한화".to_owned(),
            display: Some(101),
            start: None,
            sort: None,
        })
        .is_err());
    }

    #[test]
    fn parses_disclosures_without_exposing_the_api_key() {
        let envelope: OpenDartEnvelope = serde_json::from_value(json!({
            "status":"000","message":"정상","list":[{
                "rcept_no":"20260319001234","corp_name":"예시","report_nm":"사업보고서",
                "flr_nm":"예시","rcept_dt":"20260319","rm":""
            }]
        }))
        .unwrap();
        let items = parse_opendart(envelope).unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].source_url.ends_with("20260319001234"));
    }

    #[test]
    fn naver_markup_is_removed_from_display_fields() {
        let envelope: NaverNewsEnvelope = serde_json::from_value(json!({
            "total":1,"start":1,"display":1,"items":[{
                "title":"<b>한화</b> 뉴스","originallink":"https://publisher.example/a",
                "link":"https://news.naver.com/a","description":"&quot;요약&quot;",
                "pubDate":"Mon, 01 Sep 2026 00:00:00 +0900"
            }]
        }))
        .unwrap();
        let snapshot = parse_naver(envelope);
        assert_eq!(snapshot.items[0].title, "한화 뉴스");
        assert_eq!(snapshot.items[0].description, "\"요약\"");
    }

    #[test]
    fn rejects_non_http_news_links_and_caps_external_text() {
        let envelope: NaverNewsEnvelope = serde_json::from_value(json!({
            "total":1,"start":1,"display":1,"items":[{
                "title":"가".repeat(600),"originallink":"javascript:alert(1)",
                "link":"https://news.naver.com/a","description":"요약",
                "pubDate":"Mon, 01 Sep 2026 00:00:00 +0900"
            }]
        }))
        .unwrap();
        let snapshot = parse_naver(envelope);
        assert!(snapshot.items[0].original_link.is_empty());
        assert_eq!(snapshot.items[0].title.chars().count(), 500);
    }
}
