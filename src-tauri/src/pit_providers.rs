use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, State};

use crate::{paper_trading, persistence::PersistenceBridge, pit_dataset::PitPriceObservation};

const UPBIT_API_BASE: &str = "https://api.upbit.com";
const BINANCE_SPOT_BASE: &str = "https://api.binance.com";
const BINANCE_USDM_BASE: &str = "https://fapi.binance.com";
const BINANCE_COINM_BASE: &str = "https://dapi.binance.com";
const REQUEST_TIMEOUT_SECONDS: u64 = 12;
const PRICE_SCALE: u64 = 100_000_000;
const MAX_COLLECTION_PAGES_PER_RUN: u8 = 5;
const MAX_COLLECTION_FAILURES: u8 = 4;
const COLLECTION_LEASE_TIMEOUT_MS: u64 = 60_000;
const COLLECTION_RETRY_BASE_MS: u64 = 1_000;
const COLLECTION_SCHEDULER_POLL_SECONDS: u64 = 5;
const MAX_DUE_COLLECTION_JOBS_PER_TICK: u8 = 2;

pub struct PitProviderBridge {
    client: Client,
}

pub struct PitCollectionRuntime {
    started: AtomicBool,
    tick_in_progress: AtomicBool,
}

impl Default for PitCollectionRuntime {
    fn default() -> Self {
        Self {
            started: AtomicBool::new(false),
            tick_in_progress: AtomicBool::new(false),
        }
    }
}

struct PitCollectionTickGuard<'a>(&'a AtomicBool);

impl Drop for PitCollectionTickGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl PitCollectionRuntime {
    fn begin_tick(&self) -> Option<PitCollectionTickGuard<'_>> {
        self.tick_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| PitCollectionTickGuard(&self.tick_in_progress))
    }
}

impl Default for PitProviderBridge {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitOfficialProvider {
    UpbitSpot,
    BinanceSpot,
    BinanceUsdm,
    BinanceCoinm,
}

impl PitOfficialProvider {
    pub(crate) fn source(self) -> &'static str {
        match self {
            Self::UpbitSpot => "UPBIT_PUBLIC_CANDLES",
            Self::BinanceSpot => "BINANCE_SPOT_PUBLIC_KLINES",
            Self::BinanceUsdm => "BINANCE_USDM_PUBLIC_KLINES",
            Self::BinanceCoinm => "BINANCE_COINM_PUBLIC_KLINES",
        }
    }

    fn maximum_page_size(self) -> u16 {
        match self {
            Self::UpbitSpot => 200,
            // 모든 Binance 시장을 공식 최대값보다 보수적인 공통 크기로 요청한다.
            Self::BinanceSpot | Self::BinanceUsdm | Self::BinanceCoinm => 1_000,
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::UpbitSpot => "upbit_spot",
            Self::BinanceSpot => "binance_spot",
            Self::BinanceUsdm => "binance_usdm",
            Self::BinanceCoinm => "binance_coinm",
        }
    }

    fn from_key(value: &str) -> Result<Self, String> {
        match value {
            "upbit_spot" => Ok(Self::UpbitSpot),
            "binance_spot" => Ok(Self::BinanceSpot),
            "binance_usdm" => Ok(Self::BinanceUsdm),
            "binance_coinm" => Ok(Self::BinanceCoinm),
            _ => Err("저장된 PIT 공급자 값이 올바르지 않습니다.".to_owned()),
        }
    }

