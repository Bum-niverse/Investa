use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tauri::State;
use uuid::Uuid;

use crate::{
    backtest::{run_backtest, BacktestConfig, BacktestResult, PriceBar},
    paper_account::{execute_shadow_order, ShadowOrderRequest},
    paper_trading,
    persistence::{PersistBacktest, PersistenceBridge},
    research::{review_research_report, Market, ResearchReport, StrategyReview},
    simulation::TradingCosts,
    trading::TradeSide,
};

const UPBIT_API_BASE: &str = "https://api.upbit.com";
const REQUEST_TIMEOUT_SECONDS: u64 = 8;
const UPBIT_KEYRING_SERVICE: &str = "Investa.Upbit";
const UPBIT_ACCESS_KEY_ACCOUNT: &str = "access-key";
const UPBIT_SECRET_KEY_ACCOUNT: &str = "secret-key";

pub struct CryptoMarketBridge {
    client: Client,
}

#[derive(Debug, Clone)]
struct UpbitCredentials {
    access_key: String,
    secret_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpbitCredentialsRequest {
    access_key: String,
    secret_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpbitConnectionStatus {
    configured: bool,
    connected: bool,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpbitAccountSnapshot {
    provider: &'static str,
    fetched_at_ms: u64,
    read_only: bool,
    accounts: Vec<UpbitBalance>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct UpbitBalance {
    currency: String,
    balance: String,
    locked: String,
    avg_buy_price: String,
    unit_currency: String,
}

#[derive(Serialize)]
struct UpbitJwtHeader<'a> {
    alg: &'a str,
    typ: &'a str,
}

#[derive(Serialize)]
struct UpbitJwtPayload<'a> {
    access_key: &'a str,
    nonce: String,
}

fn upbit_credential_entry(account: &str) -> Result<Entry, String> {
    Entry::new(UPBIT_KEYRING_SERVICE, account)
        .map_err(|_| "Windows 자격 증명 저장소를 열지 못했습니다.".to_owned())
}

fn optional_keyring_password(entry: &Entry) -> Result<Option<String>, String> {
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("Windows 자격 증명 저장소를 읽지 못했습니다.".to_owned()),
    }
}

fn load_upbit_credentials() -> Result<Option<UpbitCredentials>, String> {
    let access_key = optional_keyring_password(&upbit_credential_entry(UPBIT_ACCESS_KEY_ACCOUNT)?)?;
    let secret_key = optional_keyring_password(&upbit_credential_entry(UPBIT_SECRET_KEY_ACCOUNT)?)?;
    match (access_key, secret_key) {
        (None, None) => Ok(None),
        (Some(access_key), Some(secret_key)) => Ok(Some(UpbitCredentials {
            access_key,
            secret_key,
        })),
        _ => Err(
            "업비트 자격정보가 불완전합니다. 연결 정보를 삭제한 뒤 다시 등록해 주세요.".to_owned(),
        ),
    }
}

fn validate_upbit_credentials(
    request: UpbitCredentialsRequest,
) -> Result<UpbitCredentials, String> {
    let access_key = request.access_key.trim().to_owned();
    let secret_key = request.secret_key.trim().to_owned();
    let valid = |value: &str| {
        (16..=256).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_graphic())
    };
    if !valid(&access_key) || !valid(&secret_key) {
        return Err("업비트 Access Key와 Secret Key 형식을 확인해 주세요.".to_owned());
    }
    Ok(UpbitCredentials {
        access_key,
        secret_key,
    })
}

fn store_upbit_credentials(credentials: &UpbitCredentials) -> Result<(), String> {
    let access_entry = upbit_credential_entry(UPBIT_ACCESS_KEY_ACCOUNT)?;
    let secret_entry = upbit_credential_entry(UPBIT_SECRET_KEY_ACCOUNT)?;
    access_entry
        .set_password(&credentials.access_key)
        .map_err(|_| "업비트 Access Key를 안전하게 저장하지 못했습니다.".to_owned())?;
    if secret_entry.set_password(&credentials.secret_key).is_err() {
        let _ = access_entry.delete_credential();
        return Err("업비트 Secret Key를 안전하게 저장하지 못했습니다.".to_owned());
    }
    Ok(())
}

