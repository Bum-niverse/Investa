use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, State};
use tokio::time::{interval_at, Instant, MissedTickBehavior};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::header::AUTHORIZATION, Error, Message},
};

const MAX_TOPICS_PER_CONNECTION: usize = 100;
const TOSS_STREAM_URL: &str = "wss://openapi-ws.tossinvest.com/ws/v1";
const KEEPALIVE_SECONDS: u64 = 60;
const ACK_TIMEOUT_SECONDS: u64 = 15;
const MAX_RECONNECT_DELAY_SECONDS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TossMarketStreamKind {
    TradeKr,
    TradeUs,
    OrderbookKr,
    OrderbookUs,
}

impl TossMarketStreamKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::TradeKr => "trade:kr",
            Self::TradeUs => "trade:us",
            Self::OrderbookKr => "orderbook:kr",
            Self::OrderbookUs => "orderbook:us",
        }
    }

    fn market(self) -> TossMarket {
        match self {
            Self::TradeKr | Self::OrderbookKr => TossMarket::Kr,
            Self::TradeUs | Self::OrderbookUs => TossMarket::Us,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TossMarket {
    Kr,
    Us,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TossMarketSubscription {
    pub kind: TossMarketStreamKind,
    pub symbols: Vec<String>,
}

/// 토스 실시간 구독은 매 선언마다 기존 구독 전체를 교체한다.
/// 이 빌더는 개인 주문 채널을 의도적으로 지원하지 않아 SHADOW ONLY 경계를 유지한다.
pub fn build_market_subscription_declaration(
    request_id: &str,
    subscriptions: &[TossMarketSubscription],
) -> Result<String, String> {
    let request_id = request_id.trim();
    if request_id.is_empty() || request_id.len() > 64 {
        return Err("구독 요청 ID는 1~64자여야 합니다.".to_string());
    }
    if subscriptions.is_empty() {
        return Err("구독할 시장 데이터가 없습니다.".to_string());
    }

    let mut topic_count = 0usize;
    let mut seen_topics = HashSet::new();
    let mut declarations = Vec::with_capacity(subscriptions.len() + 1);
    declarations.push(json!({ "id": request_id }));

    for subscription in subscriptions {
        if subscription.symbols.is_empty() {
            return Err(format!(
                "{} 구독에는 종목이 한 개 이상 필요합니다.",
                subscription.kind.as_str()
            ));
        }

        let mut codes = Vec::with_capacity(subscription.symbols.len());
        for raw_symbol in &subscription.symbols {
            let symbol = normalize_symbol(subscription.kind.market(), raw_symbol)?;
            let topic = format!("{}:{symbol}", subscription.kind.as_str());
            if !seen_topics.insert(topic.clone()) {
                return Err(format!("중복 구독 항목입니다: {topic}"));
            }
            topic_count += 1;
            if topic_count > MAX_TOPICS_PER_CONNECTION {
                return Err(format!(
                    "토스 실시간 연결당 구독 한도는 {MAX_TOPICS_PER_CONNECTION}개입니다."
                ));
            }
            codes.push(symbol);
        }

        declarations.push(json!({
            "type": subscription.kind.as_str(),
            "codes": codes,
        }));
    }

    serde_json::to_string(&declarations).map_err(|error| format!("구독 선언 생성 실패: {error}"))
}

fn normalize_symbol(market: TossMarket, raw_symbol: &str) -> Result<String, String> {
    let symbol = raw_symbol.trim().to_ascii_uppercase();
    let valid = match market {
        TossMarket::Kr => symbol.len() == 6 && symbol.bytes().all(|value| value.is_ascii_digit()),
        TossMarket::Us => {
            (1..=12).contains(&symbol.len())
                && symbol
                    .bytes()
                    .all(|value| value.is_ascii_alphanumeric() || value == b'.' || value == b'-')
        }
    };
    if !valid {
        return Err(format!("지원하지 않는 종목 코드 형식입니다: {raw_symbol}"));
    }
    Ok(symbol)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossRejectedSubscription {
    pub target: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossTradeTick {
    pub topic: String,
    pub symbol: String,
    pub market: String,
    pub price: String,
    pub volume: String,
    pub timestamp: String,
    pub currency: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TossOrderbookLevel {
    pub price: String,
    pub volume: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossOrderbookTick {
    pub topic: String,
    pub symbol: String,
    pub market: String,
    pub timestamp: String,
    pub currency: String,
    pub asks: Vec<TossOrderbookLevel>,
    pub bids: Vec<TossOrderbookLevel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TossMarketStreamFrame {
    Pong,
    Subscriptions {
        request_id: Option<String>,
        subscribed: Vec<String>,
        rejected: Vec<TossRejectedSubscription>,
    },
    Error {
        request_id: Option<String>,
        code: String,
        message: String,
        reconnect_required: bool,
    },
    Trade(TossTradeTick),
    Orderbook(TossOrderbookTick),
}

pub fn parse_market_stream_frame(raw: &str) -> Result<TossMarketStreamFrame, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| format!("토스 실시간 메시지 JSON이 올바르지 않습니다: {error}"))?;
    let frame_type = required_string(&value, "type")?;

    match frame_type {
        "pong" => Ok(TossMarketStreamFrame::Pong),
        "subscriptions" => parse_subscriptions_frame(&value),
        "error" => parse_error_frame(&value),
        "message" => parse_market_message(&value),
        other => Err(format!(
            "지원하지 않는 토스 실시간 메시지 유형입니다: {other}"
        )),
    }
}

fn parse_subscriptions_frame(value: &Value) -> Result<TossMarketStreamFrame, String> {
    let subscribed = string_array(value, "subscribed")?;
    for topic in &subscribed {
        if topic.starts_with("personal:") {
            return Err(
                "개인 주문 실시간 채널은 SHADOW ONLY 모드에서 처리하지 않습니다.".to_string(),
            );
        }
        parse_market_topic(topic)?;
    }

    let rejected_values = value
        .get("rejected")
        .and_then(Value::as_array)
        .ok_or_else(|| "구독 응답에 rejected 배열이 없습니다.".to_string())?;
    let mut rejected = Vec::with_capacity(rejected_values.len());
    for item in rejected_values {
        let target = required_string(item, "target")?;
        if target.starts_with("personal:") {
            return Err(
                "개인 주문 실시간 채널은 SHADOW ONLY 모드에서 처리하지 않습니다.".to_string(),
            );
        }
        rejected.push(TossRejectedSubscription {
            target: target.to_string(),
            code: required_string(item, "code")?.to_string(),
            message: required_string(item, "message")?.to_string(),
        });
    }

    Ok(TossMarketStreamFrame::Subscriptions {
        request_id: optional_string(value, "id"),
        subscribed,
        rejected,
    })
}

fn parse_error_frame(value: &Value) -> Result<TossMarketStreamFrame, String> {
    let error = value
        .get("error")
        .and_then(Value::as_object)
        .ok_or_else(|| "오류 응답에 error 객체가 없습니다.".to_string())?;
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "오류 응답에 code가 없습니다.".to_string())?
        .to_string();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "오류 응답에 message가 없습니다.".to_string())?
        .to_string();

    Ok(TossMarketStreamFrame::Error {
        request_id: optional_string(value, "id"),
        reconnect_required: code == "server-shutdown",
        code,
        message,
    })
}

fn parse_market_message(value: &Value) -> Result<TossMarketStreamFrame, String> {
    let topic = required_string(value, "topic")?;
    if topic.starts_with("personal:") {
        return Err("개인 주문 실시간 채널은 SHADOW ONLY 모드에서 처리하지 않습니다.".to_string());
    }
    let (channel, market, symbol) = parse_market_topic(topic)?;
    let data = value
        .get("data")
        .ok_or_else(|| "시장 메시지에 data가 없습니다.".to_string())?;
    let timestamp = required_string(data, "timestamp")?;
    if crate::market_data::parse_rfc3339_ms(timestamp).is_none() {
        return Err("시장 메시지의 timestamp 형식이 올바르지 않습니다.".to_string());
    }
    let currency = required_string(data, "currency")?;
    validate_currency(market, currency)?;

    match channel {
        "trade" => {
            let price = required_positive_decimal_string(data, "price")?;
            let volume = required_non_negative_decimal_string(data, "volume")?;
            Ok(TossMarketStreamFrame::Trade(TossTradeTick {
                topic: topic.to_string(),
                symbol: symbol.to_string(),
                market: market_label(market).to_string(),
                price: price.to_string(),
                volume: volume.to_string(),
                timestamp: timestamp.to_string(),
                currency: currency.to_string(),
            }))
        }
        "orderbook" => Ok(TossMarketStreamFrame::Orderbook(TossOrderbookTick {
            topic: topic.to_string(),
            symbol: symbol.to_string(),
            market: market_label(market).to_string(),
            timestamp: timestamp.to_string(),
            currency: currency.to_string(),
            asks: parse_orderbook_levels(data, "asks")?,
            bids: parse_orderbook_levels(data, "bids")?,
        })),
        _ => unreachable!("parse_market_topic only returns supported channels"),
    }
}

fn parse_market_topic(topic: &str) -> Result<(&str, TossMarket, &str), String> {
    let mut parts = topic.split(':');
    let channel = parts.next().unwrap_or_default();
    let market_raw = parts.next().unwrap_or_default();
    let symbol = parts.next().unwrap_or_default();
    if parts.next().is_some() || !matches!(channel, "trade" | "orderbook") {
        return Err(format!("지원하지 않는 시장 topic입니다: {topic}"));
    }
    let market = match market_raw {
        "kr" => TossMarket::Kr,
        "us" => TossMarket::Us,
        _ => return Err(format!("지원하지 않는 시장 topic입니다: {topic}")),
    };
    let normalized = normalize_symbol(market, symbol)?;
    if normalized != symbol {
        return Err(format!("정규화되지 않은 시장 topic입니다: {topic}"));
    }
    Ok((channel, market, symbol))
}

fn parse_orderbook_levels(value: &Value, field: &str) -> Result<Vec<TossOrderbookLevel>, String> {
    let items = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("호가 메시지에 {field} 배열이 없습니다."))?;
    if items.is_empty() {
        return Err(format!("호가 메시지의 {field} 배열이 비어 있습니다."));
    }
    items
        .iter()
        .map(|item| {
            Ok(TossOrderbookLevel {
                price: required_positive_decimal_string(item, "price")?.to_string(),
                volume: required_non_negative_decimal_string(item, "volume")?.to_string(),
            })
        })
        .collect()
}

fn required_non_negative_decimal_string<'a>(
    value: &'a Value,
    field: &str,
) -> Result<&'a str, String> {
    let raw = required_string(value, field)?;
    let parsed = raw
        .parse::<f64>()
        .map_err(|_| format!("{field} 값이 숫자 문자열이 아닙니다."))?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(format!("{field} 값은 0 이상의 유한한 숫자여야 합니다."));
    }
    Ok(raw)
}

