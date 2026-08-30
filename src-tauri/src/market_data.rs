use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::{
    backtest::{run_backtest, BacktestConfig, BacktestResult, PriceBar},
    paper_account::{execute_shadow_order, ShadowOrderRequest},
    paper_trading::{self, PaperAccountSnapshot},
    persistence::{PersistBacktest, PersistenceBridge},
    research::{review_research_report, ResearchReport, StrategyReview},
    sec_fundamentals::{self, SecFilingSnapshot, SecFundamentalSnapshot, SecFundamentalsBridge},
    simulation::TradingCosts,
    trading::TradeSide,
};

const API_BASE_URL: &str = "https://openapi.tossinvest.com";
const CREDENTIAL_SERVICE: &str = "com.bumniverse.investa.toss-open-api";
const CLIENT_ID_ACCOUNT: &str = "client-id";
const CLIENT_SECRET_ACCOUNT: &str = "client-secret";
const DEFAULT_REFRESH_AFTER_MS: u64 = 15_000;
const REQUEST_TIMEOUT_SECONDS: u64 = 8;
const STOCK_CATALOG_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const STOCK_SEARCH_LIMIT: usize = 8;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketIndexQuote {
    code: String,
    value: Option<f64>,
    change_percent: Option<f64>,
    observed_at: Option<String>,
    state: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MarketIndexSnapshot {
    provider: Option<String>,
    fetched_at: Option<String>,
    refresh_after_ms: u64,
    message: String,
    quotes: Vec<MarketIndexQuote>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TossConnectionStatus {
    configured: bool,
    connected: bool,
    message: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TossConnectionResult {
    status: TossConnectionStatus,
    snapshot: MarketIndexSnapshot,
}

#[derive(Debug, Deserialize)]
struct TossAccountsEnvelope {
    result: Vec<TossAccount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TossAccount {
    account_no: String,
    account_seq: i64,
    account_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyAmounts {
    krw: String,
    usd: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoldingsMarketValue {
    amount: CurrencyAmounts,
    amount_after_cost: CurrencyAmounts,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoldingsProfitLoss {
    amount: CurrencyAmounts,
    amount_after_cost: CurrencyAmounts,
    rate: String,
    rate_after_cost: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoldingsDailyProfitLoss {
    amount: CurrencyAmounts,
    rate: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoldingItem {
    symbol: String,
    name: String,
    market_country: String,
    currency: String,
    quantity: String,
    last_price: String,
    average_purchase_price: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HoldingsOverview {
    total_purchase_amount: CurrencyAmounts,
    market_value: HoldingsMarketValue,
    profit_loss: HoldingsProfitLoss,
    daily_profit_loss: HoldingsDailyProfitLoss,
    items: Vec<HoldingItem>,
}

#[derive(Debug, Deserialize)]
struct TossHoldingsEnvelope {
    result: HoldingsOverview,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TossAccountOverview {
    account_alias: String,
    masked_account_no: String,
    account_type: String,
    holdings: HoldingsOverview,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TossAccountSnapshot {
    provider: &'static str,
    fetched_at_ms: u64,
    read_only: bool,
    live_order_enabled: bool,
    accounts: Vec<TossAccountOverview>,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TossPaperMarketOrderRequest {
    symbol: String,
    expected_currency: String,
    side: TradeSide,
    quantity: u64,
    idempotency_key: String,
    costs: TradingCosts,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TossChartRequest {
    symbol: String,
    interval: CandleInterval,
    count: u16,
    adjusted: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossChartBar {
    pub(crate) period_start_ms: u64,
    pub(crate) period_end_ms: u64,
    pub(crate) open_minor: u64,
    pub(crate) high_minor: u64,
    pub(crate) low_minor: u64,
    pub(crate) close_minor: u64,
    pub(crate) volume: u64,
    pub(crate) completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) available_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ingested_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) contract_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) settlement_price_minor: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mark_price_minor: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) index_price_minor: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) funding_rate_bps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) funding_time_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossChartSnapshot {
    provider: &'static str,
    symbol: String,
    currency: String,
    interval: &'static str,
    adjusted: bool,
    fetched_at_ms: u64,
    bars: Vec<TossChartBar>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossMarketQuote {
    provider: &'static str,
    symbol: String,
    currency: String,
    last_price_minor: u64,
    observed_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedStock {
    symbol: Option<String>,
    name: Option<String>,
    security_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListedStocksEnvelope {
    result: Vec<ListedStock>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StockSearchResult {
    symbol: String,
    name: String,
    market: String,
    currency: String,
    security_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSearchRequest {
    market: String,
    query: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSnapshotRequest {
    pub(crate) query: String,
    #[serde(default = "default_analysis_bar_count")]
    pub(crate) count: u16,
}

fn default_analysis_bar_count() -> u16 {
    200
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisIndicatorSnapshot {
    pub(crate) sma_5: Option<f64>,
    pub(crate) sma_20: Option<f64>,
    pub(crate) sma_60: Option<f64>,
    pub(crate) rsi_14: Option<f64>,
    pub(crate) atr_14: Option<f64>,
    pub(crate) twenty_day_return_percent: Option<f64>,
    pub(crate) twenty_day_average_volume: Option<f64>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDataAvailability {
    pub(crate) price: String,
    pub(crate) technical: String,
    pub(crate) fundamentals: String,
    pub(crate) filings: String,
    pub(crate) news: String,
    pub(crate) macro_supply: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisSnapshot {
    snapshot_id: String,
    provider: &'static str,
    symbol: String,
    name: String,
    market: String,
    currency: String,
    as_of_ms: u64,
    fetched_at_ms: u64,
    interval: String,
    pub(crate) asset_class: String,
    adjusted: bool,
    pub(crate) completed_bar_count: usize,
    latest_close_minor: u64,
    latest_volume: u64,
    indicators: AnalysisIndicatorSnapshot,
    fundamentals: Option<SecFundamentalSnapshot>,
    filings: Option<SecFilingSnapshot>,
    availability: AnalysisDataAvailability,
    missing_data: Vec<String>,
    pub(crate) bars: Vec<TossChartBar>,
}

#[derive(Debug, Deserialize)]
struct TossPricesEnvelope {
    result: Vec<TossPrice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TossPrice {
    symbol: String,
    last_price: String,
    currency: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TossCredentialsRequest {
    client_id: String,
    client_secret: String,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct MarketIndicatorEnvelope {
    result: Vec<MarketIndicatorPrice>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketIndicatorPrice {
    symbol: String,
    timestamp: Option<String>,
    last_price: String,
}

#[derive(Debug, Deserialize)]
struct MarketCalendarEnvelope<T> {
    result: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarketSessionTime {
    start_time: String,
    end_time: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KrMarketSessions {
    pre_market: Option<MarketSessionTime>,
    regular_market: Option<MarketSessionTime>,
    after_market: Option<MarketSessionTime>,
}

#[derive(Debug, Deserialize)]
struct KrMarketDay {
    date: String,
    integrated: Option<KrMarketSessions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KrMarketCalendarResponse {
    today: KrMarketDay,
    previous_business_day: KrMarketDay,
    next_business_day: KrMarketDay,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsMarketDay {
    date: String,
    day_market: Option<MarketSessionTime>,
    pre_market: Option<MarketSessionTime>,
    regular_market: Option<MarketSessionTime>,
    after_market: Option<MarketSessionTime>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsMarketCalendarResponse {
    today: UsMarketDay,
    previous_business_day: UsMarketDay,
    next_business_day: UsMarketDay,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedMarketSession {
    name: String,
    start_time: String,
    end_time: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedMarketCalendar {
    market: String,
    provider: String,
    fetched_at_ms: u64,
    date: String,
    holiday: bool,
    previous_business_day: String,
    next_business_day: String,
    sessions: Vec<NormalizedMarketSession>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TossMarketCalendars {
    fetched_at_ms: u64,
    calendars: Vec<NormalizedMarketCalendar>,
}

#[derive(Debug, Deserialize)]
struct CandleEnvelope {
    result: CandlePage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CandlePage {
    candles: Vec<TossCandle>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TossCandle {
    timestamp: String,
    open_price: String,
    high_price: String,
    low_price: String,
    close_price: String,
    volume: String,
    currency: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub enum CandleInterval {
    #[serde(rename = "1m")]
    OneMinute,
    #[serde(rename = "1d")]
    OneDay,
}

impl CandleInterval {
    fn as_str(self) -> &'static str {
        match self {
            Self::OneMinute => "1m",
            Self::OneDay => "1d",
        }
    }

    fn duration_ms(self) -> u64 {
        match self {
            Self::OneMinute => 60_000,
            Self::OneDay => 86_400_000,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TossBacktestRequest {
    report: ResearchReport,
    requested_at_ms: Option<u64>,
    interval: CandleInterval,
    count: u16,
    adjusted: bool,
    config: BacktestConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossBacktestRun {
    review: StrategyReview,
    result: BacktestResult,
    provider: String,
    interval: String,
    adjusted: bool,
    warnings: Vec<String>,
}

#[derive(Clone)]
struct TokenCache {
    access_token: String,
    refresh_at: Instant,
}

#[derive(Clone)]
struct TossCredentials {
    client_id: String,
    client_secret: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ApiErrorKind {
    InvalidCredentials,
    IpDenied,
    RateLimited,
    Unauthorized,
    Maintenance,
    Unavailable,
    InvalidResponse,
}

#[derive(Clone, Debug)]
struct ApiError {
    kind: ApiErrorKind,
    message: &'static str,
}

impl ApiError {
    fn new(kind: ApiErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }

    fn provider_status(status: StatusCode, context: &'static str) -> Self {
        let kind = match status {
            StatusCode::UNAUTHORIZED => ApiErrorKind::Unauthorized,
            StatusCode::FORBIDDEN => ApiErrorKind::IpDenied,
            StatusCode::TOO_MANY_REQUESTS => ApiErrorKind::RateLimited,
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => ApiErrorKind::Maintenance,
            _ => ApiErrorKind::Unavailable,
        };
        Self::new(kind, context)
    }
}

pub struct MarketDataBridge {
    client: Client,
    token_cache: Mutex<Option<TokenCache>>,
    last_snapshot: Mutex<Option<MarketIndexSnapshot>>,
    stock_catalogs: Mutex<HashMap<String, (Instant, Vec<StockSearchResult>)>>,
}

impl Default for MarketDataBridge {
    fn default() -> Self {
        Self {
            client: Client::new(),
            token_cache: Mutex::new(None),
            last_snapshot: Mutex::new(None),
            stock_catalogs: Mutex::new(HashMap::new()),
        }
    }
}

impl MarketIndexSnapshot {
    fn unconfigured() -> Self {
        Self {
            provider: None,
            fetched_at: None,
            refresh_after_ms: DEFAULT_REFRESH_AFTER_MS,
            message: "토스증권 Open API 연결 대기".to_owned(),
            quotes: unavailable_quotes(),
        }
    }

    fn delayed(mut self, message: &str) -> Self {
        self.message = message.to_owned();
        for quote in &mut self.quotes {
            if quote.state == "live" {
                quote.state = "delayed".to_owned();
            }
        }
        self
    }
}

fn unavailable_quotes() -> Vec<MarketIndexQuote> {
    ["KOSPI", "KOSDAQ", "NASDAQ"]
        .into_iter()
        .map(|code| MarketIndexQuote {
            code: code.to_owned(),
            value: None,
            change_percent: None,
            observed_at: None,
            state: "unavailable".to_owned(),
        })
        .collect()
}

fn credential_entry(account: &str) -> Result<Entry, String> {
    Entry::new(CREDENTIAL_SERVICE, account)
        .map_err(|_| "Windows 자격 증명 관리자를 열 수 없습니다.".to_owned())
}

fn optional_password(entry: &Entry) -> Result<Option<String>, String> {
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("Windows 자격 증명 관리자에서 연결 정보를 읽지 못했습니다.".to_owned()),
    }
}

fn load_credentials() -> Result<Option<TossCredentials>, String> {
    let client_id = optional_password(&credential_entry(CLIENT_ID_ACCOUNT)?)?;
    let client_secret = optional_password(&credential_entry(CLIENT_SECRET_ACCOUNT)?)?;
    match (client_id, client_secret) {
        (Some(client_id), Some(client_secret)) => Ok(Some(TossCredentials {
            client_id,
            client_secret,
        })),
        (None, None) => Ok(None),
        _ => Err(
            "토스증권 연결 정보가 일부만 저장되어 있습니다. 삭제 후 다시 등록해 주세요.".to_owned(),
        ),
    }
}

fn validate_credentials(request: TossCredentialsRequest) -> Result<TossCredentials, String> {
    let client_id = request.client_id;
    let client_secret = request.client_secret;
    if client_id.trim() != client_id || client_secret.trim() != client_secret {
        return Err("Client ID와 Client Secret 앞뒤의 공백을 제거해 주세요.".to_owned());
    }
    if !(8..=200).contains(&client_id.len()) || !(8..=500).contains(&client_secret.len()) {
        return Err("Client ID 또는 Client Secret의 길이가 올바르지 않습니다.".to_owned());
    }
    if client_id.chars().any(char::is_control) || client_secret.chars().any(char::is_control) {
        return Err(
            "Client ID와 Client Secret에는 줄바꿈이나 제어 문자를 사용할 수 없습니다.".to_owned(),
        );
    }
    Ok(TossCredentials {
        client_id,
        client_secret,
    })
}

fn restore_password(entry: &Entry, previous: Option<&str>) {
    match previous {
        Some(value) => {
            let _ = entry.set_password(value);
        }
        None => {
            let _ = entry.delete_credential();
        }
    }
}

fn store_credentials(credentials: &TossCredentials) -> Result<(), String> {
    let client_id_entry = credential_entry(CLIENT_ID_ACCOUNT)?;
    let client_secret_entry = credential_entry(CLIENT_SECRET_ACCOUNT)?;
    let previous_id = optional_password(&client_id_entry)?;
    let previous_secret = optional_password(&client_secret_entry)?;
    client_id_entry
        .set_password(&credentials.client_id)
        .map_err(|_| "Client ID를 Windows 자격 증명 관리자에 저장하지 못했습니다.".to_owned())?;
    if client_secret_entry
        .set_password(&credentials.client_secret)
        .is_err()
    {
        restore_password(&client_id_entry, previous_id.as_deref());
        restore_password(&client_secret_entry, previous_secret.as_deref());
        return Err("Client Secret을 Windows 자격 증명 관리자에 저장하지 못했습니다.".to_owned());
    }
    Ok(())
}

fn delete_entry(entry: &Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => Err("Windows 자격 증명 관리자에서 연결 정보를 삭제하지 못했습니다.".to_owned()),
    }
}

fn delete_stored_credentials() -> Result<(), String> {
    let client_id_entry = credential_entry(CLIENT_ID_ACCOUNT)?;
    let client_secret_entry = credential_entry(CLIENT_SECRET_ACCOUNT)?;
    let previous_id = optional_password(&client_id_entry)?;
    let previous_secret = optional_password(&client_secret_entry)?;
    delete_entry(&client_id_entry)?;
    if delete_entry(&client_secret_entry).is_err() {
        restore_password(&client_id_entry, previous_id.as_deref());
        restore_password(&client_secret_entry, previous_secret.as_deref());
        return Err("Windows 자격 증명 관리자에서 연결 정보를 삭제하지 못했습니다.".to_owned());
    }
    Ok(())
}

impl MarketDataBridge {
    /// 토스 인증 WebSocket handshake 전용 토큰 조회입니다.
    /// 이 함수는 Rust 내부에서만 사용하고 Tauri command나 직렬화 DTO로 노출하지 않습니다.
    pub(crate) async fn toss_stream_access_token(&self) -> Result<String, String> {
        let credentials = load_credentials()?
            .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
        self.token_for(&credentials)
            .await
            .map_err(|error| error.message.to_owned())
    }

    pub(crate) fn clear_toss_stream_access_token(&self) -> Result<(), String> {
        *self
            .token_cache
            .lock()
            .map_err(|_| "토스증권 인증 상태를 초기화하지 못했습니다.".to_owned())? = None;
        Ok(())
    }

    pub(crate) async fn fetch_latest_strategy_bars(
        &self,
        symbol: &str,
        interval: &str,
    ) -> Result<Vec<PriceBar>, String> {
        let interval = match interval {
            "1m" => CandleInterval::OneMinute,
            "1d" => CandleInterval::OneDay,
            _ => {
                return Err(
                    "저장 전략 감시는 백테스트와 동일한 1분봉 또는 일봉만 지원합니다.".to_owned(),
                )
            }
        };
        let credentials = load_credentials()?
            .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
        let ingested_at_ms = paper_trading::now_ms()?;
        let candles = self
            .fetch_candles_with_credentials(&credentials, symbol, interval, 200, true)
            .await
            .map_err(|error| error.message.to_owned())?;
        price_bars_from_candles(symbol, interval, candles, ingested_at_ms)
    }
    async fn issue_token(&self, credentials: &TossCredentials) -> Result<TokenCache, ApiError> {
        let response = self
            .client
            .post(format!("{API_BASE_URL}/oauth2/token"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", credentials.client_id.as_str()),
                ("client_secret", credentials.client_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 인증 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidCredentials,
                    "Client ID 또는 Client Secret이 올바르지 않습니다.",
                ))
            }
            StatusCode::FORBIDDEN => {
                return Err(ApiError::new(
                    ApiErrorKind::IpDenied,
                    "현재 IP가 토스증권 Open API 허용 목록에 없습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 인증 요청 한도를 초과했습니다. 잠시 후 다시 시도해 주세요.",
                ))
            }
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => {
                return Err(ApiError::provider_status(
                    response.status(),
                    "토스증권 인증 서버가 점검 중이거나 일시 중단되었습니다.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 인증 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        let token = response.json::<OAuthTokenResponse>().await.map_err(|_| {
            ApiError::new(
                ApiErrorKind::InvalidResponse,
                "토스증권 인증 응답 형식이 올바르지 않습니다.",
            )
        })?;
        if token.access_token.is_empty() || token.token_type != "Bearer" || token.expires_in == 0 {
            return Err(ApiError::new(
                ApiErrorKind::InvalidResponse,
                "토스증권 인증 응답에 필요한 값이 없습니다.",
            ));
        }
        Ok(TokenCache {
            access_token: token.access_token,
            refresh_at: Instant::now()
                + Duration::from_secs(token.expires_in.saturating_sub(60).max(1)),
        })
    }

    async fn token_for(&self, credentials: &TossCredentials) -> Result<String, ApiError> {
        if let Some(token) = self
            .token_cache
            .lock()
            .map_err(|_| ApiError::new(ApiErrorKind::Unavailable, "인증 상태를 읽지 못했습니다."))?
            .as_ref()
            .filter(|token| Instant::now() < token.refresh_at)
        {
            return Ok(token.access_token.clone());
        }
        let token = self.issue_token(credentials).await?;
        let access_token = token.access_token.clone();
        *self.token_cache.lock().map_err(|_| {
            ApiError::new(
                ApiErrorKind::Unavailable,
                "인증 상태를 저장하지 못했습니다.",
            )
        })? = Some(token);
        Ok(access_token)
    }

    async fn request_indices(&self, access_token: &str) -> Result<MarketIndexSnapshot, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/market-indicators/prices"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .query(&[("symbols", "KOSPI,KOSDAQ")])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 시세 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 시세 요청 한도를 초과했습니다.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 시세 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        let envelope = response
            .json::<MarketIndicatorEnvelope>()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 시세 응답 형식이 올바르지 않습니다.",
                )
            })?;
        snapshot_from_prices(envelope.result)
    }

    async fn request_calendar<T: DeserializeOwned>(
        &self,
        access_token: &str,
        market: &str,
    ) -> Result<T, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/market-calendar/{market}"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 장 캘린더 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 장 캘린더 요청 한도를 초과했습니다.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 장 캘린더 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        response
            .json::<MarketCalendarEnvelope<T>>()
            .await
            .map(|value| value.result)
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 장 캘린더 응답 형식이 올바르지 않습니다.",
                )
            })
    }

    async fn fetch_market_calendars_with_credentials(
        &self,
        credentials: &TossCredentials,
    ) -> Result<TossMarketCalendars, ApiError> {
        let mut token = self.token_for(credentials).await?;
        let kr = match self
            .request_calendar::<KrMarketCalendarResponse>(&token, "KR")
            .await
        {
            Err(error) if error.kind == ApiErrorKind::Unauthorized => {
                *self.token_cache.lock().map_err(|_| {
                    ApiError::new(
                        ApiErrorKind::Unavailable,
                        "인증 상태를 갱신하지 못했습니다.",
                    )
                })? = None;
                token = self.token_for(credentials).await?;
                self.request_calendar::<KrMarketCalendarResponse>(&token, "KR")
                    .await?
            }
            result => result?,
        };
        let us = self
            .request_calendar::<UsMarketCalendarResponse>(&token, "US")
            .await?;
        let fetched_at_ms = paper_trading::now_ms().map_err(|_| {
            ApiError::new(
                ApiErrorKind::Unavailable,
                "장 캘린더 관측 시각을 만들지 못했습니다.",
            )
        })?;
        Ok(normalize_market_calendars(kr, us, fetched_at_ms)?)
    }

    async fn fetch_indices_with_credentials(
        &self,
        credentials: &TossCredentials,
    ) -> Result<MarketIndexSnapshot, ApiError> {
        let token = self.token_for(credentials).await?;
        match self.request_indices(&token).await {
            Err(error) if error.kind == ApiErrorKind::Unauthorized => {
                *self.token_cache.lock().map_err(|_| {
                    ApiError::new(
                        ApiErrorKind::Unavailable,
                        "인증 상태를 갱신하지 못했습니다.",
                    )
                })? = None;
                let renewed_token = self.token_for(credentials).await?;
                self.request_indices(&renewed_token).await
            }
            result => result,
        }
    }

    async fn request_candles(
        &self,
        access_token: &str,
        symbol: &str,
        interval: CandleInterval,
        count: u16,
        adjusted: bool,
    ) -> Result<Vec<TossCandle>, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/candles"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .query(&[
                ("symbol", symbol.to_owned()),
                ("interval", interval.as_str().to_owned()),
                ("count", count.to_string()),
                ("adjusted", adjusted.to_string()),
            ])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 캔들 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 캔들 요청 한도를 초과했습니다.",
                ))
            }
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "종목 코드 또는 캔들 조회 조건을 확인해 주세요.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 캔들 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        let envelope = response.json::<CandleEnvelope>().await.map_err(|_| {
            ApiError::new(
                ApiErrorKind::InvalidResponse,
                "토스증권 캔들 응답 형식이 올바르지 않습니다.",
            )
        })?;
        Ok(envelope.result.candles)
    }

    async fn fetch_candles_with_credentials(
        &self,
        credentials: &TossCredentials,
        symbol: &str,
        interval: CandleInterval,
        count: u16,
        adjusted: bool,
    ) -> Result<Vec<TossCandle>, ApiError> {
        let token = self.token_for(credentials).await?;
        match self
            .request_candles(&token, symbol, interval, count, adjusted)
            .await
        {
            Err(error) if error.kind == ApiErrorKind::Unauthorized => {
                *self.token_cache.lock().map_err(|_| {
                    ApiError::new(
                        ApiErrorKind::Unavailable,
                        "인증 상태를 갱신하지 못했습니다.",
                    )
                })? = None;
                let renewed_token = self.token_for(credentials).await?;
                self.request_candles(&renewed_token, symbol, interval, count, adjusted)
                    .await
            }
            result => result,
        }
    }

    async fn request_accounts(&self, access_token: &str) -> Result<Vec<TossAccount>, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/accounts"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 계좌 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::FORBIDDEN => {
                return Err(ApiError::new(
                    ApiErrorKind::IpDenied,
                    "현재 IP 또는 API 권한으로 계좌를 조회할 수 없습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 계좌 조회 한도를 초과했습니다.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 계좌 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        response
            .json::<TossAccountsEnvelope>()
            .await
            .map(|envelope| envelope.result)
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 계좌 응답 형식이 올바르지 않습니다.",
                )
            })
    }

    async fn request_holdings(
        &self,
        access_token: &str,
        account_seq: i64,
    ) -> Result<HoldingsOverview, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/holdings"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .header("X-Tossinvest-Account", account_seq.to_string())
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 보유자산 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::FORBIDDEN => {
                return Err(ApiError::new(
                    ApiErrorKind::IpDenied,
                    "현재 IP 또는 API 권한으로 보유자산을 조회할 수 없습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 보유자산 조회 한도를 초과했습니다.",
                ))
            }
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 계좌 식별자 또는 보유자산 요청을 확인해 주세요.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 보유자산 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        response
            .json::<TossHoldingsEnvelope>()
            .await
            .map(|envelope| envelope.result)
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 보유자산 응답 형식이 올바르지 않습니다.",
                )
            })
    }

    async fn request_prices(
        &self,
        access_token: &str,
        symbol: &str,
    ) -> Result<Vec<TossPrice>, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/prices"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .query(&[("symbols", symbol)])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 현재가 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 현재가 조회 한도를 초과했습니다.",
                ))
            }
            StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "종목 코드 또는 현재가 요청을 확인해 주세요.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 현재가 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        response
            .json::<TossPricesEnvelope>()
            .await
            .map(|envelope| envelope.result)
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 현재가 응답 형식이 올바르지 않습니다.",
                )
            })
    }

    async fn request_stock_catalog(
        &self,
        access_token: &str,
        market: &str,
        currency: &str,
    ) -> Result<Vec<StockSearchResult>, ApiError> {
        let response = self
            .client
            .get(format!("{API_BASE_URL}/api/v1/stocks/all"))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .query(&[("market", market), ("status", "ACTIVE")])
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "토스증권 종목 목록 서버에 연결하지 못했습니다.",
                )
            })?;
        match response.status() {
            StatusCode::UNAUTHORIZED => {
                return Err(ApiError::new(
                    ApiErrorKind::Unauthorized,
                    "토스증권 액세스 토큰이 만료되었습니다.",
                ))
            }
            StatusCode::TOO_MANY_REQUESTS => {
                return Err(ApiError::new(
                    ApiErrorKind::RateLimited,
                    "토스증권 종목 목록 요청 한도를 초과했습니다. 잠시 후 다시 검색해 주세요.",
                ))
            }
            status if !status.is_success() => {
                return Err(ApiError::provider_status(
                    status,
                    "토스증권 종목 목록 서버가 요청을 처리하지 못했습니다.",
                ))
            }
            _ => {}
        }
        response
            .json::<ListedStocksEnvelope>()
            .await
            .map(|envelope| {
                envelope
                    .result
                    .into_iter()
                    .filter_map(|stock| {
                        Some(StockSearchResult {
                            symbol: stock.symbol.filter(|value| !value.trim().is_empty())?,
                            name: stock.name.filter(|value| !value.trim().is_empty())?,
                            market: market.to_owned(),
                            currency: currency.to_owned(),
                            security_type: stock
                                .security_type
                                .unwrap_or_else(|| "UNKNOWN".to_owned()),
                        })
                    })
                    .collect()
            })
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 종목 목록 응답 형식이 올바르지 않습니다.",
                )
            })
    }

    async fn load_stock_catalog(
        &self,
        credentials: &TossCredentials,
        market_group: &str,
    ) -> Result<Vec<StockSearchResult>, ApiError> {
        if let Some((loaded_at, stocks)) = self
            .stock_catalogs
            .lock()
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "종목 검색 캐시를 읽지 못했습니다.",
                )
            })?
            .get(market_group)
            .filter(|(loaded_at, _)| loaded_at.elapsed() < STOCK_CATALOG_TTL)
        {
            let _ = loaded_at;
            return Ok(stocks.clone());
        }

        let markets: &[(&str, &str)] = match market_group {
            "kr" => &[("KOSPI", "KRW"), ("KOSDAQ", "KRW")],
            "us" => &[("NASDAQ", "USD"), ("NYSE", "USD"), ("AMEX", "USD")],
            _ => {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "지원하지 않는 종목 검색 시장입니다.",
                ))
            }
        };
        let token = self.token_for(credentials).await?;
        let mut stocks = Vec::new();
        for (index, (market, currency)) in markets.iter().enumerate() {
            if index > 0 {
                tauri::async_runtime::spawn_blocking(|| {
                    std::thread::sleep(Duration::from_millis(1_050))
                })
                .await
                .map_err(|_| {
                    ApiError::new(
                        ApiErrorKind::Unavailable,
                        "종목 목록 조회 간격을 조정하지 못했습니다.",
                    )
                })?;
            }
            stocks.extend(self.request_stock_catalog(&token, market, currency).await?);
        }
        stocks.sort_by(|left, right| left.symbol.cmp(&right.symbol));
        stocks.dedup_by(|left, right| left.symbol == right.symbol);
        self.stock_catalogs
            .lock()
            .map_err(|_| {
                ApiError::new(
                    ApiErrorKind::Unavailable,
                    "종목 검색 캐시를 저장하지 못했습니다.",
                )
            })?
            .insert(market_group.to_owned(), (Instant::now(), stocks.clone()));
        Ok(stocks)
    }

    async fn account_snapshot_with_token(
        &self,
        access_token: &str,
    ) -> Result<Vec<TossAccountOverview>, ApiError> {
        let accounts = self.request_accounts(access_token).await?;
        let mut result = Vec::with_capacity(accounts.len());
        for (index, account) in accounts.into_iter().enumerate() {
            if account.account_seq < 0 || account.account_no.is_empty() {
                return Err(ApiError::new(
                    ApiErrorKind::InvalidResponse,
                    "토스증권 계좌 응답에 유효하지 않은 식별자가 있습니다.",
                ));
            }
            let holdings = self
                .request_holdings(access_token, account.account_seq)
                .await?;
            result.push(TossAccountOverview {
                account_alias: format!("ACCOUNT-{}", index + 1),
                masked_account_no: mask_account_no(&account.account_no),
                account_type: account.account_type,
                holdings,
            });
        }
        Ok(result)
    }

    async fn fetch_account_snapshot_with_credentials(
        &self,
        credentials: &TossCredentials,
    ) -> Result<Vec<TossAccountOverview>, ApiError> {
        let token = self.token_for(credentials).await?;
        match self.account_snapshot_with_token(&token).await {
            Err(error) if error.kind == ApiErrorKind::Unauthorized => {
                *self.token_cache.lock().map_err(|_| {
                    ApiError::new(
                        ApiErrorKind::Unavailable,
                        "인증 상태를 갱신하지 못했습니다.",
                    )
                })? = None;
                let renewed_token = self.token_for(credentials).await?;
                self.account_snapshot_with_token(&renewed_token).await
            }
            result => result,
        }
    }

    async fn fetch_prices_with_credentials(
        &self,
        credentials: &TossCredentials,
        symbol: &str,
    ) -> Result<Vec<TossPrice>, ApiError> {
        let token = self.token_for(credentials).await?;
        match self.request_prices(&token, symbol).await {
            Err(error) if error.kind == ApiErrorKind::Unauthorized => {
                *self.token_cache.lock().map_err(|_| {
                    ApiError::new(
                        ApiErrorKind::Unavailable,
                        "인증 상태를 갱신하지 못했습니다.",
                    )
                })? = None;
                let renewed_token = self.token_for(credentials).await?;
                self.request_prices(&renewed_token, symbol).await
            }
            result => result,
        }
    }

    pub(crate) async fn fetch_krw_current_price(&self, symbol: &str) -> Result<(u64, u64), String> {
        let credentials = load_credentials()?
            .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
        let prices = self
            .fetch_prices_with_credentials(&credentials, symbol)
            .await
            .map_err(|error| error.message.to_owned())?;
        let price = prices
            .into_iter()
            .find(|price| price.symbol.eq_ignore_ascii_case(symbol))
            .ok_or_else(|| "토스증권 현재가 응답에서 요청 종목을 찾지 못했습니다.".to_owned())?;
        if price.currency != "KRW" {
            return Err("현재 내부 모의원장은 KRW 종목만 지원합니다.".to_owned());
        }
        Ok((
            parse_krw_price_minor(&price.last_price)?,
            paper_trading::now_ms()?,
        ))
    }

    fn remember_snapshot(&self, snapshot: &MarketIndexSnapshot) {
        if let Ok(mut stored) = self.last_snapshot.lock() {
            *stored = Some(snapshot.clone());
        }
    }

    fn fallback_snapshot(&self, error: &ApiError) -> MarketIndexSnapshot {
        let message = match error.kind {
            ApiErrorKind::RateLimited => "요청 한도 도달 · 마지막 시세 표시 중",
            ApiErrorKind::Unauthorized => "토큰 갱신 실패 · 마지막 시세 표시 중",
            _ => "토스증권 연결 지연 · 마지막 시세 표시 중",
        };
        self.last_snapshot
            .lock()
            .ok()
            .and_then(|snapshot| snapshot.clone())
            .map(|snapshot| snapshot.delayed(message))
            .unwrap_or_else(|| MarketIndexSnapshot {
                provider: Some("TOSS".to_owned()),
                fetched_at: None,
                refresh_after_ms: DEFAULT_REFRESH_AFTER_MS,
                message: error.message.to_owned(),
                quotes: unavailable_quotes(),
            })
    }
}