fn delete_upbit_credentials() -> Result<(), String> {
    for account in [UPBIT_ACCESS_KEY_ACCOUNT, UPBIT_SECRET_KEY_ACCOUNT] {
        let entry = upbit_credential_entry(account)?;
        match entry.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(_) => return Err("업비트 연결 정보를 삭제하지 못했습니다.".to_owned()),
        }
    }
    Ok(())
}

fn create_upbit_bearer(credentials: &UpbitCredentials) -> Result<String, String> {
    let header = serde_json::to_vec(&UpbitJwtHeader {
        alg: "HS256",
        typ: "JWT",
    })
    .map_err(|_| "업비트 인증 헤더를 만들지 못했습니다.".to_owned())?;
    let payload = serde_json::to_vec(&UpbitJwtPayload {
        access_key: &credentials.access_key,
        nonce: Uuid::new_v4().to_string(),
    })
    .map_err(|_| "업비트 인증 요청을 만들지 못했습니다.".to_owned())?;
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(credentials.secret_key.as_bytes())
        .map_err(|_| "업비트 인증 서명 키를 처리하지 못했습니다.".to_owned())?;
    mac.update(signing_input.as_bytes());
    Ok(format!(
        "Bearer {}.{}",
        signing_input,
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    ))
}

async fn fetch_upbit_accounts(
    bridge: &CryptoMarketBridge,
    credentials: &UpbitCredentials,
) -> Result<Vec<UpbitBalance>, String> {
    let response = bridge
        .client
        .get(format!("{UPBIT_API_BASE}/v1/accounts"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .header("Accept", "application/json")
        .header("Authorization", create_upbit_bearer(credentials)?)
        .send()
        .await
        .map_err(|_| "업비트 개인계좌 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                "업비트 API 키 권한, 허용 IP 또는 만료 상태를 확인해 주세요.".to_owned()
            }
            StatusCode::TOO_MANY_REQUESTS => "업비트 계좌 조회 한도를 초과했습니다.".to_owned(),
            _ => "업비트 개인계좌 서버가 요청을 처리하지 못했습니다.".to_owned(),
        });
    }
    let accounts = response
        .json::<Vec<UpbitBalance>>()
        .await
        .map_err(|_| "업비트 계좌 응답 형식이 올바르지 않습니다.".to_owned())?;
    if accounts.len() > 500 {
        return Err("업비트 계좌 응답 항목이 허용 범위를 초과했습니다.".to_owned());
    }
    Ok(accounts)
}

#[tauri::command]
pub fn upbit_connection_status() -> Result<UpbitConnectionStatus, String> {
    Ok(match load_upbit_credentials()? {
        Some(_) => UpbitConnectionStatus {
            configured: true,
            connected: false,
            message: "자격정보 저장됨 · 계좌 조회로 연결을 확인해 주세요.".to_owned(),
        },
        None => UpbitConnectionStatus {
            configured: false,
            connected: false,
            message: "개인계좌 API가 연결되지 않았습니다.".to_owned(),
        },
    })
}

#[tauri::command]
pub async fn upbit_save_credentials(
    request: UpbitCredentialsRequest,
    bridge: State<'_, CryptoMarketBridge>,
) -> Result<UpbitConnectionStatus, String> {
    let credentials = validate_upbit_credentials(request)?;
    fetch_upbit_accounts(&bridge, &credentials).await?;
    store_upbit_credentials(&credentials)?;
    Ok(UpbitConnectionStatus {
        configured: true,
        connected: true,
        message: "읽기 전용 개인계좌 연결을 확인했습니다.".to_owned(),
    })
}

#[tauri::command]
pub fn upbit_delete_credentials() -> Result<UpbitConnectionStatus, String> {
    delete_upbit_credentials()?;
    Ok(UpbitConnectionStatus {
        configured: false,
        connected: false,
        message: "개인계좌 연결 정보를 이 PC에서 삭제했습니다.".to_owned(),
    })
}

#[tauri::command]
pub async fn upbit_account_snapshot(
    bridge: State<'_, CryptoMarketBridge>,
) -> Result<UpbitAccountSnapshot, String> {
    let credentials = load_upbit_credentials()?
        .ok_or_else(|| "업비트 개인계좌 연결 정보가 없습니다.".to_owned())?;
    Ok(UpbitAccountSnapshot {
        provider: "Upbit",
        fetched_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "현재 시각을 확인하지 못했습니다.".to_owned())?
            .as_millis() as u64,
        read_only: true,
        accounts: fetch_upbit_accounts(&bridge, &credentials).await?,
    })
}