fn required_positive_decimal_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    let raw = required_non_negative_decimal_string(value, field)?;
    if raw.parse::<f64>().unwrap_or_default() == 0.0 {
        return Err(format!("{field} 값은 0보다 커야 합니다."));
    }
    Ok(raw)
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("메시지에 {field} 문자열이 없습니다."))
}

fn optional_string(value: &Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array(value: &Value, field: &str) -> Result<Vec<String>, String> {
    let values = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("메시지에 {field} 배열이 없습니다."))?;
    values
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("{field} 배열에는 문자열만 허용됩니다."))
        })
        .collect()
}

fn validate_currency(market: TossMarket, currency: &str) -> Result<(), String> {
    let expected = match market {
        TossMarket::Kr => "KRW",
        TossMarket::Us => "USD",
    };
    if currency != expected {
        return Err(format!(
            "시장과 통화가 일치하지 않습니다: 기대 {expected}, 수신 {currency}"
        ));
    }
    Ok(())
}

fn market_label(market: TossMarket) -> &'static str {
    match market {
        TossMarket::Kr => "KR",
        TossMarket::Us => "US",
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TossMarketStreamStartRequest {
    pub kr_symbols: Vec<String>,
    pub us_symbols: Vec<String>,
    pub include_orderbook: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TossMarketStreamStatus {
    pub phase: String,
    pub attempt: u32,
    pub configured_topics: usize,
    pub subscribed_topics: usize,
    pub rejected_topics: usize,
    pub trade_message_count: u64,
    pub orderbook_message_count: u64,
    pub completed_minute_bar_count: u64,
    pub connected_at_ms: Option<u64>,
    pub last_received_at_ms: Option<u64>,
    pub last_pong_at_ms: Option<u64>,
    pub next_reconnect_at_ms: Option<u64>,
    pub issue: Option<String>,
    pub live_order_allowed: bool,
}

impl Default for TossMarketStreamStatus {
    fn default() -> Self {
        Self {
            phase: "idle".to_owned(),
            attempt: 0,
            configured_topics: 0,
            subscribed_topics: 0,
            rejected_topics: 0,
            trade_message_count: 0,
            orderbook_message_count: 0,
            completed_minute_bar_count: 0,
            connected_at_ms: None,
            last_received_at_ms: None,
            last_pong_at_ms: None,
            next_reconnect_at_ms: None,
            issue: None,
            live_order_allowed: false,
        }
    }
}

#[derive(Default)]
pub struct TossMarketStreamBridge {
    generation: AtomicU64,
    status: Mutex<TossMarketStreamStatus>,
}

fn stream_subscriptions(
    request: &TossMarketStreamStartRequest,
) -> Result<Vec<TossMarketSubscription>, String> {
    let mut subscriptions = Vec::new();
    if !request.kr_symbols.is_empty() {
        subscriptions.push(TossMarketSubscription {
            kind: TossMarketStreamKind::TradeKr,
            symbols: request.kr_symbols.clone(),
        });
        if request.include_orderbook {
            subscriptions.push(TossMarketSubscription {
                kind: TossMarketStreamKind::OrderbookKr,
                symbols: request.kr_symbols.clone(),
            });
        }
    }
    if !request.us_symbols.is_empty() {
        subscriptions.push(TossMarketSubscription {
            kind: TossMarketStreamKind::TradeUs,
            symbols: request.us_symbols.clone(),
        });
        if request.include_orderbook {
            subscriptions.push(TossMarketSubscription {
                kind: TossMarketStreamKind::OrderbookUs,
                symbols: request.us_symbols.clone(),
            });
        }
    }
    if subscriptions.is_empty() {
        return Err("토스 실시간 구독에는 국장 또는 미장 종목이 한 개 이상 필요합니다.".to_owned());
    }
    Ok(subscriptions)
}

fn configured_topic_count(subscriptions: &[TossMarketSubscription]) -> usize {
    subscriptions
        .iter()
        .map(|subscription| subscription.symbols.len())
        .sum()
}

fn update_status(
    bridge: &TossMarketStreamBridge,
    generation: u64,
    update: impl FnOnce(&mut TossMarketStreamStatus),
) -> bool {
    if bridge.generation.load(Ordering::SeqCst) != generation {
        return false;
    }
    if let Ok(mut status) = bridge.status.lock() {
        update(&mut status);
        true
    } else {
        false
    }
}

fn clear_resolved_stream_issue(status: &mut TossMarketStreamStatus) {
    if status.rejected_topics == 0 && status.subscribed_topics == status.configured_topics {
        status.issue = None;
    }
}

fn now_ms() -> u64 {
    crate::paper_trading::now_ms().unwrap_or_default()
}

fn reconnect_delay(attempt: u32, generation: u64) -> Duration {
    let exponent = attempt.saturating_sub(1).min(5);
    let base = 1u64 << exponent;
    let capped = base.min(MAX_RECONNECT_DELAY_SECONDS);
    let jitter_ms = (generation
        .wrapping_mul(1_103_515_245)
        .wrapping_add(attempt as u64 * 97)
        % (capped.saturating_mul(200).max(1))) as u64;
    Duration::from_millis(capped * 1_000 + jitter_ms)
}

fn sanitized_connect_error(error: &Error) -> (&'static str, bool, bool) {
    match error {
        Error::Http(response) if response.status().as_u16() == 401 => (
            "토스 실시간 인증이 거부되었습니다. 토큰을 갱신해 다시 연결합니다.",
            true,
            true,
        ),
        Error::Http(response) if response.status().as_u16() == 403 => (
            "현재 IP가 토스증권 Open API 허용 목록에 없어 실시간 연결이 거부되었습니다.",
            false,
            false,
        ),
        Error::Http(response) if response.status().as_u16() == 429 => (
            "토스 실시간 연결 한도를 초과했습니다. 백오프 후 다시 연결합니다.",
            true,
            false,
        ),
        Error::Http(response) if response.status().as_u16() == 503 => (
            "토스 실시간 서버가 일시적으로 사용할 수 없습니다.",
            true,
            false,
        ),
        _ => ("토스 실시간 서버에 연결하지 못했습니다.", true, false),
    }
}

fn trade_to_aggregation_input(
    tick: &TossTradeTick,
    received_at_ms: u64,
) -> Result<crate::market_aggregation::MarketStreamTickInput, String> {
    let event_at_ms = crate::market_data::parse_rfc3339_ms(&tick.timestamp)
        .ok_or_else(|| "토스 체결 시각을 변환하지 못했습니다.".to_owned())?;
    if event_at_ms > received_at_ms.saturating_add(60_000) {
        return Err("토스 체결 시각이 현재 관측 시각보다 지나치게 미래입니다.".to_owned());
    }
    Ok(crate::market_aggregation::MarketStreamTickInput {
        stream_id: format!("toss_{}_{}", tick.market.to_ascii_lowercase(), tick.symbol),
        provider: "Toss".to_owned(),
        asset_class: "stock".to_owned(),
        symbol: tick.symbol.clone(),
        currency: tick.currency.clone(),
        event_at_ms,
        received_at_ms,
        sequence: None,
        price: tick.price.clone(),
        quantity: tick.volume.clone(),
    })
}

fn ingest_trade_tick(
    app: &AppHandle,
    tick: &TossTradeTick,
    received_at_ms: u64,
) -> Result<usize, String> {
    let input = trade_to_aggregation_input(tick, received_at_ms)?;
    let aggregation = app.state::<crate::market_aggregation::MarketAggregationBridge>();
    let persistence = app.state::<crate::persistence::PersistenceBridge>();
    let update = crate::market_aggregation::ingest_stream_tick(&aggregation, &persistence, input)?;
    Ok(update.completed_minute_bars.len())
}

async fn wait_for_reconnect_or_stop(app: &AppHandle, generation: u64, delay: Duration) -> bool {
    let bridge = app.state::<TossMarketStreamBridge>();
    let deadline = Instant::now() + delay;
    loop {
        if bridge.generation.load(Ordering::SeqCst) != generation {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        tokio::time::sleep((deadline - now).min(Duration::from_millis(250))).await;
    }
}

async fn run_toss_stream(
    app: AppHandle,
    generation: u64,
    declaration: String,
    configured_topics: usize,
) {
    let mut attempt = 0u32;
    loop {
        let bridge = app.state::<TossMarketStreamBridge>();
        if bridge.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        attempt = attempt.saturating_add(1);
        update_status(&bridge, generation, |status| {
            status.phase = if attempt == 1 {
                "connecting"
            } else {
                "reconnecting"
            }
            .to_owned();
            status.attempt = attempt;
            status.next_reconnect_at_ms = None;
            status.issue = None;
        });

        let market_data = app.state::<crate::market_data::MarketDataBridge>();
        let access_token = match market_data.toss_stream_access_token().await {
            Ok(token) => token,
            Err(message) => {
                update_status(&bridge, generation, |status| {
                    status.phase = "error".to_owned();
                    status.issue = Some(message);
                });
                return;
            }
        };
        let mut handshake = match TOSS_STREAM_URL.into_client_request() {
            Ok(request) => request,
            Err(_) => {
                update_status(&bridge, generation, |status| {
                    status.phase = "error".to_owned();
                    status.issue = Some("고정 토스 WebSocket 주소가 올바르지 않습니다.".to_owned());
                });
                return;
            }
        };
        let authorization = match format!("Bearer {access_token}").parse() {
            Ok(value) => value,
            Err(_) => {
                update_status(&bridge, generation, |status| {
                    status.phase = "error".to_owned();
                    status.issue = Some("토스 실시간 인증 헤더를 구성하지 못했습니다.".to_owned());
                });
                return;
            }
        };
        handshake.headers_mut().insert(AUTHORIZATION, authorization);

        let socket = match connect_async(handshake).await {
            Ok((socket, _)) => socket,
            Err(error) => {
                let (message, retry, refresh_token) = sanitized_connect_error(&error);
                if refresh_token {
                    let _ = market_data.clear_toss_stream_access_token();
                }
                if !retry {
                    update_status(&bridge, generation, |status| {
                        status.phase = "error".to_owned();
                        status.issue = Some(message.to_owned());
                    });
                    return;
                }
                let delay = reconnect_delay(attempt, generation);
                update_status(&bridge, generation, |status| {
                    status.phase = "reconnecting".to_owned();
                    status.issue = Some(message.to_owned());
                    status.next_reconnect_at_ms =
                        Some(now_ms().saturating_add(delay.as_millis() as u64));
                });
                if !wait_for_reconnect_or_stop(&app, generation, delay).await {
                    return;
                }
                continue;
            }
        };

        let (mut writer, mut reader) = socket.split();
        if writer
            .send(Message::Text(declaration.clone().into()))
            .await
            .is_err()
        {
            let delay = reconnect_delay(attempt, generation);
            update_status(&bridge, generation, |status| {
                status.phase = "reconnecting".to_owned();
                status.issue = Some("토스 실시간 구독 선언을 전송하지 못했습니다.".to_owned());
                status.next_reconnect_at_ms =
                    Some(now_ms().saturating_add(delay.as_millis() as u64));
            });
            if !wait_for_reconnect_or_stop(&app, generation, delay).await {
                return;
            }
            continue;
        }

        let connected_at_ms = now_ms();
        update_status(&bridge, generation, |status| {
            status.phase = "awaiting_ack".to_owned();
            status.attempt = attempt;
            status.configured_topics = configured_topics;
            status.connected_at_ms = Some(connected_at_ms);
            status.next_reconnect_at_ms = None;
            status.issue = None;
        });
        let mut keepalive = interval_at(
            Instant::now() + Duration::from_secs(KEEPALIVE_SECONDS),
            Duration::from_secs(KEEPALIVE_SECONDS),
        );
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut cancellation = interval_at(
            Instant::now() + Duration::from_secs(1),
            Duration::from_secs(1),
        );
        cancellation.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let ack_timeout = tokio::time::sleep(Duration::from_secs(ACK_TIMEOUT_SECONDS));
        tokio::pin!(ack_timeout);
        let mut acknowledged = false;
        let mut reconnect_reason = "토스 실시간 연결이 종료되었습니다.".to_owned();

        loop {
            tokio::select! {
                _ = cancellation.tick() => {
                    if bridge.generation.load(Ordering::SeqCst) != generation {
                        let _ = writer.send(Message::Close(None)).await;
                        return;
                    }
                }
                _ = &mut ack_timeout, if !acknowledged => {
                    reconnect_reason = "토스 실시간 구독 확인이 제한 시간 안에 도착하지 않았습니다.".to_owned();
                    break;
                }
                _ = keepalive.tick() => {
                    if writer.send(Message::Text("PING".into())).await.is_err() {
                        reconnect_reason = "토스 실시간 keepalive 전송에 실패했습니다.".to_owned();
                        break;
                    }
                }
                incoming = reader.next() => {
                    let Some(incoming) = incoming else {
                        break;
                    };
                    let message = match incoming {
                        Ok(message) => message,
                        Err(_) => {
                            reconnect_reason = "토스 실시간 수신이 중단되었습니다.".to_owned();
                            break;
                        }
                    };
                    let raw = match message {
                        Message::Text(text) => text.to_string(),
                        Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                            Ok(text) => text,
                            Err(_) => {
                                update_status(&bridge, generation, |status| {
                                    status.issue = Some("토스 실시간 이진 메시지가 UTF-8이 아닙니다.".to_owned());
                                });
                                continue;
                            }
                        },
                        Message::Ping(payload) => {
                            if writer.send(Message::Pong(payload)).await.is_err() {
                                reconnect_reason = "토스 실시간 표준 pong 전송에 실패했습니다.".to_owned();
                                break;
                            }
                            continue;
                        }
                        Message::Pong(_) => continue,
                        Message::Close(_) => break,
                        _ => continue,
                    };
                    let received_at_ms = now_ms();
                    match parse_market_stream_frame(&raw) {
                        Ok(TossMarketStreamFrame::Pong) => {
                            update_status(&bridge, generation, |status| {
                                status.last_pong_at_ms = Some(received_at_ms);
                                status.last_received_at_ms = Some(received_at_ms);
                            });
                        }
                        Ok(TossMarketStreamFrame::Subscriptions { subscribed, rejected, .. }) => {
                            acknowledged = true;
                            let rejected_count = rejected.len();
                            update_status(&bridge, generation, |status| {
                                status.phase = if subscribed.is_empty() { "error" } else { "live" }.to_owned();
                                status.subscribed_topics = subscribed.len();
                                status.rejected_topics = rejected_count;
                                status.last_received_at_ms = Some(received_at_ms);
                                status.issue = if rejected_count > 0 {
                                    Some(format!("구독 {rejected_count}건이 공급자에 의해 거부되었습니다."))
                                } else if subscribed.len() != configured_topics {
                                    Some("확정된 구독 수가 요청 수와 일치하지 않습니다.".to_owned())
                                } else {
                                    None
                                };
                            });
                            if subscribed.is_empty() {
                                return;
                            }
                        }
                        Ok(TossMarketStreamFrame::Error { code, reconnect_required, .. }) => {
                            update_status(&bridge, generation, |status| {
                                status.issue = Some(format!("토스 실시간 프로토콜 오류: {code}"));
                            });
                            if reconnect_required || code == "rate-limit-exceeded" || code == "internal-error" {
                                reconnect_reason = format!("토스 실시간 서버 재연결 필요: {code}");
                                break;
                            }
                        }
                        Ok(TossMarketStreamFrame::Trade(tick)) => {
                            match ingest_trade_tick(&app, &tick, received_at_ms) {
                                Ok(completed_count) => update_status(&bridge, generation, |status| {
                                    status.phase = "live".to_owned();
                                    status.trade_message_count = status.trade_message_count.saturating_add(1);
                                    status.completed_minute_bar_count = status.completed_minute_bar_count.saturating_add(completed_count as u64);
                                    status.last_received_at_ms = Some(received_at_ms);
                                    clear_resolved_stream_issue(status);
                                }),
                                Err(issue) => update_status(&bridge, generation, |status| {
                                    status.issue = Some(issue);
                                    status.last_received_at_ms = Some(received_at_ms);
                                }),
                            };
                        }
                        Ok(TossMarketStreamFrame::Orderbook(_)) => {
                            update_status(&bridge, generation, |status| {
                                status.phase = "live".to_owned();
                                status.orderbook_message_count = status.orderbook_message_count.saturating_add(1);
                                status.last_received_at_ms = Some(received_at_ms);
                            });
                        }
                        Err(issue) => {
                            update_status(&bridge, generation, |status| {
                                status.issue = Some(issue);
                                status.last_received_at_ms = Some(received_at_ms);
                            });
                        }
                    }
                }
            }
        }
        let _ = writer.send(Message::Close(None)).await;
        let delay = reconnect_delay(attempt, generation);
        update_status(&bridge, generation, |status| {
            status.phase = "reconnecting".to_owned();
            status.issue = Some(reconnect_reason);
            status.next_reconnect_at_ms = Some(now_ms().saturating_add(delay.as_millis() as u64));
        });
        if !wait_for_reconnect_or_stop(&app, generation, delay).await {
            return;
        }
    }
}

#[tauri::command]
pub fn toss_market_stream_start(
    request: TossMarketStreamStartRequest,
    app: AppHandle,
    bridge: State<'_, TossMarketStreamBridge>,
) -> Result<TossMarketStreamStatus, String> {
    let subscriptions = stream_subscriptions(&request)?;
    let generation = bridge
        .generation
        .fetch_add(1, Ordering::SeqCst)
        .saturating_add(1);
    let configured_topics = configured_topic_count(&subscriptions);
    let declaration =
        build_market_subscription_declaration(&format!("investa-{generation}"), &subscriptions)?;
    let status = TossMarketStreamStatus {
        phase: "connecting".to_owned(),
        configured_topics,
        ..TossMarketStreamStatus::default()
    };
    *bridge
        .status
        .lock()
        .map_err(|_| "토스 실시간 상태 잠금을 획득하지 못했습니다.".to_owned())? = status.clone();
    tauri::async_runtime::spawn(run_toss_stream(
        app,
        generation,
        declaration,
        configured_topics,
    ));
    Ok(status)
}

#[tauri::command]
pub fn toss_market_stream_stop(
    bridge: State<'_, TossMarketStreamBridge>,
) -> Result<TossMarketStreamStatus, String> {
    bridge.generation.fetch_add(1, Ordering::SeqCst);
    let mut status = bridge
        .status
        .lock()
        .map_err(|_| "토스 실시간 상태 잠금을 획득하지 못했습니다.".to_owned())?;
    status.phase = "stopped".to_owned();
    status.next_reconnect_at_ms = None;
    status.issue = None;
    Ok(status.clone())
}

#[tauri::command]
pub fn toss_market_stream_status(
    bridge: State<'_, TossMarketStreamBridge>,
) -> Result<TossMarketStreamStatus, String> {
    bridge
        .status
        .lock()
        .map(|status| status.clone())
        .map_err(|_| "토스 실시간 상태 잠금을 획득하지 못했습니다.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_full_replace_market_subscription_without_personal_channels() {
        let payload = build_market_subscription_declaration(
            "req-1",
            &[
                TossMarketSubscription {
                    kind: TossMarketStreamKind::TradeUs,
                    symbols: vec!["aapl".to_string(), "TSLA".to_string()],
                },
                TossMarketSubscription {
                    kind: TossMarketStreamKind::OrderbookKr,
                    symbols: vec!["005930".to_string()],
                },
            ],
        )
        .expect("declaration");
        let value: Value = serde_json::from_str(&payload).expect("json");

        assert_eq!(value[0], json!({ "id": "req-1" }));
        assert_eq!(value[1]["type"], "trade:us");
        assert_eq!(value[1]["codes"], json!(["AAPL", "TSLA"]));
        assert_eq!(value[2]["type"], "orderbook:kr");
        assert!(!payload.contains("personal:order"));
    }

    #[test]
    fn rejects_duplicate_invalid_and_over_limit_topics() {
        let duplicate = build_market_subscription_declaration(
            "req-2",
            &[TossMarketSubscription {
                kind: TossMarketStreamKind::TradeUs,
                symbols: vec!["AAPL".to_string(), "aapl".to_string()],
            }],
        );
        assert!(duplicate.expect_err("duplicate").contains("중복"));

        let invalid = build_market_subscription_declaration(
            "req-3",
            &[TossMarketSubscription {
                kind: TossMarketStreamKind::TradeKr,
                symbols: vec!["삼성전자".to_string()],
            }],
        );
        assert!(invalid.expect_err("invalid").contains("종목 코드"));

        let too_many = build_market_subscription_declaration(
            "req-4",
            &[TossMarketSubscription {
                kind: TossMarketStreamKind::TradeUs,
                symbols: (0..101).map(|index| format!("A{index}")).collect(),
            }],
        );
        assert!(too_many.expect_err("limit").contains("100개"));
    }

    #[test]
    fn parses_ack_partial_rejection_and_shutdown() {
        let ack = parse_market_stream_frame(
            r#"{"type":"subscriptions","id":"req-2","subscribed":["trade:kr:005930"],"rejected":[{"target":"trade:kr:999999","code":"stock-not-found","message":"not found"}]}"#,
        )
        .expect("ack");
        match ack {
            TossMarketStreamFrame::Subscriptions {
                request_id,
                subscribed,
                rejected,
            } => {
                assert_eq!(request_id.as_deref(), Some("req-2"));
                assert_eq!(subscribed, vec!["trade:kr:005930"]);
                assert_eq!(rejected[0].code, "stock-not-found");
            }
            other => panic!("unexpected frame: {other:?}"),
        }

        let shutdown = parse_market_stream_frame(
            r#"{"type":"error","error":{"code":"server-shutdown","message":"restart"}}"#,
        )
        .expect("shutdown");
        assert!(matches!(
            shutdown,
            TossMarketStreamFrame::Error {
                reconnect_required: true,
                ..
            }
        ));
    }

    #[test]
    fn parses_trade_and_orderbook_without_float_rounding() {
        let trade = parse_market_stream_frame(
            r#"{"type":"message","topic":"trade:us:AAPL","data":{"price":"243.2600","volume":"8","timestamp":"2026-06-18T23:30:00+09:00","currency":"USD"}}"#,
        )
        .expect("trade");
        match trade {
            TossMarketStreamFrame::Trade(tick) => {
                assert_eq!(tick.price, "243.2600");
                assert_eq!(tick.symbol, "AAPL");
                assert_eq!(tick.market, "US");
            }
            other => panic!("unexpected frame: {other:?}"),
        }

        let orderbook = parse_market_stream_frame(
            r#"{"type":"message","topic":"orderbook:kr:005930","data":{"timestamp":"2026-06-18T23:30:00+09:00","currency":"KRW","asks":[{"price":"71500","volume":"5"}],"bids":[{"price":"71400","volume":"10"}]}}"#,
        )
        .expect("orderbook");
        match orderbook {
            TossMarketStreamFrame::Orderbook(tick) => {
                assert_eq!(tick.asks[0].price, "71500");
                assert_eq!(tick.bids[0].volume, "10");
            }
            other => panic!("unexpected frame: {other:?}"),
        }
    }

    #[test]
    fn rejects_personal_order_and_inconsistent_market_data() {
        let personal =
            parse_market_stream_frame(r#"{"type":"message","topic":"personal:order:3","data":{}}"#);
        assert!(personal.expect_err("personal").contains("SHADOW ONLY"));

        let wrong_currency = parse_market_stream_frame(
            r#"{"type":"message","topic":"trade:kr:005930","data":{"price":"100","volume":"1","timestamp":"2026-06-18T23:30:00+09:00","currency":"USD"}}"#,
        );
        assert!(wrong_currency.expect_err("currency").contains("통화"));

        let negative = parse_market_stream_frame(
            r#"{"type":"message","topic":"trade:us:AAPL","data":{"price":"-1","volume":"1","timestamp":"2026-06-18T23:30:00+09:00","currency":"USD"}}"#,
        );
        assert!(negative.expect_err("negative").contains("0 이상"));

        let zero_price = parse_market_stream_frame(
            r#"{"type":"message","topic":"trade:us:AAPL","data":{"price":"0","volume":"0","timestamp":"2026-06-18T23:30:00+09:00","currency":"USD"}}"#,
        );
        assert!(zero_price.expect_err("zero price").contains("0보다 커야"));

        let unknown_ack = parse_market_stream_frame(
            r#"{"type":"subscriptions","subscribed":["news:us:AAPL"],"rejected":[]}"#,
        );
        assert!(unknown_ack.expect_err("unknown topic").contains("topic"));
    }

    #[test]
    fn parses_pong_and_rejects_unknown_frames() {
        assert_eq!(
            parse_market_stream_frame(r#"{"type":"pong"}"#).expect("pong"),
            TossMarketStreamFrame::Pong
        );
        assert!(parse_market_stream_frame(r#"{"type":"mystery"}"#).is_err());
    }

    #[test]
    fn runtime_request_builds_market_topics_only() {
        let subscriptions = stream_subscriptions(&TossMarketStreamStartRequest {
            kr_symbols: vec!["005930".to_owned()],
            us_symbols: vec!["AAPL".to_owned()],
            include_orderbook: true,
        })
        .expect("subscriptions");
        let declaration = build_market_subscription_declaration("runtime-1", &subscriptions)
            .expect("declaration");

        assert_eq!(configured_topic_count(&subscriptions), 4);
        assert!(declaration.contains("trade:kr"));
        assert!(declaration.contains("orderbook:us"));
        assert!(!declaration.contains("personal:order"));
        assert!(!declaration.contains("account"));
    }

    #[test]
    fn runtime_request_rejects_empty_or_invalid_symbols() {
        assert!(stream_subscriptions(&TossMarketStreamStartRequest {
            kr_symbols: vec![],
            us_symbols: vec![],
            include_orderbook: false,
        })
        .expect_err("empty")
        .contains("한 개 이상"));

        let invalid = stream_subscriptions(&TossMarketStreamStartRequest {
            kr_symbols: vec!["삼성전자".to_owned()],
            us_symbols: vec![],
            include_orderbook: false,
        })
        .expect("request shape");
        assert!(build_market_subscription_declaration("runtime-2", &invalid).is_err());
    }

    #[test]
    fn trade_tick_preserves_point_in_time_fields_for_rust_aggregation() {
        let event_at_ms =
            crate::market_data::parse_rfc3339_ms("2026-06-18T09:30:00+09:00").expect("event time");
        let input = trade_to_aggregation_input(
            &TossTradeTick {
                topic: "trade:kr:005930".to_owned(),
                symbol: "005930".to_owned(),
                market: "KR".to_owned(),
                price: "71500".to_owned(),
                volume: "2".to_owned(),
                timestamp: "2026-06-18T09:30:00+09:00".to_owned(),
                currency: "KRW".to_owned(),
            },
            event_at_ms + 1_000,
        )
        .expect("aggregation input");

        assert_eq!(input.stream_id, "toss_kr_005930");
        assert_eq!(input.provider, "Toss");
        assert_eq!(input.price, "71500");
        assert_eq!(input.quantity, "2");
        assert_eq!(input.sequence, None);
    }

    #[test]
    fn runtime_status_never_enables_live_orders() {
        let serialized = serde_json::to_value(TossMarketStreamStatus::default()).expect("status");
        assert_eq!(serialized["liveOrderAllowed"], false);
        assert!(serialized.get("accessToken").is_none());
        assert!(serialized.get("authorization").is_none());
    }

    #[test]
    fn successful_trade_does_not_hide_a_partial_subscription_rejection() {
        let mut partial = TossMarketStreamStatus {
            configured_topics: 2,
            subscribed_topics: 1,
            rejected_topics: 1,
            issue: Some("구독 일부 거부".to_owned()),
            ..TossMarketStreamStatus::default()
        };
        clear_resolved_stream_issue(&mut partial);
        assert_eq!(partial.issue.as_deref(), Some("구독 일부 거부"));

        let mut healthy = TossMarketStreamStatus {
            configured_topics: 2,
            subscribed_topics: 2,
            issue: Some("이전 오류".to_owned()),
            ..TossMarketStreamStatus::default()
        };
        clear_resolved_stream_issue(&mut healthy);
        assert!(healthy.issue.is_none());
    }

    #[test]
    #[ignore = "저장된 토스 자격정보와 허용 IP가 필요한 읽기 전용 공식 왕복"]
    fn live_toss_market_stream_handshake_and_ack() {
        tauri::async_runtime::block_on(async {
            let market_data = crate::market_data::MarketDataBridge::default();
            let token = market_data
                .toss_stream_access_token()
                .await
                .expect("stored Toss credentials");
            let mut request = TOSS_STREAM_URL
                .into_client_request()
                .expect("fixed websocket URL");
            request.headers_mut().insert(
                AUTHORIZATION,
                format!("Bearer {token}").parse().expect("authorization"),
            );
            let (mut socket, response) = connect_async(request).await.expect("websocket handshake");
            assert_eq!(response.status().as_u16(), 101);
            let declaration = build_market_subscription_declaration(
                "investa-live-smoke",
                &[TossMarketSubscription {
                    kind: TossMarketStreamKind::TradeKr,
                    symbols: vec!["005930".to_owned()],
                }],
            )
            .expect("declaration");
            socket
                .send(Message::Text(declaration.into()))
                .await
                .expect("send declaration");
            let received = tokio::time::timeout(Duration::from_secs(ACK_TIMEOUT_SECONDS), async {
                while let Some(frame) = socket.next().await {
                    let frame = frame.expect("websocket frame");
                    if let Message::Text(text) = frame {
                        let parsed =
                            parse_market_stream_frame(text.as_str()).expect("provider frame");
                        if let TossMarketStreamFrame::Subscriptions {
                            subscribed,
                            rejected,
                            ..
                        } = parsed
                        {
                            return (subscribed, rejected);
                        }
                    }
                }
                panic!("connection closed before subscription ack");
            })
            .await
            .expect("subscription ack timeout");
            assert_eq!(received.0, vec!["trade:kr:005930"]);
            assert!(received.1.is_empty());
            socket
                .send(Message::Text("PING".into()))
                .await
                .expect("send keepalive");
            let pong = tokio::time::timeout(Duration::from_secs(ACK_TIMEOUT_SECONDS), async {
                while let Some(frame) = socket.next().await {
                    let frame = frame.expect("websocket frame");
                    if let Message::Text(text) = frame {
                        if parse_market_stream_frame(text.as_str()).expect("provider frame")
                            == TossMarketStreamFrame::Pong
                        {
                            return true;
                        }
                    }
                }
                false
            })
            .await
            .expect("pong timeout");
            assert!(pong);
            let _ = socket.close(None).await;
        });
    }
}
