use keyring::{Entry, Error as KeyringError};
use reqwest::Client;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::State;

use crate::{
    market_data::{
        public_market_analysis_snapshot, AnalysisSnapshot, AnalysisSnapshotRequest, TossChartBar,
    },
    persistence::PersistenceBridge,
};

const CREDENTIAL_SERVICE: &str = "com.bumniverse.investa.kis-paper";
const CONFIG_ACCOUNT: &str = "paper-config-v1";
const PAPER_API_BASE: &str = "https://openapivts.koreainvestment.com:29443";

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperConfigRequest {
    app_key: String,
    app_secret: String,
    hts_id: String,
    account_number: String,
    product_code: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperConfigStatus {
    configured: bool,
    connected: bool,
    masked_account_number: Option<String>,
    product_code: Option<String>,
    live_order_enabled: bool,
    paper_order_enabled: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisFuturesConnectionStatus {
    configured: bool,
    connected: bool,
    provider: &'static str,
    market_data_ready: bool,
    message: String,
}

#[derive(Debug, Deserialize)]
struct HashResponse {
    #[serde(rename = "HASH")]
    hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperOrderRequest {
    request_id: String,
    symbol: String,
    side: String,
    quantity: u64,
    order_type: String,
    price: u64,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperCancelRequest {
    request_id: String,
    remote_order_id: String,
    order_branch: String,
    quantity: u64,
    confirmation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperOrdersRequest {
    trade_date: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperRemoteResult {
    request_id: String,
    status: String,
    remote_order_id: Option<String>,
    order_branch: Option<String>,
    order_time: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperReconciliation {
    observed_at_ms: u64,
    matched: bool,
    discrepancies: Vec<String>,
    remote_position_count: usize,
    internal_position_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperPosition {
    symbol: String,
    name: String,
    quantity: String,
    average_price: String,
    current_price: String,
    evaluation_amount: String,
    unrealized_pnl: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KisPaperAccountSnapshot {
    provider: &'static str,
    masked_account_number: String,
    product_code: String,
    deposit: Option<String>,
    total_evaluation_amount: Option<String>,
    positions: Vec<KisPaperPosition>,
    observed_at_ms: u64,
    live_order_enabled: bool,
}

fn entry() -> Result<Entry, String> {
    Entry::new(CREDENTIAL_SERVICE, CONFIG_ACCOUNT)
        .map_err(|_| "Windows 자격 증명 관리자를 열 수 없습니다.".to_owned())
}

fn load() -> Result<Option<KisPaperConfigRequest>, String> {
    match entry()?.get_password() {
        Ok(json) => {
            let config: KisPaperConfigRequest = serde_json::from_str(&json)
                .map_err(|_| "저장된 KIS 모의계좌 연결 정보를 해석하지 못했습니다.".to_owned())?;
            validate(&config).map_err(|_| {
                "저장된 KIS 모의계좌 연결 정보의 형식이 손상됐습니다. 삭제 후 다시 등록해 주세요."
                    .to_owned()
            })?;
            Ok(Some(config))
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("Windows 자격 증명 관리자에서 KIS 연결 정보를 읽지 못했습니다.".to_owned()),
    }
}

fn valid_plain_secret(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len())
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn validate(request: &KisPaperConfigRequest) -> Result<(), String> {
    if !valid_plain_secret(&request.app_key, 8, 256)
        || !valid_plain_secret(&request.app_secret, 8, 512)
        || !valid_plain_secret(&request.hts_id, 2, 64)
    {
        return Err("KIS 모의 App Key·Secret과 HTS ID 형식을 확인해 주세요.".to_owned());
    }
    if request.account_number.len() != 8
        || !request
            .account_number
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        || request.product_code.len() != 2
        || !request
            .product_code
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err("KIS 모의계좌 앞 8자리와 상품코드 2자리를 숫자로 입력해 주세요.".to_owned());
    }
    Ok(())
}

fn status(config: Option<&KisPaperConfigRequest>) -> KisPaperConfigStatus {
    match config {
        Some(config) => KisPaperConfigStatus {
            configured: true,
            connected: false,
            masked_account_number: Some(format!("{}****", &config.account_number[..4])),
            product_code: Some(config.product_code.clone()),
            live_order_enabled: false,
            paper_order_enabled: true,
            message: "KIS 모의 자격정보를 이 PC에 저장했습니다. 잔고 조회와 사용자 확인형 국내 모의주문을 사용할 수 있습니다.".to_owned(),
        },
        None => KisPaperConfigStatus {
            configured: false,
            connected: false,
            masked_account_number: None,
            product_code: None,
            live_order_enabled: false,
            paper_order_enabled: false,
            message: "KIS Developers의 모의 App Key·Secret, HTS ID와 모의계좌가 필요합니다.".to_owned(),
        },
    }
}

async fn issue_paper_token(
    client: &Client,
    config: &KisPaperConfigRequest,
) -> Result<String, String> {
    let response = client.post(format!("{PAPER_API_BASE}/oauth2/tokenP"))
        .timeout(Duration::from_secs(10))
        .json(&json!({"grant_type": "client_credentials", "appkey": config.app_key, "appsecret": config.app_secret}))
        .send().await.map_err(|_| "KIS 모의 인증 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(
            "KIS 모의 인증에 실패했습니다. 모의 App Key·Secret과 이용 신청 상태를 확인해 주세요."
                .to_owned(),
        );
    }
    let token = response
        .json::<TokenResponse>()
        .await
        .map_err(|_| "KIS 모의 인증 응답을 해석하지 못했습니다.".to_owned())?
        .access_token;
    if token.is_empty() {
        return Err("KIS 모의 인증 응답에 접근 토큰이 없습니다.".to_owned());
    }
    Ok(token)
}

fn resolve_kis_futures_contract(query: &str) -> Result<String, String> {
    let normalized = query.trim().to_ascii_uppercase();
    if normalized.is_empty()
        || normalized.chars().count() > 500
        || normalized.chars().any(char::is_control)
    {
        return Err("KIS 증권선물 분석 요청은 한 줄 1자 이상 500자 이하여야 합니다.".to_owned());
    }
    normalized
        .split(|character: char| !character.is_ascii_alphanumeric())
        .find(|token| {
            (6..=12).contains(&token.len())
                && token.bytes().any(|byte| byte.is_ascii_alphabetic())
                && token.bytes().any(|byte| byte.is_ascii_digit())
                && !token.ends_with("USDT")
        })
        .map(str::to_owned)
        .ok_or_else(|| {
            "현재 만기의 KIS 선물 계약코드가 필요합니다. 예: 101W09. 지수명만으로 근월물을 추정하지 않습니다."
                .to_owned()
        })
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let adjusted = days_since_epoch + 719_468;
    let era = if adjusted >= 0 {
        adjusted
    } else {
        adjusted - 146_096
    } / 146_097;
    let day_of_era = adjusted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let adjusted_year = i64::from(year) - i64::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let month_prime = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

fn format_yyyymmdd(days_since_epoch: i64) -> String {
    let (year, month, day) = civil_from_days(days_since_epoch);
    format!("{year:04}{month:02}{day:02}")
}

fn parse_kis_business_date(value: &str) -> Option<u64> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = value[0..4].parse::<i32>().ok()?;
    let month = value[4..6].parse::<u32>().ok()?;
    let day = value[6..8].parse::<u32>().ok()?;
    let days = days_from_civil(year, month, day)?;
    if civil_from_days(days) != (year, month, day) {
        return None;
    }
    // KRX 일봉 시작은 한국 표준시 자정이며 다음 한국 표준시 자정에 공개 가능한 것으로 둔다.
    u64::try_from(days.checked_mul(86_400_000)?.checked_sub(9 * 3_600_000)?).ok()
}

fn parse_futures_point_minor(value: &str) -> Option<u64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    let scaled = parsed * 100.0;
    if !scaled.is_finite() || scaled <= 0.0 || scaled > u64::MAX as f64 {
        return None;
    }
    Some(scaled.round() as u64)
}

fn parse_futures_volume(value: &str) -> Option<u64> {
    let parsed = value.trim().parse::<f64>().ok()?;
    if !parsed.is_finite() || parsed < 0.0 || parsed > u64::MAX as f64 {
        return None;
    }
    Some(parsed.round() as u64)
}

fn parse_kis_futures_bars(
    rows: &[Value],
    contract_code: &str,
    fetched_at_ms: u64,
) -> Result<Vec<TossChartBar>, String> {
    let mut bars = Vec::with_capacity(rows.len());
    for row in rows {
        let session_id = string_field(row, "stck_bsop_date");
        let period_start_ms = parse_kis_business_date(&session_id)
            .ok_or_else(|| "KIS 선물 일봉의 영업일자를 해석하지 못했습니다.".to_owned())?;
        let period_end_ms = period_start_ms
            .checked_add(86_400_000)
            .ok_or_else(|| "KIS 선물 일봉 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        let open_minor = parse_futures_point_minor(&string_field(row, "futs_oprc"))
            .ok_or_else(|| "KIS 선물 일봉 시가를 해석하지 못했습니다.".to_owned())?;
        let high_minor = parse_futures_point_minor(&string_field(row, "futs_hgpr"))
            .ok_or_else(|| "KIS 선물 일봉 고가를 해석하지 못했습니다.".to_owned())?;
        let low_minor = parse_futures_point_minor(&string_field(row, "futs_lwpr"))
            .ok_or_else(|| "KIS 선물 일봉 저가를 해석하지 못했습니다.".to_owned())?;
        let close_minor = parse_futures_point_minor(&string_field(row, "futs_prpr"))
            .ok_or_else(|| "KIS 선물 일봉 종가를 해석하지 못했습니다.".to_owned())?;
        if low_minor > open_minor
            || low_minor > close_minor
            || high_minor < open_minor
            || high_minor < close_minor
        {
            return Err("KIS 선물 일봉 OHLC 관계가 올바르지 않습니다.".to_owned());
        }
        bars.push(TossChartBar {
            period_start_ms,
            period_end_ms,
            open_minor,
            high_minor,
            low_minor,
            close_minor,
            volume: parse_futures_volume(&string_field(row, "acml_vol"))
                .ok_or_else(|| "KIS 선물 일봉 거래량을 해석하지 못했습니다.".to_owned())?,
            completed: period_end_ms <= fetched_at_ms,
            available_at_ms: Some(period_end_ms),
            ingested_at_ms: Some(fetched_at_ms),
            session_id: Some(session_id),
            contract_code: Some(contract_code.to_owned()),
            settlement_price_minor: None,
            mark_price_minor: None,
            index_price_minor: None,
            funding_rate_bps: None,
            funding_time_ms: None,
        });
    }
    bars.sort_by_key(|bar| bar.period_start_ms);
    if bars
        .windows(2)
        .any(|pair| pair[0].period_start_ms >= pair[1].period_start_ms)
    {
        return Err("KIS 선물 일봉에 중복 또는 역순 영업일자가 있습니다.".to_owned());
    }
    Ok(bars)
}

#[tauri::command]
pub fn kis_futures_connection_status() -> Result<KisFuturesConnectionStatus, String> {
    Ok(match load()? {
        Some(_) => KisFuturesConnectionStatus {
            configured: true,
            connected: false,
            provider: "KIS_VIRTUAL_MARKET_DATA",
            market_data_ready: true,
            message: "KIS 모의 App Key·Secret이 저장되어 있습니다. 현재 만기 계약코드로 읽기 전용 일봉을 조회할 수 있습니다.".to_owned(),
        },
        None => KisFuturesConnectionStatus {
            configured: false,
            connected: false,
            provider: "KIS_VIRTUAL_MARKET_DATA",
            market_data_ready: false,
            message: "KIS 모의 App Key·Secret을 연결하면 국내 지수선물 일봉을 읽기 전용으로 조회합니다.".to_owned(),
        },
    })
}

#[tauri::command]
pub async fn kis_futures_analysis_snapshot(
    request: AnalysisSnapshotRequest,
) -> Result<AnalysisSnapshot, String> {
    let contract_code = resolve_kis_futures_contract(&request.query)?;
    let config = load()?.ok_or_else(|| "KIS 모의 자격정보를 먼저 연결해 주세요.".to_owned())?;
    let client = Client::new();
    let token = issue_paper_token(&client, &config).await?;
    let fetched_at_ms = crate::persistence::now_ms()?;
    let today = i64::try_from(fetched_at_ms / 86_400_000)
        .map_err(|_| "현재 날짜가 지원 범위를 초과했습니다.".to_owned())?;
    let start_date = format_yyyymmdd(today - 370);
    let end_date = format_yyyymmdd(today);
    let response = client
        .get(format!("{PAPER_API_BASE}/uapi/domestic-futureoption/v1/quotations/inquire-daily-fuopchartprice"))
        .timeout(Duration::from_secs(10))
        .bearer_auth(token)
        .header("appkey", &config.app_key)
        .header("appsecret", &config.app_secret)
        .header("tr_id", "FHKIF03020100")
        .header("custtype", "P")
        .query(&[
            ("FID_COND_MRKT_DIV_CODE", "F"),
            ("FID_INPUT_ISCD", contract_code.as_str()),
            ("FID_INPUT_DATE_1", start_date.as_str()),
            ("FID_INPUT_DATE_2", end_date.as_str()),
            ("FID_PERIOD_DIV_CODE", "D"),
        ])
        .send()
        .await
        .map_err(|_| "KIS 국내선물 일봉 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(
            "KIS 국내선물 일봉 조회가 실패했습니다. API 신청 범위와 계약코드를 확인해 주세요."
                .to_owned(),
        );
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|_| "KIS 국내선물 일봉 응답을 해석하지 못했습니다.".to_owned())?;
    if body.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(body
            .get("msg1")
            .and_then(Value::as_str)
            .unwrap_or("KIS 국내선물 일봉 조회가 거절됐습니다.")
            .to_owned());
    }
    let name = body
        .get("output1")
        .map(|row| string_field(row, "hts_kor_isnm"))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| contract_code.clone());
    let rows = body
        .get("output2")
        .and_then(Value::as_array)
        .ok_or_else(|| "KIS 국내선물 일봉 응답에 차트 배열이 없습니다.".to_owned())?;
    let mut bars = parse_kis_futures_bars(rows, &contract_code, fetched_at_ms)?;
    let requested_count = usize::from(request.count.clamp(20, 100));
    if bars.len() > requested_count {
        bars = bars.split_off(bars.len() - requested_count);
    }
    public_market_analysis_snapshot(
        "KIS_VIRTUAL_MARKET_DATA",
        contract_code,
        name,
        "securities_futures".to_owned(),
        "securities_future",
        "POINT".to_owned(),
        "1d".to_owned(),
        fetched_at_ms,
        bars,
        vec![
            "KIS 공식 기간별시세 응답에 별도 일별 정산가 필드가 없어 종가를 정산가로 대체하지 않습니다.".to_owned(),
            "근월물 자동 선택과 연속선물 보정은 적용하지 않으며 사용자가 지정한 단일 계약만 조회합니다.".to_owned(),
        ],
    )
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 120
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

async fn issue_hash(
    client: &Client,
    config: &KisPaperConfigRequest,
    payload: &Value,
) -> Result<String, String> {
    let response = client
        .post(format!("{PAPER_API_BASE}/uapi/hashkey"))
        .timeout(Duration::from_secs(10))
        .header("appkey", &config.app_key)
        .header("appsecret", &config.app_secret)
        .json(payload)
        .send()
        .await
        .map_err(|_| "KIS 모의 주문 검증 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err("KIS 모의 주문 HashKey 발급이 실패했습니다.".to_owned());
    }
    let hash = response
        .json::<HashResponse>()
        .await
        .map_err(|_| "KIS HashKey 응답을 해석하지 못했습니다.".to_owned())?
        .hash;
    if hash.is_empty() {
        return Err("KIS HashKey 응답이 비어 있습니다.".to_owned());
    }
    Ok(hash)
}

fn audit_existing(
    bridge: &PersistenceBridge,
    request_id: &str,
) -> Result<Option<KisPaperRemoteResult>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "KIS 주문 감사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let payload: Option<String> = connection
        .query_row(
            "SELECT payload_json FROM kis_paper_order_audit WHERE request_id = ?1",
            params![request_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("KIS 주문 감사 기록을 조회하지 못했습니다: {error}"))?;
    payload
        .map(|json| {
            serde_json::from_str(&json).map_err(|_| "KIS 주문 감사 기록이 손상됐습니다.".to_owned())
        })
        .transpose()
}

fn save_audit(
    bridge: &PersistenceBridge,
    action: &str,
    symbol: Option<&str>,
    result: &KisPaperRemoteResult,
) -> Result<(), String> {
    let payload = serde_json::to_string(result)
        .map_err(|_| "KIS 주문 결과를 직렬화하지 못했습니다.".to_owned())?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "KIS 주문 감사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection.execute(
        "INSERT INTO kis_paper_order_audit (request_id, action, symbol, remote_order_id, status, payload_json, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![result.request_id, action, symbol, result.remote_order_id, result.status, payload, crate::persistence::now_ms()?],
    ).map_err(|error| format!("KIS 주문 감사 기록을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

fn string_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[tauri::command]
pub async fn kis_paper_account_snapshot() -> Result<KisPaperAccountSnapshot, String> {
    let config =
        load()?.ok_or_else(|| "KIS 모의계좌 연결 정보를 먼저 저장해 주세요.".to_owned())?;
    let client = Client::new();
    let token = issue_paper_token(&client, &config).await?;
    let response = client
        .get(format!(
            "{PAPER_API_BASE}/uapi/domestic-stock/v1/trading/inquire-balance"
        ))
        .timeout(Duration::from_secs(10))
        .bearer_auth(token)
        .header("appkey", &config.app_key)
        .header("appsecret", &config.app_secret)
        .header("tr_id", "VTTC8434R")
        .header("custtype", "P")
        .query(&[
            ("CANO", config.account_number.as_str()),
            ("ACNT_PRDT_CD", config.product_code.as_str()),
            ("AFHR_FLPR_YN", "N"),
            ("OFL_YN", ""),
            ("INQR_DVSN", "02"),
            ("UNPR_DVSN", "01"),
            ("FUND_STTL_ICLD_YN", "N"),
            ("FNCG_AMT_AUTO_RDPT_YN", "N"),
            ("PRCS_DVSN", "01"),
            ("CTX_AREA_FK100", ""),
            ("CTX_AREA_NK100", ""),
        ])
        .send()
        .await
        .map_err(|_| "KIS 모의 잔고 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err("KIS 모의 잔고 조회가 실패했습니다.".to_owned());
    }
    let body = response
        .json::<Value>()
        .await
        .map_err(|_| "KIS 모의 잔고 응답을 해석하지 못했습니다.".to_owned())?;
    if body.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(body
            .get("msg1")
            .and_then(Value::as_str)
            .unwrap_or("KIS 모의 잔고 조회가 거절됐습니다.")
            .to_owned());
    }
    let positions = body
        .get("output1")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let symbol = string_field(row, "pdno");
            if symbol.is_empty() {
                return None;
            }
            Some(KisPaperPosition {
                symbol,
                name: string_field(row, "prdt_name"),
                quantity: string_field(row, "hldg_qty"),
                average_price: string_field(row, "pchs_avg_pric"),
                current_price: string_field(row, "prpr"),
                evaluation_amount: string_field(row, "evlu_amt"),
                unrealized_pnl: string_field(row, "evlu_pfls_amt"),
            })
        })
        .collect();
    let summary = body
        .get("output2")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first());
    Ok(KisPaperAccountSnapshot {
        provider: "KIS_PAPER_API",
        masked_account_number: format!("{}****", &config.account_number[..4]),
        product_code: config.product_code,
        deposit: summary
            .map(|row| string_field(row, "dnca_tot_amt"))
            .filter(|value| !value.is_empty()),
        total_evaluation_amount: summary
            .map(|row| string_field(row, "tot_evlu_amt"))
            .filter(|value| !value.is_empty()),
        positions,
        observed_at_ms: crate::persistence::now_ms()?,
        live_order_enabled: false,
    })
}

#[tauri::command]
pub async fn kis_paper_order_submit(
    request: KisPaperOrderRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<KisPaperRemoteResult, String> {
    if !valid_request_id(&request.request_id)
        || request.symbol.len() != 6
        || !request.symbol.bytes().all(|byte| byte.is_ascii_digit())
        || !matches!(request.side.as_str(), "buy" | "sell")
        || !matches!(request.order_type.as_str(), "market" | "limit")
        || request.quantity == 0
        || (request.order_type == "limit" && request.price == 0)
        || request.confirmation != "KIS 모의주문 전송"
    {
        return Err(
            "국내 종목·방향·수량·가격을 확인하고 ‘KIS 모의주문 전송’을 정확히 입력해 주세요."
                .to_owned(),
        );
    }
    if let Some(existing) = audit_existing(&bridge, &request.request_id)? {
        return Ok(existing);
    }
    let config =
        load()?.ok_or_else(|| "KIS 모의계좌 연결 정보를 먼저 저장해 주세요.".to_owned())?;
    let client = Client::new();
    let token = issue_paper_token(&client, &config).await?;
    let payload = json!({
        "CANO": config.account_number,
        "ACNT_PRDT_CD": config.product_code,
        "PDNO": request.symbol,
        "ORD_DVSN": if request.order_type == "market" { "01" } else { "00" },
        "ORD_QTY": request.quantity.to_string(),
        "ORD_UNPR": if request.order_type == "market" { "0".to_owned() } else { request.price.to_string() },
    });
    let hash = issue_hash(&client, &config, &payload).await?;
    let response = client
        .post(format!(
            "{PAPER_API_BASE}/uapi/domestic-stock/v1/trading/order-cash"
        ))
        .timeout(Duration::from_secs(10))
        .bearer_auth(token)
        .header("appkey", &config.app_key)
        .header("appsecret", &config.app_secret)
        .header(
            "tr_id",
            if request.side == "buy" {
                "VTTC0802U"
            } else {
                "VTTC0801U"
            },
        )
        .header("custtype", "P")
        .header("hashkey", hash)
        .json(&payload)
        .send()
        .await
        .map_err(|_| "KIS 모의 주문 서버에 연결하지 못했습니다.".to_owned())?;
    let body = response
        .json::<Value>()
        .await
        .map_err(|_| "KIS 모의 주문 응답을 해석하지 못했습니다.".to_owned())?;
    let accepted = body.get("rt_cd").and_then(Value::as_str) == Some("0");
    let output = body.get("output").unwrap_or(&Value::Null);
    let result = KisPaperRemoteResult {
        request_id: request.request_id,
        status: if accepted { "submitted" } else { "rejected" }.to_owned(),
        remote_order_id: Some(string_field(output, "ODNO")).filter(|value| !value.is_empty()),
        order_branch: Some(string_field(output, "KRX_FWDG_ORD_ORGNO"))
            .filter(|value| !value.is_empty()),
        order_time: Some(string_field(output, "ORD_TMD")).filter(|value| !value.is_empty()),
        message: body
            .get("msg1")
            .and_then(Value::as_str)
            .unwrap_or("KIS 모의 주문 응답")
            .to_owned(),
    };
    save_audit(&bridge, "submit", Some(&request.symbol), &result)?;
    Ok(result)
}

#[tauri::command]
pub async fn kis_paper_order_cancel(
    request: KisPaperCancelRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<KisPaperRemoteResult, String> {
    if !valid_request_id(&request.request_id)
        || request.remote_order_id.is_empty()
        || request.remote_order_id.len() > 32
        || request.order_branch.is_empty()
        || request.order_branch.len() > 8
        || request.quantity == 0
        || request.confirmation != "KIS 모의주문 취소"
    {
        return Err(
            "원주문번호·주문지점·수량을 확인하고 ‘KIS 모의주문 취소’를 정확히 입력해 주세요."
                .to_owned(),
        );
    }
    if let Some(existing) = audit_existing(&bridge, &request.request_id)? {
        return Ok(existing);
    }
    let config =
        load()?.ok_or_else(|| "KIS 모의계좌 연결 정보를 먼저 저장해 주세요.".to_owned())?;
    let client = Client::new();
    let token = issue_paper_token(&client, &config).await?;
    let payload = json!({
        "CANO": config.account_number, "ACNT_PRDT_CD": config.product_code,
        "KRX_FWDG_ORD_ORGNO": request.order_branch, "ORGN_ODNO": request.remote_order_id,
        "ORD_DVSN": "00", "RVSE_CNCL_DVSN_CD": "02", "ORD_QTY": request.quantity.to_string(),
        "ORD_UNPR": "0", "QTY_ALL_ORD_YN": "N",
    });
    let hash = issue_hash(&client, &config, &payload).await?;
    let response = client
        .post(format!(
            "{PAPER_API_BASE}/uapi/domestic-stock/v1/trading/order-rvsecncl"
        ))
        .timeout(Duration::from_secs(10))
        .bearer_auth(token)
        .header("appkey", &config.app_key)
        .header("appsecret", &config.app_secret)
        .header("tr_id", "VTTC0803U")
        .header("custtype", "P")
        .header("hashkey", hash)
        .json(&payload)
        .send()
        .await
        .map_err(|_| "KIS 모의 취소 서버에 연결하지 못했습니다.".to_owned())?;
    let body = response
        .json::<Value>()
        .await
        .map_err(|_| "KIS 모의 취소 응답을 해석하지 못했습니다.".to_owned())?;
    let accepted = body.get("rt_cd").and_then(Value::as_str) == Some("0");
    let output = body.get("output").unwrap_or(&Value::Null);
    let result = KisPaperRemoteResult {
        request_id: request.request_id,
        status: if accepted {
            "cancel_submitted"
        } else {
            "cancel_rejected"
        }
        .to_owned(),
        remote_order_id: Some(string_field(output, "ODNO"))
            .filter(|value| !value.is_empty())
            .or(Some(request.remote_order_id)),
        order_branch: Some(string_field(output, "KRX_FWDG_ORD_ORGNO"))
            .filter(|value| !value.is_empty())
            .or(Some(request.order_branch)),
        order_time: Some(string_field(output, "ORD_TMD")).filter(|value| !value.is_empty()),
        message: body
            .get("msg1")
            .and_then(Value::as_str)
            .unwrap_or("KIS 모의 취소 응답")
            .to_owned(),
    };
    save_audit(&bridge, "cancel", None, &result)?;
    Ok(result)
}

#[tauri::command]
pub async fn kis_paper_orders_today(request: KisPaperOrdersRequest) -> Result<Value, String> {
    if request.trade_date.len() != 8
        || !request.trade_date.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("주문 조회일은 YYYYMMDD 형식이어야 합니다.".to_owned());
    }
    let config =
        load()?.ok_or_else(|| "KIS 모의계좌 연결 정보를 먼저 저장해 주세요.".to_owned())?;
    let client = Client::new();
    let token = issue_paper_token(&client, &config).await?;
    let response = client
        .get(format!(
            "{PAPER_API_BASE}/uapi/domestic-stock/v1/trading/inquire-daily-ccld"
        ))
        .timeout(Duration::from_secs(10))
        .bearer_auth(token)
        .header("appkey", &config.app_key)
        .header("appsecret", &config.app_secret)
        .header("tr_id", "VTTC8001R")
        .header("custtype", "P")
        .query(&[
            ("CANO", config.account_number.as_str()),
            ("ACNT_PRDT_CD", config.product_code.as_str()),
            ("INQR_STRT_DT", request.trade_date.as_str()),
            ("INQR_END_DT", request.trade_date.as_str()),
            ("SLL_BUY_DVSN_CD", "00"),
            ("INQR_DVSN", "00"),
            ("PDNO", ""),
            ("CCLD_DVSN", "00"),
            ("ORD_GNO_BRNO", ""),
            ("ODNO", ""),
            ("INQR_DVSN_3", "00"),
            ("INQR_DVSN_1", ""),
            ("CTX_AREA_FK100", ""),
            ("CTX_AREA_NK100", ""),
        ])
        .send()
        .await
        .map_err(|_| "KIS 모의 체결조회 서버에 연결하지 못했습니다.".to_owned())?;
    let body = response
        .json::<Value>()
        .await
        .map_err(|_| "KIS 모의 체결조회 응답을 해석하지 못했습니다.".to_owned())?;
    if body.get("rt_cd").and_then(Value::as_str) != Some("0") {
        return Err(body
            .get("msg1")
            .and_then(Value::as_str)
            .unwrap_or("KIS 모의 체결조회가 거절됐습니다.")
            .to_owned());
    }
    Ok(
        json!({"provider":"KIS_PAPER_API","tradeDate":request.trade_date,"orders":body.get("output1").cloned().unwrap_or_else(|| json!([])),"observedAtMs":crate::persistence::now_ms()?}),
    )
}

#[tauri::command]
pub async fn kis_paper_reconcile(
    request_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<KisPaperReconciliation, String> {
    if !valid_request_id(&request_id) {
        return Err("대사 요청 ID가 올바르지 않습니다.".to_owned());
    }
    let remote = kis_paper_account_snapshot().await?;
    let internal = crate::paper_trading::load_or_open_account(&bridge)?;
    let mut discrepancies = Vec::new();
    for position in &remote.positions {
        let remote_quantity = position
            .quantity
            .parse::<u64>()
            .map_err(|_| "KIS 보유수량을 해석하지 못했습니다.".to_owned())?;
        let internal_quantity = internal
            .positions
            .get(&position.symbol)
            .map_or(0, |item| item.quantity / item.quantity_scale);
        if remote_quantity != internal_quantity {
            discrepancies.push(format!(
                "{} 수량 불일치: KIS {} / 내부 {}",
                position.symbol, remote_quantity, internal_quantity
            ));
        }
    }
    for (symbol, position) in &internal.positions {
        if symbol.starts_with("KRW-") {
            continue;
        }
        if !remote.positions.iter().any(|item| &item.symbol == symbol) && position.quantity > 0 {
            discrepancies.push(format!("{} 내부 포지션이 KIS 모의계좌에 없습니다.", symbol));
        }
    }
    let result = KisPaperReconciliation {
        observed_at_ms: remote.observed_at_ms,
        matched: discrepancies.is_empty(),
        discrepancies,
        remote_position_count: remote.positions.len(),
        internal_position_count: internal
            .positions
            .iter()
            .filter(|(symbol, _)| !symbol.starts_with("KRW-"))
            .count(),
    };
    let audit = KisPaperRemoteResult {
        request_id,
        status: if result.matched {
            "matched"
        } else {
            "mismatch"
        }
        .to_owned(),
        remote_order_id: None,
        order_branch: None,
        order_time: None,
        message: serde_json::to_string(&result).unwrap_or_else(|_| "대사 결과".to_owned()),
    };
    save_audit(&bridge, "reconcile", None, &audit)?;
    Ok(result)
}

#[tauri::command]
pub fn kis_paper_config_status() -> Result<KisPaperConfigStatus, String> {
    let config = load()?;
    Ok(status(config.as_ref()))
}

#[tauri::command]
pub fn kis_paper_config_save(
    request: KisPaperConfigRequest,
) -> Result<KisPaperConfigStatus, String> {
    validate(&request)?;
    let json = serde_json::to_string(&request)
        .map_err(|_| "KIS 모의계좌 연결 정보를 직렬화하지 못했습니다.".to_owned())?;
    entry()?.set_password(&json).map_err(|_| {
        "KIS 모의계좌 연결 정보를 Windows 자격 증명 관리자에 저장하지 못했습니다.".to_owned()
    })?;
    Ok(status(Some(&request)))
}

#[tauri::command]
pub fn kis_paper_config_delete() -> Result<KisPaperConfigStatus, String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(status(None)),
        Err(_) => {
            Err("Windows 자격 증명 관리자에서 KIS 연결 정보를 삭제하지 못했습니다.".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_kis_paper_account_parts_and_secrets() {
        let valid = KisPaperConfigRequest {
            app_key: "12345678".to_owned(),
            app_secret: "abcdefgh".to_owned(),
            hts_id: "tester".to_owned(),
            account_number: "12345678".to_owned(),
            product_code: "01".to_owned(),
        };
        assert!(validate(&valid).is_ok());
        let invalid = KisPaperConfigRequest {
            account_number: "1234-678".to_owned(),
            ..valid
        };
        assert!(validate(&invalid).is_err());
    }

    #[test]
    fn resolves_explicit_kis_futures_contract_without_guessing_nearby_month() {
        assert_eq!(
            resolve_kis_futures_contract("KIS 지수선물 101W09 일봉").expect("contract"),
            "101W09"
        );
        assert!(resolve_kis_futures_contract("코스피200 근월물 분석").is_err());
        assert!(resolve_kis_futures_contract("BTCUSDT 코인 선물").is_err());
    }

    #[test]
    fn parses_kis_futures_daily_bars_with_contract_and_pit_boundary() {
        let rows = vec![json!({
            "stck_bsop_date": "20260825",
            "futs_oprc": "355.10",
            "futs_hgpr": "358.25",
            "futs_lwpr": "352.80",
            "futs_prpr": "357.40",
            "acml_vol": "123456"
        })];
        let fetched_at_ms = parse_kis_business_date("20260827").expect("date");
        let bars = parse_kis_futures_bars(&rows, "101W09", fetched_at_ms).expect("bars");
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].contract_code.as_deref(), Some("101W09"));
        assert_eq!(bars[0].session_id.as_deref(), Some("20260825"));
        assert_eq!(bars[0].close_minor, 35_740);
        assert!(bars[0].settlement_price_minor.is_none());
        assert!(bars[0].completed);
    }

    #[test]
    fn rejects_invalid_kis_business_dates_and_oversized_points() {
        assert!(parse_kis_business_date("20260231").is_none());
        assert!(parse_kis_business_date("20261301").is_none());
        assert!(parse_futures_point_minor("1e30").is_none());
    }

    #[test]
    #[ignore = "저장된 KIS 모의 자격정보와 외부 모의 서버를 사용하는 읽기 전용 검사"]
    fn live_kis_paper_balance_uses_only_the_virtual_domain() {
        if std::env::var("INVESTA_RUN_KIS_LIVE").as_deref() != Ok("1") {
            eprintln!(
                "KIS_LIVE_SKIPPED INVESTA_RUN_KIS_LIVE=1이 아니므로 모의계좌 검사를 건너뜁니다."
            );
            return;
        }
        let snapshot = tauri::async_runtime::block_on(kis_paper_account_snapshot())
            .expect("KIS paper balance");
        assert_eq!(snapshot.provider, "KIS_PAPER_API");
        assert!(!snapshot.live_order_enabled);
        assert!(snapshot.masked_account_number.ends_with("****"));
    }
}
