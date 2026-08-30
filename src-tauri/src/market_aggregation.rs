use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Mutex,
};

use rusqlite::{params, OptionalExtension};
use tauri::State;

use crate::{
    persistence::PersistenceBridge,
    pit_providers::{
        fetch_official_minute_bars, OfficialMinuteBar, PitOfficialProvider, PitProviderBridge,
    },
};

const MINUTE_MS: u64 = 60_000;
const SUPPORTED_MINUTE_INTERVALS: [u16; 7] = [1, 3, 5, 15, 30, 60, 240];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedMarketTick {
    pub provider: String,
    pub asset_class: String,
    pub symbol: String,
    pub currency: String,
    pub event_at_ms: u64,
    pub received_at_ms: u64,
    pub sequence: Option<u64>,
    pub price_minor: u64,
    pub quantity_base_units: u64,
    pub quantity_scale: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedMarketBar {
    pub provider: String,
    pub asset_class: String,
    pub symbol: String,
    pub currency: String,
    pub interval_minutes: u16,
    pub period_start_ms: u64,
    pub period_end_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub open_minor: u64,
    pub high_minor: u64,
    pub low_minor: u64,
    pub close_minor: u64,
    pub volume_base_units: u64,
    pub quantity_scale: u64,
    pub source_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDataGap {
    pub period_start_ms: u64,
    pub period_end_ms: u64,
    pub missing_unit_count: u32,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickAggregationResult {
    pub completed_bars: Vec<CompletedMarketBar>,
    pub partial_bar: Option<CompletedMarketBar>,
    pub gaps: Vec<MarketDataGap>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketStreamTickInput {
    pub stream_id: String,
    pub provider: String,
    pub asset_class: String,
    pub symbol: String,
    pub currency: String,
    pub event_at_ms: u64,
    pub received_at_ms: u64,
    pub sequence: Option<u64>,
    pub price: String,
    pub quantity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketAggregationUpdate {
    pub stream_id: String,
    pub restored_from_checkpoint: bool,
    pub completed_minute_bars: Vec<CompletedMarketBar>,
    pub completed_higher_bars: Vec<CompletedMarketBar>,
    pub partial_bar: Option<CompletedMarketBar>,
    pub gaps: Vec<MarketDataGap>,
    pub retained_minute_bar_count: usize,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketAggregationStatus {
    pub stream_id: String,
    pub provider: String,
    pub symbol: String,
    pub currency: String,
    pub partial_period_start_ms: Option<u64>,
    pub retained_minute_bar_count: usize,
    pub latest_completed_at_ms: Option<u64>,
    pub gap_count: usize,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketGapBackfillRequest {
    pub stream_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketGapBackfillResult {
    pub stream_id: String,
    pub requested_period_start_ms: u64,
    pub requested_period_end_ms: u64,
    pub official_bar_count: usize,
    pub inserted_bar_count: usize,
    pub remaining_gap_count: usize,
    pub updated_at_ms: u64,
    pub source: String,
    pub credentials_required: bool,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamAggregationState {
    stream_id: String,
    provider: String,
    asset_class: String,
    symbol: String,
    currency: String,
    partial_bar: Option<CompletedMarketBar>,
    completed_minute_bars: Vec<CompletedMarketBar>,
    gaps: Vec<MarketDataGap>,
    emitted_upper_end_ms: BTreeMap<u16, u64>,
    last_event_at_ms: Option<u64>,
    last_sequence: Option<u64>,
    last_price_minor: Option<u64>,
    last_quantity_base_units: Option<u64>,
    updated_at_ms: u64,
}

#[derive(Default)]
pub struct MarketAggregationBridge {
    states: Mutex<HashMap<String, StreamAggregationState>>,
    active_backfills: Mutex<HashSet<String>>,
    last_backfill_attempt_ms: Mutex<HashMap<String, u64>>,
}

const RETAINED_MINUTE_BAR_LIMIT: usize = 480;
const RETAINED_GAP_LIMIT: usize = 256;
const QUANTITY_SCALE: u64 = 100_000_000;
const BACKFILL_MINIMUM_INTERVAL_MS: u64 = 2_000;

struct GapBackfillGuard<'a> {
    bridge: &'a MarketAggregationBridge,
    stream_id: String,
}

impl Drop for GapBackfillGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.bridge.active_backfills.lock() {
            active.remove(&self.stream_id);
        }
    }
}

fn begin_gap_backfill<'a>(
    bridge: &'a MarketAggregationBridge,
    stream_id: &str,
    attempted_at_ms: u64,
) -> Result<GapBackfillGuard<'a>, String> {
    let mut active = bridge
        .active_backfills
        .lock()
        .map_err(|_| "REST gap 복구 실행 잠금을 획득하지 못했습니다.".to_owned())?;
    if !active.insert(stream_id.to_owned()) {
        return Err("같은 스트림의 REST gap 복구가 이미 진행 중입니다.".to_owned());
    }
    let mut attempts = match bridge.last_backfill_attempt_ms.lock() {
        Ok(attempts) => attempts,
        Err(_) => {
            active.remove(stream_id);
            return Err("REST gap 복구 요청 제한 상태를 확인하지 못했습니다.".to_owned());
        }
    };
    if attempts.get(stream_id).is_some_and(|previous| {
        attempted_at_ms.saturating_sub(*previous) < BACKFILL_MINIMUM_INTERVAL_MS
    }) {
        active.remove(stream_id);
        return Err("REST gap 복구는 스트림별로 2초 뒤 다시 시도할 수 있습니다.".to_owned());
    }
    attempts.insert(stream_id.to_owned(), attempted_at_ms);
    drop(attempts);
    drop(active);
    Ok(GapBackfillGuard {
        bridge,
        stream_id: stream_id.to_owned(),
    })
}

pub fn aggregate_ticks_to_one_minute(
    ticks: &[NormalizedMarketTick],
    watermark_ms: u64,
) -> Result<TickAggregationResult, String> {
    validate_ticks(ticks, watermark_ms)?;
    if ticks.is_empty() {
        return Ok(TickAggregationResult {
            completed_bars: vec![],
            partial_bar: None,
            gaps: vec![],
        });
    }

    let mut bars = Vec::new();
    let mut current = bar_from_tick(&ticks[0], 1)?;
    for tick in &ticks[1..] {
        let bucket_start = minute_floor(tick.event_at_ms);
        if bucket_start == current.period_start_ms {
            current.high_minor = current.high_minor.max(tick.price_minor);
            current.low_minor = current.low_minor.min(tick.price_minor);
            current.close_minor = tick.price_minor;
            current.volume_base_units = current
                .volume_base_units
                .checked_add(tick.quantity_base_units)
                .ok_or_else(|| "1분봉 거래량 합계가 지원 범위를 초과했습니다.".to_owned())?;
            current.source_count = current
                .source_count
                .checked_add(1)
                .ok_or_else(|| "1분봉 원천 Tick 수가 지원 범위를 초과했습니다.".to_owned())?;
            current.ingested_at_ms = current.ingested_at_ms.max(tick.received_at_ms);
        } else {
            bars.push(current);
            current = bar_from_tick(tick, 1)?;
        }
    }
    bars.push(current);

    let gaps = gaps_between_bars(&bars, MINUTE_MS, "관측 Tick이 없는 1분 구간")?;
    let mut completed_bars = Vec::new();
    let mut partial_bar = None;
    for bar in bars {
        if bar.period_end_ms <= watermark_ms {
            completed_bars.push(bar);
        } else if partial_bar.replace(bar).is_some() {
            return Err("watermark 뒤에 둘 이상의 미완료 1분봉이 있습니다.".to_owned());
        }
    }
    Ok(TickAggregationResult {
        completed_bars,
        partial_bar,
        gaps,
    })
}

pub fn aggregate_completed_minute_bars(
    minute_bars: &[CompletedMarketBar],
    interval_minutes: u16,
) -> Result<(Vec<CompletedMarketBar>, Vec<MarketDataGap>), String> {
    if !SUPPORTED_MINUTE_INTERVALS.contains(&interval_minutes) || interval_minutes == 1 {
        return Err("상위 봉 주기는 3·5·15·30·60·240분 중 하나여야 합니다.".to_owned());
    }
    validate_minute_bars(minute_bars)?;
    if minute_bars.is_empty() {
        return Ok((vec![], vec![]));
    }

    let interval_ms = u64::from(interval_minutes)
        .checked_mul(MINUTE_MS)
        .ok_or_else(|| "상위 봉 주기가 지원 범위를 초과했습니다.".to_owned())?;
    let mut output = Vec::new();
    let mut gaps = gaps_between_bars(minute_bars, MINUTE_MS, "원천 1분봉 누락")?;
    let mut index = 0usize;
    while index < minute_bars.len() {
        let bucket_start = floor_to_interval(minute_bars[index].period_start_ms, interval_ms);
        let bucket_end = bucket_start
            .checked_add(interval_ms)
            .ok_or_else(|| "상위 봉 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        let start_index = index;
        while index < minute_bars.len() && minute_bars[index].period_start_ms < bucket_end {
            index += 1;
        }
        let source = &minute_bars[start_index..index];
        let is_complete = source.len() == usize::from(interval_minutes)
            && source
                .first()
                .is_some_and(|bar| bar.period_start_ms == bucket_start)
            && source
                .last()
                .is_some_and(|bar| bar.period_end_ms == bucket_end)
            && source
                .windows(2)
                .all(|pair| pair[0].period_end_ms == pair[1].period_start_ms);
        if !is_complete {
            gaps.push(MarketDataGap {
                period_start_ms: bucket_start,
                period_end_ms: bucket_end,
                missing_unit_count: u32::from(interval_minutes)
                    .saturating_sub(source.len().try_into().unwrap_or(u32::MAX)),
                reason: format!(
                    "{}분봉 생성에 필요한 완료 1분봉이 부족합니다.",
                    interval_minutes
                ),
            });
            continue;
        }
        output.push(merge_bars(
            source,
            interval_minutes,
            bucket_start,
            bucket_end,
        )?);
    }
    gaps.sort_by_key(|gap| (gap.period_start_ms, gap.period_end_ms));
    gaps.dedup();
    Ok((output, gaps))
}

fn validate_ticks(ticks: &[NormalizedMarketTick], watermark_ms: u64) -> Result<(), String> {
    let mut identity: Option<(&str, &str, &str, &str, u64)> = None;
    let mut previous: Option<&NormalizedMarketTick> = None;
    let mut seen = HashSet::new();
    for tick in ticks {
        if tick.provider.trim().is_empty()
            || tick.asset_class.trim().is_empty()
            || tick.symbol.trim().is_empty()
            || tick.currency.trim().is_empty()
            || tick.price_minor == 0
            || tick.quantity_base_units == 0
            || tick.quantity_scale == 0
            || tick.event_at_ms == 0
            || tick.received_at_ms < tick.event_at_ms
            || tick.received_at_ms > watermark_ms
            || tick.event_at_ms > watermark_ms
        {
            return Err("Tick의 식별자·가격·수량·시각 계약이 올바르지 않습니다.".to_owned());
        }
        let current_identity = (
            tick.provider.as_str(),
            tick.asset_class.as_str(),
            tick.symbol.as_str(),
            tick.currency.as_str(),
            tick.quantity_scale,
        );
        if identity.is_some_and(|expected| expected != current_identity) {
            return Err(
                "서로 다른 공급자·자산·종목·통화·수량 단위 Tick을 함께 집계할 수 없습니다."
                    .to_owned(),
            );
        }
        identity = Some(current_identity);
        if let Some(previous) = previous {
            if tick.event_at_ms < previous.event_at_ms {
                return Err("Tick 이벤트 시각이 역행했습니다.".to_owned());
            }
            if let (Some(previous_sequence), Some(sequence)) = (previous.sequence, tick.sequence) {
                if sequence <= previous_sequence {
                    return Err("Tick 공급자 순번이 중복되거나 역행했습니다.".to_owned());
                }
            }
        }
        let duplicate_key = (
            tick.event_at_ms,
            tick.sequence,
            tick.price_minor,
            tick.quantity_base_units,
        );
        if !seen.insert(duplicate_key) {
            return Err("동일한 Tick이 중복 입력됐습니다.".to_owned());
        }
        previous = Some(tick);
    }
    Ok(())
}

fn validate_minute_bars(bars: &[CompletedMarketBar]) -> Result<(), String> {
    let mut identity: Option<(&str, &str, &str, &str, u64)> = None;
    let mut previous_start = None;
    for bar in bars {
        let expected_end_ms = bar
            .period_start_ms
            .checked_add(MINUTE_MS)
            .ok_or_else(|| "1분봉 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
        if bar.interval_minutes != 1
            || bar.period_start_ms % MINUTE_MS != 0
            || bar.period_end_ms != expected_end_ms
            || bar.available_at_ms < bar.period_end_ms
            || bar.open_minor == 0
            || bar.low_minor > bar.open_minor
            || bar.low_minor > bar.close_minor
            || bar.high_minor < bar.open_minor
            || bar.high_minor < bar.close_minor
            || bar.quantity_scale == 0
            || bar.source_count == 0
        {
            return Err("상위 봉 입력에는 유효한 완료 1분봉만 사용할 수 있습니다.".to_owned());
        }
        let current_identity = (
            bar.provider.as_str(),
            bar.asset_class.as_str(),
            bar.symbol.as_str(),
            bar.currency.as_str(),
            bar.quantity_scale,
        );
        if identity.is_some_and(|expected| expected != current_identity) {
            return Err("서로 다른 원천의 1분봉을 함께 집계할 수 없습니다.".to_owned());
        }
        if previous_start.is_some_and(|start| bar.period_start_ms <= start) {
            return Err("1분봉이 중복되거나 시각이 역행했습니다.".to_owned());
        }
        identity = Some(current_identity);
        previous_start = Some(bar.period_start_ms);
    }
    Ok(())
}

fn bar_from_tick(
    tick: &NormalizedMarketTick,
    interval_minutes: u16,
) -> Result<CompletedMarketBar, String> {
    let period_start_ms = minute_floor(tick.event_at_ms);
    let period_end_ms = period_start_ms
        .checked_add(MINUTE_MS)
        .ok_or_else(|| "1분봉 종료 시각이 지원 범위를 초과했습니다.".to_owned())?;
    Ok(CompletedMarketBar {
        provider: tick.provider.clone(),
        asset_class: tick.asset_class.clone(),
        symbol: tick.symbol.clone(),
        currency: tick.currency.clone(),
        interval_minutes,
        period_start_ms,
        period_end_ms,
        available_at_ms: period_end_ms,
        ingested_at_ms: tick.received_at_ms,
        open_minor: tick.price_minor,
        high_minor: tick.price_minor,
        low_minor: tick.price_minor,
        close_minor: tick.price_minor,
        volume_base_units: tick.quantity_base_units,
        quantity_scale: tick.quantity_scale,
        source_count: 1,
    })
}

fn merge_bars(
    source: &[CompletedMarketBar],
    interval_minutes: u16,
    period_start_ms: u64,
    period_end_ms: u64,
) -> Result<CompletedMarketBar, String> {
    let first = source
        .first()
        .ok_or_else(|| "집계할 1분봉이 없습니다.".to_owned())?;
    let last = source
        .last()
        .ok_or_else(|| "집계할 1분봉이 없습니다.".to_owned())?;
    let volume_base_units = source.iter().try_fold(0_u64, |total, bar| {
        total
            .checked_add(bar.volume_base_units)
            .ok_or_else(|| "상위 봉 거래량 합계가 지원 범위를 초과했습니다.".to_owned())
    })?;
    let source_count = source.iter().try_fold(0_u32, |total, bar| {
        total
            .checked_add(bar.source_count)
            .ok_or_else(|| "상위 봉 원천 Tick 수가 지원 범위를 초과했습니다.".to_owned())
    })?;
    Ok(CompletedMarketBar {
        provider: first.provider.clone(),
        asset_class: first.asset_class.clone(),
        symbol: first.symbol.clone(),
        currency: first.currency.clone(),
        interval_minutes,
        period_start_ms,
        period_end_ms,
        available_at_ms: period_end_ms,
        ingested_at_ms: source
            .iter()
            .map(|bar| bar.ingested_at_ms)
            .max()
            .unwrap_or(period_end_ms),
        open_minor: first.open_minor,
        high_minor: source
            .iter()
            .map(|bar| bar.high_minor)
            .max()
            .unwrap_or(first.high_minor),
        low_minor: source
            .iter()
            .map(|bar| bar.low_minor)
            .min()
            .unwrap_or(first.low_minor),
        close_minor: last.close_minor,
        volume_base_units,
        quantity_scale: first.quantity_scale,
        source_count,
    })
}

fn gaps_between_bars(
    bars: &[CompletedMarketBar],
    unit_ms: u64,
    reason: &str,
) -> Result<Vec<MarketDataGap>, String> {
    let mut gaps = Vec::new();
    for pair in bars.windows(2) {
        if pair[1].period_start_ms > pair[0].period_end_ms {
            let duration = pair[1].period_start_ms - pair[0].period_end_ms;
            let missing_unit_count: u32 = (duration / unit_ms)
                .try_into()
                .map_err(|_| "시장 데이터 gap 길이가 지원 범위를 초과했습니다.".to_owned())?;
            gaps.push(MarketDataGap {
                period_start_ms: pair[0].period_end_ms,
                period_end_ms: pair[1].period_start_ms,
                missing_unit_count,
                reason: reason.to_owned(),
            });
        }
    }
    Ok(gaps)
}

fn minute_floor(timestamp_ms: u64) -> u64 {
    floor_to_interval(timestamp_ms, MINUTE_MS)
}

fn floor_to_interval(timestamp_ms: u64, interval_ms: u64) -> u64 {
    timestamp_ms / interval_ms * interval_ms
}

fn decimal_to_units(raw: &str, scale: u64, label: &str) -> Result<u64, String> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('-')
        || value.starts_with('+')
        || value.contains('e')
        || value.contains('E')
    {
        return Err(format!("{label}은 0보다 큰 일반 소수 문자열이어야 합니다."));
    }
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("{label} 소수 형식이 올바르지 않습니다."));
    }
    let precision = scale.ilog10() as usize;
    if 10_u64.pow(precision as u32) != scale {
        return Err(format!("{label} 단위는 10의 거듭제곱이어야 합니다."));
    }
    let significant_fraction = fraction.trim_end_matches('0');
    if significant_fraction.len() > precision {
        return Err(format!("{label}의 소수 정밀도가 지원 단위를 초과했습니다."));
    }
    let whole_units = whole
        .parse::<u64>()
        .map_err(|_| format!("{label} 정수부가 지원 범위를 초과했습니다."))?
        .checked_mul(scale)
        .ok_or_else(|| format!("{label} 값이 지원 범위를 초과했습니다."))?;
    let padded_fraction = format!("{significant_fraction:0<precision$}");
    let fraction_units = if padded_fraction.is_empty() {
        0
    } else {
        padded_fraction
            .parse::<u64>()
            .map_err(|_| format!("{label} 소수부가 지원 범위를 초과했습니다."))?
    };
    let units = whole_units
        .checked_add(fraction_units)
        .ok_or_else(|| format!("{label} 값이 지원 범위를 초과했습니다."))?;
    if units == 0 {
        return Err(format!("{label}은 0보다 커야 합니다."));
    }
    Ok(units)
}

fn stream_identity(
    stream_id: &str,
) -> Result<(&'static str, &'static str, &'static str, &'static str), String> {
    match stream_id {
        "upbit_spot" => Ok(("Upbit", "crypto_spot", "KRW-BTC", "KRW")),
        "binance_spot" => Ok(("Binance", "crypto_spot", "BTCUSDT", "USD")),
        "binance_usdm" => Ok(("Binance", "crypto_futures", "BTCUSDT", "USD")),
        "binance_coinm" => Ok(("Binance", "crypto_futures", "BTCUSD_PERP", "USD")),
        _ => Err("허용되지 않은 공개 시장 스트림입니다.".to_owned()),
    }
}

fn normalize_stream_tick(input: MarketStreamTickInput) -> Result<NormalizedMarketTick, String> {
    let expected = stream_identity(&input.stream_id)?;
    if (
        input.provider.as_str(),
        input.asset_class.as_str(),
        input.symbol.as_str(),
        input.currency.as_str(),
    ) != expected
    {
        return Err("스트림 ID와 공급자·자산·종목·통화 계약이 일치하지 않습니다.".to_owned());
    }
    if input.sequence.is_none() {
        return Err("공개 체결 Tick에는 공급자 순번이 필요합니다.".to_owned());
    }
    let price_scale = if input.currency == "KRW" { 1 } else { 100 };
    let tick = NormalizedMarketTick {
        provider: input.provider,
        asset_class: input.asset_class,
        symbol: input.symbol,
        currency: input.currency,
        event_at_ms: input.event_at_ms,
        received_at_ms: input.received_at_ms,
        sequence: input.sequence,
        price_minor: decimal_to_units(&input.price, price_scale, "체결 가격")?,
        quantity_base_units: decimal_to_units(&input.quantity, QUANTITY_SCALE, "체결 수량")?,
        quantity_scale: QUANTITY_SCALE,
    };
    validate_ticks(std::slice::from_ref(&tick), tick.received_at_ms)?;
    Ok(tick)
}

fn empty_stream_state(stream_id: String, tick: &NormalizedMarketTick) -> StreamAggregationState {
    StreamAggregationState {
        stream_id,
        provider: tick.provider.clone(),
        asset_class: tick.asset_class.clone(),
        symbol: tick.symbol.clone(),
        currency: tick.currency.clone(),
        partial_bar: None,
        completed_minute_bars: vec![],
        gaps: vec![],
        emitted_upper_end_ms: BTreeMap::new(),
        last_event_at_ms: None,
        last_sequence: None,
        last_price_minor: None,
        last_quantity_base_units: None,
        updated_at_ms: tick.received_at_ms,
    }
}

fn validate_tick_against_state(
    state: &StreamAggregationState,
    tick: &NormalizedMarketTick,
) -> Result<(), String> {
    if state.provider != tick.provider
        || state.asset_class != tick.asset_class
        || state.symbol != tick.symbol
        || state.currency != tick.currency
    {
        return Err("복구된 집계 상태와 새 Tick의 원천 계약이 다릅니다.".to_owned());
    }
    if state
        .completed_minute_bars
        .last()
        .is_some_and(|bar| tick.event_at_ms < bar.period_end_ms)
    {
        return Err("이미 완료된 봉 구간의 지연 Tick은 반영하지 않습니다.".to_owned());
    }
    if state
        .last_event_at_ms
        .is_some_and(|last| tick.event_at_ms < last)
    {
        return Err("Tick 이벤트 시각이 복구된 상태보다 역행했습니다.".to_owned());
    }
    if let (Some(last), Some(sequence)) = (state.last_sequence, tick.sequence) {
        if sequence <= last {
            return Err("Tick 공급자 순번이 복구된 상태와 중복되거나 역행했습니다.".to_owned());
        }
    }
    if state.last_event_at_ms == Some(tick.event_at_ms)
        && state.last_sequence.is_none()
        && tick.sequence.is_none()
        && state.last_price_minor == Some(tick.price_minor)
        && state.last_quantity_base_units == Some(tick.quantity_base_units)
    {
        return Err("순번 없는 동일 Tick이 중복 입력됐습니다.".to_owned());
    }
    Ok(())
}

fn append_gap_before_partial(
    state: &mut StreamAggregationState,
    partial_start_ms: u64,
) -> Result<(), String> {
    if let Some(previous) = state.completed_minute_bars.last() {
        if partial_start_ms > previous.period_end_ms {
            let missing: u32 = ((partial_start_ms - previous.period_end_ms) / MINUTE_MS)
                .try_into()
                .map_err(|_| "시장 데이터 gap 길이가 지원 범위를 초과했습니다.".to_owned())?;
            state.gaps.push(MarketDataGap {
                period_start_ms: previous.period_end_ms,
                period_end_ms: partial_start_ms,
                missing_unit_count: missing,
                reason: "공개 체결 Tick이 없는 1분 구간".to_owned(),
            });
        }
    }
    if state.gaps.len() > RETAINED_GAP_LIMIT {
        state.gaps.drain(..state.gaps.len() - RETAINED_GAP_LIMIT);
    }
    Ok(())
}

fn finalize_partial(
    state: &mut StreamAggregationState,
    watermark_ms: u64,
    force: bool,
) -> Option<CompletedMarketBar> {
    let should_finalize = state
        .partial_bar
        .as_ref()
        .is_some_and(|bar| force || bar.period_end_ms <= watermark_ms);
    if !should_finalize {
        return None;
    }
    let mut completed = state.partial_bar.take()?;
    completed.available_at_ms = completed.period_end_ms.max(watermark_ms);
    state.completed_minute_bars.push(completed.clone());
    if state.completed_minute_bars.len() > RETAINED_MINUTE_BAR_LIMIT {
        state
            .completed_minute_bars
            .drain(..state.completed_minute_bars.len() - RETAINED_MINUTE_BAR_LIMIT);
    }
    Some(completed)
}

fn merge_tick_into_partial(
    state: &mut StreamAggregationState,
    tick: &NormalizedMarketTick,
) -> Result<Option<CompletedMarketBar>, String> {
    let tick_start = minute_floor(tick.event_at_ms);
    let mut completed = None;
    if let Some(partial) = state.partial_bar.as_ref() {
        if tick_start < partial.period_start_ms {
            return Err("새 Tick이 현재 partial 봉보다 이전 구간입니다.".to_owned());
        }
        if tick_start > partial.period_start_ms {
            completed = finalize_partial(state, tick.received_at_ms, true);
        }
    }
    if state.partial_bar.is_none() {
        append_gap_before_partial(state, tick_start)?;
        state.partial_bar = Some(bar_from_tick(tick, 1)?);
    } else if let Some(partial) = state.partial_bar.as_mut() {
        partial.high_minor = partial.high_minor.max(tick.price_minor);
        partial.low_minor = partial.low_minor.min(tick.price_minor);
        partial.close_minor = tick.price_minor;
        partial.volume_base_units = partial
            .volume_base_units
            .checked_add(tick.quantity_base_units)
            .ok_or_else(|| "1분봉 거래량 합계가 지원 범위를 초과했습니다.".to_owned())?;
        partial.source_count = partial
            .source_count
            .checked_add(1)
            .ok_or_else(|| "1분봉 원천 Tick 수가 지원 범위를 초과했습니다.".to_owned())?;
        partial.ingested_at_ms = partial.ingested_at_ms.max(tick.received_at_ms);
    }
    state.last_event_at_ms = Some(tick.event_at_ms);
    state.last_sequence = tick.sequence.or(state.last_sequence);
    state.last_price_minor = Some(tick.price_minor);
    state.last_quantity_base_units = Some(tick.quantity_base_units);
    state.updated_at_ms = tick.received_at_ms;
    Ok(completed)
}

fn collect_new_higher_bars(
    state: &mut StreamAggregationState,
) -> Result<Vec<CompletedMarketBar>, String> {
    let mut output = Vec::new();
    for interval in [3_u16, 5, 15, 30, 60, 240] {
        let (bars, _) = aggregate_completed_minute_bars(&state.completed_minute_bars, interval)?;
        let last_emitted = state
            .emitted_upper_end_ms
            .get(&interval)
            .copied()
            .unwrap_or(0);
        for bar in bars
            .into_iter()
            .filter(|bar| bar.period_end_ms > last_emitted)
        {
            state
                .emitted_upper_end_ms
                .insert(interval, bar.period_end_ms);
            output.push(bar);
        }
    }
    output.sort_by_key(|bar| (bar.period_end_ms, bar.interval_minutes));
    Ok(output)
}

fn validate_checkpoint_state(state: &StreamAggregationState) -> Result<(), String> {
    let expected = stream_identity(&state.stream_id)?;
    if (
        state.provider.as_str(),
        state.asset_class.as_str(),
        state.symbol.as_str(),
        state.currency.as_str(),
    ) != expected
        || state.updated_at_ms == 0
        || state.completed_minute_bars.len() > RETAINED_MINUTE_BAR_LIMIT
        || state.gaps.len() > RETAINED_GAP_LIMIT
        || state
            .emitted_upper_end_ms
            .keys()
            .any(|interval| ![3_u16, 5, 15, 30, 60, 240].contains(interval))
    {
        return Err("시장 집계 체크포인트 계약이 올바르지 않습니다.".to_owned());
    }
    validate_minute_bars(&state.completed_minute_bars)?;
    if let Some(partial) = &state.partial_bar {
        validate_minute_bars(std::slice::from_ref(partial))?;
        if state
            .completed_minute_bars
            .last()
            .is_some_and(|completed| completed.period_end_ms > partial.period_start_ms)
        {
            return Err("시장 집계 partial 봉이 완료 봉과 겹칩니다.".to_owned());
        }
    }
    Ok(())
}

fn load_checkpoint(
    persistence: &PersistenceBridge,
    stream_id: &str,
) -> Result<Option<StreamAggregationState>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "시장 집계 체크포인트 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let state_json: Option<String> = connection
        .query_row(
            "SELECT state_json FROM market_stream_checkpoints WHERE stream_id=?1",
            params![stream_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("시장 집계 체크포인트를 읽지 못했습니다: {error}"))?;
    state_json
        .map(|value| {
            let state = serde_json::from_str::<StreamAggregationState>(&value)
                .map_err(|error| format!("시장 집계 체크포인트가 손상됐습니다: {error}"))?;
            validate_checkpoint_state(&state)?;
            Ok(state)
        })
        .transpose()
}

fn load_all_checkpoints(
    persistence: &PersistenceBridge,
) -> Result<Vec<StreamAggregationState>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "시장 집계 체크포인트 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare("SELECT state_json FROM market_stream_checkpoints ORDER BY stream_id")
        .map_err(|error| format!("시장 집계 체크포인트 조회를 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("시장 집계 체크포인트를 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        let raw =
            row.map_err(|error| format!("시장 집계 체크포인트를 읽지 못했습니다: {error}"))?;
        let state: StreamAggregationState = serde_json::from_str(&raw)
            .map_err(|error| format!("시장 집계 체크포인트가 손상됐습니다: {error}"))?;
        validate_checkpoint_state(&state)?;
        Ok(state)
    })
    .collect()
}

fn save_checkpoint(
    persistence: &PersistenceBridge,
    state: &StreamAggregationState,
) -> Result<(), String> {
    let state_json = serde_json::to_string(state)
        .map_err(|error| format!("시장 집계 체크포인트 직렬화에 실패했습니다: {error}"))?;
    if state_json.len() > 2_000_000 {
        return Err("시장 집계 체크포인트가 저장 한도를 초과했습니다.".to_owned());
    }
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "시장 집계 체크포인트 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "INSERT INTO market_stream_checkpoints(stream_id,state_json,updated_at_ms)
             VALUES(?1,?2,?3)
             ON CONFLICT(stream_id) DO UPDATE SET state_json=excluded.state_json,updated_at_ms=excluded.updated_at_ms",
            params![state.stream_id, state_json, state.updated_at_ms],
        )
        .map_err(|error| format!("시장 집계 체크포인트를 저장하지 못했습니다: {error}"))?;
    Ok(())
}

fn aggregation_update(
    state: &mut StreamAggregationState,
    restored_from_checkpoint: bool,
    completed_minute_bars: Vec<CompletedMarketBar>,
) -> Result<MarketAggregationUpdate, String> {
    let completed_higher_bars = if completed_minute_bars.is_empty() {
        vec![]
    } else {
        collect_new_higher_bars(state)?
    };
    Ok(MarketAggregationUpdate {
        stream_id: state.stream_id.clone(),
        restored_from_checkpoint,
        completed_minute_bars,
        completed_higher_bars,
        partial_bar: state.partial_bar.clone(),
        gaps: state.gaps.clone(),
        retained_minute_bar_count: state.completed_minute_bars.len(),
        updated_at_ms: state.updated_at_ms,
    })
}

pub(crate) fn ingest_stream_tick(
    bridge: &MarketAggregationBridge,
    persistence: &PersistenceBridge,
    input: MarketStreamTickInput,
) -> Result<MarketAggregationUpdate, String> {
    let stream_id = input.stream_id.clone();
    let tick = normalize_stream_tick(input)?;
    let mut states = bridge
        .states
        .lock()
        .map_err(|_| "시장 집계 런타임 잠금을 획득하지 못했습니다.".to_owned())?;
    let restored = if states.contains_key(&stream_id) {
        false
    } else if let Some(checkpoint) = load_checkpoint(persistence, &stream_id)? {
        if checkpoint.stream_id != stream_id {
            return Err("시장 집계 체크포인트의 스트림 ID가 일치하지 않습니다.".to_owned());
        }
        states.insert(stream_id.clone(), checkpoint);
        true
    } else {
        states.insert(
            stream_id.clone(),
            empty_stream_state(stream_id.clone(), &tick),
        );
        false
    };
    let state = states
        .get_mut(&stream_id)
        .ok_or_else(|| "시장 집계 상태를 준비하지 못했습니다.".to_owned())?;
    validate_tick_against_state(state, &tick)?;
    let completed = merge_tick_into_partial(state, &tick)?.into_iter().collect();
    let update = aggregation_update(state, restored, completed)?;
    save_checkpoint(persistence, state)?;
    Ok(update)
}

fn flush_streams(
    bridge: &MarketAggregationBridge,
    persistence: &PersistenceBridge,
    watermark_ms: u64,
) -> Result<Vec<MarketAggregationUpdate>, String> {
    if watermark_ms == 0 {
        return Err("시장 집계 watermark가 올바르지 않습니다.".to_owned());
    }
    let checkpoints = load_all_checkpoints(persistence)?;
    let mut states = bridge
        .states
        .lock()
        .map_err(|_| "시장 집계 런타임 잠금을 획득하지 못했습니다.".to_owned())?;
    for checkpoint in checkpoints {
        states
            .entry(checkpoint.stream_id.clone())
            .or_insert(checkpoint);
    }
    let mut updates = Vec::new();
    for state in states.values_mut() {
        let Some(completed) = finalize_partial(state, watermark_ms, false) else {
            continue;
        };
        state.updated_at_ms = watermark_ms;
        let update = aggregation_update(state, false, vec![completed])?;
        save_checkpoint(persistence, state)?;
        updates.push(update);
    }
    updates.sort_by(|left, right| left.stream_id.cmp(&right.stream_id));
    Ok(updates)
}

fn official_provider_for_stream(stream_id: &str) -> Result<PitOfficialProvider, String> {
    match stream_id {
        "upbit_spot" => Ok(PitOfficialProvider::UpbitSpot),
        "binance_spot" => Ok(PitOfficialProvider::BinanceSpot),
        "binance_usdm" => Ok(PitOfficialProvider::BinanceUsdm),
        "binance_coinm" => Ok(PitOfficialProvider::BinanceCoinm),
        _ => Err("REST gap 복구를 허용하지 않은 시장 스트림입니다.".to_owned()),
    }
}

fn rounded_rescale(value: i64, source_scale: u64, target_scale: u64) -> Result<u64, String> {
    let value =
        u64::try_from(value).map_err(|_| "공식 REST 봉 가격은 양수여야 합니다.".to_owned())?;
    if source_scale == 0 || target_scale == 0 || source_scale % target_scale != 0 {
        return Err("공식 REST 봉 가격 단위를 시장 최소 단위로 변환할 수 없습니다.".to_owned());
    }
    let divisor = source_scale / target_scale;
    value
        .checked_add(divisor / 2)
        .map(|rounded| rounded / divisor)
        .filter(|scaled| *scaled > 0)
        .ok_or_else(|| "공식 REST 봉 가격 변환 결과가 지원 범위를 벗어났습니다.".to_owned())
}

fn official_bar_to_completed(
    state: &StreamAggregationState,
    bar: OfficialMinuteBar,
) -> Result<CompletedMarketBar, String> {
    let price_scale = if state.currency == "KRW" { 1 } else { 100 };
    if bar.quantity_scale != QUANTITY_SCALE {
        return Err("공식 REST 봉 거래량 단위가 스트림 계약과 일치하지 않습니다.".to_owned());
    }
    let completed = CompletedMarketBar {
        provider: state.provider.clone(),
        asset_class: state.asset_class.clone(),
        symbol: state.symbol.clone(),
        currency: state.currency.clone(),
        interval_minutes: 1,
        period_start_ms: bar.period_start_ms,
        period_end_ms: bar.period_end_ms,
        available_at_ms: bar.available_at_ms,
        ingested_at_ms: bar.ingested_at_ms,
        open_minor: rounded_rescale(bar.open_scaled, bar.price_scale, price_scale)?,
        high_minor: rounded_rescale(bar.high_scaled, bar.price_scale, price_scale)?,
        low_minor: rounded_rescale(bar.low_scaled, bar.price_scale, price_scale)?,
        close_minor: rounded_rescale(bar.close_scaled, bar.price_scale, price_scale)?,
        volume_base_units: bar.volume_scaled,
        quantity_scale: bar.quantity_scale,
        source_count: 1,
    };
    validate_minute_bars(std::slice::from_ref(&completed))?;
    Ok(completed)
}

fn recompute_state_gaps(state: &mut StreamAggregationState) -> Result<(), String> {
    let mut gaps = gaps_between_bars(
        &state.completed_minute_bars,
        MINUTE_MS,
        "공식 REST 복구 후에도 남은 1분봉 누락",
    )?;
    if let (Some(last), Some(partial)) = (
        state.completed_minute_bars.last(),
        state.partial_bar.as_ref(),
    ) {
        if partial.period_start_ms > last.period_end_ms {
            let missing_unit_count = ((partial.period_start_ms - last.period_end_ms) / MINUTE_MS)
                .try_into()
                .map_err(|_| "REST 복구 후 gap 길이가 지원 범위를 초과했습니다.".to_owned())?;
            gaps.push(MarketDataGap {
                period_start_ms: last.period_end_ms,
                period_end_ms: partial.period_start_ms,
                missing_unit_count,
                reason: "공식 REST 복구 후에도 남은 partial 이전 누락".to_owned(),
            });
        }
    }
    if gaps.len() > RETAINED_GAP_LIMIT {
        gaps.drain(..gaps.len() - RETAINED_GAP_LIMIT);
    }
    state.gaps = gaps;
    Ok(())
}

fn apply_official_gap_backfill(
    state: &mut StreamAggregationState,
    request_start_ms: u64,
    request_end_ms: u64,
    bars: Vec<OfficialMinuteBar>,
) -> Result<usize, String> {
    if !state
        .gaps
        .iter()
        .any(|gap| gap.period_start_ms <= request_start_ms && gap.period_end_ms >= request_end_ms)
    {
        return Err("REST 복구 대상 gap이 현재 체크포인트와 일치하지 않습니다.".to_owned());
    }
    let mut converted = Vec::with_capacity(bars.len());
    for bar in bars {
        if bar.period_start_ms < request_start_ms
            || bar.period_end_ms > request_end_ms
            || bar.period_end_ms > bar.ingested_at_ms
        {
            return Err("공식 REST 봉이 요청 gap 또는 관측 시각 경계를 벗어났습니다.".to_owned());
        }
        if state
            .completed_minute_bars
            .iter()
            .any(|existing| existing.period_start_ms == bar.period_start_ms)
        {
            continue;
        }
        converted.push(official_bar_to_completed(state, bar)?);
    }
    converted.sort_by_key(|bar| bar.period_start_ms);
    if converted
        .windows(2)
        .any(|pair| pair[0].period_start_ms >= pair[1].period_start_ms)
    {
        return Err("공식 REST 복구 봉에 중복 또는 역순 데이터가 있습니다.".to_owned());
    }
    let inserted = converted.len();
    if inserted == 0 {
        return Ok(0);
    }
    let ingested_at_ms = converted
        .iter()
        .map(|bar| bar.ingested_at_ms)
        .max()
        .unwrap_or(state.updated_at_ms);
    state.completed_minute_bars.extend(converted);
    state
        .completed_minute_bars
        .sort_by_key(|bar| bar.period_start_ms);
    validate_minute_bars(&state.completed_minute_bars)?;
    if state.completed_minute_bars.len() > RETAINED_MINUTE_BAR_LIMIT {
        state
            .completed_minute_bars
            .drain(..state.completed_minute_bars.len() - RETAINED_MINUTE_BAR_LIMIT);
    }
    recompute_state_gaps(state)?;
    state.updated_at_ms = state.updated_at_ms.max(ingested_at_ms);
    validate_checkpoint_state(state)?;
    Ok(inserted)
}

async fn backfill_first_gap(
    request: MarketGapBackfillRequest,
    bridge: &MarketAggregationBridge,
    provider_bridge: &PitProviderBridge,
    persistence: &PersistenceBridge,
) -> Result<MarketGapBackfillResult, String> {
    let stream_id = request.stream_id.trim().to_owned();
    let provider = official_provider_for_stream(&stream_id)?;
    let checkpoint = load_checkpoint(persistence, &stream_id)?
        .ok_or_else(|| "REST 복구에 필요한 시장 집계 체크포인트가 없습니다.".to_owned())?;
    let gap = checkpoint
        .gaps
        .first()
        .cloned()
        .ok_or_else(|| "복구할 시장 데이터 gap이 없습니다.".to_owned())?;
    let provider_limit = if provider == PitOfficialProvider::UpbitSpot {
        200_u64
    } else {
        1_000_u64
    };
    let request_end_ms = gap
        .period_start_ms
        .checked_add(u64::from(gap.missing_unit_count).min(provider_limit) * MINUTE_MS)
        .ok_or_else(|| "REST 복구 종료 시각이 지원 범위를 초과했습니다.".to_owned())?
        .min(gap.period_end_ms);
    let official_bars = fetch_official_minute_bars(
        provider,
        &checkpoint.symbol,
        gap.period_start_ms,
        request_end_ms,
        provider_bridge,
    )
    .await?;
    let official_bar_count = official_bars.len();
    let mut states = bridge
        .states
        .lock()
        .map_err(|_| "시장 집계 런타임 잠금을 획득하지 못했습니다.".to_owned())?;
    let state = if let Some(state) = states.get_mut(&stream_id) {
        state
    } else {
        states.insert(stream_id.clone(), checkpoint.clone());
        states
            .get_mut(&stream_id)
            .ok_or_else(|| "시장 집계 체크포인트를 복구하지 못했습니다.".to_owned())?
    };
    if state.updated_at_ms != checkpoint.updated_at_ms {
        return Err(
            "REST 조회 중 스트림 상태가 변경됐습니다. 최신 gap으로 다시 시도해 주세요.".to_owned(),
        );
    }
    let mut candidate = state.clone();
    let inserted_bar_count = apply_official_gap_backfill(
        &mut candidate,
        gap.period_start_ms,
        request_end_ms,
        official_bars,
    )?;
    save_checkpoint(persistence, &candidate)?;
    *state = candidate;
    Ok(MarketGapBackfillResult {
        stream_id,
        requested_period_start_ms: gap.period_start_ms,
        requested_period_end_ms: request_end_ms,
        official_bar_count,
        inserted_bar_count,
        remaining_gap_count: state.gaps.len(),
        updated_at_ms: state.updated_at_ms,
        source: provider.source().to_owned(),
        credentials_required: false,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn market_stream_tick_ingest(
    input: MarketStreamTickInput,
    bridge: State<'_, MarketAggregationBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<MarketAggregationUpdate, String> {
    ingest_stream_tick(&bridge, &persistence, input)
}

#[tauri::command]
pub async fn market_stream_gap_backfill(
    request: MarketGapBackfillRequest,
    bridge: State<'_, MarketAggregationBridge>,
    provider_bridge: State<'_, PitProviderBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<MarketGapBackfillResult, String> {
    let stream_id = request.stream_id.trim().to_owned();
    official_provider_for_stream(&stream_id)?;
    let _guard = begin_gap_backfill(&bridge, &stream_id, crate::paper_trading::now_ms()?)?;
    backfill_first_gap(request, &bridge, &provider_bridge, &persistence).await
}

#[tauri::command]
pub fn market_stream_aggregation_flush(
    watermark_ms: u64,
    bridge: State<'_, MarketAggregationBridge>,
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<MarketAggregationUpdate>, String> {
    flush_streams(&bridge, &persistence, watermark_ms)
}

#[tauri::command]
pub fn market_stream_aggregation_status(
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<MarketAggregationStatus>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "시장 집계 체크포인트 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare("SELECT state_json FROM market_stream_checkpoints ORDER BY stream_id")
        .map_err(|error| format!("시장 집계 상태 조회를 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("시장 집계 상태를 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        let raw = row.map_err(|error| format!("시장 집계 상태를 읽지 못했습니다: {error}"))?;
        let state: StreamAggregationState = serde_json::from_str(&raw)
            .map_err(|error| format!("시장 집계 체크포인트가 손상됐습니다: {error}"))?;
        validate_checkpoint_state(&state)?;
        Ok(MarketAggregationStatus {
            stream_id: state.stream_id,
            provider: state.provider,
            symbol: state.symbol,
            currency: state.currency,
            partial_period_start_ms: state.partial_bar.map(|bar| bar.period_start_ms),
            retained_minute_bar_count: state.completed_minute_bars.len(),
            latest_completed_at_ms: state
                .completed_minute_bars
                .last()
                .map(|bar| bar.period_end_ms),
            gap_count: state.gaps.len(),
            updated_at_ms: state.updated_at_ms,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(
        minute: u64,
        offset_ms: u64,
        sequence: u64,
        price: u64,
        quantity: u64,
    ) -> NormalizedMarketTick {
        let event_at_ms = minute * MINUTE_MS + offset_ms;
        NormalizedMarketTick {
            provider: "TEST_STREAM".to_owned(),
            asset_class: "crypto_spot".to_owned(),
            symbol: "KRW-BTC".to_owned(),
            currency: "KRW".to_owned(),
            event_at_ms,
            received_at_ms: event_at_ms + 10,
            sequence: Some(sequence),
            price_minor: price,
            quantity_base_units: quantity,
            quantity_scale: 100_000_000,
        }
    }

    #[test]
    fn aggregates_ticks_into_completed_ohlcv_without_using_partial_bar() {
        let ticks = vec![
            tick(10, 1_000, 1, 100, 2),
            tick(10, 20_000, 2, 120, 3),
            tick(10, 50_000, 3, 90, 5),
            tick(11, 1_000, 4, 110, 7),
        ];
        let result = aggregate_ticks_to_one_minute(&ticks, 11 * MINUTE_MS + 30_000).unwrap();
        assert_eq!(result.completed_bars.len(), 1);
        assert_eq!(
            result.partial_bar.as_ref().unwrap().period_start_ms,
            11 * MINUTE_MS
        );
        let bar = &result.completed_bars[0];
        assert_eq!(
            (
                bar.open_minor,
                bar.high_minor,
                bar.low_minor,
                bar.close_minor
            ),
            (100, 120, 90, 90)
        );
        assert_eq!(bar.volume_base_units, 10);
        assert_eq!(bar.source_count, 3);
        assert_eq!(bar.available_at_ms, bar.period_end_ms);
    }

    #[test]
    fn records_missing_minutes_without_fabricating_flat_bars() {
        let ticks = vec![tick(1, 1, 1, 100, 1), tick(4, 1, 2, 110, 1)];
        let result = aggregate_ticks_to_one_minute(&ticks, 5 * MINUTE_MS).unwrap();
        assert_eq!(result.completed_bars.len(), 2);
        assert_eq!(result.gaps.len(), 1);
        assert_eq!(result.gaps[0].missing_unit_count, 2);
    }

    #[test]
    fn rejects_duplicate_regressing_future_and_mixed_ticks() {
        let first = tick(1, 1, 1, 100, 1);
        assert!(
            aggregate_ticks_to_one_minute(&[first.clone(), first.clone()], 2 * MINUTE_MS).is_err()
        );
        assert!(aggregate_ticks_to_one_minute(
            &[tick(2, 1, 2, 100, 1), tick(1, 1, 3, 100, 1)],
            3 * MINUTE_MS
        )
        .is_err());
        assert!(aggregate_ticks_to_one_minute(
            &[tick(1, 1, 2, 100, 1), tick(1, 2, 1, 100, 1)],
            2 * MINUTE_MS
        )
        .is_err());
        assert!(aggregate_ticks_to_one_minute(&[tick(3, 1, 1, 100, 1)], 2 * MINUTE_MS).is_err());
        let mut future_receive = tick(1, 1, 1, 100, 1);
        future_receive.received_at_ms = 3 * MINUTE_MS;
        assert!(aggregate_ticks_to_one_minute(&[future_receive], 2 * MINUTE_MS).is_err());
        let mut mixed = tick(1, 2, 2, 101, 1);
        mixed.symbol = "KRW-ETH".to_owned();
        assert!(aggregate_ticks_to_one_minute(&[first, mixed], 2 * MINUTE_MS).is_err());
    }

    #[test]
    fn builds_only_fully_aligned_higher_timeframe_bars() {
        let ticks = (0..15)
            .map(|minute| tick(minute, 1, minute + 1, 100 + minute, 1))
            .collect::<Vec<_>>();
        let one_minute = aggregate_ticks_to_one_minute(&ticks, 15 * MINUTE_MS)
            .unwrap()
            .completed_bars;
        let (five_minute, gaps) = aggregate_completed_minute_bars(&one_minute, 5).unwrap();
        assert_eq!(five_minute.len(), 3);
        assert!(gaps.is_empty());
        assert_eq!(five_minute[0].open_minor, 100);
        assert_eq!(five_minute[0].close_minor, 104);
        assert_eq!(five_minute[0].source_count, 5);
    }

    #[test]
    fn refuses_to_emit_a_higher_bar_when_one_minute_is_missing() {
        let ticks = [0, 1, 3, 4]
            .into_iter()
            .enumerate()
            .map(|(index, minute)| tick(minute, 1, index as u64 + 1, 100 + minute, 1))
            .collect::<Vec<_>>();
        let one_minute = aggregate_ticks_to_one_minute(&ticks, 5 * MINUTE_MS)
            .unwrap()
            .completed_bars;
        let (five_minute, gaps) = aggregate_completed_minute_bars(&one_minute, 5).unwrap();
        assert!(five_minute.is_empty());
        assert!(gaps.iter().any(|gap| gap.missing_unit_count == 1));
    }

    #[test]
    fn supports_the_declared_upper_minute_intervals_and_rejects_others() {
        let one_minute = aggregate_ticks_to_one_minute(&[tick(0, 1, 1, 100, 1)], MINUTE_MS)
            .unwrap()
            .completed_bars;
        for interval in [3, 5, 15, 30, 60, 240] {
            assert!(aggregate_completed_minute_bars(&one_minute, interval).is_ok());
        }
        for interval in [0, 1, 2, 120] {
            assert!(aggregate_completed_minute_bars(&one_minute, interval).is_err());
        }
    }

    #[test]
    fn detects_volume_overflow_in_tick_and_higher_bar_aggregation() {
        let ticks = vec![tick(0, 1, 1, 100, u64::MAX), tick(0, 2, 2, 100, 1)];
        assert!(aggregate_ticks_to_one_minute(&ticks, MINUTE_MS).is_err());

        let first = bar_from_tick(&tick(0, 1, 1, 100, u64::MAX), 1).unwrap();
        let second = bar_from_tick(&tick(1, 1, 2, 100, 1), 1).unwrap();
        let third = bar_from_tick(&tick(2, 1, 3, 100, 1), 1).unwrap();
        assert!(aggregate_completed_minute_bars(&[first, second, third], 3).is_err());
    }

    #[test]
    fn rejects_misaligned_or_overflowing_completed_minute_bars() {
        let valid = bar_from_tick(&tick(0, 1, 1, 100, 1), 1).unwrap();
        let mut misaligned = valid.clone();
        misaligned.period_start_ms = 1;
        misaligned.period_end_ms = MINUTE_MS + 1;
        assert!(aggregate_completed_minute_bars(&[misaligned], 3).is_err());

        let mut overflowing = valid;
        overflowing.period_start_ms = u64::MAX;
        overflowing.period_end_ms = u64::MAX;
        assert!(aggregate_completed_minute_bars(&[overflowing], 3).is_err());
    }

    fn stream_input(
        event_at_ms: u64,
        sequence: u64,
        price: &str,
        quantity: &str,
    ) -> MarketStreamTickInput {
        MarketStreamTickInput {
            stream_id: "upbit_spot".to_owned(),
            provider: "Upbit".to_owned(),
            asset_class: "crypto_spot".to_owned(),
            symbol: "KRW-BTC".to_owned(),
            currency: "KRW".to_owned(),
            event_at_ms,
            received_at_ms: event_at_ms + 10,
            sequence: Some(sequence),
            price: price.to_owned(),
            quantity: quantity.to_owned(),
        }
    }

    fn official_minute_bar(minute: u64, price: i64, ingested_at_ms: u64) -> OfficialMinuteBar {
        OfficialMinuteBar {
            period_start_ms: minute * MINUTE_MS,
            period_end_ms: (minute + 1) * MINUTE_MS,
            available_at_ms: (minute + 1) * MINUTE_MS,
            ingested_at_ms,
            open_scaled: price * 100_000_000,
            high_scaled: (price + 2) * 100_000_000,
            low_scaled: (price - 2) * 100_000_000,
            close_scaled: (price + 1) * 100_000_000,
            volume_scaled: 100_000_000,
            price_scale: 100_000_000,
            quantity_scale: QUANTITY_SCALE,
        }
    }

    #[test]
    fn official_rest_backfill_repairs_only_the_recorded_gap() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let runtime = MarketAggregationBridge::default();
        ingest_stream_tick(
            &runtime,
            &persistence,
            stream_input(MINUTE_MS + 1_000, 1, "100", "0.1"),
        )
        .expect("first minute");
        ingest_stream_tick(
            &runtime,
            &persistence,
            stream_input(4 * MINUTE_MS + 1_000, 2, "110", "0.1"),
        )
        .expect("gap minute");

        let mut states = runtime.states.lock().expect("runtime");
        let state = states.get_mut("upbit_spot").expect("state");
        assert_eq!(state.gaps[0].missing_unit_count, 2);
        let inserted = apply_official_gap_backfill(
            state,
            2 * MINUTE_MS,
            4 * MINUTE_MS,
            vec![
                official_minute_bar(2, 102, 5 * MINUTE_MS),
                official_minute_bar(3, 105, 5 * MINUTE_MS),
            ],
        )
        .expect("backfill");
        assert_eq!(inserted, 2);
        assert!(state.gaps.is_empty());
        assert_eq!(state.completed_minute_bars.len(), 3);
        assert_eq!(state.completed_minute_bars[1].open_minor, 102);
        assert_eq!(state.completed_minute_bars[2].close_minor, 106);
        assert_eq!(state.last_sequence, Some(2));
    }

    #[test]
    fn official_rest_backfill_rejects_bars_outside_the_gap_or_observation_time() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let runtime = MarketAggregationBridge::default();
        ingest_stream_tick(
            &runtime,
            &persistence,
            stream_input(MINUTE_MS + 1_000, 1, "100", "0.1"),
        )
        .expect("first minute");
        ingest_stream_tick(
            &runtime,
            &persistence,
            stream_input(4 * MINUTE_MS + 1_000, 2, "110", "0.1"),
        )
        .expect("gap minute");

        let mut states = runtime.states.lock().expect("runtime");
        let state = states.get_mut("upbit_spot").expect("state");
        assert!(apply_official_gap_backfill(
            state,
            2 * MINUTE_MS,
            4 * MINUTE_MS,
            vec![official_minute_bar(1, 100, 5 * MINUTE_MS)],
        )
        .is_err());
        assert!(apply_official_gap_backfill(
            state,
            2 * MINUTE_MS,
            4 * MINUTE_MS,
            vec![official_minute_bar(2, 102, 2 * MINUTE_MS)],
        )
        .is_err());
        assert_eq!(state.gaps[0].missing_unit_count, 2);
    }

    #[test]
    fn official_rest_backfill_rejects_overlap_and_request_bursts() {
        let runtime = MarketAggregationBridge::default();
        let guard = begin_gap_backfill(&runtime, "upbit_spot", 10_000).expect("first attempt");
        assert!(begin_gap_backfill(&runtime, "upbit_spot", 10_001)
            .err()
            .expect("overlap")
            .contains("진행 중"));
        drop(guard);
        assert!(begin_gap_backfill(&runtime, "upbit_spot", 11_999)
            .err()
            .expect("rate limit")
            .contains("2초"));
        assert!(begin_gap_backfill(&runtime, "upbit_spot", 12_000).is_ok());
    }

    #[test]
    fn stateful_stream_ingestion_restores_partial_and_completed_bars() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let first_runtime = MarketAggregationBridge::default();
        let first = ingest_stream_tick(
            &first_runtime,
            &persistence,
            stream_input(1_000, 1, "100000000", "0.1"),
        )
        .expect("first tick");
        assert!(first.completed_minute_bars.is_empty());
        assert_eq!(first.partial_bar.expect("partial").source_count, 1);

        let second = ingest_stream_tick(
            &first_runtime,
            &persistence,
            stream_input(MINUTE_MS + 1_000, 2, "100000100", "0.2"),
        )
        .expect("next minute");
        assert_eq!(second.completed_minute_bars.len(), 1);

        let restored_runtime = MarketAggregationBridge::default();
        let restored = ingest_stream_tick(
            &restored_runtime,
            &persistence,
            stream_input(MINUTE_MS + 2_000, 3, "100000200", "0.3"),
        )
        .expect("restored tick");
        assert!(restored.restored_from_checkpoint);
        assert_eq!(
            restored.partial_bar.expect("restored partial").source_count,
            2
        );

        let flushed = flush_streams(&restored_runtime, &persistence, 2 * MINUTE_MS)
            .expect("flush completed minute");
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].completed_minute_bars.len(), 1);
    }

    #[test]
    fn stateful_stream_rejects_duplicate_sequence_and_unsupported_precision() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let runtime = MarketAggregationBridge::default();
        ingest_stream_tick(
            &runtime,
            &persistence,
            stream_input(1_000, 1, "100000000", "0.1"),
        )
        .expect("first tick");
        assert!(ingest_stream_tick(
            &runtime,
            &persistence,
            stream_input(2_000, 1, "100000001", "0.1"),
        )
        .expect_err("duplicate sequence")
        .contains("중복"));

        let mut invalid = stream_input(3_000, 2, "100000000.1", "0.1");
        invalid.currency = "KRW".to_owned();
        assert!(normalize_stream_tick(invalid)
            .expect_err("KRW precision")
            .contains("정밀도"));
    }

    #[test]
    fn flush_restores_a_saved_partial_before_any_new_tick_arrives() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let first_runtime = MarketAggregationBridge::default();
        ingest_stream_tick(
            &first_runtime,
            &persistence,
            stream_input(1_000, 1, "100000000", "0.1"),
        )
        .expect("partial checkpoint");

        let restarted_runtime = MarketAggregationBridge::default();
        let updates =
            flush_streams(&restarted_runtime, &persistence, MINUTE_MS).expect("restart flush");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].completed_minute_bars.len(), 1);
        assert!(updates[0].partial_bar.is_none());
    }
}