fn mask_account_no(account_no: &str) -> String {
    let visible_count = if account_no.chars().count() > 4 { 4 } else { 0 };
    let hidden_count = account_no.chars().count().saturating_sub(visible_count);
    format!(
        "{}{}",
        "*".repeat(hidden_count),
        account_no.chars().skip(hidden_count).collect::<String>()
    )
}

fn parse_krw_price_minor(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("토스증권 원화 현재가가 정수 형식이 아닙니다.".to_owned());
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|price| *price > 0)
        .ok_or_else(|| "토스증권 원화 현재가를 안전하게 계산할 수 없습니다.".to_owned())
}

fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day > month_days[month as usize - 1] {
        return None;
    }
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = i64::from(month) + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

pub(crate) fn parse_rfc3339_ms(value: &str) -> Option<u64> {
    if value.len() < 20
        || value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(10) != Some(&b'T')
    {
        return None;
    }
    let year = value.get(0..4)?.parse::<i64>().ok()?;
    let month = value.get(5..7)?.parse::<u32>().ok()?;
    let day = value.get(8..10)?.parse::<u32>().ok()?;
    let hour = value.get(11..13)?.parse::<u32>().ok()?;
    let minute = value.get(14..16)?.parse::<u32>().ok()?;
    let second = value.get(17..19)?.parse::<u32>().ok()?;
    if value.as_bytes().get(13) != Some(&b':')
        || value.as_bytes().get(16) != Some(&b':')
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let suffix_start = value[19..].find(['Z', '+', '-']).map(|index| index + 19)?;
    let fraction = value.get(19..suffix_start)?;
    let millis = if fraction.is_empty() {
        0_u64
    } else {
        let digits = fraction.strip_prefix('.')?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let padded = format!("{digits:0<3}");
        padded.get(..3)?.parse::<u64>().ok()?
    };
    let offset_seconds = match value.get(suffix_start..)? {
        "Z" => 0_i64,
        suffix if suffix.len() == 6 && suffix.as_bytes().get(3) == Some(&b':') => {
            let sign = if suffix.starts_with('+') {
                1_i64
            } else if suffix.starts_with('-') {
                -1_i64
            } else {
                return None;
            };
            let hours = suffix.get(1..3)?.parse::<i64>().ok()?;
            let minutes = suffix.get(4..6)?.parse::<i64>().ok()?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            sign * (hours * 3_600 + minutes * 60)
        }
        _ => return None,
    };
    let seconds = days_from_civil(year, month, day)?
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second))?
        .checked_sub(offset_seconds)?;
    u64::try_from(seconds)
        .ok()?
        .checked_mul(1_000)?
        .checked_add(millis)
}