impl Default for CryptoMarketBridge {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl CryptoMarketBridge {
    pub(crate) async fn fetch_strategy_bars(&self, symbol: &str) -> Result<Vec<PriceBar>, String> {
        let snapshot = fetch_chart_snapshot(
            CryptoChartRequest {
                symbol: symbol.to_owned(),
                interval: "1d".to_owned(),
                count: 200,
            },
            self,
        )
        .await?;
        Ok(snapshot
            .bars
            .into_iter()
            .filter(|bar| bar.completed)
            .map(|bar| PriceBar {
                symbol: snapshot.symbol.clone(),
                currency: "KRW".to_owned(),
                source: snapshot.provider.to_owned(),
                period_start_ms: bar.period_start_ms,
                period_end_ms: bar.period_end_ms,
                available_at_ms: bar.period_end_ms,
                ingested_at_ms: snapshot.fetched_at_ms,
                open_minor: bar.open_minor,
                high_minor: bar.high_minor,
                low_minor: bar.low_minor,
                close_minor: bar.close_minor,
                volume: bar.volume,
            })
            .collect())
    }

    pub(crate) async fn fetch_price(&self, symbol: &str) -> Result<(u64, u64), String> {
        let market = validate_market(symbol)?;
        let ticker = fetch_ticker(self, &market).await?;
        Ok((krw_minor(ticker.trade_price)?, ticker.timestamp))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoChartRequest {
    symbol: String,
    interval: String,
    count: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoChartBar {
    period_start_ms: u64,
    period_end_ms: u64,
    open_minor: u64,
    high_minor: u64,
    low_minor: u64,
    close_minor: u64,
    volume: u64,
    completed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoChartSnapshot {
    provider: &'static str,
    symbol: String,
    currency: &'static str,
    interval: String,
    adjusted: bool,
    fetched_at_ms: u64,
    bars: Vec<CryptoChartBar>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoMarketQuote {
    provider: &'static str,
    symbol: String,
    currency: &'static str,
    last_price_minor: u64,
    observed_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoBacktestRequest {
    report: ResearchReport,
    requested_at_ms: Option<u64>,
    interval: String,
    count: u16,
    adjusted: bool,
    config: BacktestConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoBacktestRun {
    review: StrategyReview,
    result: BacktestResult,
    provider: String,
    interval: String,
    adjusted: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CryptoPaperMarketOrderRequest {
    symbol: String,
    side: TradeSide,
    quantity: u64,
    idempotency_key: String,
    costs: TradingCosts,
}

#[derive(Debug, Deserialize)]
struct UpbitCandle {
    market: String,
    candle_date_time_kst: String,
    opening_price: f64,
    high_price: f64,
    low_price: f64,
    trade_price: f64,
    candle_acc_trade_volume: f64,
}

#[derive(Debug, Deserialize)]
struct UpbitTicker {
    market: String,
    trade_price: f64,
    timestamp: u64,
}

fn validate_market(value: &str) -> Result<String, String> {
    let market = value.trim().to_ascii_uppercase();
    if market.len() < 7
        || market.len() > 24
        || !market.starts_with("KRW-")
        || !market
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("업비트 원화 마켓 코드를 입력해 주세요. 예: KRW-BTC".to_owned());
    }
    Ok(market)
}

fn krw_minor(value: f64) -> Result<u64, String> {
    if !value.is_finite() || value <= 0.0 || value > u64::MAX as f64 {
        return Err("업비트 가격 응답이 지원 범위를 벗어났습니다.".to_owned());
    }
    Ok(value.round() as u64)
}

fn request_error(status: StatusCode, kind: &str) -> String {
    match status {
        StatusCode::TOO_MANY_REQUESTS => format!("업비트 {kind} 요청 한도를 초과했습니다."),
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
            format!("업비트 {kind} 마켓 코드 또는 조회 조건을 확인해 주세요.")
        }
        _ => format!("업비트 {kind} 서버가 요청을 처리하지 못했습니다."),
    }
}

async fn fetch_ticker(bridge: &CryptoMarketBridge, market: &str) -> Result<UpbitTicker, String> {
    let response = bridge
        .client
        .get(format!("{UPBIT_API_BASE}/v1/ticker"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .query(&[("markets", market)])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| "업비트 현재가 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(request_error(response.status(), "현재가"));
    }
    response
        .json::<Vec<UpbitTicker>>()
        .await
        .map_err(|_| "업비트 현재가 응답 형식이 올바르지 않습니다.".to_owned())?
        .into_iter()
        .find(|ticker| ticker.market == market)
        .ok_or_else(|| "업비트 현재가 응답에서 요청 마켓을 찾지 못했습니다.".to_owned())
}

async fn fetch_chart_snapshot(
    request: CryptoChartRequest,
    bridge: &CryptoMarketBridge,
) -> Result<CryptoChartSnapshot, String> {
    let market = validate_market(&request.symbol)?;
    if !(20..=200).contains(&request.count) {
        return Err("업비트 차트 캔들 개수는 20개에서 200개 사이여야 합니다.".to_owned());
    }
    let (path, duration_ms) = match request.interval.as_str() {
        "1m" => ("/v1/candles/minutes/1", 60_000_u64),
        "1d" => ("/v1/candles/days", 86_400_000_u64),
        _ => return Err("코인 차트 주기는 1분봉 또는 일봉이어야 합니다.".to_owned()),
    };
    let count = request.count.to_string();
    let response = bridge
        .client
        .get(format!("{UPBIT_API_BASE}{path}"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .query(&[("market", market.as_str()), ("count", count.as_str())])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| "업비트 캔들 서버에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(request_error(response.status(), "캔들"));
    }
    let mut candles = response
        .json::<Vec<UpbitCandle>>()
        .await
        .map_err(|_| "업비트 캔들 응답 형식이 올바르지 않습니다.".to_owned())?;
    if candles.is_empty() || candles.iter().any(|candle| candle.market != market) {
        return Err("업비트 캔들 응답에 요청 마켓 데이터가 없습니다.".to_owned());
    }
    let mut parsed_candles = candles
        .drain(..)
        .map(|candle| {
            let period_start_ms = crate::market_data::parse_rfc3339_ms(&format!(
                "{}+09:00",
                candle.candle_date_time_kst
            ))
            .ok_or_else(|| "업비트 캔들 시각을 해석하지 못했습니다.".to_owned())?;
            Ok((period_start_ms, candle))
        })
        .collect::<Result<Vec<_>, String>>()?;
    parsed_candles.sort_by_key(|(period_start_ms, _)| *period_start_ms);
    let fetched_at_ms = paper_trading::now_ms()?;
    let mut bars = Vec::with_capacity(parsed_candles.len());
    for (period_start_ms, candle) in parsed_candles {
        let period_end_ms = period_start_ms
            .checked_add(duration_ms)
            .ok_or_else(|| "업비트 캔들 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        let open_minor = krw_minor(candle.opening_price)?;
        let high_minor = krw_minor(candle.high_price)?;
        let low_minor = krw_minor(candle.low_price)?;
        let close_minor = krw_minor(candle.trade_price)?;
        if low_minor > open_minor
            || low_minor > close_minor
            || high_minor < open_minor
            || high_minor < close_minor
        {
            return Err("업비트 캔들 OHLC 관계가 올바르지 않습니다.".to_owned());
        }
        bars.push(CryptoChartBar {
            period_start_ms,
            period_end_ms,
            open_minor,
            high_minor,
            low_minor,
            close_minor,
            volume: if candle.candle_acc_trade_volume.is_finite()
                && candle.candle_acc_trade_volume >= 0.0
            {
                (candle.candle_acc_trade_volume * 100_000_000.0).round() as u64
            } else {
                return Err("업비트 캔들 거래량을 해석하지 못했습니다.".to_owned());
            },
            completed: period_end_ms <= fetched_at_ms,
        });
    }
    Ok(CryptoChartSnapshot {
        provider: "UPBIT_PUBLIC_API",
        symbol: market,
        currency: "KRW",
        interval: request.interval,
        adjusted: false,
        fetched_at_ms,
        bars,
    })
}

#[tauri::command]
pub async fn upbit_chart_snapshot(
    request: CryptoChartRequest,
    bridge: State<'_, CryptoMarketBridge>,
) -> Result<CryptoChartSnapshot, String> {
    fetch_chart_snapshot(request, &bridge).await
}

#[tauri::command]
pub async fn upbit_run_research_backtest(
    request: CryptoBacktestRequest,
    bridge: State<'_, CryptoMarketBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<CryptoBacktestRun, String> {
    if request.interval != "1d" {
        return Err("코인 전략 검증은 현재 완료된 업비트 일봉만 지원합니다.".to_owned());
    }
    if request.adjusted {
        return Err("업비트 코인 캔들에는 수정주가 옵션을 사용할 수 없습니다.".to_owned());
    }
    if request.report.strategy_candidate.market != Market::Crypto {
        return Err("코인 백테스트에는 market=crypto 연구 계약이 필요합니다.".to_owned());
    }
    let review = review_research_report(&request.report);
    if !review.executable {
        return Err(
            "검증 오류나 미해결 항목이 있는 연구 보고서는 백테스트할 수 없습니다.".to_owned(),
        );
    }
    let spec = &request.report.strategy_candidate;
    let snapshot = fetch_chart_snapshot(
        CryptoChartRequest {
            symbol: spec.symbol.clone(),
            interval: request.interval.clone(),
            count: request.count,
        },
        &bridge,
    )
    .await?;
    let bars = snapshot
        .bars
        .iter()
        .filter(|bar| bar.completed)
        .map(|bar| PriceBar {
            symbol: snapshot.symbol.clone(),
            currency: snapshot.currency.to_owned(),
            source: snapshot.provider.to_owned(),
            period_start_ms: bar.period_start_ms,
            period_end_ms: bar.period_end_ms,
            available_at_ms: bar.period_end_ms,
            ingested_at_ms: snapshot.fetched_at_ms,
            open_minor: bar.open_minor,
            high_minor: bar.high_minor,
            low_minor: bar.low_minor,
            close_minor: bar.close_minor,
            volume: bar.volume,
        })
        .collect::<Vec<_>>();
    let result = run_backtest(spec, &bars, &request.config)
        .map_err(|error| format!("코인 백테스트를 실행하지 못했습니다: {}", error.message))?;
    let warnings = vec![
        "업비트 공개 API의 최대 200개 완료 일봉을 사용하는 탐색 백테스트이며 성과 합격 판정은 하지 않습니다.".to_owned(),
        "암호화폐 시장은 24시간 운영되며 일봉 경계는 업비트 KST 기준입니다.".to_owned(),
        "현재 백테스트 주문 수량은 내부 원장과 동일한 정수 코인 단위입니다. 소수 수량은 원장 고정소수점 마이그레이션 전까지 지원하지 않습니다.".to_owned(),
    ];
    let run = CryptoBacktestRun {
        review,
        result,
        provider: snapshot.provider.to_owned(),
        interval: snapshot.interval,
        adjusted: false,
        warnings,
    };
    persistence.persist_backtest(PersistBacktest {
        report: &request.report,
        review: &run.review,
        bars: &bars,
        config: &request.config,
        result: &run.result,
        provider: &run.provider,
        interval: &run.interval,
        adjusted: run.adjusted,
        warnings: &run.warnings,
        requested_at_ms: request.requested_at_ms,
        classification: "research_experiment",
    })?;
    Ok(run)
}

#[tauri::command]
pub async fn upbit_market_quote(
    symbol: String,
    bridge: State<'_, CryptoMarketBridge>,
) -> Result<CryptoMarketQuote, String> {
    let market = validate_market(&symbol)?;
    let ticker = fetch_ticker(&bridge, &market).await?;
    Ok(CryptoMarketQuote {
        provider: "UPBIT_PUBLIC_API",
        symbol: market,
        currency: "KRW",
        last_price_minor: krw_minor(ticker.trade_price)?,
        observed_at_ms: ticker.timestamp,
    })
}

#[tauri::command]
pub async fn upbit_execute_paper_market_order(
    request: CryptoPaperMarketOrderRequest,
    bridge: State<'_, CryptoMarketBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<paper_trading::PaperAccountSnapshot, String> {
    let market = validate_market(&request.symbol)?;
    if request.quantity == 0
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 120
        || !request
            .idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("유효한 코인 모의주문 수량과 요청 식별자가 필요합니다.".to_owned());
    }
    let ticker = fetch_ticker(&bridge, &market).await?;
    let account = paper_trading::load_or_open_account_for_currency(&persistence, "KRW")?;
    let mut ledger = persistence.paper_ledger(paper_trading::ledger_id_for_currency("KRW")?)?;
    let state = execute_shadow_order(
        &mut ledger,
        ShadowOrderRequest {
            account_id: account.account_id,
            order_id: format!("coin-paper-{}", request.idempotency_key),
            idempotency_key: request.idempotency_key,
            symbol: market,
            currency: "KRW".to_owned(),
            side: request.side,
            quantity: request.quantity,
            quantity_scale: 100_000_000,
            reference_price_minor: krw_minor(ticker.trade_price)?,
            occurred_at_ms: paper_trading::now_ms()?,
        },
        request.costs,
    )
    .map_err(|error| error.message)?;
    Ok(paper_trading::snapshot(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::{run_backtest_with_risk, BacktestRiskLimits};
    use crate::research::{
        CrossDirection, EvidenceKind, ReferenceEvidence, SignalSpec, StrategySpec,
    };
    use std::{env, path::PathBuf};

    #[test]
    fn accepts_only_upbit_krw_market_codes() {
        assert_eq!(validate_market("krw-btc").unwrap(), "KRW-BTC");
        assert!(validate_market("BTC-USD").is_err());
        assert!(validate_market("KRW-비트코인").is_err());
    }

    #[test]
    fn converts_positive_krw_crypto_prices_without_fabrication() {
        assert_eq!(krw_minor(123_456.4).unwrap(), 123_456);
        assert_eq!(krw_minor(123_456.6).unwrap(), 123_457);
        assert!(krw_minor(f64::NAN).is_err());
        assert!(krw_minor(0.0).is_err());
    }

    #[test]
    fn rejects_malformed_upbit_credentials_before_network_use() {
        assert!(validate_upbit_credentials(UpbitCredentialsRequest {
            access_key: "short".to_owned(),
            secret_key: "also-short".to_owned(),
        })
        .is_err());
        assert!(validate_upbit_credentials(UpbitCredentialsRequest {
            access_key: "valid-access-key-123456".to_owned(),
            secret_key: "invalid secret with spaces".to_owned(),
        })
        .is_err());
    }

    #[test]
    fn creates_an_hs256_upbit_bearer_without_plain_secret_disclosure() {
        let credentials = UpbitCredentials {
            access_key: "test-access-key-123456".to_owned(),
            secret_key: "test-secret-key-123456".to_owned(),
        };
        let bearer = create_upbit_bearer(&credentials).expect("bearer");
        assert!(bearer.starts_with("Bearer "));
        assert_eq!(bearer.trim_start_matches("Bearer ").split('.').count(), 3);
        assert!(!bearer.contains(&credentials.secret_key));
    }

    #[test]
    #[ignore = "업비트 공개 시세 API와 외부 네트워크를 사용하는 명시적 통합 검사"]
    fn live_upbit_public_ticker_returns_the_requested_market() {
        let ticker =
            tauri::async_runtime::block_on(fetch_ticker(&CryptoMarketBridge::default(), "KRW-XRP"))
                .expect("public ticker");
        assert_eq!(ticker.market, "KRW-XRP");
        assert!(ticker.trade_price > 0.0);
        assert!(ticker.timestamp > 0);
    }

    #[test]
    #[ignore = "업비트 공개 캔들 API와 외부 네트워크를 사용하는 명시적 통합 검사"]
    fn live_upbit_daily_candles_run_a_deterministic_crypto_backtest() {
        let snapshot = tauri::async_runtime::block_on(fetch_chart_snapshot(
            CryptoChartRequest {
                symbol: "KRW-BTC".to_owned(),
                interval: "1d".to_owned(),
                count: 200,
            },
            &CryptoMarketBridge::default(),
        ))
        .expect("public daily candles");
        let bars = snapshot
            .bars
            .iter()
            .filter(|bar| bar.completed)
            .map(|bar| PriceBar {
                symbol: snapshot.symbol.clone(),
                currency: "KRW".to_owned(),
                source: snapshot.provider.to_owned(),
                period_start_ms: bar.period_start_ms,
                period_end_ms: bar.period_end_ms,
                available_at_ms: bar.period_end_ms,
                ingested_at_ms: snapshot.fetched_at_ms,
                open_minor: bar.open_minor,
                high_minor: bar.high_minor,
                low_minor: bar.low_minor,
                close_minor: bar.close_minor,
                volume: bar.volume,
            })
            .collect::<Vec<_>>();
        let report = ResearchReport {
            trace_id: "upbit-live-smoke".to_owned(),
            request: "공개 일봉 연결 검사".to_owned(),
            evidence: vec![ReferenceEvidence {
                evidence_id: "upbit-public".to_owned(),
                kind: EvidenceKind::Documentation,
                source_url: "https://docs.upbit.com/".to_owned(),
                revision: None,
                license: None,
                summary: "업비트 공개 캔들".to_owned(),
                claimed_result: None,
            }],
            strategy_candidate: StrategySpec {
                schema_version: "1".to_owned(),
                strategy_id: "crypto-ma-smoke".to_owned(),
                name: "코인 MA 연결 검사".to_owned(),
                market: Market::Crypto,
                symbol: "KRW-BTC".to_owned(),
                currency: "KRW".to_owned(),
                hypothesis: "시스템 연결만 검사한다.".to_owned(),
                source_evidence_ids: vec!["upbit-public".to_owned()],
                entry_signal: SignalSpec::MovingAverageCross {
                    fast_window: 5,
                    slow_window: 20,
                    direction: CrossDirection::Above,
                },
                exit_signal: SignalSpec::MovingAverageCross {
                    fast_window: 5,
                    slow_window: 20,
                    direction: CrossDirection::Below,
                },
                limitations: vec!["투자 성과 판정이 아니다.".to_owned()],
                unknowns: vec![],
            },
        };
        let base_config = BacktestConfig {
            experiment_id: format!("upbit-live-smoke-{}", snapshot.fetched_at_ms),
            dataset_id: format!("upbit-live-smoke-{}", snapshot.fetched_at_ms),
            code_version: "test".to_owned(),
            initial_cash_minor: 100_000_000,
            order_quantity: 10_000_000,
            quantity_scale: 100_000_000,
            close_open_position_at_end: true,
            costs: TradingCosts {
                buy_fee_bps: 5.0,
                sell_fee_bps: 5.0,
                sell_tax_bps: 0.0,
                slippage_bps: 0.0,
            },
            risk_limits: None,
        };
        let result =
            run_backtest(&report.strategy_candidate, &bars, &base_config).expect("crypto backtest");
        assert!(result.input_bar_count >= 100);
        let risk_config = BacktestConfig {
            experiment_id: format!("upbit-live-risk-smoke-{}", snapshot.fetched_at_ms),
            ..base_config.clone()
        };
        let risk_result = run_backtest_with_risk(
            &report.strategy_candidate,
            &bars,
            &risk_config,
            Some(BacktestRiskLimits {
                stop_loss_bps: 500,
                take_profit_bps: 1_000,
                daily_loss_limit_minor: 5_000_000,
            }),
        )
        .expect("crypto risk backtest");
        assert_eq!(risk_result.input_bar_count, result.input_bar_count);
        if let Some(path) = env::var_os("INVESTA_RECORD_BACKTEST_DB").map(PathBuf::from) {
            let persistence = PersistenceBridge::open(&path).expect("backtest persistence");
            let review = review_research_report(&report);
            let warnings = vec![
                "실제 업비트 공개 일봉을 사용한 시스템 연결 검사이며 성과 승격 근거가 아니다."
                    .to_owned(),
            ];
            for (config, stored_result) in [(&base_config, &result), (&risk_config, &risk_result)] {
                persistence
                    .persist_backtest(PersistBacktest {
                        report: &report,
                        review: &review,
                        bars: &bars,
                        config,
                        result: stored_result,
                        provider: &snapshot.provider,
                        interval: &snapshot.interval,
                        adjusted: false,
                        warnings: &warnings,
                        requested_at_ms: None,
                        classification: "system_check",
                    })
                    .expect("persist live system backtest");
            }
        }
        let robustness = result.robustness.as_ref().expect("robustness report");
        println!(
            "UPBIT_BACKTEST_RESULT bars={} return_bps={} mdd_bps={} trades={} win_rate_bps={:?} bootstrap_computed={} bootstrap_p05_bps={:?} bootstrap_loss_probability_bps={:?} bootstrap_ruin_probability_bps={:?} risk_return_bps={} risk_mdd_bps={}",
            result.input_bar_count,
            result.total_return_bps,
            result.max_drawdown_bps,
            result.completed_trade_count,
            result.win_rate_bps,
            robustness.computed,
            robustness.lower_return_bps,
            robustness.probability_of_loss_bps,
            robustness.probability_of_ruin_bps,
            risk_result.total_return_bps,
            risk_result.max_drawdown_bps
        );
    }
}