    fn minimum_request_gap_ms(self) -> u64 {
        match self {
            // Upbit 문서의 초당 10회보다 보수적으로 8회 이하로 제한한다.
            Self::UpbitSpot => 125,
            // 공개 Kline weight가 낮아도 여러 Investa 작업 사이 burst를 막는다.
            Self::BinanceSpot | Self::BinanceUsdm | Self::BinanceCoinm => 100,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitProviderInterval {
    Minute1,
    Minute3,
    Minute5,
    Minute15,
    Minute30,
    Hour1,
    Hour4,
    Day1,
}

impl PitProviderInterval {
    fn duration_ms(self) -> u64 {
        match self {
            Self::Minute1 => 60_000,
            Self::Minute3 => 180_000,
            Self::Minute5 => 300_000,
            Self::Minute15 => 900_000,
            Self::Minute30 => 1_800_000,
            Self::Hour1 => 3_600_000,
            Self::Hour4 => 14_400_000,
            Self::Day1 => 86_400_000,
        }
    }

    fn binance_value(self) -> &'static str {
        match self {
            Self::Minute1 => "1m",
            Self::Minute3 => "3m",
            Self::Minute5 => "5m",
            Self::Minute15 => "15m",
            Self::Minute30 => "30m",
            Self::Hour1 => "1h",
            Self::Hour4 => "4h",
            Self::Day1 => "1d",
        }
    }

    fn upbit_path(self) -> &'static str {
        match self {
            Self::Minute1 => "/v1/candles/minutes/1",
            Self::Minute3 => "/v1/candles/minutes/3",
            Self::Minute5 => "/v1/candles/minutes/5",
            Self::Minute15 => "/v1/candles/minutes/15",
            Self::Minute30 => "/v1/candles/minutes/30",
            Self::Hour1 => "/v1/candles/minutes/60",
            Self::Hour4 => "/v1/candles/minutes/240",
            Self::Day1 => "/v1/candles/days",
        }
    }

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Minute1 => "minute1",
            Self::Minute3 => "minute3",
            Self::Minute5 => "minute5",
            Self::Minute15 => "minute15",
            Self::Minute30 => "minute30",
            Self::Hour1 => "hour1",
            Self::Hour4 => "hour4",
            Self::Day1 => "day1",
        }
    }

    fn from_key(value: &str) -> Result<Self, String> {
        match value {
            "minute1" => Ok(Self::Minute1),
            "minute3" => Ok(Self::Minute3),
            "minute5" => Ok(Self::Minute5),
            "minute15" => Ok(Self::Minute15),
            "minute30" => Ok(Self::Minute30),
            "hour1" => Ok(Self::Hour1),
            "hour4" => Ok(Self::Hour4),
            "day1" => Ok(Self::Day1),
            _ => Err("저장된 PIT 주기 값이 올바르지 않습니다.".to_owned()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitProviderPageRequest {
    provider: PitOfficialProvider,
    symbol: String,
    interval: PitProviderInterval,
    start_ms: u64,
    end_exclusive_ms: u64,
    limit: u16,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitPageDirection {
    Backward,
    Forward,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitProviderPage {
    provider: PitOfficialProvider,
    source: String,
    symbol: String,
    interval: PitProviderInterval,
    requested_start_ms: u64,
    requested_end_exclusive_ms: u64,
    fetched_at_ms: u64,
    page_direction: PitPageDirection,
    next_start_ms: Option<u64>,
    next_end_exclusive_ms: Option<u64>,
    observations: Vec<PitPriceObservation>,
    warnings: Vec<String>,
    credentials_required: bool,
    live_order_allowed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredPitProviderPage {
    page_id: String,
    inserted_observation_count: usize,
    reused_observation_count: usize,
    page: PitProviderPage,
    live_order_allowed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitStoredRangeRequest {
    pub(crate) provider: PitOfficialProvider,
    pub(crate) symbol: String,
    pub(crate) interval: PitProviderInterval,
    pub(crate) start_ms: u64,
    pub(crate) end_exclusive_ms: u64,
    pub(crate) maximum_rows: u16,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitStoredRange {
    pub(crate) provider: PitOfficialProvider,
    pub(crate) symbol: String,
    pub(crate) interval: PitProviderInterval,
    pub(crate) requested_start_ms: u64,
    pub(crate) requested_end_exclusive_ms: u64,
    pub(crate) observations: Vec<PitPriceObservation>,
    pub(crate) internal_gap_count: u64,
    pub(crate) truncated: bool,
    pub(crate) gap_policy: &'static str,
    pub(crate) live_order_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitCollectionJobStatus {
    Queued,
    Running,
    RetryWait,
    Completed,
    Failed,
    Cancelled,
}

impl PitCollectionJobStatus {
    fn key(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::RetryWait => "retry_wait",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_key(value: &str) -> Result<Self, String> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "retry_wait" => Ok(Self::RetryWait),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err("저장된 PIT 수집 작업 상태가 올바르지 않습니다.".to_owned()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionJobCreateRequest {
    job_id: String,
    idempotency_key: String,
    provider: PitOfficialProvider,
    symbol: String,
    interval: PitProviderInterval,
    start_ms: u64,
    end_exclusive_ms: u64,
    page_size: u16,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionJobRunRequest {
    job_id: String,
    maximum_pages: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionJobHistoryRequest {
    limit: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionJob {
    job_id: String,
    provider: PitOfficialProvider,
    symbol: String,
    interval: PitProviderInterval,
    requested_start_ms: u64,
    requested_end_exclusive_ms: u64,
    page_size: u16,
    status: PitCollectionJobStatus,
    cursor_start_ms: u64,
    cursor_end_exclusive_ms: u64,
    page_count: u64,
    observation_count: u64,
    failure_count: u8,
    next_retry_at_ms: Option<u64>,
    last_error: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
    live_order_allowed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionJobEvent {
    job_id: String,
    event_index: u64,
    event_type: String,
    detail: Value,
    occurred_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionJobDetail {
    job: PitCollectionJob,
    events: Vec<PitCollectionJobEvent>,
}

#[derive(Debug, Deserialize)]
struct UpbitPitCandle {
    market: String,
    candle_date_time_utc: String,
    opening_price: Value,
    high_price: Value,
    low_price: Value,
    trade_price: Value,
    candle_acc_trade_volume: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct OfficialMinuteBar {
    pub(crate) period_start_ms: u64,
    pub(crate) period_end_ms: u64,
    pub(crate) available_at_ms: u64,
    pub(crate) ingested_at_ms: u64,
    pub(crate) open_scaled: i64,
    pub(crate) high_scaled: i64,
    pub(crate) low_scaled: i64,
    pub(crate) close_scaled: i64,
    pub(crate) volume_scaled: u64,
    pub(crate) price_scale: u64,
    pub(crate) quantity_scale: u64,
}

fn validate_request(request: &PitProviderPageRequest) -> Result<String, String> {
    if request.start_ms == 0
        || request.start_ms >= request.end_exclusive_ms
        || request.limit == 0
        || request.limit > request.provider.maximum_page_size()
    {
        return Err("PIT 공급자 수집 범위 또는 페이지 크기가 올바르지 않습니다.".to_owned());
    }
    let symbol = request.symbol.trim().to_ascii_uppercase();
    if symbol.len() < 5
        || symbol.len() > 32
        || !symbol.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
    {
        return Err("PIT 공급자 심볼 형식이 올바르지 않습니다.".to_owned());
    }
    match request.provider {
        PitOfficialProvider::UpbitSpot if !symbol.starts_with("KRW-") => {
            Err("Upbit PIT 수집은 KRW 마켓 코드만 지원합니다.".to_owned())
        }
        PitOfficialProvider::BinanceSpot
        | PitOfficialProvider::BinanceUsdm
        | PitOfficialProvider::BinanceCoinm
            if symbol.contains('-') =>
        {
            Err("Binance PIT 심볼은 구분자 없는 공식 심볼을 사용해야 합니다.".to_owned())
        }
        _ => Ok(symbol),
    }
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

fn unix_ms_to_rfc3339(timestamp_ms: u64) -> Result<String, String> {
    let seconds = timestamp_ms / 1_000;
    let days = i64::try_from(seconds / 86_400)
        .map_err(|_| "PIT 수집 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = seconds_of_day % 3_600 / 60;
    let second = seconds_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn decimal_scaled(value: &Value) -> Result<i64, String> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('-') || raw.contains(['e', 'E']) {
        return Err("PIT 가격 숫자 형식이 올바르지 않습니다.".to_owned());
    }
    let mut parts = raw.split('.');
    let whole = parts
        .next()
        .ok_or_else(|| "PIT 가격 정수부가 없습니다.".to_owned())?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("PIT 가격 숫자 형식이 올바르지 않습니다.".to_owned());
    }
    let whole_scaled = whole
        .parse::<u64>()
        .ok()
        .and_then(|parsed| parsed.checked_mul(PRICE_SCALE))
        .ok_or_else(|| "PIT 가격이 지원 범위를 초과했습니다.".to_owned())?;
    let mut fraction_scaled = 0_u64;
    let mut factor = PRICE_SCALE / 10;
    for byte in fraction.bytes().take(8) {
        fraction_scaled = fraction_scaled
            .checked_add(u64::from(byte - b'0') * factor)
            .ok_or_else(|| "PIT 가격이 지원 범위를 초과했습니다.".to_owned())?;
        factor /= 10;
    }
    let scaled = whole_scaled
        .checked_add(fraction_scaled)
        .ok_or_else(|| "PIT 가격이 지원 범위를 초과했습니다.".to_owned())?;
    if scaled == 0 {
        return Err("PIT 가격은 0보다 커야 합니다.".to_owned());
    }
    i64::try_from(scaled).map_err(|_| "PIT 가격이 지원 범위를 초과했습니다.".to_owned())
}

fn nonnegative_decimal_scaled(value: &Value) -> Result<u64, String> {
    let raw = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('-') || raw.contains(['e', 'E']) {
        return Err("PIT 거래량 숫자 형식이 올바르지 않습니다.".to_owned());
    }
    let mut parts = raw.split('.');
    let whole = parts
        .next()
        .ok_or_else(|| "PIT 거래량 정수부가 없습니다.".to_owned())?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("PIT 거래량 숫자 형식이 올바르지 않습니다.".to_owned());
    }
    let whole_scaled = whole
        .parse::<u64>()
        .ok()
        .and_then(|parsed| parsed.checked_mul(PRICE_SCALE))
        .ok_or_else(|| "PIT 거래량이 지원 범위를 초과했습니다.".to_owned())?;
    let mut fraction_scaled = 0_u64;
    let mut factor = PRICE_SCALE / 10;
    for byte in fraction.bytes().take(8) {
        fraction_scaled = fraction_scaled
            .checked_add(u64::from(byte - b'0') * factor)
            .ok_or_else(|| "PIT 거래량이 지원 범위를 초과했습니다.".to_owned())?;
        factor /= 10;
    }
    whole_scaled
        .checked_add(fraction_scaled)
        .ok_or_else(|| "PIT 거래량이 지원 범위를 초과했습니다.".to_owned())
}

fn validate_official_ohlcv(open: i64, high: i64, low: i64, close: i64) -> Result<(), String> {
    if low > open || low > close || high < open || high < close {
        return Err("공식 PIT 봉의 OHLC 관계가 올바르지 않습니다.".to_owned());
    }
    Ok(())
}

fn revision(source: &str, symbol: &str, bar_end_ms: u64, close_scaled: i64) -> String {
    let canonical = format!("{source}|{symbol}|{bar_end_ms}|{close_scaled}|{PRICE_SCALE}");
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

fn page_id(page_json: &str) -> String {
    format!("{:x}", Sha256::digest(page_json.as_bytes()))
}

fn observation(
    source: &str,
    symbol: &str,
    bar_end_ms: u64,
    fetched_at_ms: u64,
    close_scaled: i64,
) -> PitPriceObservation {
    PitPriceObservation {
        record_id: format!("{source}:{symbol}:{bar_end_ms}"),
        bar_end_ms,
        available_at_ms: bar_end_ms,
        ingested_at_ms: fetched_at_ms,
        source: source.to_owned(),
        source_revision: revision(source, symbol, bar_end_ms, close_scaled),
        close_scaled,
        price_scale: PRICE_SCALE,
        final_bar: true,
    }
}

fn parse_binance_rows(
    rows: Vec<Vec<Value>>,
    source: &str,
    symbol: &str,
    request: &PitProviderPageRequest,
    fetched_at_ms: u64,
) -> Result<Vec<PitPriceObservation>, String> {
    let mut observations = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() < 7 {
            return Err("Binance PIT 봉 응답 필드가 부족합니다.".to_owned());
        }
        let start_ms = row[0]
            .as_u64()
            .ok_or_else(|| "Binance PIT 봉 시작 시각이 올바르지 않습니다.".to_owned())?;
        let close_time_inclusive = row[6]
            .as_u64()
            .ok_or_else(|| "Binance PIT 봉 종료 시각이 올바르지 않습니다.".to_owned())?;
        let bar_end_ms = close_time_inclusive
            .checked_add(1)
            .ok_or_else(|| "Binance PIT 봉 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        if bar_end_ms <= start_ms {
            return Err("Binance PIT 봉 시간 관계가 올바르지 않습니다.".to_owned());
        }
        if start_ms < request.start_ms
            || bar_end_ms > request.end_exclusive_ms
            || bar_end_ms > fetched_at_ms
        {
            continue;
        }
        observations.push(observation(
            source,
            symbol,
            bar_end_ms,
            fetched_at_ms,
            decimal_scaled(&row[4])?,
        ));
    }
    observations.sort_by_key(|item| item.bar_end_ms);
    ensure_unique_ascending(&observations)?;
    Ok(observations)
}

fn parse_upbit_rows(
    rows: Vec<UpbitPitCandle>,
    source: &str,
    symbol: &str,
    request: &PitProviderPageRequest,
    fetched_at_ms: u64,
) -> Result<Vec<PitPriceObservation>, String> {
    let mut observations = Vec::with_capacity(rows.len());
    for row in rows {
        if row.market != symbol {
            return Err("Upbit PIT 응답에 다른 마켓이 포함되었습니다.".to_owned());
        }
        let start_ms =
            crate::market_data::parse_rfc3339_ms(&format!("{}Z", row.candle_date_time_utc))
                .ok_or_else(|| "Upbit PIT 봉 시각을 해석하지 못했습니다.".to_owned())?;
        let bar_end_ms = start_ms
            .checked_add(request.interval.duration_ms())
            .ok_or_else(|| "Upbit PIT 봉 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        if start_ms < request.start_ms
            || bar_end_ms > request.end_exclusive_ms
            || bar_end_ms > fetched_at_ms
        {
            continue;
        }
        observations.push(observation(
            source,
            symbol,
            bar_end_ms,
            fetched_at_ms,
            decimal_scaled(&row.trade_price)?,
        ));
    }
    observations.sort_by_key(|item| item.bar_end_ms);
    ensure_unique_ascending(&observations)?;
    Ok(observations)
}

fn ensure_unique_ascending(observations: &[PitPriceObservation]) -> Result<(), String> {
    if observations
        .windows(2)
        .any(|pair| pair[0].bar_end_ms >= pair[1].bar_end_ms)
    {
        return Err("PIT 공급자 페이지에 중복 또는 역순 봉이 있습니다.".to_owned());
    }
    Ok(())
}

fn internal_gap_count(
    observations: &[PitPriceObservation],
    interval: PitProviderInterval,
) -> Result<u64, String> {
    let duration_ms = interval.duration_ms();
    observations.windows(2).try_fold(0_u64, |total, pair| {
        let difference = pair[1]
            .bar_end_ms
            .checked_sub(pair[0].bar_end_ms)
            .ok_or_else(|| "PIT 저장 범위가 역순입니다.".to_owned())?;
        if difference % duration_ms != 0 {
            return Err("PIT 저장 범위의 봉 경계가 요청 주기와 일치하지 않습니다.".to_owned());
        }
        total
            .checked_add(difference / duration_ms - 1)
            .ok_or_else(|| "PIT gap 수가 지원 범위를 초과했습니다.".to_owned())
    })
}

fn provider_error(provider: PitOfficialProvider, status: StatusCode) -> String {
    match status {
        StatusCode::TOO_MANY_REQUESTS => {
            format!("{provider:?} PIT 공개 시세 요청 한도를 초과했습니다.")
        }
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND => {
            format!("{provider:?} PIT 심볼·주기·기간을 확인해 주세요.")
        }
        _ => format!("{provider:?} PIT 공개 시세 서버가 요청을 처리하지 못했습니다."),
    }
}

async fn fetch_upbit_page(
    request: &PitProviderPageRequest,
    symbol: &str,
    bridge: &PitProviderBridge,
    fetched_at_ms: u64,
) -> Result<PitProviderPage, String> {
    let count = request.limit.to_string();
    let to = unix_ms_to_rfc3339(request.end_exclusive_ms)?;
    let response = bridge
        .client
        .get(format!("{UPBIT_API_BASE}{}", request.interval.upbit_path()))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .query(&[
            ("market", symbol),
            ("to", to.as_str()),
            ("count", count.as_str()),
        ])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| "Upbit PIT 공개 시세에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(provider_error(request.provider, response.status()));
    }
    let raw = response
        .json::<Vec<UpbitPitCandle>>()
        .await
        .map_err(|_| "Upbit PIT 공개 시세 형식이 올바르지 않습니다.".to_owned())?;
    let raw_len = raw.len();
    let oldest_raw_start_ms = raw
        .iter()
        .map(|row| {
            crate::market_data::parse_rfc3339_ms(&format!("{}Z", row.candle_date_time_utc))
                .ok_or_else(|| "Upbit PIT 봉 시각을 해석하지 못했습니다.".to_owned())
        })
        .collect::<Result<Vec<_>, String>>()?
        .into_iter()
        .min();
    let observations = parse_upbit_rows(
        raw,
        request.provider.source(),
        symbol,
        request,
        fetched_at_ms,
    )?;
    let next_end_exclusive_ms = if raw_len == usize::from(request.limit) {
        oldest_raw_start_ms.filter(|cursor| *cursor > request.start_ms)
    } else {
        None
    };
    Ok(PitProviderPage {
        provider: request.provider,
        source: request.provider.source().to_owned(),
        symbol: symbol.to_owned(),
        interval: request.interval,
        requested_start_ms: request.start_ms,
        requested_end_exclusive_ms: request.end_exclusive_ms,
        fetched_at_ms,
        page_direction: PitPageDirection::Backward,
        next_start_ms: None,
        next_end_exclusive_ms,
        observations,
        warnings: vec![
            "Upbit는 거래가 없던 구간의 봉을 생성하지 않으므로 시간 gap 자체를 임의 보간하지 않습니다."
                .to_owned(),
        ],
        credentials_required: false,
        live_order_allowed: false,
    })
}

async fn fetch_binance_page(
    request: &PitProviderPageRequest,
    symbol: &str,
    bridge: &PitProviderBridge,
    fetched_at_ms: u64,
) -> Result<PitProviderPage, String> {
    let (base, path) = match request.provider {
        PitOfficialProvider::BinanceSpot => (BINANCE_SPOT_BASE, "/api/v3/klines"),
        PitOfficialProvider::BinanceUsdm => (BINANCE_USDM_BASE, "/fapi/v1/klines"),
        PitOfficialProvider::BinanceCoinm => (BINANCE_COINM_BASE, "/dapi/v1/klines"),
        PitOfficialProvider::UpbitSpot => {
            return Err("Upbit 요청을 Binance 어댑터로 실행할 수 없습니다.".to_owned())
        }
    };
    let start = request.start_ms.to_string();
    let end = request.end_exclusive_ms.saturating_sub(1).to_string();
    let limit = request.limit.to_string();
    let response = bridge
        .client
        .get(format!("{base}{path}"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .query(&[
            ("symbol", symbol),
            ("interval", request.interval.binance_value()),
            ("startTime", start.as_str()),
            ("endTime", end.as_str()),
            ("limit", limit.as_str()),
        ])
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| "Binance PIT 공개 시세에 연결하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err(provider_error(request.provider, response.status()));
    }
    let raw = response
        .json::<Vec<Vec<Value>>>()
        .await
        .map_err(|_| "Binance PIT 공개 시세 형식이 올바르지 않습니다.".to_owned())?;
    let raw_len = raw.len();
    let last_raw_end_ms = raw
        .last()
        .and_then(|row| row.get(6))
        .and_then(Value::as_u64)
        .and_then(|value| value.checked_add(1));
    let observations = parse_binance_rows(
        raw,
        request.provider.source(),
        symbol,
        request,
        fetched_at_ms,
    )?;
    let next_start_ms = if raw_len == usize::from(request.limit) {
        last_raw_end_ms.filter(|cursor| *cursor < request.end_exclusive_ms)
    } else {
        None
    };
    Ok(PitProviderPage {
        provider: request.provider,
        source: request.provider.source().to_owned(),
        symbol: symbol.to_owned(),
        interval: request.interval,
        requested_start_ms: request.start_ms,
        requested_end_exclusive_ms: request.end_exclusive_ms,
        fetched_at_ms,
        page_direction: PitPageDirection::Forward,
        next_start_ms,
        next_end_exclusive_ms: None,
        observations,
        warnings: Vec::new(),
        credentials_required: false,
        live_order_allowed: false,
    })
}

pub(crate) async fn fetch_official_minute_bars(
    provider: PitOfficialProvider,
    symbol: &str,
    start_ms: u64,
    end_exclusive_ms: u64,
    bridge: &PitProviderBridge,
) -> Result<Vec<OfficialMinuteBar>, String> {
    if start_ms == 0
        || start_ms >= end_exclusive_ms
        || start_ms % 60_000 != 0
        || end_exclusive_ms % 60_000 != 0
    {
        return Err("REST gap 복구 범위가 1분 경계와 일치하지 않습니다.".to_owned());
    }
    let requested_minutes = (end_exclusive_ms - start_ms) / 60_000;
    if requested_minutes == 0 || requested_minutes > u64::from(provider.maximum_page_size()) {
        return Err("REST gap 복구 범위가 공급자별 단일 요청 한도를 초과했습니다.".to_owned());
    }
    let request = PitProviderPageRequest {
        provider,
        symbol: symbol.to_owned(),
        interval: PitProviderInterval::Minute1,
        start_ms,
        end_exclusive_ms,
        limit: requested_minutes as u16,
    };
    let symbol = validate_request(&request)?;
    let fetched_at_ms = paper_trading::now_ms()?;
    let rows = match provider {
        PitOfficialProvider::UpbitSpot => {
            let count = request.limit.to_string();
            let to = unix_ms_to_rfc3339(end_exclusive_ms)?;
            let response = bridge
                .client
                .get(format!("{UPBIT_API_BASE}/v1/candles/minutes/1"))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
                .query(&[
                    ("market", symbol.as_str()),
                    ("to", to.as_str()),
                    ("count", count.as_str()),
                ])
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|_| "Upbit REST gap 복구 시세에 연결하지 못했습니다.".to_owned())?;
            if !response.status().is_success() {
                return Err(provider_error(provider, response.status()));
            }
            response
                .json::<Vec<UpbitPitCandle>>()
                .await
                .map_err(|_| "Upbit REST gap 복구 응답 형식이 올바르지 않습니다.".to_owned())?
                .into_iter()
                .map(|row| {
                    if row.market != symbol {
                        return Err(
                            "Upbit REST gap 복구 응답에 다른 마켓이 포함되었습니다.".to_owned()
                        );
                    }
                    let period_start_ms = crate::market_data::parse_rfc3339_ms(&format!(
                        "{}Z",
                        row.candle_date_time_utc
                    ))
                    .ok_or_else(|| {
                        "Upbit REST gap 복구 봉 시각을 해석하지 못했습니다.".to_owned()
                    })?;
                    let period_end_ms = period_start_ms.checked_add(60_000).ok_or_else(|| {
                        "Upbit REST gap 복구 봉 종료 시각이 지원 범위를 초과했습니다.".to_owned()
                    })?;
                    let open_scaled = decimal_scaled(&row.opening_price)?;
                    let high_scaled = decimal_scaled(&row.high_price)?;
                    let low_scaled = decimal_scaled(&row.low_price)?;
                    let close_scaled = decimal_scaled(&row.trade_price)?;
                    validate_official_ohlcv(open_scaled, high_scaled, low_scaled, close_scaled)?;
                    Ok(OfficialMinuteBar {
                        period_start_ms,
                        period_end_ms,
                        available_at_ms: period_end_ms,
                        ingested_at_ms: fetched_at_ms,
                        open_scaled,
                        high_scaled,
                        low_scaled,
                        close_scaled,
                        volume_scaled: nonnegative_decimal_scaled(&row.candle_acc_trade_volume)?,
                        price_scale: PRICE_SCALE,
                        quantity_scale: PRICE_SCALE,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
        }
        PitOfficialProvider::BinanceSpot
        | PitOfficialProvider::BinanceUsdm
        | PitOfficialProvider::BinanceCoinm => {
            let (base, path) = match provider {
                PitOfficialProvider::BinanceSpot => (BINANCE_SPOT_BASE, "/api/v3/klines"),
                PitOfficialProvider::BinanceUsdm => (BINANCE_USDM_BASE, "/fapi/v1/klines"),
                PitOfficialProvider::BinanceCoinm => (BINANCE_COINM_BASE, "/dapi/v1/klines"),
                PitOfficialProvider::UpbitSpot => unreachable!(),
            };
            let start = start_ms.to_string();
            let end = end_exclusive_ms.saturating_sub(1).to_string();
            let limit = request.limit.to_string();
            let response = bridge
                .client
                .get(format!("{base}{path}"))
                .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
                .query(&[
                    ("symbol", symbol.as_str()),
                    ("interval", "1m"),
                    ("startTime", start.as_str()),
                    ("endTime", end.as_str()),
                    ("limit", limit.as_str()),
                ])
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|_| "Binance REST gap 복구 시세에 연결하지 못했습니다.".to_owned())?;
            if !response.status().is_success() {
                return Err(provider_error(provider, response.status()));
            }
            response
                .json::<Vec<Vec<Value>>>()
                .await
                .map_err(|_| "Binance REST gap 복구 응답 형식이 올바르지 않습니다.".to_owned())?
                .into_iter()
                .map(|row| {
                    if row.len() < 7 {
                        return Err("Binance REST gap 복구 봉 응답 필드가 부족합니다.".to_owned());
                    }
                    let period_start_ms = row[0].as_u64().ok_or_else(|| {
                        "Binance REST gap 복구 봉 시작 시각이 올바르지 않습니다.".to_owned()
                    })?;
                    let period_end_ms = row[6]
                        .as_u64()
                        .and_then(|value| value.checked_add(1))
                        .ok_or_else(|| {
                            "Binance REST gap 복구 봉 종료 시각이 올바르지 않습니다.".to_owned()
                        })?;
                    let open_scaled = decimal_scaled(&row[1])?;
                    let high_scaled = decimal_scaled(&row[2])?;
                    let low_scaled = decimal_scaled(&row[3])?;
                    let close_scaled = decimal_scaled(&row[4])?;
                    validate_official_ohlcv(open_scaled, high_scaled, low_scaled, close_scaled)?;
                    Ok(OfficialMinuteBar {
                        period_start_ms,
                        period_end_ms,
                        available_at_ms: period_end_ms,
                        ingested_at_ms: fetched_at_ms,
                        open_scaled,
                        high_scaled,
                        low_scaled,
                        close_scaled,
                        volume_scaled: nonnegative_decimal_scaled(&row[5])?,
                        price_scale: PRICE_SCALE,
                        quantity_scale: PRICE_SCALE,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
        }
    };
    let mut rows = rows
        .into_iter()
        .filter(|bar| {
            bar.period_start_ms >= start_ms
                && bar.period_end_ms <= end_exclusive_ms
                && bar.period_end_ms <= fetched_at_ms
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|bar| bar.period_start_ms);
    if rows
        .windows(2)
        .any(|pair| pair[0].period_start_ms >= pair[1].period_start_ms)
    {
        return Err("REST gap 복구 응답에 중복 또는 역순 봉이 있습니다.".to_owned());
    }
    Ok(rows)
}

#[tauri::command]
pub async fn pit_provider_page_fetch(
    request: PitProviderPageRequest,
    bridge: State<'_, PitProviderBridge>,
) -> Result<PitProviderPage, String> {
    let symbol = validate_request(&request)?;
    let fetched_at_ms = paper_trading::now_ms()?;
    match request.provider {
        PitOfficialProvider::UpbitSpot => {
            fetch_upbit_page(&request, &symbol, &bridge, fetched_at_ms).await
        }
        PitOfficialProvider::BinanceSpot
        | PitOfficialProvider::BinanceUsdm
        | PitOfficialProvider::BinanceCoinm => {
            fetch_binance_page(&request, &symbol, &bridge, fetched_at_ms).await
        }
    }
}

fn store_page(
    bridge: &PersistenceBridge,
    page: PitProviderPage,
) -> Result<StoredPitProviderPage, String> {
    if page.observations.len() > usize::from(page.provider.maximum_page_size()) {
        return Err("PIT 공급자 페이지가 허용 행 수를 초과했습니다.".to_owned());
    }
    ensure_unique_ascending(&page.observations)?;
    for item in &page.observations {
        if item.source != page.source
            || item.ingested_at_ms != page.fetched_at_ms
            || item.available_at_ms > item.ingested_at_ms
            || !item.final_bar
            || item.source_revision
                != revision(
                    &item.source,
                    &page.symbol,
                    item.bar_end_ms,
                    item.close_scaled,
                )
        {
            return Err(
                "PIT 공급자 페이지의 원천 계보 또는 완료 시각이 올바르지 않습니다.".to_owned(),
            );
        }
    }
    let page_json = serde_json::to_string(&page)
        .map_err(|_| "PIT 공급자 페이지를 직렬화하지 못했습니다.".to_owned())?;
    let page_id = page_id(&page_json);
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 페이지 저장 트랜잭션을 시작하지 못했습니다.".to_owned())?;
    let existing_page = transaction
        .query_row(
            "SELECT page_json FROM pit_provider_pages WHERE page_id = ?1",
            params![page_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "기존 PIT 페이지를 확인하지 못했습니다.".to_owned())?;
    if existing_page
        .as_deref()
        .is_some_and(|value| value != page_json)
    {
        return Err("같은 PIT 페이지 해시에 다른 내용이 이미 저장되어 있습니다.".to_owned());
    }

    let mut inserted_observation_count = 0;
    let mut reused_observation_count = 0;
    for item in &page.observations {
        let existing = transaction
            .query_row(
                "SELECT source_revision, close_scaled, price_scale
                 FROM pit_price_observations WHERE record_id = ?1",
                params![item.record_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| "기존 PIT 가격 관측을 확인하지 못했습니다.".to_owned())?;
        match existing {
            Some((revision, close_scaled, price_scale))
                if revision == item.source_revision
                    && close_scaled == item.close_scaled
                    && price_scale == item.price_scale =>
            {
                reused_observation_count += 1;
            }
            Some(_) => {
                return Err(
                    "같은 PIT 가격 관측 식별자에 다른 값이 이미 저장되어 있습니다.".to_owned(),
                )
            }
            None => {
                transaction
                    .execute(
                        "INSERT INTO pit_price_observations
                         (record_id, provider, symbol, interval, bar_end_ms, available_at_ms,
                          ingested_at_ms, source, source_revision, close_scaled, price_scale, final_bar)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1)",
                        params![
                            item.record_id,
                            page.provider.key(),
                            page.symbol,
                            page.interval.key(),
                            item.bar_end_ms,
                            item.available_at_ms,
                            item.ingested_at_ms,
                            item.source,
                            item.source_revision,
                            item.close_scaled,
                            item.price_scale,
                        ],
                    )
                    .map_err(|_| "PIT 가격 관측을 저장하지 못했습니다.".to_owned())?;
                inserted_observation_count += 1;
            }
        }
    }
    if existing_page.is_none() {
        transaction
            .execute(
                "INSERT INTO pit_provider_pages
                 (page_id, provider, symbol, interval, requested_start_ms,
                  requested_end_exclusive_ms, page_json, fetched_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    page_id,
                    page.provider.key(),
                    page.symbol,
                    page.interval.key(),
                    page.requested_start_ms,
                    page.requested_end_exclusive_ms,
                    page_json,
                    page.fetched_at_ms,
                ],
            )
            .map_err(|_| "PIT 공급자 페이지를 저장하지 못했습니다.".to_owned())?;
    }
    transaction
        .commit()
        .map_err(|_| "PIT 페이지 저장 트랜잭션을 완료하지 못했습니다.".to_owned())?;
    Ok(StoredPitProviderPage {
        page_id,
        inserted_observation_count,
        reused_observation_count,
        page,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub async fn pit_provider_page_fetch_store(
    request: PitProviderPageRequest,
    provider_bridge: State<'_, PitProviderBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<StoredPitProviderPage, String> {
    let symbol = validate_request(&request)?;
    let fetched_at_ms = paper_trading::now_ms()?;
    let page = match request.provider {
        PitOfficialProvider::UpbitSpot => {
            fetch_upbit_page(&request, &symbol, &provider_bridge, fetched_at_ms).await?
        }
        PitOfficialProvider::BinanceSpot
        | PitOfficialProvider::BinanceUsdm
        | PitOfficialProvider::BinanceCoinm => {
            fetch_binance_page(&request, &symbol, &provider_bridge, fetched_at_ms).await?
        }
    };
    store_page(&persistence, page)
}

pub(crate) fn load_stored_range(
    bridge: &PersistenceBridge,
    request: PitStoredRangeRequest,
) -> Result<PitStoredRange, String> {
    let validation_request = PitProviderPageRequest {
        provider: request.provider,
        symbol: request.symbol.clone(),
        interval: request.interval,
        start_ms: request.start_ms,
        end_exclusive_ms: request.end_exclusive_ms,
        limit: 1,
    };
    let symbol = validate_request(&validation_request)?;
    if request.maximum_rows == 0 || request.maximum_rows > 20_001 {
        return Err("PIT 저장 범위 조회는 최대 20,001봉까지 지원합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let query_limit = u32::from(request.maximum_rows) + 1;
    let mut statement = connection
        .prepare(
            "SELECT record_id, bar_end_ms, available_at_ms, ingested_at_ms, source,
                    source_revision, close_scaled, price_scale, final_bar
             FROM pit_price_observations
             WHERE provider = ?1 AND symbol = ?2 AND interval = ?3
               AND bar_end_ms > ?4 AND bar_end_ms <= ?5
             ORDER BY bar_end_ms ASC
             LIMIT ?6",
        )
        .map_err(|_| "PIT 저장 범위 조회를 준비하지 못했습니다.".to_owned())?;
    let mapped = statement
        .query_map(
            params![
                request.provider.key(),
                symbol,
                request.interval.key(),
                request.start_ms,
                request.end_exclusive_ms,
                query_limit,
            ],
            |row| {
                Ok(PitPriceObservation {
                    record_id: row.get(0)?,
                    bar_end_ms: row.get(1)?,
                    available_at_ms: row.get(2)?,
                    ingested_at_ms: row.get(3)?,
                    source: row.get(4)?,
                    source_revision: row.get(5)?,
                    close_scaled: row.get(6)?,
                    price_scale: row.get(7)?,
                    final_bar: row.get::<_, u8>(8)? == 1,
                })
            },
        )
        .map_err(|_| "PIT 저장 범위를 조회하지 못했습니다.".to_owned())?;
    let mut observations = mapped
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "PIT 저장 범위를 해석하지 못했습니다.".to_owned())?;
    let truncated = observations.len() > usize::from(request.maximum_rows);
    observations.truncate(usize::from(request.maximum_rows));
    ensure_unique_ascending(&observations)?;
    let internal_gap_count = internal_gap_count(&observations, request.interval)?;
    Ok(PitStoredRange {
        provider: request.provider,
        symbol,
        interval: request.interval,
        requested_start_ms: request.start_ms,
        requested_end_exclusive_ms: request.end_exclusive_ms,
        observations,
        internal_gap_count,
        truncated,
        gap_policy: if request.provider == PitOfficialProvider::UpbitSpot {
            "preserve_missing_no_trade_candles"
        } else {
            "investigate_unexpected_provider_gap"
        },
        live_order_allowed: false,
    })
}

pub(crate) fn completed_collection_covers(
    bridge: &PersistenceBridge,
    request: &PitStoredRangeRequest,
) -> Result<bool, String> {
    let validation_request = PitProviderPageRequest {
        provider: request.provider,
        symbol: request.symbol.clone(),
        interval: request.interval,
        start_ms: request.start_ms,
        end_exclusive_ms: request.end_exclusive_ms,
        limit: 1,
    };
    let symbol = validate_request(&validation_request)?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM pit_collection_jobs
             WHERE provider = ?1 AND symbol = ?2 AND interval = ?3
               AND requested_start_ms <= ?4 AND requested_end_exclusive_ms >= ?5
               AND status = 'completed'",
            params![
                request.provider.key(),
                symbol,
                request.interval.key(),
                request.start_ms,
                request.end_exclusive_ms,
            ],
            |row| row.get::<_, u64>(0),
        )
        .map_err(|_| "완료된 PIT 수집 범위를 확인하지 못했습니다.".to_owned())?;
    Ok(count > 0)
}

#[tauri::command]
pub fn pit_provider_stored_range(
    request: PitStoredRangeRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PitStoredRange, String> {
    load_stored_range(&persistence, request)
}

fn validate_job_identifier(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(format!("{label} 형식이 올바르지 않습니다."));
    }
    Ok(value.to_owned())
}

fn collection_request_hash(
    job_id: &str,
    symbol: &str,
    request: &PitCollectionJobCreateRequest,
) -> String {
    let canonical = format!(
        "{job_id}|{}|{symbol}|{}|{}|{}|{}",
        request.provider.key(),
        request.interval.key(),
        request.start_ms,
        request.end_exclusive_ms,
        request.page_size
    );
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

type StoredCollectionJobRow = (
    String,
    String,
    String,
    String,
    u64,
    u64,
    u16,
    String,
    u64,
    u64,
    u64,
    u64,
    u8,
    Option<u64>,
    Option<String>,
    u64,
    u64,
);

fn decode_collection_job(row: StoredCollectionJobRow) -> Result<PitCollectionJob, String> {
    Ok(PitCollectionJob {
        job_id: row.0,
        provider: PitOfficialProvider::from_key(&row.1)?,
        symbol: row.2,
        interval: PitProviderInterval::from_key(&row.3)?,
        requested_start_ms: row.4,
        requested_end_exclusive_ms: row.5,
        page_size: row.6,
        status: PitCollectionJobStatus::from_key(&row.7)?,
        cursor_start_ms: row.8,
        cursor_end_exclusive_ms: row.9,
        page_count: row.10,
        observation_count: row.11,
        failure_count: row.12,
        next_retry_at_ms: row.13,
        last_error: row.14,
        created_at_ms: row.15,
        updated_at_ms: row.16,
        live_order_allowed: false,
    })
}

fn load_collection_job(
    bridge: &PersistenceBridge,
    job_id: &str,
) -> Result<PitCollectionJob, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let row = connection
        .query_row(
            "SELECT job_id, provider, symbol, interval, requested_start_ms,
                    requested_end_exclusive_ms, page_size, status, cursor_start_ms,
                    cursor_end_exclusive_ms, page_count, observation_count, failure_count,
                    next_retry_at_ms, last_error, created_at_ms, updated_at_ms
             FROM pit_collection_jobs WHERE job_id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get(13)?,
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "PIT 수집 작업을 조회하지 못했습니다.".to_owned())?
        .ok_or_else(|| "PIT 수집 작업을 찾지 못했습니다.".to_owned())?;
    decode_collection_job(row)
}

fn append_collection_event(
    transaction: &Transaction<'_>,
    job_id: &str,
    event_type: &str,
    detail: Value,
    occurred_at_ms: u64,
) -> Result<(), String> {
    let detail_json = serde_json::to_string(&detail)
        .map_err(|_| "PIT 수집 작업 이벤트를 직렬화하지 못했습니다.".to_owned())?;
    transaction
        .execute(
            "INSERT INTO pit_collection_job_events
             (job_id, event_index, event_type, detail_json, occurred_at_ms)
             SELECT ?1, COALESCE(MAX(event_index), -1) + 1, ?2, ?3, ?4
             FROM pit_collection_job_events WHERE job_id = ?1",
            params![job_id, event_type, detail_json, occurred_at_ms],
        )
        .map_err(|_| "PIT 수집 작업 이벤트를 저장하지 못했습니다.".to_owned())?;
    Ok(())
}

fn create_collection_job(
    bridge: &PersistenceBridge,
    request: PitCollectionJobCreateRequest,
    now_ms: u64,
) -> Result<PitCollectionJob, String> {
    let job_id = validate_job_identifier(&request.job_id, "PIT 작업 ID")?;
    let idempotency_key = validate_job_identifier(&request.idempotency_key, "PIT 멱등성 키")?;
    let validation = PitProviderPageRequest {
        provider: request.provider,
        symbol: request.symbol.clone(),
        interval: request.interval,
        start_ms: request.start_ms,
        end_exclusive_ms: request.end_exclusive_ms,
        limit: request.page_size,
    };
    let symbol = validate_request(&validation)?;
    let request_hash = collection_request_hash(&job_id, &symbol, &request);
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 수집 작업 트랜잭션을 시작하지 못했습니다.".to_owned())?;
    let existing = transaction
        .query_row(
            "SELECT job_id, request_hash FROM pit_collection_jobs WHERE idempotency_key = ?1",
            params![idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| "기존 PIT 수집 작업을 확인하지 못했습니다.".to_owned())?;
    if let Some((existing_job_id, existing_hash)) = existing {
        if existing_hash != request_hash || existing_job_id != job_id {
            return Err("같은 PIT 멱등성 키가 다른 수집 요청에 이미 사용되었습니다.".to_owned());
        }
        drop(transaction);
        drop(connection);
        return load_collection_job(bridge, &job_id);
    }
    transaction
        .execute(
            "INSERT INTO pit_collection_jobs
             (job_id, idempotency_key, request_hash, provider, symbol, interval,
              requested_start_ms, requested_end_exclusive_ms, page_size, status,
              cursor_start_ms, cursor_end_exclusive_ms, page_count, observation_count,
              failure_count, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', ?7, ?8, 0, 0, 0, ?10, ?10)",
            params![
                job_id,
                idempotency_key,
                request_hash,
                request.provider.key(),
                symbol,
                request.interval.key(),
                request.start_ms,
                request.end_exclusive_ms,
                request.page_size,
                now_ms,
            ],
        )
        .map_err(|_| "PIT 수집 작업을 생성하지 못했습니다.".to_owned())?;
    append_collection_event(
        &transaction,
        &job_id,
        "created",
        json!({"status": "queued"}),
        now_ms,
    )?;
    transaction
        .commit()
        .map_err(|_| "PIT 수집 작업 생성을 완료하지 못했습니다.".to_owned())?;
    drop(connection);
    load_collection_job(bridge, &job_id)
}

fn claim_collection_job(
    bridge: &PersistenceBridge,
    job_id: &str,
    now_ms: u64,
) -> Result<PitCollectionJob, String> {
    let current = load_collection_job(bridge, job_id)?;
    match current.status {
        PitCollectionJobStatus::Completed
        | PitCollectionJobStatus::Failed
        | PitCollectionJobStatus::Cancelled => return Ok(current),
        PitCollectionJobStatus::RetryWait
            if current.next_retry_at_ms.is_some_and(|due| due > now_ms) =>
        {
            return Ok(current)
        }
        PitCollectionJobStatus::Running
            if current
                .updated_at_ms
                .saturating_add(COLLECTION_LEASE_TIMEOUT_MS)
                > now_ms =>
        {
            return Err("PIT 수집 작업이 이미 실행 중입니다.".to_owned())
        }
        _ => {}
    }
    let recovered = current.status == PitCollectionJobStatus::Running;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 수집 실행권 트랜잭션을 시작하지 못했습니다.".to_owned())?;
    let stale_before = now_ms.saturating_sub(COLLECTION_LEASE_TIMEOUT_MS);
    let changed = transaction
        .execute(
            "UPDATE pit_collection_jobs SET status = 'running', next_retry_at_ms = NULL,
                    updated_at_ms = ?2
             WHERE job_id = ?1 AND (
                 status = 'queued'
                 OR (status = 'retry_wait' AND (next_retry_at_ms IS NULL OR next_retry_at_ms <= ?2))
                 OR (status = 'running' AND updated_at_ms <= ?3)
             )",
            params![job_id, now_ms, stale_before],
        )
        .map_err(|_| "PIT 수집 실행권을 얻지 못했습니다.".to_owned())?;
    if changed != 1 {
        return Err("PIT 수집 실행권 상태가 변경되어 실행하지 못했습니다.".to_owned());
    }
    append_collection_event(
        &transaction,
        job_id,
        if recovered { "recovered" } else { "claimed" },
        json!({"leaseTimeoutMs": COLLECTION_LEASE_TIMEOUT_MS}),
        now_ms,
    )?;
    transaction
        .commit()
        .map_err(|_| "PIT 수집 실행권 저장을 완료하지 못했습니다.".to_owned())?;
    drop(connection);
    load_collection_job(bridge, job_id)
}

fn is_retryable_collection_error(message: &str) -> bool {
    message.contains("요청 한도를 초과")
        || message.contains("연결하지 못했습니다")
        || message.contains("서버가 요청을 처리하지 못했습니다")
}

fn reserve_provider_request_slot(
    bridge: &PersistenceBridge,
    provider: PitOfficialProvider,
    now_ms: u64,
) -> Result<u64, String> {
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 공급자 호출 제한 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 공급자 호출 제한 트랜잭션을 시작하지 못했습니다.".to_owned())?;
    let stored_next = transaction
        .query_row(
            "SELECT next_allowed_at_ms FROM pit_provider_rate_limits WHERE provider = ?1",
            params![provider.key()],
            |row| row.get::<_, u64>(0),
        )
        .optional()
        .map_err(|_| "PIT 공급자 호출 가능 시각을 확인하지 못했습니다.".to_owned())?;
    let reserved_at_ms = stored_next.unwrap_or(now_ms).max(now_ms);
    let next_allowed_at_ms = reserved_at_ms
        .checked_add(provider.minimum_request_gap_ms())
        .ok_or_else(|| "PIT 공급자 호출 가능 시각이 범위를 초과했습니다.".to_owned())?;
    transaction
        .execute(
            "INSERT INTO pit_provider_rate_limits(provider, next_allowed_at_ms) VALUES (?1, ?2)
             ON CONFLICT(provider) DO UPDATE SET next_allowed_at_ms = excluded.next_allowed_at_ms",
            params![provider.key(), next_allowed_at_ms],
        )
        .map_err(|_| "PIT 공급자 호출 가능 시각을 저장하지 못했습니다.".to_owned())?;
    transaction
        .commit()
        .map_err(|_| "PIT 공급자 호출 제한 저장을 완료하지 못했습니다.".to_owned())?;
    Ok(reserved_at_ms.saturating_sub(now_ms))
}

fn persist_collection_failure(
    bridge: &PersistenceBridge,
    job_id: &str,
    message: &str,
    now_ms: u64,
) -> Result<PitCollectionJob, String> {
    let current = load_collection_job(bridge, job_id)?;
    if current.status == PitCollectionJobStatus::Cancelled {
        return Ok(current);
    }
    let failure_count = current.failure_count.saturating_add(1);
    let retryable =
        is_retryable_collection_error(message) && failure_count < MAX_COLLECTION_FAILURES;
    let (status, next_retry_at_ms, event_type) = if retryable {
        let delay = COLLECTION_RETRY_BASE_MS
            .checked_mul(1_u64 << u32::from(failure_count.saturating_sub(1)))
            .ok_or_else(|| "PIT 재시도 대기 시간을 계산하지 못했습니다.".to_owned())?;
        (
            PitCollectionJobStatus::RetryWait,
            Some(now_ms.saturating_add(delay)),
            "retry_scheduled",
        )
    } else {
        (PitCollectionJobStatus::Failed, None, "failed")
    };
    let safe_message: String = message.chars().take(500).collect();
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 수집 실패 저장을 시작하지 못했습니다.".to_owned())?;
    let changed = transaction
        .execute(
            "UPDATE pit_collection_jobs SET status = ?2, failure_count = ?3,
                    next_retry_at_ms = ?4, last_error = ?5, updated_at_ms = ?6
             WHERE job_id = ?1 AND status = 'running'",
            params![
                job_id,
                status.key(),
                failure_count,
                next_retry_at_ms,
                safe_message,
                now_ms,
            ],
        )
        .map_err(|_| "PIT 수집 실패 상태를 저장하지 못했습니다.".to_owned())?;
    if changed == 1 {
        append_collection_event(
            &transaction,
            job_id,
            event_type,
            json!({
                "failureCount": failure_count,
                "nextRetryAtMs": next_retry_at_ms,
                "error": safe_message,
            }),
            now_ms,
        )?;
    }
    transaction
        .commit()
        .map_err(|_| "PIT 수집 실패 상태 저장을 완료하지 못했습니다.".to_owned())?;
    drop(connection);
    load_collection_job(bridge, job_id)
}

fn persist_collection_page(
    bridge: &PersistenceBridge,
    job_id: &str,
    page: &PitProviderPage,
    now_ms: u64,
) -> Result<PitCollectionJob, String> {
    let (next_start_ms, next_end_exclusive_ms) = match page.page_direction {
        PitPageDirection::Forward => (
            page.next_start_ms
                .unwrap_or(page.requested_end_exclusive_ms),
            page.requested_end_exclusive_ms,
        ),
        PitPageDirection::Backward => (
            page.requested_start_ms,
            page.next_end_exclusive_ms
                .unwrap_or(page.requested_start_ms),
        ),
    };
    let completed = match page.page_direction {
        PitPageDirection::Forward => page.next_start_ms.is_none(),
        PitPageDirection::Backward => page.next_end_exclusive_ms.is_none(),
    };
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 수집 체크포인트 저장을 시작하지 못했습니다.".to_owned())?;
    let changed = transaction
        .execute(
            "UPDATE pit_collection_jobs
             SET cursor_start_ms = CASE WHEN ?2 > cursor_start_ms THEN ?2 ELSE cursor_start_ms END,
                 cursor_end_exclusive_ms = CASE WHEN ?3 < cursor_end_exclusive_ms THEN ?3 ELSE cursor_end_exclusive_ms END,
                 page_count = page_count + 1,
                 observation_count = observation_count + ?4,
                 failure_count = 0, next_retry_at_ms = NULL, last_error = NULL,
                 status = ?5, updated_at_ms = ?6
             WHERE job_id = ?1 AND status = 'running'",
            params![
                job_id,
                next_start_ms,
                next_end_exclusive_ms,
                page.observations.len(),
                if completed { "completed" } else { "running" },
                now_ms,
            ],
        )
        .map_err(|_| "PIT 수집 체크포인트를 저장하지 못했습니다.".to_owned())?;
    if changed == 0 {
        drop(transaction);
        drop(connection);
        return load_collection_job(bridge, job_id);
    }
    append_collection_event(
        &transaction,
        job_id,
        "page_stored",
        json!({
            "direction": page.page_direction,
            "requestedStartMs": page.requested_start_ms,
            "requestedEndExclusiveMs": page.requested_end_exclusive_ms,
            "observationCount": page.observations.len(),
        }),
        now_ms,
    )?;
    if completed {
        append_collection_event(
            &transaction,
            job_id,
            "completed",
            json!({"reason": "provider_cursor_exhausted"}),
            now_ms,
        )?;
    }
    transaction
        .commit()
        .map_err(|_| "PIT 수집 체크포인트 저장을 완료하지 못했습니다.".to_owned())?;
    drop(connection);
    load_collection_job(bridge, job_id)
}

fn release_collection_job(
    bridge: &PersistenceBridge,
    job_id: &str,
    now_ms: u64,
) -> Result<PitCollectionJob, String> {
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 수집 실행권 반환을 시작하지 못했습니다.".to_owned())?;
    let changed = transaction
        .execute(
            "UPDATE pit_collection_jobs SET status = 'queued', updated_at_ms = ?2
             WHERE job_id = ?1 AND status = 'running'",
            params![job_id, now_ms],
        )
        .map_err(|_| "PIT 수집 실행권을 반환하지 못했습니다.".to_owned())?;
    if changed == 1 {
        append_collection_event(
            &transaction,
            job_id,
            "released",
            json!({"reason": "page_budget_exhausted"}),
            now_ms,
        )?;
    }
    transaction
        .commit()
        .map_err(|_| "PIT 수집 실행권 반환을 완료하지 못했습니다.".to_owned())?;
    drop(connection);
    load_collection_job(bridge, job_id)
}

fn cancel_collection_job(
    bridge: &PersistenceBridge,
    job_id: &str,
    now_ms: u64,
) -> Result<PitCollectionJob, String> {
    validate_job_identifier(job_id, "PIT 작업 ID")?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|_| "PIT 수집 취소를 시작하지 못했습니다.".to_owned())?;
    let changed = transaction
        .execute(
            "UPDATE pit_collection_jobs SET status = 'cancelled', next_retry_at_ms = NULL,
                    updated_at_ms = ?2
             WHERE job_id = ?1 AND status IN ('queued','running','retry_wait')",
            params![job_id, now_ms],
        )
        .map_err(|_| "PIT 수집 작업을 취소하지 못했습니다.".to_owned())?;
    if changed == 1 {
        append_collection_event(
            &transaction,
            job_id,
            "cancelled",
            json!({"reason": "user_requested"}),
            now_ms,
        )?;
    }
    transaction
        .commit()
        .map_err(|_| "PIT 수집 취소를 완료하지 못했습니다.".to_owned())?;
    drop(connection);
    load_collection_job(bridge, job_id)
}

fn load_collection_history(
    bridge: &PersistenceBridge,
    request: PitCollectionJobHistoryRequest,
) -> Result<Vec<PitCollectionJob>, String> {
    if request.limit == 0 || request.limit > 100 {
        return Err("PIT 수집 작업 이력은 한 번에 1~100개만 조회할 수 있습니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT job_id, provider, symbol, interval, requested_start_ms,
                    requested_end_exclusive_ms, page_size, status, cursor_start_ms,
                    cursor_end_exclusive_ms, page_count, observation_count, failure_count,
                    next_retry_at_ms, last_error, created_at_ms, updated_at_ms
             FROM pit_collection_jobs ORDER BY updated_at_ms DESC, job_id DESC LIMIT ?1",
        )
        .map_err(|_| "PIT 수집 작업 이력 조회를 준비하지 못했습니다.".to_owned())?;
    let rows = statement
        .query_map(params![request.limit], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
                row.get(9)?,
                row.get(10)?,
                row.get(11)?,
                row.get(12)?,
                row.get(13)?,
                row.get(14)?,
                row.get(15)?,
                row.get(16)?,
            ))
        })
        .map_err(|_| "PIT 수집 작업 이력을 조회하지 못했습니다.".to_owned())?;
    rows.map(|row| {
        row.map_err(|_| "PIT 수집 작업 이력을 해석하지 못했습니다.".to_owned())
            .and_then(decode_collection_job)
    })
    .collect()
}

fn load_collection_detail(
    bridge: &PersistenceBridge,
    job_id: &str,
) -> Result<PitCollectionJobDetail, String> {
    let job = load_collection_job(bridge, job_id)?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 수집 작업 저장소를 사용할 수 없습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT event_index, event_type, detail_json, occurred_at_ms
             FROM pit_collection_job_events WHERE job_id = ?1 ORDER BY event_index ASC",
        )
        .map_err(|_| "PIT 수집 이벤트 조회를 준비하지 못했습니다.".to_owned())?;
    let rows = statement
        .query_map(params![job_id], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
            ))
        })
        .map_err(|_| "PIT 수집 이벤트를 조회하지 못했습니다.".to_owned())?;
    let events = rows
        .map(|row| {
            let (event_index, event_type, detail_json, occurred_at_ms) =
                row.map_err(|_| "PIT 수집 이벤트를 해석하지 못했습니다.".to_owned())?;
            let detail = serde_json::from_str(&detail_json)
                .map_err(|_| "PIT 수집 이벤트 JSON이 올바르지 않습니다.".to_owned())?;
            Ok(PitCollectionJobEvent {
                job_id: job_id.to_owned(),
                event_index,
                event_type,
                detail,
                occurred_at_ms,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(PitCollectionJobDetail { job, events })
}

fn due_collection_job_ids(
    bridge: &PersistenceBridge,
    now_ms: u64,
    limit: u8,
) -> Result<Vec<String>, String> {
    if limit == 0 || limit > MAX_DUE_COLLECTION_JOBS_PER_TICK {
        return Err("PIT 스케줄러 작업 수 제한이 올바르지 않습니다.".to_owned());
    }
    let stale_before = now_ms.saturating_sub(COLLECTION_LEASE_TIMEOUT_MS);
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "PIT 스케줄러 저장소를 사용할 수 없습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT job_id FROM pit_collection_jobs
             WHERE status = 'queued'
                OR (status = 'retry_wait' AND (next_retry_at_ms IS NULL OR next_retry_at_ms <= ?1))
                OR (status = 'running' AND updated_at_ms <= ?2)
             ORDER BY created_at_ms ASC, job_id ASC LIMIT ?3",
        )
        .map_err(|_| "PIT 스케줄러 due 작업 조회를 준비하지 못했습니다.".to_owned())?;
    let rows = statement
        .query_map(params![now_ms, stale_before, limit], |row| row.get(0))
        .map_err(|_| "PIT 스케줄러 due 작업을 조회하지 못했습니다.".to_owned())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|_| "PIT 스케줄러 due 작업을 해석하지 못했습니다.".to_owned())
}

async fn run_collection_job(
    provider_bridge: &PitProviderBridge,
    persistence: &PersistenceBridge,
    job_id: &str,
    maximum_pages: u8,
) -> Result<PitCollectionJob, String> {
    let job_id = validate_job_identifier(job_id, "PIT 작업 ID")?;
    if maximum_pages == 0 || maximum_pages > MAX_COLLECTION_PAGES_PER_RUN {
        return Err(format!(
            "PIT 수집은 한 번에 1~{MAX_COLLECTION_PAGES_PER_RUN}페이지만 실행할 수 있습니다."
        ));
    }
    let mut job = claim_collection_job(persistence, &job_id, paper_trading::now_ms()?)?;
    if job.status != PitCollectionJobStatus::Running {
        return Ok(job);
    }
    for _ in 0..maximum_pages {
        let reservation_time_ms = paper_trading::now_ms()?;
        let wait_ms =
            reserve_provider_request_slot(persistence, job.provider, reservation_time_ms)?;
        if wait_ms > 0 {
            tauri::async_runtime::spawn_blocking(move || {
                std::thread::sleep(Duration::from_millis(wait_ms));
            })
            .await
            .map_err(|_| "PIT 공급자 호출 제한 대기를 완료하지 못했습니다.".to_owned())?;
        }
        job = load_collection_job(persistence, &job_id)?;
        if job.status != PitCollectionJobStatus::Running {
            return Ok(job);
        }
        let page_request = PitProviderPageRequest {
            provider: job.provider,
            symbol: job.symbol.clone(),
            interval: job.interval,
            start_ms: job.cursor_start_ms,
            end_exclusive_ms: job.cursor_end_exclusive_ms,
            limit: job.page_size,
        };
        let fetched_at_ms = paper_trading::now_ms()?;
        let page_result = match job.provider {
            PitOfficialProvider::UpbitSpot => {
                fetch_upbit_page(&page_request, &job.symbol, provider_bridge, fetched_at_ms).await
            }
            PitOfficialProvider::BinanceSpot
            | PitOfficialProvider::BinanceUsdm
            | PitOfficialProvider::BinanceCoinm => {
                fetch_binance_page(&page_request, &job.symbol, provider_bridge, fetched_at_ms).await
            }
        };
        let page = match page_result {
            Ok(page) => page,
            Err(error) => {
                return persist_collection_failure(
                    persistence,
                    &job_id,
                    &error,
                    paper_trading::now_ms()?,
                )
            }
        };
        if let Err(error) = store_page(persistence, page.clone()) {
            return persist_collection_failure(
                persistence,
                &job_id,
                &error,
                paper_trading::now_ms()?,
            );
        }
        job = persist_collection_page(persistence, &job_id, &page, paper_trading::now_ms()?)?;
        if job.status != PitCollectionJobStatus::Running {
            return Ok(job);
        }
    }
    release_collection_job(persistence, &job_id, paper_trading::now_ms()?)
}

async fn run_due_collection_jobs(
    provider_bridge: &PitProviderBridge,
    persistence: &PersistenceBridge,
    runtime: &PitCollectionRuntime,
) -> Result<(), String> {
    let Some(_guard) = runtime.begin_tick() else {
        return Ok(());
    };
    let job_ids = due_collection_job_ids(
        persistence,
        paper_trading::now_ms()?,
        MAX_DUE_COLLECTION_JOBS_PER_TICK,
    )?;
    let mut first_error = None;
    for job_id in job_ids {
        if let Err(error) = run_collection_job(
            provider_bridge,
            persistence,
            &job_id,
            MAX_COLLECTION_PAGES_PER_RUN,
        )
        .await
        {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// PIT 수집 작업은 UI 컴포넌트 수명과 분리해 앱 프로세스에서 반복한다.
/// 작업·lease·체크포인트는 SQLite가 단일 원본이며 외부 주문 경로는 없다.
pub fn start_collection_scheduler(app: AppHandle) {
    let runtime = app.state::<PitCollectionRuntime>();
    if runtime.started.swap(true, Ordering::AcqRel) {
        return;
    }
    drop(runtime);

    tauri::async_runtime::spawn_blocking(move || loop {
        std::thread::sleep(Duration::from_secs(COLLECTION_SCHEDULER_POLL_SECONDS));
        let result = tauri::async_runtime::block_on(async {
            let provider = app.state::<PitProviderBridge>();
            let persistence = app.state::<PersistenceBridge>();
            let runtime = app.state::<PitCollectionRuntime>();
            run_due_collection_jobs(&provider, &persistence, &runtime).await
        });
        if let Err(error) = result {
            eprintln!("PIT 로컬 수집 스케줄러 오류: {error}");
        }
    });
}

#[tauri::command]
pub fn pit_collection_job_create(
    request: PitCollectionJobCreateRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PitCollectionJob, String> {
    create_collection_job(&persistence, request, paper_trading::now_ms()?)
}

#[tauri::command]
pub async fn pit_collection_job_run(
    request: PitCollectionJobRunRequest,
    provider_bridge: State<'_, PitProviderBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PitCollectionJob, String> {
    run_collection_job(
        &provider_bridge,
        &persistence,
        &request.job_id,
        request.maximum_pages,
    )
    .await
}

#[tauri::command]
pub fn pit_collection_job_cancel(
    job_id: String,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PitCollectionJob, String> {
    cancel_collection_job(&persistence, &job_id, paper_trading::now_ms()?)
}

#[tauri::command]
pub fn pit_collection_job_detail(
    job_id: String,
    persistence: State<'_, PersistenceBridge>,
) -> Result<PitCollectionJobDetail, String> {
    let job_id = validate_job_identifier(&job_id, "PIT 작업 ID")?;
    load_collection_detail(&persistence, &job_id)
}

#[tauri::command]
pub fn pit_collection_job_history(
    request: PitCollectionJobHistoryRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<PitCollectionJob>, String> {
    load_collection_history(&persistence, request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(provider: PitOfficialProvider) -> PitProviderPageRequest {
        PitProviderPageRequest {
            provider,
            symbol: if provider == PitOfficialProvider::UpbitSpot {
                "KRW-BTC".to_owned()
            } else {
                "BTCUSDT".to_owned()
            },
            interval: PitProviderInterval::Minute1,
            start_ms: 1_700_000_000_000,
            end_exclusive_ms: 1_700_000_180_000,
            limit: 200,
        }
    }

    #[test]
    fn formats_utc_cursor_without_timezone_dependency() {
        assert_eq!(
            unix_ms_to_rfc3339(0).expect("epoch"),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            unix_ms_to_rfc3339(1_774_137_600_000).expect("date"),
            "2026-03-22T00:00:00Z"
        );
    }

    #[test]
    fn decimal_parser_preserves_eight_places() {
        assert_eq!(
            decimal_scaled(&Value::String("123.45678901".to_owned())).expect("price"),
            12_345_678_901
        );
        assert!(decimal_scaled(&Value::String("-1".to_owned())).is_err());
        assert!(decimal_scaled(&Value::String("0".to_owned())).is_err());
    }

    #[test]
    fn provider_limits_and_symbols_fail_closed() {
        let mut upbit = request(PitOfficialProvider::UpbitSpot);
        assert_eq!(validate_request(&upbit).expect("valid"), "KRW-BTC");
        upbit.limit = 201;
        assert!(validate_request(&upbit).is_err());

        let mut binance = request(PitOfficialProvider::BinanceSpot);
        binance.symbol = "KRW-BTC".to_owned();
        assert!(validate_request(&binance).is_err());

        let mut coin_m = request(PitOfficialProvider::BinanceCoinm);
        coin_m.symbol = "BTCUSD_PERP".to_owned();
        assert_eq!(validate_request(&coin_m).expect("coin-m"), "BTCUSD_PERP");
    }

    #[test]
    fn parses_binance_close_time_as_exclusive_bar_end() {
        let request = request(PitOfficialProvider::BinanceSpot);
        let rows = vec![vec![
            Value::from(1_700_000_000_000_u64),
            Value::from("100.0"),
            Value::from("102.0"),
            Value::from("99.0"),
            Value::from("101.12345678"),
            Value::from("12.0"),
            Value::from(1_700_000_059_999_u64),
        ]];
        let parsed = parse_binance_rows(
            rows,
            request.provider.source(),
            "BTCUSDT",
            &request,
            1_700_000_180_000,
        )
        .expect("parse");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].bar_end_ms, 1_700_000_060_000);
        assert_eq!(parsed[0].close_scaled, 10_112_345_678);
        assert!(parsed[0].source_revision.starts_with("sha256:"));
    }

    #[test]
    fn filters_open_or_out_of_window_binance_bars() {
        let request = request(PitOfficialProvider::BinanceUsdm);
        let rows = vec![vec![
            Value::from(1_700_000_120_000_u64),
            Value::from("100"),
            Value::from("102"),
            Value::from("99"),
            Value::from("101"),
            Value::from("12"),
            Value::from(1_700_000_179_999_u64),
        ]];
        let parsed = parse_binance_rows(
            rows,
            request.provider.source(),
            "BTCUSDT",
            &request,
            1_700_000_150_000,
        )
        .expect("parse");
        assert!(parsed.is_empty());
    }

    #[test]
    fn rejects_duplicate_normalized_bars() {
        let item = observation("SOURCE", "BTCUSDT", 100, 200, 10);
        assert!(ensure_unique_ascending(&[item.clone(), item]).is_err());
    }

    #[test]
    fn immutable_store_reuses_same_observation_and_rejects_revision_change() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let fetched_at_ms = 1_700_000_180_000;
        let page = PitProviderPage {
            provider: PitOfficialProvider::BinanceSpot,
            source: PitOfficialProvider::BinanceSpot.source().to_owned(),
            symbol: "BTCUSDT".to_owned(),
            interval: PitProviderInterval::Minute1,
            requested_start_ms: 1_700_000_000_000,
            requested_end_exclusive_ms: fetched_at_ms,
            fetched_at_ms,
            page_direction: PitPageDirection::Forward,
            next_start_ms: None,
            next_end_exclusive_ms: None,
            observations: vec![observation(
                PitOfficialProvider::BinanceSpot.source(),
                "BTCUSDT",
                1_700_000_060_000,
                fetched_at_ms,
                10_000_000_000,
            )],
            warnings: Vec::new(),
            credentials_required: false,
            live_order_allowed: false,
        };
        let first = store_page(&bridge, page.clone()).expect("first");
        assert_eq!(first.inserted_observation_count, 1);
        let second = store_page(&bridge, page.clone()).expect("second");
        assert_eq!(second.reused_observation_count, 1);

        let mut changed = page;
        changed.observations[0].close_scaled += 1;
        changed.observations[0].source_revision = revision(
            &changed.source,
            &changed.symbol,
            changed.observations[0].bar_end_ms,
            changed.observations[0].close_scaled,
        );
        assert!(store_page(&bridge, changed).is_err());
    }

    #[test]
    fn stored_range_merges_pages_and_reports_internal_gaps() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let fetched_at_ms = 1_700_000_300_000;
        let page = PitProviderPage {
            provider: PitOfficialProvider::BinanceSpot,
            source: PitOfficialProvider::BinanceSpot.source().to_owned(),
            symbol: "BTCUSDT".to_owned(),
            interval: PitProviderInterval::Minute1,
            requested_start_ms: 1_700_000_000_000,
            requested_end_exclusive_ms: fetched_at_ms,
            fetched_at_ms,
            page_direction: PitPageDirection::Forward,
            next_start_ms: None,
            next_end_exclusive_ms: None,
            observations: vec![
                observation(
                    PitOfficialProvider::BinanceSpot.source(),
                    "BTCUSDT",
                    1_700_000_060_000,
                    fetched_at_ms,
                    10_000_000_000,
                ),
                observation(
                    PitOfficialProvider::BinanceSpot.source(),
                    "BTCUSDT",
                    1_700_000_180_000,
                    fetched_at_ms,
                    10_100_000_000,
                ),
            ],
            warnings: Vec::new(),
            credentials_required: false,
            live_order_allowed: false,
        };
        store_page(&bridge, page).expect("store");
        let range = load_stored_range(
            &bridge,
            PitStoredRangeRequest {
                provider: PitOfficialProvider::BinanceSpot,
                symbol: "BTCUSDT".to_owned(),
                interval: PitProviderInterval::Minute1,
                start_ms: 1_700_000_000_000,
                end_exclusive_ms: fetched_at_ms,
                maximum_rows: 20_001,
            },
        )
        .expect("range");
        assert_eq!(range.observations.len(), 2);
        assert_eq!(range.internal_gap_count, 1);
        assert!(!range.truncated);
    }

    fn collection_request(job_id: &str, idempotency_key: &str) -> PitCollectionJobCreateRequest {
        PitCollectionJobCreateRequest {
            job_id: job_id.to_owned(),
            idempotency_key: idempotency_key.to_owned(),
            provider: PitOfficialProvider::BinanceSpot,
            symbol: "BTCUSDT".to_owned(),
            interval: PitProviderInterval::Minute1,
            start_ms: 1_700_000_000_000,
            end_exclusive_ms: 1_700_000_300_000,
            page_size: 2,
        }
    }

    #[test]
    fn collection_create_is_idempotent_and_rejects_changed_request() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = collection_request("job-1", "idem-1");
        let first = create_collection_job(&bridge, request.clone(), 100).expect("create");
        let repeated = create_collection_job(&bridge, request, 200).expect("repeat");
        assert_eq!(first.job_id, repeated.job_id);
        assert_eq!(first.created_at_ms, repeated.created_at_ms);

        let mut changed = collection_request("job-1", "idem-1");
        changed.page_size = 3;
        assert!(create_collection_job(&bridge, changed, 300).is_err());
    }

    #[test]
    fn collection_lease_is_atomic_and_stale_work_is_recovered() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        create_collection_job(&bridge, collection_request("job-lease", "idem-lease"), 100)
            .expect("create");
        let running = claim_collection_job(&bridge, "job-lease", 200).expect("claim");
        assert_eq!(running.status, PitCollectionJobStatus::Running);
        assert!(claim_collection_job(&bridge, "job-lease", 201).is_err());

        let recovered =
            claim_collection_job(&bridge, "job-lease", 200 + COLLECTION_LEASE_TIMEOUT_MS)
                .expect("recover");
        assert_eq!(recovered.status, PitCollectionJobStatus::Running);
        let detail = load_collection_detail(&bridge, "job-lease").expect("detail");
        assert!(detail
            .events
            .iter()
            .any(|event| event.event_type == "recovered"));
    }

    #[test]
    fn collection_checkpoint_advances_forward_and_completes_without_double_count() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        create_collection_job(
            &bridge,
            collection_request("job-progress", "idem-progress"),
            100,
        )
        .expect("create");
        claim_collection_job(&bridge, "job-progress", 200).expect("claim");
        let first_page = PitProviderPage {
            provider: PitOfficialProvider::BinanceSpot,
            source: PitOfficialProvider::BinanceSpot.source().to_owned(),
            symbol: "BTCUSDT".to_owned(),
            interval: PitProviderInterval::Minute1,
            requested_start_ms: 1_700_000_000_000,
            requested_end_exclusive_ms: 1_700_000_300_000,
            fetched_at_ms: 1_700_000_400_000,
            page_direction: PitPageDirection::Forward,
            next_start_ms: Some(1_700_000_120_000),
            next_end_exclusive_ms: None,
            observations: vec![observation(
                PitOfficialProvider::BinanceSpot.source(),
                "BTCUSDT",
                1_700_000_060_000,
                1_700_000_400_000,
                10_000_000_000,
            )],
            warnings: Vec::new(),
            credentials_required: false,
            live_order_allowed: false,
        };
        let progress =
            persist_collection_page(&bridge, "job-progress", &first_page, 300).expect("checkpoint");
        assert_eq!(progress.cursor_start_ms, 1_700_000_120_000);
        assert_eq!(progress.page_count, 1);
        assert_eq!(progress.observation_count, 1);

        let mut final_page = first_page;
        final_page.requested_start_ms = progress.cursor_start_ms;
        final_page.next_start_ms = None;
        final_page.observations.clear();
        let completed =
            persist_collection_page(&bridge, "job-progress", &final_page, 400).expect("complete");
        assert_eq!(completed.status, PitCollectionJobStatus::Completed);
        assert_eq!(completed.page_count, 2);
        assert_eq!(completed.observation_count, 1);
    }

    #[test]
    fn collection_checkpoint_moves_only_backward_cursor_for_upbit() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let mut request = collection_request("job-backward", "idem-backward");
        request.provider = PitOfficialProvider::UpbitSpot;
        request.symbol = "KRW-BTC".to_owned();
        create_collection_job(&bridge, request, 100).expect("create");
        claim_collection_job(&bridge, "job-backward", 200).expect("claim");
        let page = PitProviderPage {
            provider: PitOfficialProvider::UpbitSpot,
            source: PitOfficialProvider::UpbitSpot.source().to_owned(),
            symbol: "KRW-BTC".to_owned(),
            interval: PitProviderInterval::Minute1,
            requested_start_ms: 1_700_000_000_000,
            requested_end_exclusive_ms: 1_700_000_300_000,
            fetched_at_ms: 1_700_000_400_000,
            page_direction: PitPageDirection::Backward,
            next_start_ms: None,
            next_end_exclusive_ms: Some(1_700_000_180_000),
            observations: Vec::new(),
            warnings: Vec::new(),
            credentials_required: false,
            live_order_allowed: false,
        };
        let progress =
            persist_collection_page(&bridge, "job-backward", &page, 300).expect("checkpoint");
        assert_eq!(progress.cursor_start_ms, 1_700_000_000_000);
        assert_eq!(progress.cursor_end_exclusive_ms, 1_700_000_180_000);
        assert_eq!(progress.status, PitCollectionJobStatus::Running);
    }

    #[test]
    fn collection_retry_backoff_is_bounded_and_terminal_on_fourth_failure() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        create_collection_job(&bridge, collection_request("job-retry", "idem-retry"), 100)
            .expect("create");
        let mut now = 200;
        for expected_failure in 1..=MAX_COLLECTION_FAILURES {
            claim_collection_job(&bridge, "job-retry", now).expect("claim");
            let failed = persist_collection_failure(
                &bridge,
                "job-retry",
                "Binance PIT 공개 시세에 연결하지 못했습니다.",
                now + 1,
            )
            .expect("failure");
            assert_eq!(failed.failure_count, expected_failure);
            if expected_failure < MAX_COLLECTION_FAILURES {
                assert_eq!(failed.status, PitCollectionJobStatus::RetryWait);
                now = failed.next_retry_at_ms.expect("retry due");
            } else {
                assert_eq!(failed.status, PitCollectionJobStatus::Failed);
                assert!(failed.next_retry_at_ms.is_none());
            }
        }
    }

    #[test]
    fn provider_request_slots_are_persisted_across_jobs_without_bursting() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        assert_eq!(
            reserve_provider_request_slot(&bridge, PitOfficialProvider::UpbitSpot, 1_000)
                .expect("first"),
            0
        );
        assert_eq!(
            reserve_provider_request_slot(&bridge, PitOfficialProvider::UpbitSpot, 1_000)
                .expect("second"),
            125
        );
        assert_eq!(
            reserve_provider_request_slot(&bridge, PitOfficialProvider::BinanceSpot, 1_000)
                .expect("independent provider"),
            0
        );
    }

    #[test]
    fn collection_cancel_prevents_late_checkpoint_override() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        create_collection_job(
            &bridge,
            collection_request("job-cancel", "idem-cancel"),
            100,
        )
        .expect("create");
        claim_collection_job(&bridge, "job-cancel", 200).expect("claim");
        let cancelled = cancel_collection_job(&bridge, "job-cancel", 300).expect("cancel");
        assert_eq!(cancelled.status, PitCollectionJobStatus::Cancelled);

        let page = PitProviderPage {
            provider: PitOfficialProvider::BinanceSpot,
            source: PitOfficialProvider::BinanceSpot.source().to_owned(),
            symbol: "BTCUSDT".to_owned(),
            interval: PitProviderInterval::Minute1,
            requested_start_ms: 1_700_000_000_000,
            requested_end_exclusive_ms: 1_700_000_300_000,
            fetched_at_ms: 1_700_000_400_000,
            page_direction: PitPageDirection::Forward,
            next_start_ms: None,
            next_end_exclusive_ms: None,
            observations: Vec::new(),
            warnings: Vec::new(),
            credentials_required: false,
            live_order_allowed: false,
        };
        let after_late_page =
            persist_collection_page(&bridge, "job-cancel", &page, 400).expect("late checkpoint");
        assert_eq!(after_late_page.status, PitCollectionJobStatus::Cancelled);
        assert_eq!(after_late_page.page_count, 0);
    }

    #[test]
    fn scheduler_selects_only_due_recoverable_jobs_with_a_bounded_batch() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        for (index, (job_id, idempotency_key)) in [
            ("job-due-queued", "idem-due-queued"),
            ("job-future-retry", "idem-future-retry"),
            ("job-completed", "idem-completed"),
            ("job-stale-running", "idem-stale-running"),
        ]
        .into_iter()
        .enumerate()
        {
            create_collection_job(
                &bridge,
                collection_request(job_id, idempotency_key),
                100 + u64::try_from(index).expect("index"),
            )
            .expect("create");
        }
        {
            let connection = bridge.connection.lock().expect("connection");
            connection
                .execute(
                    "UPDATE pit_collection_jobs SET status = 'retry_wait', next_retry_at_ms = 200000, updated_at_ms = 101 WHERE job_id = 'job-future-retry'",
                    [],
                )
                .expect("future retry");
            connection
                .execute(
                    "UPDATE pit_collection_jobs SET status = 'completed', updated_at_ms = 102 WHERE job_id = 'job-completed'",
                    [],
                )
                .expect("completed");
            connection
                .execute(
                    "UPDATE pit_collection_jobs SET status = 'running', updated_at_ms = 103 WHERE job_id = 'job-stale-running'",
                    [],
                )
                .expect("stale running");
        }

        let due = due_collection_job_ids(&bridge, 100_000, 2).expect("due");
        assert_eq!(
            due,
            vec!["job-due-queued".to_owned(), "job-stale-running".to_owned()]
        );
        assert!(due_collection_job_ids(&bridge, 100_000, 0).is_err());
        assert!(due_collection_job_ids(&bridge, 100_000, 3).is_err());
    }

    #[test]
    fn scheduler_tick_guard_prevents_overlapping_runs_and_releases_on_drop() {
        let runtime = PitCollectionRuntime::default();
        let guard = runtime.begin_tick().expect("first tick");
        assert!(runtime.begin_tick().is_none());
        drop(guard);
        assert!(runtime.begin_tick().is_some());
    }

    #[test]
    #[ignore = "공식 공개 네트워크를 사용하는 읽기 전용 smoke"]
    fn official_gap_backfill_fetches_completed_minute_ohlcv() {
        tauri::async_runtime::block_on(async {
            let bridge = PitProviderBridge::default();
            let now_ms = paper_trading::now_ms().expect("clock");
            let end_exclusive_ms = now_ms / 60_000 * 60_000;
            let start_ms = end_exclusive_ms - 5 * 60_000;
            for (provider, symbol) in [
                (PitOfficialProvider::UpbitSpot, "KRW-BTC"),
                (PitOfficialProvider::BinanceSpot, "BTCUSDT"),
                (PitOfficialProvider::BinanceUsdm, "BTCUSDT"),
                (PitOfficialProvider::BinanceCoinm, "BTCUSD_PERP"),
            ] {
                let bars = fetch_official_minute_bars(
                    provider,
                    symbol,
                    start_ms,
                    end_exclusive_ms,
                    &bridge,
                )
                .await
                .expect("official minute bars");
                assert!(!bars.is_empty(), "{provider:?} returned no completed bars");
                assert!(bars.iter().all(|bar| {
                    bar.period_start_ms >= start_ms
                        && bar.period_end_ms <= end_exclusive_ms
                        && bar.period_end_ms <= bar.ingested_at_ms
                        && bar.period_end_ms - bar.period_start_ms == 60_000
                }));
            }
        });
    }
}