fn parse_price_minor(value: &str, currency: &str) -> Option<u64> {
    let decimals = match currency {
        "KRW" => 0,
        "USD" => 2,
        _ => return None,
    };
    if value.starts_with('-') || value.starts_with('+') {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let factor = 10_u64.pow(decimals as u32);
    let whole_minor = whole.parse::<u64>().ok()?.checked_mul(factor)?;
    let retained_fraction = if decimals == 0 {
        0_u64
    } else {
        let retained = fraction.get(..fraction.len().min(decimals))?;
        format!("{retained:0<width$}", width = decimals)
            .parse::<u64>()
            .ok()?
    };
    // 토스 캔들 가격은 미국주식에서 센트보다 세밀한 자릿수를 제공할 수 있다.
    // 내부 원장과 UI의 USD 단위는 센트이므로 부동소수점 없이 반올림한다.
    let round_up = fraction
        .as_bytes()
        .get(decimals)
        .is_some_and(|digit| *digit >= b'5');
    whole_minor
        .checked_add(retained_fraction)?
        .checked_add(u64::from(round_up))
}

fn price_bars_from_candles(
    symbol: &str,
    interval: CandleInterval,
    candles: Vec<TossCandle>,
    ingested_at_ms: u64,
) -> Result<Vec<PriceBar>, String> {
    let mut bars = Vec::with_capacity(candles.len());
    for candle in candles {
        let period_start_ms = parse_rfc3339_ms(&candle.timestamp)
            .ok_or_else(|| "토스증권 캔들 시각을 해석하지 못했습니다.".to_owned())?;
        let period_end_ms = period_start_ms
            .checked_add(interval.duration_ms())
            .ok_or_else(|| "캔들 종료 시각이 범위를 초과했습니다.".to_owned())?;
        // 토스는 진행 중인 최신 봉도 반환할 수 있다. 완료 시각이 수집 시각보다
        // 미래인 봉은 백테스트 입력에서 제외해 부분 봉과 미래정보 사용을 막는다.
        if period_end_ms > ingested_at_ms {
            continue;
        }
        let open_minor = parse_price_minor(&candle.open_price, &candle.currency)
            .ok_or_else(|| "토스증권 캔들 시가를 해석하지 못했습니다.".to_owned())?;
        let high_minor = parse_price_minor(&candle.high_price, &candle.currency)
            .ok_or_else(|| "토스증권 캔들 고가를 해석하지 못했습니다.".to_owned())?;
        let low_minor = parse_price_minor(&candle.low_price, &candle.currency)
            .ok_or_else(|| "토스증권 캔들 저가를 해석하지 못했습니다.".to_owned())?;
        let close_minor = parse_price_minor(&candle.close_price, &candle.currency)
            .ok_or_else(|| "토스증권 캔들 종가를 해석하지 못했습니다.".to_owned())?;
        if low_minor == 0
            || low_minor > open_minor
            || low_minor > close_minor
            || high_minor < open_minor
            || high_minor < close_minor
        {
            return Err("토스증권 캔들 OHLC 관계가 올바르지 않습니다.".to_owned());
        }
        bars.push(PriceBar {
            symbol: symbol.to_owned(),
            currency: candle.currency,
            source: "TOSS_OPEN_API".to_owned(),
            period_start_ms,
            period_end_ms,
            available_at_ms: period_end_ms,
            ingested_at_ms,
            open_minor,
            high_minor,
            low_minor,
            close_minor,
            volume: candle
                .volume
                .parse::<u64>()
                .map_err(|_| "토스증권 캔들 거래량을 해석하지 못했습니다.".to_owned())?,
        });
    }
    bars.sort_by_key(|bar| bar.period_start_ms);
    for index in 0..bars.len().saturating_sub(1) {
        let next_start = bars[index + 1].period_start_ms;
        if next_start > bars[index].period_start_ms && next_start < bars[index].period_end_ms {
            bars[index].period_end_ms = next_start;
            bars[index].available_at_ms = next_start;
        }
    }
    Ok(bars)
}

fn validate_market_symbol(symbol: &str) -> Result<String, String> {
    let normalized = symbol.trim().to_ascii_uppercase();
    if normalized.is_empty()
        || normalized.len() > 24
        || !normalized
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err("영문·숫자·점·하이픈으로 된 유효한 종목 코드가 필요합니다.".to_owned());
    }
    Ok(normalized)
}

fn chart_bars_from_candles(
    interval: CandleInterval,
    candles: Vec<TossCandle>,
    fetched_at_ms: u64,
) -> Result<(String, Vec<TossChartBar>), String> {
    let currency = candles
        .first()
        .map(|candle| candle.currency.clone())
        .ok_or_else(|| "조회된 캔들이 없습니다.".to_owned())?;
    let mut bars = Vec::with_capacity(candles.len());
    for candle in candles {
        if candle.currency != currency {
            return Err("서로 다른 통화의 캔들이 한 응답에 섞여 있습니다.".to_owned());
        }
        let period_start_ms = parse_rfc3339_ms(&candle.timestamp)
            .ok_or_else(|| "토스증권 캔들 시각을 해석하지 못했습니다.".to_owned())?;
        let period_end_ms = period_start_ms
            .checked_add(interval.duration_ms())
            .ok_or_else(|| "캔들 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        let open_minor = parse_price_minor(&candle.open_price, &currency)
            .ok_or_else(|| "토스증권 캔들 시가를 해석하지 못했습니다.".to_owned())?;
        let high_minor = parse_price_minor(&candle.high_price, &currency)
            .ok_or_else(|| "토스증권 캔들 고가를 해석하지 못했습니다.".to_owned())?;
        let low_minor = parse_price_minor(&candle.low_price, &currency)
            .ok_or_else(|| "토스증권 캔들 저가를 해석하지 못했습니다.".to_owned())?;
        let close_minor = parse_price_minor(&candle.close_price, &currency)
            .ok_or_else(|| "토스증권 캔들 종가를 해석하지 못했습니다.".to_owned())?;
        if low_minor == 0
            || low_minor > open_minor
            || low_minor > close_minor
            || high_minor < open_minor
            || high_minor < close_minor
        {
            return Err("토스증권 캔들 OHLC 관계가 올바르지 않습니다.".to_owned());
        }
        bars.push(TossChartBar {
            period_start_ms,
            period_end_ms,
            open_minor,
            high_minor,
            low_minor,
            close_minor,
            volume: candle
                .volume
                .parse::<u64>()
                .map_err(|_| "토스증권 캔들 거래량을 해석하지 못했습니다.".to_owned())?,
            completed: period_end_ms <= fetched_at_ms,
            available_at_ms: Some(period_end_ms),
            ingested_at_ms: Some(fetched_at_ms),
            session_id: None,
            contract_code: None,
            settlement_price_minor: None,
            mark_price_minor: None,
            index_price_minor: None,
            funding_rate_bps: None,
            funding_time_ms: None,
        });
    }
    bars.sort_by_key(|bar| bar.period_start_ms);
    Ok((currency, bars))
}

fn snapshot_from_prices(
    prices: Vec<MarketIndicatorPrice>,
) -> Result<MarketIndexSnapshot, ApiError> {
    let mut quotes = unavailable_quotes();
    let mut fetched_at = None;
    for price in prices {
        if !matches!(price.symbol.as_str(), "KOSPI" | "KOSDAQ") {
            continue;
        }
        let value = price.last_price.parse::<f64>().map_err(|_| {
            ApiError::new(
                ApiErrorKind::InvalidResponse,
                "토스증권 시세 값을 해석하지 못했습니다.",
            )
        })?;
        if !value.is_finite() || value < 0.0 {
            return Err(ApiError::new(
                ApiErrorKind::InvalidResponse,
                "토스증권 시세 값이 유효하지 않습니다.",
            ));
        }
        if fetched_at.is_none() {
            fetched_at = price.timestamp.clone();
        }
        if let Some(quote) = quotes.iter_mut().find(|quote| quote.code == price.symbol) {
            quote.value = Some(value);
            quote.observed_at = price.timestamp;
            quote.state = "live".to_owned();
        }
    }
    if quotes.iter().take(2).any(|quote| quote.value.is_none()) {
        return Err(ApiError::new(
            ApiErrorKind::InvalidResponse,
            "토스증권 지수 응답에 KOSPI 또는 KOSDAQ이 없습니다.",
        ));
    }
    Ok(MarketIndexSnapshot {
        provider: Some("TOSS".to_owned()),
        fetched_at,
        refresh_after_ms: DEFAULT_REFRESH_AFTER_MS,
        message: "KOSPI·KOSDAQ 토스증권 시세 · NASDAQ 연결 대기".to_owned(),
        quotes,
    })
}

fn validate_calendar_date(value: &str) -> Result<(), ApiError> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, value)| index != 4 && index != 7 && !value.is_ascii_digit())
    {
        return Err(ApiError::new(
            ApiErrorKind::InvalidResponse,
            "토스증권 장 캘린더 날짜 형식이 올바르지 않습니다.",
        ));
    }
    Ok(())
}

fn parse_official_session_timestamp_ms(value: &str) -> Result<u64, ApiError> {
    if !(value.ends_with("+09:00") || value.ends_with('Z')) {
        return Err(ApiError::new(
            ApiErrorKind::InvalidResponse,
            "토스증권 장 세션 시간대가 올바르지 않습니다.",
        ));
    }
    parse_rfc3339_ms(value).ok_or_else(|| {
        ApiError::new(
            ApiErrorKind::InvalidResponse,
            "토스증권 장 세션 시각 형식이 올바르지 않습니다.",
        )
    })
}

fn normalize_calendar_session(
    name: &str,
    value: Option<MarketSessionTime>,
) -> Result<Option<NormalizedMarketSession>, ApiError> {
    let Some(value) = value else { return Ok(None) };
    let start_time_ms = parse_official_session_timestamp_ms(&value.start_time)?;
    let end_time_ms = parse_official_session_timestamp_ms(&value.end_time)?;
    if start_time_ms >= end_time_ms {
        return Err(ApiError::new(
            ApiErrorKind::InvalidResponse,
            "토스증권 장 세션 시간이 올바르지 않습니다.",
        ));
    }
    Ok(Some(NormalizedMarketSession {
        name: name.to_owned(),
        start_time: value.start_time,
        end_time: value.end_time,
    }))
}

fn ensure_regular_market_session(
    calendars: &TossMarketCalendars,
    currency: &str,
    now_ms: u64,
) -> Result<(), String> {
    let market = match currency {
        "KRW" => "KR",
        "USD" => "US",
        _ => return Err("국장·미장 시장가 주문 통화를 확인해 주세요.".to_owned()),
    };
    if calendars.fetched_at_ms > now_ms.saturating_add(60_000)
        || now_ms.saturating_sub(calendars.fetched_at_ms) > 300_000
    {
        return Err(
            "공식 장 캘린더 관측이 오래됐거나 미래 시각입니다. 다시 조회해 주세요.".to_owned(),
        );
    }
    let calendar = calendars
        .calendars
        .iter()
        .find(|calendar| calendar.market == market)
        .ok_or_else(|| "공식 장 캘린더에서 선택 시장을 찾지 못했습니다.".to_owned())?;
    if calendar.holiday {
        return Err(format!("{} 시장은 공식 장 캘린더상 휴장입니다.", market));
    }
    let regular = calendar
        .sessions
        .iter()
        .find(|session| session.name == "regularMarket")
        .ok_or_else(|| "정규장 세션이 확인되지 않아 시장가 모의체결을 차단했습니다.".to_owned())?;
    let start_ms = parse_official_session_timestamp_ms(&regular.start_time)
        .map_err(|error| error.message.to_owned())?;
    let end_ms = parse_official_session_timestamp_ms(&regular.end_time)
        .map_err(|error| error.message.to_owned())?;
    if now_ms < start_ms || now_ms >= end_ms {
        return Err("현재 정규장 시간이 아니므로 즉시 시장가 모의체결을 차단했습니다. 지정가 대기 주문은 사용할 수 있습니다.".to_owned());
    }
    Ok(())
}

fn normalize_market_calendars(
    kr: KrMarketCalendarResponse,
    us: UsMarketCalendarResponse,
    fetched_at_ms: u64,
) -> Result<TossMarketCalendars, ApiError> {
    for date in [
        &kr.today.date,
        &kr.previous_business_day.date,
        &kr.next_business_day.date,
        &us.today.date,
        &us.previous_business_day.date,
        &us.next_business_day.date,
    ] {
        validate_calendar_date(date)?;
    }
    let mut kr_sessions = Vec::new();
    if let Some(integrated) = kr.today.integrated {
        for session in [
            normalize_calendar_session("preMarket", integrated.pre_market)?,
            normalize_calendar_session("regularMarket", integrated.regular_market)?,
            normalize_calendar_session("afterMarket", integrated.after_market)?,
        ]
        .into_iter()
        .flatten()
        {
            kr_sessions.push(session);
        }
    }
    let mut us_sessions = Vec::new();
    for session in [
        normalize_calendar_session("dayMarket", us.today.day_market)?,
        normalize_calendar_session("preMarket", us.today.pre_market)?,
        normalize_calendar_session("regularMarket", us.today.regular_market)?,
        normalize_calendar_session("afterMarket", us.today.after_market)?,
    ]
    .into_iter()
    .flatten()
    {
        us_sessions.push(session);
    }
    Ok(TossMarketCalendars {
        fetched_at_ms,
        calendars: vec![
            NormalizedMarketCalendar {
                market: "KR".to_owned(),
                provider: "TOSS".to_owned(),
                fetched_at_ms,
                date: kr.today.date,
                holiday: kr_sessions.is_empty(),
                previous_business_day: kr.previous_business_day.date,
                next_business_day: kr.next_business_day.date,
                sessions: kr_sessions,
            },
            NormalizedMarketCalendar {
                market: "US".to_owned(),
                provider: "TOSS".to_owned(),
                fetched_at_ms,
                date: us.today.date,
                holiday: us_sessions.is_empty(),
                previous_business_day: us.previous_business_day.date,
                next_business_day: us.next_business_day.date,
                sessions: us_sessions,
            },
        ],
    })
}

#[tauri::command]
pub fn toss_connection_status() -> Result<TossConnectionStatus, String> {
    match load_credentials()? {
        Some(_) => Ok(TossConnectionStatus {
            configured: true,
            connected: false,
            message: "연결 정보 저장됨 · 시세 요청 시 연결 확인".to_owned(),
        }),
        None => Ok(TossConnectionStatus {
            configured: false,
            connected: false,
            message: "토스증권 Open API 연결 정보가 없습니다.".to_owned(),
        }),
    }
}

#[tauri::command]
pub async fn toss_save_credentials(
    request: TossCredentialsRequest,
    bridge: State<'_, MarketDataBridge>,
) -> Result<TossConnectionResult, String> {
    let credentials = validate_credentials(request)?;
    // 새 연결 정보는 기존 캐시 토큰과 섞지 않고 직접 검증한다.
    let token = bridge
        .issue_token(&credentials)
        .await
        .map_err(|error| error.message.to_owned())?;
    let snapshot = bridge
        .request_indices(&token.access_token)
        .await
        .map_err(|error| error.message.to_owned())?;
    store_credentials(&credentials)?;
    *bridge
        .token_cache
        .lock()
        .map_err(|_| "인증 상태를 저장하지 못했습니다.".to_owned())? = Some(token);
    bridge.remember_snapshot(&snapshot);
    Ok(TossConnectionResult {
        status: TossConnectionStatus {
            configured: true,
            connected: true,
            message: "토스증권 KOSPI·KOSDAQ 연결 확인 완료".to_owned(),
        },
        snapshot,
    })
}

#[tauri::command]
pub fn toss_delete_credentials(
    bridge: State<'_, MarketDataBridge>,
) -> Result<TossConnectionStatus, String> {
    delete_stored_credentials()?;
    *bridge
        .token_cache
        .lock()
        .map_err(|_| "인증 상태를 초기화하지 못했습니다.".to_owned())? = None;
    *bridge
        .last_snapshot
        .lock()
        .map_err(|_| "시세 상태를 초기화하지 못했습니다.".to_owned())? = None;
    Ok(TossConnectionStatus {
        configured: false,
        connected: false,
        message: "토스증권 연결 정보를 삭제했습니다.".to_owned(),
    })
}

#[tauri::command]
pub async fn market_indices_snapshot(
    bridge: State<'_, MarketDataBridge>,
) -> Result<MarketIndexSnapshot, String> {
    let credentials = match load_credentials() {
        Ok(Some(credentials)) => credentials,
        Ok(None) => return Ok(MarketIndexSnapshot::unconfigured()),
        Err(message) => {
            return Ok(MarketIndexSnapshot {
                provider: Some("TOSS".to_owned()),
                fetched_at: None,
                refresh_after_ms: DEFAULT_REFRESH_AFTER_MS,
                message,
                quotes: unavailable_quotes(),
            })
        }
    };
    match bridge.fetch_indices_with_credentials(&credentials).await {
        Ok(snapshot) => {
            bridge.remember_snapshot(&snapshot);
            Ok(snapshot)
        }
        Err(error) => Ok(bridge.fallback_snapshot(&error)),
    }
}

#[tauri::command]
pub async fn toss_market_calendars(
    bridge: State<'_, MarketDataBridge>,
) -> Result<TossMarketCalendars, String> {
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    bridge
        .fetch_market_calendars_with_credentials(&credentials)
        .await
        .map_err(|error| error.message.to_owned())
}

#[tauri::command]
pub async fn toss_account_snapshot(
    bridge: State<'_, MarketDataBridge>,
) -> Result<TossAccountSnapshot, String> {
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let accounts = bridge
        .fetch_account_snapshot_with_credentials(&credentials)
        .await
        .map_err(|error| error.message.to_owned())?;
    Ok(TossAccountSnapshot {
        provider: "TOSS_OPEN_API",
        fetched_at_ms: paper_trading::now_ms()?,
        read_only: true,
        live_order_enabled: paper_trading::LIVE_ORDER_ENABLED,
        message: if accounts.is_empty() {
            "조회 가능한 종합매매 계좌가 없습니다.".to_owned()
        } else {
            format!(
                "토스증권 계좌 {}개와 보유자산을 읽기 전용으로 확인했습니다.",
                accounts.len()
            )
        },
        accounts,
    })
}

#[tauri::command]
pub async fn toss_chart_snapshot(
    request: TossChartRequest,
    bridge: State<'_, MarketDataBridge>,
) -> Result<TossChartSnapshot, String> {
    let symbol = validate_market_symbol(&request.symbol)?;
    if !(20..=500).contains(&request.count) {
        return Err("차트 캔들 개수는 20개에서 500개 사이여야 합니다.".to_owned());
    }
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let candles = bridge
        .fetch_candles_with_credentials(
            &credentials,
            &symbol,
            request.interval,
            request.count,
            request.adjusted,
        )
        .await
        .map_err(|error| error.message.to_owned())?;
    let fetched_at_ms = paper_trading::now_ms()?;
    let (currency, bars) = chart_bars_from_candles(request.interval, candles, fetched_at_ms)?;
    Ok(TossChartSnapshot {
        provider: "TOSS_OPEN_API",
        symbol,
        currency,
        interval: request.interval.as_str(),
        adjusted: request.adjusted,
        fetched_at_ms,
        bars,
    })
}

#[tauri::command]
pub async fn toss_market_quote(
    symbol: String,
    bridge: State<'_, MarketDataBridge>,
) -> Result<TossMarketQuote, String> {
    let symbol = validate_market_symbol(&symbol)?;
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let price = bridge
        .fetch_prices_with_credentials(&credentials, &symbol)
        .await
        .map_err(|error| error.message.to_owned())?
        .into_iter()
        .find(|price| price.symbol.eq_ignore_ascii_case(&symbol))
        .ok_or_else(|| "토스증권 현재가 응답에서 요청 종목을 찾지 못했습니다.".to_owned())?;
    let last_price_minor = parse_price_minor(&price.last_price, &price.currency)
        .ok_or_else(|| "토스증권 현재가를 해석하지 못했습니다.".to_owned())?;
    Ok(TossMarketQuote {
        provider: "TOSS_OPEN_API",
        symbol,
        currency: price.currency,
        last_price_minor,
        observed_at_ms: paper_trading::now_ms()?,
    })
}

fn normalize_stock_search(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect()
}

fn stock_aliases(symbol: &str) -> &'static [&'static str] {
    match symbol {
        "000660" => &["하이닉스", "SK하이닉스", "에스케이하이닉스"],
        "005930" => &["삼성", "삼성전자"],
        "AAPL" => &["애플", "APPLE"],
        "MSFT" => &["마이크로소프트", "MICROSOFT"],
        "GOOGL" | "GOOG" => &["구글", "알파벳", "GOOGLE", "ALPHABET"],
        "TSLA" => &["테슬라", "TESLA"],
        "NVDA" => &["엔비디아", "NVIDIA"],
        "AMZN" => &["아마존", "AMAZON"],
        "META" => &["메타", "페이스북", "META", "FACEBOOK"],
        _ => &[],
    }
}

fn stock_match_score(stock: &StockSearchResult, query: &str) -> Option<u8> {
    let symbol = normalize_stock_search(&stock.symbol);
    let name = normalize_stock_search(&stock.name);
    let aliases: Vec<String> = stock_aliases(&stock.symbol)
        .iter()
        .map(|alias| normalize_stock_search(alias))
        .collect();
    if symbol == query || name == query || aliases.iter().any(|alias| alias == query) {
        Some(0)
    } else if symbol.starts_with(query)
        || name.starts_with(query)
        || aliases.iter().any(|alias| alias.starts_with(query))
    {
        Some(1)
    } else if symbol.contains(query)
        || name.contains(query)
        || aliases.iter().any(|alias| alias.contains(query))
    {
        Some(2)
    } else {
        None
    }
}

fn search_stock_catalog(stocks: &[StockSearchResult], query: &str) -> Vec<StockSearchResult> {
    let query = normalize_stock_search(query);
    if query.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<(u8, &StockSearchResult)> = stocks
        .iter()
        .filter_map(|stock| stock_match_score(stock, &query).map(|score| (score, stock)))
        .collect();
    matches.sort_by(|(left_score, left), (right_score, right)| {
        left_score
            .cmp(right_score)
            .then_with(|| left.name.chars().count().cmp(&right.name.chars().count()))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    matches
        .into_iter()
        .take(STOCK_SEARCH_LIMIT)
        .map(|(_, stock)| stock.clone())
        .collect()
}

fn resolve_stock_from_text(stocks: &[StockSearchResult], query: &str) -> Option<StockSearchResult> {
    let normalized_query = normalize_stock_search(query);
    let mut matches: Vec<(usize, &StockSearchResult)> = stocks
        .iter()
        .filter_map(|stock| {
            let mut terms = vec![
                normalize_stock_search(&stock.symbol),
                normalize_stock_search(&stock.name),
            ];
            terms.extend(
                stock_aliases(&stock.symbol)
                    .iter()
                    .map(|value| normalize_stock_search(value)),
            );
            terms
                .into_iter()
                .filter(|term| term.chars().count() >= 2 && normalized_query.contains(term))
                .map(|term| term.chars().count())
                .max()
                .map(|length| (length, stock))
        })
        .collect();
    matches.sort_by(|(left_length, left), (right_length, right)| {
        right_length
            .cmp(left_length)
            .then_with(|| left.symbol.cmp(&right.symbol))
    });
    matches.first().map(|(_, stock)| (*stock).clone())
}

fn explicit_kr_stock_from_text(query: &str) -> Option<StockSearchResult> {
    let trimmed = query.trim();
    let explicit_symbol = if trimmed.len() == 6 && trimmed.bytes().all(|byte| byte.is_ascii_digit())
    {
        Some(trimmed)
    } else {
        query.split('(').skip(1).find_map(|suffix| {
            let candidate = suffix.split_once(')')?.0.trim();
            (candidate.len() == 6 && candidate.bytes().all(|byte| byte.is_ascii_digit()))
                .then_some(candidate)
        })
    }?;
    Some(StockSearchResult {
        symbol: explicit_symbol.to_owned(),
        name: explicit_symbol.to_owned(),
        market: "KRX".to_owned(),
        currency: "KRW".to_owned(),
        security_type: "STOCK".to_owned(),
    })
}

fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

pub(crate) fn analysis_indicators(bars: &[TossChartBar]) -> AnalysisIndicatorSnapshot {
    let closes: Vec<f64> = bars.iter().map(|bar| bar.close_minor as f64).collect();
    let sma = |period: usize| {
        closes
            .len()
            .checked_sub(period)
            .and_then(|start| average(&closes[start..]))
    };
    let rsi_14 = if closes.len() >= 15 {
        let changes = &closes[closes.len() - 15..];
        let (gains, losses) = changes
            .windows(2)
            .fold((0.0, 0.0), |(gains, losses), pair| {
                let change = pair[1] - pair[0];
                (gains + change.max(0.0), losses + (-change).max(0.0))
            });
        if losses == 0.0 {
            Some(100.0)
        } else {
            Some(100.0 - 100.0 / (1.0 + gains / losses))
        }
    } else {
        None
    };
    let atr_14 = if bars.len() >= 15 {
        let sample = &bars[bars.len() - 15..];
        let ranges: Vec<f64> = sample
            .windows(2)
            .map(|pair| {
                let current = &pair[1];
                let previous_close = pair[0].close_minor as f64;
                let intraday = (current.high_minor - current.low_minor) as f64;
                intraday
                    .max((current.high_minor as f64 - previous_close).abs())
                    .max((current.low_minor as f64 - previous_close).abs())
            })
            .collect();
        average(&ranges)
    } else {
        None
    };
    let twenty_day_return_percent = if closes.len() >= 21 {
        let base = closes[closes.len() - 21];
        (base > 0.0).then(|| (closes[closes.len() - 1] / base - 1.0) * 100.0)
    } else {
        None
    };
    let twenty_day_average_volume = if bars.len() >= 20 {
        average(
            &bars[bars.len() - 20..]
                .iter()
                .map(|bar| bar.volume as f64)
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    AnalysisIndicatorSnapshot {
        sma_5: sma(5),
        sma_20: sma(20),
        sma_60: sma(60),
        rsi_14,
        atr_14,
        twenty_day_return_percent,
        twenty_day_average_volume,
    }
}

pub(crate) fn public_market_analysis_snapshot(
    provider: &'static str,
    symbol: String,
    name: String,
    market: String,
    asset_class: &str,
    currency: String,
    interval: String,
    fetched_at_ms: u64,
    bars: Vec<TossChartBar>,
    mut missing_data: Vec<String>,
) -> Result<AnalysisSnapshot, String> {
    let completed_bars: Vec<TossChartBar> = bars.into_iter().filter(|bar| bar.completed).collect();
    let latest = completed_bars
        .last()
        .ok_or_else(|| "완료된 봉이 없어 분석 스냅샷을 만들 수 없습니다.".to_owned())?;
    if completed_bars.len() < 20 {
        return Err("분석 스냅샷에는 완료된 봉이 최소 20개 필요합니다.".to_owned());
    }
    let latest_close_minor = latest.close_minor;
    let latest_volume = latest.volume;
    missing_data.extend([
        "공식 재무 공급자 미연결".to_owned(),
        "공식 공시 공급자 미연결".to_owned(),
        "뉴스 공급자 미연결".to_owned(),
        "수급·거시 공급자 미연결".to_owned(),
    ]);
    Ok(AnalysisSnapshot {
        snapshot_id: format!(
            "{}-{}-{}",
            provider.to_ascii_lowercase(),
            symbol,
            fetched_at_ms
        ),
        provider,
        symbol,
        name,
        market,
        currency,
        as_of_ms: fetched_at_ms,
        fetched_at_ms,
        interval,
        asset_class: asset_class.to_owned(),
        adjusted: false,
        completed_bar_count: completed_bars.len(),
        latest_close_minor,
        latest_volume,
        indicators: analysis_indicators(&completed_bars),
        fundamentals: None,
        filings: None,
        availability: AnalysisDataAvailability {
            price: "available".to_owned(),
            technical: "available".to_owned(),
            fundamentals: "provider_not_connected".to_owned(),
            filings: "provider_not_connected".to_owned(),
            news: "provider_not_connected".to_owned(),
            macro_supply: "provider_not_connected".to_owned(),
        },
        missing_data,
        bars: completed_bars,
    })
}

#[tauri::command]
pub async fn toss_analysis_snapshot(
    request: AnalysisSnapshotRequest,
    bridge: State<'_, MarketDataBridge>,
    sec_bridge: State<'_, SecFundamentalsBridge>,
) -> Result<AnalysisSnapshot, String> {
    let query = request.query.trim();
    if query.is_empty() || query.chars().count() > 500 || query.chars().any(char::is_control) {
        return Err("분석 요청은 한 줄 1자 이상 500자 이하여야 합니다.".to_owned());
    }
    if !(60..=500).contains(&request.count) {
        return Err("분석 스냅샷 캔들은 60개에서 500개 사이여야 합니다.".to_owned());
    }
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let mut candidates = explicit_kr_stock_from_text(query)
        .into_iter()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        for market in ["kr", "us"] {
            let stocks = bridge
                .load_stock_catalog(&credentials, market)
                .await
                .map_err(|error| error.message.to_owned())?;
            if let Some(stock) = resolve_stock_from_text(&stocks, query) {
                candidates.push(stock);
            }
        }
    }
    if candidates.is_empty() {
        return Err("분석 요청에서 국장·미장 종목명 또는 티커를 확정하지 못했습니다.".to_owned());
    }
    if candidates.len() > 1 {
        return Err("분석 요청에 국장·미장 종목이 함께 감지됐습니다. 공통 스냅샷은 한 종목씩 요청해 주세요.".to_owned());
    }
    let stock = candidates.remove(0);
    let candles = bridge
        .fetch_candles_with_credentials(
            &credentials,
            &stock.symbol,
            CandleInterval::OneDay,
            request.count,
            true,
        )
        .await
        .map_err(|error| error.message.to_owned())?;
    let fetched_at_ms = paper_trading::now_ms()?;
    let (currency, bars) = chart_bars_from_candles(CandleInterval::OneDay, candles, fetched_at_ms)?;
    let completed_bars: Vec<TossChartBar> = bars.into_iter().filter(|bar| bar.completed).collect();
    let latest = completed_bars
        .last()
        .ok_or_else(|| "완료된 일봉이 없어 분석 스냅샷을 만들 수 없습니다.".to_owned())?;
    let as_of_ms = latest.period_end_ms;
    let latest_close_minor = latest.close_minor;
    let latest_volume = latest.volume;
    let indicators = analysis_indicators(&completed_bars);
    let (fundamentals, fundamentals_state, fundamentals_missing) = if stock.currency == "USD" {
        match sec_fundamentals::snapshot_for_ticker(&sec_bridge, &stock.symbol, as_of_ms).await {
            Ok(Some(snapshot)) if !snapshot.metrics.is_empty() => {
                let missing = if snapshot.missing_metrics.is_empty() {
                    None
                } else {
                    Some(format!(
                        "SEC 일부 재무 항목 결측: {}",
                        snapshot.missing_metrics.join(", ")
                    ))
                };
                (Some(snapshot), "available", missing)
            }
            Ok(Some(snapshot)) => (
                Some(snapshot),
                "provider_no_point_in_time_data",
                Some("분석 기준일 이전 SEC 정기보고서 재무 항목이 없습니다.".to_owned()),
            ),
            Ok(None) => (
                None,
                "provider_not_configured",
                Some("SEC 요청 연락처 미등록".to_owned()),
            ),
            Err(_) => (
                None,
                "provider_error",
                Some("SEC 공식 재무 조회 실패".to_owned()),
            ),
        }
    } else {
        (
            None,
            "provider_not_connected",
            Some("국장 공식 재무 공급자 미연결".to_owned()),
        )
    };
    let (filings, filings_state, filings_missing) = if stock.currency == "USD" {
        match sec_fundamentals::filings_for_ticker(&sec_bridge, &stock.symbol, as_of_ms).await {
            Ok(Some(snapshot)) if !snapshot.filings.is_empty() => {
                (Some(snapshot), "available", None)
            }
            Ok(Some(snapshot)) => (
                Some(snapshot),
                "provider_no_point_in_time_data",
                Some("분석 기준일 이전 SEC 주요 공시가 없습니다.".to_owned()),
            ),
            Ok(None) => (
                None,
                "provider_not_configured",
                Some("SEC 요청 연락처 미등록".to_owned()),
            ),
            Err(_) => (
                None,
                "provider_error",
                Some("SEC 공식 공시 조회 실패".to_owned()),
            ),
        }
    } else {
        (
            None,
            "provider_not_connected",
            Some("국장 공식 공시 공급자 미연결".to_owned()),
        )
    };
    let mut missing_data = vec![
        "뉴스 공급자 미연결".to_owned(),
        "수급·거시 공급자 미연결".to_owned(),
    ];
    if let Some(missing) = fundamentals_missing {
        missing_data.insert(0, missing);
    }
    if let Some(missing) = filings_missing {
        missing_data.insert(0, missing);
    }
    Ok(AnalysisSnapshot {
        snapshot_id: format!("toss-{}-{}", stock.symbol, as_of_ms),
        provider: "TOSS_OPEN_API",
        symbol: stock.symbol,
        name: stock.name,
        market: stock.market,
        currency,
        as_of_ms,
        fetched_at_ms,
        interval: "1d".to_owned(),
        asset_class: "equity".to_owned(),
        adjusted: true,
        completed_bar_count: completed_bars.len(),
        latest_close_minor,
        latest_volume,
        indicators,
        fundamentals,
        filings,
        availability: AnalysisDataAvailability {
            price: "available".to_owned(),
            technical: "available".to_owned(),
            fundamentals: fundamentals_state.to_owned(),
            filings: filings_state.to_owned(),
            news: "provider_not_connected".to_owned(),
            macro_supply: "provider_not_connected".to_owned(),
        },
        missing_data,
        bars: completed_bars,
    })
}

#[tauri::command]
pub async fn toss_search_stocks(
    request: StockSearchRequest,
    bridge: State<'_, MarketDataBridge>,
) -> Result<Vec<StockSearchResult>, String> {
    let market = request.market.trim().to_ascii_lowercase();
    let query = request.query.trim();
    if !matches!(market.as_str(), "kr" | "us") {
        return Err("국장 또는 미장 검색만 지원합니다.".to_owned());
    }
    if query.chars().count() < 1 || query.chars().count() > 60 {
        return Err("검색어는 1자에서 60자 사이로 입력해 주세요.".to_owned());
    }
    if query.chars().any(char::is_control) {
        return Err("검색어에 줄바꿈이나 제어 문자를 사용할 수 없습니다.".to_owned());
    }
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let stocks = bridge
        .load_stock_catalog(&credentials, &market)
        .await
        .map_err(|error| error.message.to_owned())?;
    Ok(search_stock_catalog(&stocks, query))
}

#[tauri::command]
pub async fn toss_execute_paper_market_order(
    request: TossPaperMarketOrderRequest,
    bridge: State<'_, MarketDataBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PaperAccountSnapshot, String> {
    let symbol = validate_market_symbol(&request.symbol)?;
    if request.quantity == 0
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 120
        || !request
            .idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("유효한 종목 코드와 1주 이상의 모의주문 수량이 필요합니다.".to_owned());
    }
    if !matches!(request.expected_currency.as_str(), "KRW" | "USD") {
        return Err("국장·미장 시장가 주문 통화를 확인해 주세요.".to_owned());
    }
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let calendars = bridge
        .fetch_market_calendars_with_credentials(&credentials)
        .await
        .map_err(|error| error.message.to_owned())?;
    let prices = bridge
        .fetch_prices_with_credentials(&credentials, &symbol)
        .await
        .map_err(|error| error.message.to_owned())?;
    let price = prices
        .into_iter()
        .find(|price| price.symbol.eq_ignore_ascii_case(&symbol))
        .ok_or_else(|| "토스증권 현재가 응답에서 요청 종목을 찾지 못했습니다.".to_owned())?;
    if !matches!(request.expected_currency.as_str(), "KRW" | "USD")
        || price.currency != request.expected_currency
    {
        return Err("선택한 시장의 통화와 토스증권 종목 통화가 일치하지 않습니다.".to_owned());
    }
    let reference_price_minor = parse_price_minor(&price.last_price, &price.currency)
        .ok_or_else(|| "토스증권 현재가를 해석하지 못했습니다.".to_owned())?;
    let occurred_at_ms = paper_trading::now_ms()?;
    ensure_regular_market_session(&calendars, &request.expected_currency, occurred_at_ms)?;
    let account = paper_trading::load_or_open_account_for_currency(&persistence, &price.currency)?;
    let mut ledger =
        persistence.paper_ledger(paper_trading::ledger_id_for_currency(&price.currency)?)?;
    let state = execute_shadow_order(
        &mut ledger,
        ShadowOrderRequest {
            account_id: account.account_id,
            order_id: format!("paper-{}", request.idempotency_key),
            idempotency_key: request.idempotency_key,
            symbol,
            currency: price.currency,
            side: request.side,
            quantity: request.quantity,
            quantity_scale: 1,
            reference_price_minor,
            occurred_at_ms,
        },
        request.costs,
    )
    .map_err(|error| error.message)?;
    Ok(paper_trading::snapshot(state))
}

#[tauri::command]
pub async fn toss_run_research_backtest(
    request: TossBacktestRequest,
    bridge: State<'_, MarketDataBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<TossBacktestRun, String> {
    run_research_backtest(request, &bridge, &persistence, "research_experiment").await
}

async fn run_research_backtest(
    mut request: TossBacktestRequest,
    bridge: &MarketDataBridge,
    persistence: &PersistenceBridge,
    classification: &str,
) -> Result<TossBacktestRun, String> {
    if !(20..=200).contains(&request.count) {
        return Err("캔들 조회 수는 20~200개여야 합니다.".to_owned());
    }
    let review = review_research_report(&request.report);
    if !review.executable {
        return Err(
            "검증 오류나 미해결 항목이 있는 연구 보고서는 백테스트할 수 없습니다.".to_owned(),
        );
    }
    let credentials = load_credentials()?
        .ok_or_else(|| "토스증권 Open API 연결 정보를 먼저 등록해 주세요.".to_owned())?;
    let spec = &request.report.strategy_candidate;
    let candles = bridge
        .fetch_candles_with_credentials(
            &credentials,
            &spec.symbol,
            request.interval,
            request.count,
            request.adjusted,
        )
        .await
        .map_err(|error| error.message.to_owned())?;
    let ingested_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "현재 시각을 확인하지 못했습니다.".to_owned())?
        .as_millis()
        .try_into()
        .map_err(|_| "현재 시각이 지원 범위를 초과했습니다.".to_owned())?;
    let bars = price_bars_from_candles(&spec.symbol, request.interval, candles, ingested_at_ms)?;
    let used_automatic_position_sizing = request.config.order_quantity == 0;
    if used_automatic_position_sizing {
        let first_open = bars
            .first()
            .map(|bar| bar.open_minor)
            .filter(|value| *value > 0)
            .ok_or_else(|| "자동 주문 수량을 계산할 첫 시가가 없습니다.".to_owned())?;
        let target_notional = request.config.initial_cash_minor / 5;
        let scaled_quantity = u128::from(target_notional)
            .checked_mul(u128::from(request.config.quantity_scale))
            .ok_or_else(|| "자동 주문 수량 계산이 지원 범위를 초과했습니다.".to_owned())?
            / u128::from(first_open);
        request.config.order_quantity = u64::try_from(scaled_quantity)
            .map_err(|_| "자동 주문 수량 계산이 지원 범위를 초과했습니다.".to_owned())?
            .max(1);
    }
    let result = run_backtest(spec, &bars, &request.config)
        .map_err(|error| format!("백테스트를 실행하지 못했습니다: {}", error.message))?;
    let costs = request.config.costs;
    let mut warnings = vec![
        "최대 200개 최신 캔들만 사용하는 탐색 백테스트이며 성과 합격 판정은 하지 않습니다."
            .to_owned(),
    ];
    if used_automatic_position_sizing {
        warnings.push(format!(
            "자동 연구 실행은 첫 시가 기준 초기 예수금의 20% 이내인 {}주로 고정 수량을 산정했습니다.",
            request.config.order_quantity
        ));
    }
    if matches!(request.interval, CandleInterval::OneMinute) {
        warnings.push(
            "1분봉 200개는 약 3시간 20분의 짧은 구간이므로 강건성·승격 근거로 사용할 수 없습니다."
                .to_owned(),
        );
    }
    if costs.buy_fee_bps == 0.0
        && costs.sell_fee_bps == 0.0
        && costs.sell_tax_bps == 0.0
        && costs.slippage_bps == 0.0
    {
        warnings.push(
            "거래 비용이 모두 0bp입니다. 실제 비용을 입력하기 전 결과를 승격 근거로 사용할 수 없습니다."
                .to_owned(),
        );
    }
    let run = TossBacktestRun {
        review,
        result,
        provider: "TOSS_OPEN_API".to_owned(),
        interval: request.interval.as_str().to_owned(),
        adjusted: request.adjusted,
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
        classification,
    })?;
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::{
        CrossDirection, EvidenceKind, Market, ReferenceEvidence, SignalSpec, StrategySpec,
    };
    use std::{env, fs, path::PathBuf};

    #[test]
    fn unconfigured_snapshot_never_returns_fake_prices() {
        let snapshot = MarketIndexSnapshot::unconfigured();
        assert_eq!(snapshot.provider, None);
        assert_eq!(snapshot.quotes.len(), 3);
        assert!(snapshot.quotes.iter().all(|quote| quote.value.is_none()));
        assert!(snapshot
            .quotes
            .iter()
            .all(|quote| quote.state == "unavailable"));
    }

    #[test]
    #[ignore = "연결된 토스증권 자격정보와 외부 네트워크를 사용하는 명시적 통합 검사"]
    fn live_toss_smoke_backtest_uses_real_daily_candles() {
        let symbol =
            env::var("INVESTA_LIVE_BACKTEST_SYMBOL").unwrap_or_else(|_| "005930".to_owned());
        let use_kr_costs =
            env::var("INVESTA_LIVE_BACKTEST_USE_KR_COSTS").is_ok_and(|value| value == "1");
        let order_quantity = env::var("INVESTA_LIVE_BACKTEST_ORDER_QUANTITY")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1);
        let run_id: u64 = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_millis()
            .try_into()
            .expect("supported timestamp");
        let record_database = env::var_os("INVESTA_RECORD_BACKTEST_DB").map(PathBuf::from);
        let database_path = record_database.clone().unwrap_or_else(|| {
            env::temp_dir().join(format!(
                "investa-live-backtest-{}-{run_id}.sqlite3",
                std::process::id()
            ))
        });
        let persistence = PersistenceBridge::open(&database_path).expect("backtest persistence");
        let report = ResearchReport {
            trace_id: format!("system-smoke-{run_id}"),
            request: "토스증권 실제 수정주가 일봉과 전체 백테스트 파이프라인의 연결 상태를 확인한다. 투자 성과 합격 판정이 아니다.".to_owned(),
            evidence: vec![ReferenceEvidence {
                evidence_id: "toss-api-docs".to_owned(),
                kind: EvidenceKind::Documentation,
                source_url: "https://developers.tossinvest.com/".to_owned(),
                revision: None,
                license: None,
                summary: "토스증권 Open API를 실제 시세 입력 공급자로 사용한다.".to_owned(),
                claimed_result: None,
            }],
            strategy_candidate: StrategySpec {
                schema_version: "1".to_owned(),
                strategy_id: "system-smoke-ma-5-20".to_owned(),
                name: "시스템 연결 확인용 5/20 이동평균 교차".to_owned(),
                market: Market::Korea,
                symbol: symbol.clone(),
                currency: "KRW".to_owned(),
                hypothesis: "고정된 이동평균 규칙으로 데이터 수집·신호·체결·성과 계산의 연결만 확인한다.".to_owned(),
                source_evidence_ids: vec!["toss-api-docs".to_owned()],
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
                limitations: vec![
                    if use_kr_costs {
                        "최신 200개 일봉, 1주, 국내주식 기본 비용 프리셋을 사용하는 시스템 연결 검사다."
                            .to_owned()
                    } else {
                        "최신 200개 일봉, 1주, 거래비용 0bp를 사용하는 시스템 연결 검사다."
                            .to_owned()
                    },
                    "수익률이나 승률을 투자전략 채택 기준으로 사용하지 않는다.".to_owned(),
                ],
                unknowns: vec![],
            },
        };
        let request = TossBacktestRequest {
            report,
            requested_at_ms: None,
            interval: CandleInterval::OneDay,
            count: 200,
            adjusted: true,
            config: BacktestConfig {
                experiment_id: format!("system-smoke-{run_id}"),
                dataset_id: format!("toss-{symbol}-adjusted-{run_id}"),
                code_version: env!("CARGO_PKG_VERSION").to_owned(),
                initial_cash_minor: 100_000_000,
                order_quantity,
                quantity_scale: 1,
                close_open_position_at_end: true,
                costs: if use_kr_costs {
                    TradingCosts {
                        buy_fee_bps: 1.5,
                        sell_fee_bps: 1.5,
                        sell_tax_bps: 20.0,
                        slippage_bps: 0.0,
                    }
                } else {
                    TradingCosts {
                        buy_fee_bps: 0.0,
                        sell_fee_bps: 0.0,
                        sell_tax_bps: 0.0,
                        slippage_bps: 0.0,
                    }
                },
                risk_limits: None,
            },
        };

        let result = tauri::async_runtime::block_on(run_research_backtest(
            request,
            &MarketDataBridge::default(),
            &persistence,
            "system_check",
        ))
        .expect("live Toss backtest");
        println!(
            "LIVE_BACKTEST_RESULT bars={} return_bps={} mdd_bps={} trades={} win_rate_bps={:?}",
            result.result.input_bar_count,
            result.result.total_return_bps,
            result.result.max_drawdown_bps,
            result.result.completed_trade_count,
            result.result.win_rate_bps
        );
        if order_quantity == 0 {
            assert!(result.result.fills.iter().all(|fill| fill.quantity > 1));
            assert!(result
                .warnings
                .iter()
                .any(|warning| warning.contains("초기 예수금의 20%")));
        }
        drop(persistence);
        if record_database.is_none() {
            let _ = fs::remove_file(database_path);
        }
    }

    #[test]
    #[ignore = "연결된 토스증권 자격정보와 외부 계좌 서버를 사용하는 명시적 읽기 전용 검사"]
    fn live_toss_account_snapshot_is_read_only_and_masked() {
        let credentials = load_credentials()
            .expect("credential store")
            .expect("stored Toss credentials");
        let accounts = tauri::async_runtime::block_on(
            MarketDataBridge::default().fetch_account_snapshot_with_credentials(&credentials),
        )
        .expect("Toss read-only account snapshot");
        assert!(accounts
            .iter()
            .all(|account| account.masked_account_no.contains('*')));
    }

    #[test]
    #[ignore = "연결된 토스증권 자격정보와 외부 장 캘린더 서버를 사용하는 명시적 읽기 전용 검사"]
    fn live_toss_market_calendars_use_official_sessions() {
        let credentials = load_credentials()
            .expect("credential store")
            .expect("stored Toss credentials");
        let calendars = tauri::async_runtime::block_on(
            MarketDataBridge::default().fetch_market_calendars_with_credentials(&credentials),
        )
        .expect("Toss market calendars");
        assert_eq!(calendars.calendars.len(), 2);
        assert!(calendars
            .calendars
            .iter()
            .all(|calendar| calendar.provider == "TOSS"));
        assert!(calendars
            .calendars
            .iter()
            .all(|calendar| calendar.holiday || !calendar.sessions.is_empty()));
    }

    #[test]
    fn official_prices_leave_unsupported_fields_empty() {
        let snapshot = snapshot_from_prices(vec![
            MarketIndicatorPrice {
                symbol: "KOSPI".to_owned(),
                timestamp: Some("2026-08-21T15:30:00+09:00".to_owned()),
                last_price: "2812.45".to_owned(),
            },
            MarketIndicatorPrice {
                symbol: "KOSDAQ".to_owned(),
                timestamp: Some("2026-08-21T15:30:00+09:00".to_owned()),
                last_price: "845.32".to_owned(),
            },
        ])
        .expect("official response should parse");
        assert_eq!(snapshot.quotes[0].value, Some(2812.45));
        assert_eq!(snapshot.quotes[1].value, Some(845.32));
        assert_eq!(snapshot.quotes[0].change_percent, None);
        assert_eq!(snapshot.quotes[2].value, None);
    }

    #[test]
    fn official_market_calendar_preserves_holiday_and_overnight_sessions() {
        let session = |start: &str, end: &str| {
            Some(MarketSessionTime {
                start_time: start.to_owned(),
                end_time: end.to_owned(),
            })
        };
        let kr = KrMarketCalendarResponse {
            today: KrMarketDay {
                date: "2026-05-05".to_owned(),
                integrated: None,
            },
            previous_business_day: KrMarketDay {
                date: "2026-05-04".to_owned(),
                integrated: None,
            },
            next_business_day: KrMarketDay {
                date: "2026-05-06".to_owned(),
                integrated: None,
            },
        };
        let us = UsMarketCalendarResponse {
            today: UsMarketDay {
                date: "2026-03-25".to_owned(),
                day_market: None,
                pre_market: None,
                regular_market: session("2026-03-25T22:30:00+09:00", "2026-03-26T05:00:00+09:00"),
                after_market: None,
            },
            previous_business_day: UsMarketDay {
                date: "2026-03-24".to_owned(),
                day_market: None,
                pre_market: None,
                regular_market: None,
                after_market: None,
            },
            next_business_day: UsMarketDay {
                date: "2026-03-26".to_owned(),
                day_market: None,
                pre_market: None,
                regular_market: None,
                after_market: None,
            },
        };
        let normalized = normalize_market_calendars(kr, us, 123).expect("official calendar");
        assert!(normalized.calendars[0].holiday);
        assert!(!normalized.calendars[1].holiday);
        assert_eq!(normalized.calendars[1].sessions[0].name, "regularMarket");
        assert_eq!(
            normalized.calendars[1].sessions[0].end_time,
            "2026-03-26T05:00:00+09:00"
        );
    }

    #[test]
    fn market_calendar_rejects_malformed_dates_and_session_times() {
        assert_eq!(
            parse_official_session_timestamp_ms("1970-01-01T09:00:00+09:00").unwrap(),
            0
        );
        assert_eq!(
            parse_official_session_timestamp_ms("1970-01-01T00:00:00Z").unwrap(),
            0
        );
        assert!(validate_calendar_date("2026/03/25").is_err());
        assert!(parse_official_session_timestamp_ms("2026-02-30T09:00:00+09:00").is_err());
        assert!(normalize_calendar_session(
            "regularMarket",
            Some(MarketSessionTime {
                start_time: "2026-03-25T15:30:00+09:00".to_owned(),
                end_time: "2026-03-25T09:00:00+09:00".to_owned(),
            })
        )
        .is_err());
    }

    #[test]
    fn stock_paper_market_order_gate_allows_only_fresh_regular_session() {
        let session = NormalizedMarketSession {
            name: "regularMarket".to_owned(),
            start_time: "2026-08-27T09:00:00+09:00".to_owned(),
            end_time: "2026-08-27T15:30:00+09:00".to_owned(),
        };
        let now = parse_official_session_timestamp_ms("2026-08-27T10:00:00+09:00").unwrap();
        let calendars = TossMarketCalendars {
            fetched_at_ms: now,
            calendars: vec![NormalizedMarketCalendar {
                market: "KR".to_owned(),
                provider: "TOSS".to_owned(),
                fetched_at_ms: now,
                date: "2026-08-27".to_owned(),
                holiday: false,
                previous_business_day: "2026-08-26".to_owned(),
                next_business_day: "2026-08-28".to_owned(),
                sessions: vec![session],
            }],
        };
        assert!(ensure_regular_market_session(&calendars, "KRW", now).is_ok());
        assert!(ensure_regular_market_session(
            &calendars,
            "KRW",
            parse_official_session_timestamp_ms("2026-08-27T16:00:00+09:00").unwrap()
        )
        .is_err());

        let mut holiday = calendars.clone();
        holiday.calendars[0].holiday = true;
        assert!(ensure_regular_market_session(&holiday, "KRW", now).is_err());

        let mut stale = calendars;
        stale.fetched_at_ms = now - 300_001;
        assert!(ensure_regular_market_session(&stale, "KRW", now).is_err());
    }

    #[test]
    fn masks_toss_account_numbers_before_they_cross_ipc() {
        assert_eq!(mask_account_no("12345678901"), "*******8901");
        assert_eq!(mask_account_no("123"), "***");
    }

    #[test]
    fn accepts_only_positive_whole_krw_prices_for_the_krw_ledger() {
        assert_eq!(parse_krw_price_minor("72000").unwrap(), 72_000);
        assert!(parse_krw_price_minor("72.50").is_err());
        assert!(parse_krw_price_minor("0").is_err());
    }

    #[test]
    fn chart_contract_keeps_ohlcv_and_marks_incomplete_bars() {
        let start = parse_rfc3339_ms("2026-08-21T15:30:00+09:00").expect("time");
        let (currency, bars) = chart_bars_from_candles(
            CandleInterval::OneDay,
            vec![TossCandle {
                timestamp: "2026-08-21T15:30:00+09:00".to_owned(),
                open_price: "70000".to_owned(),
                high_price: "73000".to_owned(),
                low_price: "69000".to_owned(),
                close_price: "72000".to_owned(),
                volume: "123456".to_owned(),
                currency: "KRW".to_owned(),
            }],
            start + 1_000,
        )
        .expect("chart bars");
        assert_eq!(currency, "KRW");
        assert_eq!(bars[0].high_minor, 73_000);
        assert_eq!(bars[0].low_minor, 69_000);
        assert_eq!(bars[0].volume, 123_456);
        assert!(!bars[0].completed);
    }

    #[test]
    fn parses_the_official_account_and_holdings_contract_without_exposing_account_seq() {
        let accounts: TossAccountsEnvelope = serde_json::from_value(serde_json::json!({
            "result": [{"accountNo": "12345678901", "accountSeq": 1, "accountType": "BROKERAGE"}]
        }))
        .expect("official account example");
        assert_eq!(accounts.result[0].account_seq, 1);

        let holdings: TossHoldingsEnvelope = serde_json::from_value(serde_json::json!({
            "result": {
                "totalPurchaseAmount": {"krw": "6500000", "usd": null},
                "marketValue": {"amount": {"krw": "7200000", "usd": null}, "amountAfterCost": {"krw": "7050000", "usd": null}},
                "profitLoss": {"amount": {"krw": "700000", "usd": null}, "amountAfterCost": {"krw": "550000", "usd": null}, "rate": "0.1077", "rateAfterCost": "0.0846"},
                "dailyProfitLoss": {"amount": {"krw": "100000", "usd": null}, "rate": "0.0141"},
                "items": [{"symbol": "005930", "name": "삼성전자", "marketCountry": "KR", "currency": "KRW", "quantity": "100", "lastPrice": "72000", "averagePurchasePrice": "65000", "marketValue": {}, "profitLoss": {}, "dailyProfitLoss": {}, "cost": {}}]
            }
        }))
        .expect("official holdings example");
        assert_eq!(holdings.result.items[0].symbol, "005930");
        assert_eq!(holdings.result.market_value.amount.krw, "7200000");
    }

    #[test]
    fn invalid_or_incomplete_market_response_is_rejected() {
        let missing = snapshot_from_prices(vec![MarketIndicatorPrice {
            symbol: "KOSPI".to_owned(),
            timestamp: None,
            last_price: "2812.45".to_owned(),
        }]);
        assert!(missing.is_err());
        let malformed = snapshot_from_prices(vec![
            MarketIndicatorPrice {
                symbol: "KOSPI".to_owned(),
                timestamp: None,
                last_price: "not-a-number".to_owned(),
            },
            MarketIndicatorPrice {
                symbol: "KOSDAQ".to_owned(),
                timestamp: None,
                last_price: "845.32".to_owned(),
            },
        ]);
        assert!(malformed.is_err());
    }

    #[test]
    fn credential_validation_rejects_whitespace_and_control_characters() {
        assert!(validate_credentials(TossCredentialsRequest {
            client_id: " client-id".to_owned(),
            client_secret: "secret-value".to_owned()
        })
        .is_err());
        assert!(validate_credentials(TossCredentialsRequest {
            client_id: "client-id".to_owned(),
            client_secret: "secret\nvalue".to_owned()
        })
        .is_err());
    }

    #[test]
    fn parses_toss_candle_time_and_currency_minor_units() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T09:00:00+09:00"), Some(0));
        assert_eq!(parse_price_minor("72000", "KRW"), Some(72_000));
        assert_eq!(parse_price_minor("72000.49", "KRW"), Some(72_000));
        assert_eq!(parse_price_minor("72000.50", "KRW"), Some(72_001));
        assert_eq!(parse_price_minor("185.70", "USD"), Some(18_570));
        assert_eq!(parse_price_minor("185.701", "USD"), Some(18_570));
        assert_eq!(parse_price_minor("185.705", "USD"), Some(18_571));
        assert_eq!(parse_price_minor("0.0049", "USD"), Some(0));
        assert_eq!(parse_price_minor("0.005", "USD"), Some(1));
    }

    #[test]
    fn normalizes_fractional_us_stock_candles_to_cents() {
        let (currency, bars) = chart_bars_from_candles(
            CandleInterval::OneDay,
            vec![TossCandle {
                timestamp: "2026-03-25T09:00:00-04:00".to_owned(),
                open_price: "9.1234".to_owned(),
                high_price: "9.5678".to_owned(),
                low_price: "8.9999".to_owned(),
                close_price: "9.4321".to_owned(),
                volume: "3521000".to_owned(),
                currency: "USD".to_owned(),
            }],
            1_800_000_000_000,
        )
        .expect("fractional US candle");

        assert_eq!(currency, "USD");
        assert_eq!(bars.len(), 1);
        assert_eq!(bars[0].open_minor, 912);
        assert_eq!(bars[0].high_minor, 957);
        assert_eq!(bars[0].low_minor, 900);
        assert_eq!(bars[0].close_minor, 943);
    }

    #[test]
    fn normalizes_newest_first_candles_to_point_in_time_bars() {
        let bars = price_bars_from_candles(
            "005930",
            CandleInterval::OneDay,
            vec![
                TossCandle {
                    timestamp: "2026-03-25T09:00:00+09:00".to_owned(),
                    open_price: "71600".to_owned(),
                    high_price: "72300".to_owned(),
                    low_price: "71500".to_owned(),
                    close_price: "72000".to_owned(),
                    volume: "3521000".to_owned(),
                    currency: "KRW".to_owned(),
                },
                TossCandle {
                    timestamp: "2026-03-24T09:00:00+09:00".to_owned(),
                    open_price: "71200".to_owned(),
                    high_price: "71800".to_owned(),
                    low_price: "71000".to_owned(),
                    close_price: "71600".to_owned(),
                    volume: "2984000".to_owned(),
                    currency: "KRW".to_owned(),
                },
            ],
            1_800_000_000_000,
        )
        .expect("valid candles");

        assert_eq!(bars.len(), 2);
        assert!(bars[0].period_start_ms < bars[1].period_start_ms);
        assert_eq!(bars[0].available_at_ms, bars[0].period_end_ms);
        assert_eq!(bars[0].source, "TOSS_OPEN_API");
    }

    #[test]
    fn excludes_an_incomplete_candle_at_ingestion_time() {
        let bars = price_bars_from_candles(
            "005930",
            CandleInterval::OneDay,
            vec![TossCandle {
                timestamp: "2026-03-25T09:00:00+09:00".to_owned(),
                open_price: "71600".to_owned(),
                high_price: "72300".to_owned(),
                low_price: "71500".to_owned(),
                close_price: "72000".to_owned(),
                volume: "3521000".to_owned(),
                currency: "KRW".to_owned(),
            }],
            parse_rfc3339_ms("2026-03-25T12:00:00+09:00").expect("fixture time"),
        )
        .expect("valid candle response");

        assert!(bars.is_empty());
    }

    #[test]
    fn uses_the_next_observed_start_when_daily_candles_are_less_than_24_hours_apart() {
        let candle = |timestamp: &str| TossCandle {
            timestamp: timestamp.to_owned(),
            open_price: "71600".to_owned(),
            high_price: "72300".to_owned(),
            low_price: "71500".to_owned(),
            close_price: "72000".to_owned(),
            volume: "3521000".to_owned(),
            currency: "KRW".to_owned(),
        };
        let bars = price_bars_from_candles(
            "005930",
            CandleInterval::OneDay,
            vec![
                candle("2026-03-24T15:30:00+09:00"),
                candle("2026-03-25T09:00:00+09:00"),
            ],
            parse_rfc3339_ms("2026-03-27T09:00:00+09:00").expect("fixture time"),
        )
        .expect("valid candles");

        assert_eq!(bars[0].period_end_ms, bars[1].period_start_ms);
        assert_eq!(bars[0].available_at_ms, bars[1].period_start_ms);
    }

    #[test]
    fn stock_search_matches_partial_names_korean_aliases_and_tickers() {
        let stocks = vec![
            StockSearchResult {
                symbol: "000660".to_owned(),
                name: "SK하이닉스".to_owned(),
                market: "KOSPI".to_owned(),
                currency: "KRW".to_owned(),
                security_type: "STOCK".to_owned(),
            },
            StockSearchResult {
                symbol: "AAPL".to_owned(),
                name: "Apple Inc.".to_owned(),
                market: "NASDAQ".to_owned(),
                currency: "USD".to_owned(),
                security_type: "STOCK".to_owned(),
            },
        ];

        assert_eq!(
            search_stock_catalog(&stocks, "하이닉스")[0].symbol,
            "000660"
        );
        assert_eq!(search_stock_catalog(&stocks, "애플")[0].symbol, "AAPL");
        assert_eq!(search_stock_catalog(&stocks, "aap")[0].symbol, "AAPL");
        assert_eq!(
            search_stock_catalog(&stocks, "SK 하이닉스")[0].symbol,
            "000660"
        );
    }

    #[test]
    fn stock_search_prefers_exact_and_prefix_matches_and_caps_results() {
        let mut stocks = (0..12)
            .map(|index| StockSearchResult {
                symbol: format!("TEST{index}"),
                name: format!("Test Company {index}"),
                market: "NASDAQ".to_owned(),
                currency: "USD".to_owned(),
                security_type: "STOCK".to_owned(),
            })
            .collect::<Vec<_>>();
        stocks.push(StockSearchResult {
            symbol: "TEST".to_owned(),
            name: "Exact".to_owned(),
            market: "NASDAQ".to_owned(),
            currency: "USD".to_owned(),
            security_type: "STOCK".to_owned(),
        });

        let matches = search_stock_catalog(&stocks, "test");
        assert_eq!(matches.len(), STOCK_SEARCH_LIMIT);
        assert_eq!(matches[0].symbol, "TEST");
    }

    #[test]
    fn analysis_request_resolves_a_stock_without_requiring_an_exact_query() {
        let stocks = vec![StockSearchResult {
            symbol: "000660".to_owned(),
            name: "SK하이닉스".to_owned(),
            market: "KOSPI".to_owned(),
            currency: "KRW".to_owned(),
            security_type: "STOCK".to_owned(),
        }];
        let resolved = resolve_stock_from_text(&stocks, "SK하이닉스 최근 추세와 위험을 분석해줘")
            .expect("stock in natural request");
        assert_eq!(resolved.symbol, "000660");
        assert!(resolve_stock_from_text(&stocks, "종목을 분석해줘").is_none());
    }

    #[test]
    fn analysis_request_uses_an_explicit_kr_symbol_without_catalog_lookup() {
        let stock = explicit_kr_stock_from_text("한화(000880) 보유 포지션을 분석해")
            .expect("explicit Korean stock symbol");
        assert_eq!(stock.symbol, "000880");
        assert_eq!(stock.market, "KRX");
        assert_eq!(stock.currency, "KRW");
        assert!(explicit_kr_stock_from_text("현재가 123800원 분석").is_none());
    }

    #[test]
    fn analysis_indicators_use_only_completed_point_in_time_bars() {
        let bars = (0..80)
            .map(|index| TossChartBar {
                period_start_ms: index * 86_400_000,
                period_end_ms: (index + 1) * 86_400_000,
                open_minor: 10_000 + index,
                high_minor: 10_100 + index,
                low_minor: 9_900 + index,
                close_minor: 10_000 + index,
                volume: 1_000 + index,
                completed: true,
                available_at_ms: Some((index + 1) * 86_400_000),
                ingested_at_ms: Some(81 * 86_400_000),
                session_id: None,
                contract_code: None,
                settlement_price_minor: None,
                mark_price_minor: None,
                index_price_minor: None,
                funding_rate_bps: None,
                funding_time_ms: None,
            })
            .collect::<Vec<_>>();
        let indicators = analysis_indicators(&bars);
        assert!(indicators.sma_60.is_some());
        assert!(indicators.rsi_14.is_some());
        assert!(indicators.atr_14.is_some());
        assert_eq!(indicators.twenty_day_average_volume, Some(1_069.5));
    }
}
#[cfg(test)]
mod broker_error_tests {
    use super::*;

    #[test]
    fn broker_errors_separate_permission_rate_limit_and_maintenance() {
        assert_eq!(
            ApiError::provider_status(StatusCode::FORBIDDEN, "permission").kind,
            ApiErrorKind::IpDenied
        );
        assert_eq!(
            ApiError::provider_status(StatusCode::TOO_MANY_REQUESTS, "rate").kind,
            ApiErrorKind::RateLimited
        );
        assert_eq!(
            ApiError::provider_status(StatusCode::SERVICE_UNAVAILABLE, "maintenance").kind,
            ApiErrorKind::Maintenance
        );
    }
}
