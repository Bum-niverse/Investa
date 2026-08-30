use std::{collections::HashMap, time::Duration};

use hmac::{Hmac, Mac};
use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;
use tauri::State;

use crate::market_data::{
    public_market_analysis_snapshot, AnalysisSnapshot, AnalysisSnapshotRequest, TossChartBar,
};

const SPOT_BASE: &str = "https://api.binance.com";
const USDM_BASE: &str = "https://fapi.binance.com";
const COINM_BASE: &str = "https://dapi.binance.com";
const REQUEST_TIMEOUT_SECONDS: u64 = 8;
const RECV_WINDOW_MS: u64 = 5_000;
const KEYRING_SERVICE: &str = "Investa.Binance";
const API_KEY_ACCOUNT: &str = "api-key";
const SECRET_KEY_ACCOUNT: &str = "secret-key";

pub struct BinanceBridge {
    client: Client,
}

impl Default for BinanceBridge {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[derive(Clone)]
struct BinanceCredentials {
    api_key: String,
    secret_key: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceCredentialsRequest {
    api_key: String,
    secret_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceConnectionStatus {
    configured: bool,
    connected: bool,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinancePublicSnapshot {
    fetched_at_ms: u64,
    spot: PublicQuote,
    usd_m: PublicQuote,
    coin_m: PublicQuote,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicQuote {
    market: &'static str,
    symbol: String,
    price: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceAccountSnapshot {
    provider: &'static str,
    fetched_at_ms: u64,
    read_only: bool,
    permission_verified: bool,
    permission_message: String,
    spot: BinanceAccountSection,
    usd_m: BinanceAccountSection,
    coin_m: BinanceAccountSection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BinanceApiRestrictions {
    #[serde(default)]
    ip_restrict: bool,
    #[serde(default)]
    enable_reading: bool,
    #[serde(default)]
    enable_withdrawals: bool,
    #[serde(default)]
    enable_internal_transfer: bool,
    #[serde(default)]
    enable_margin: bool,
    #[serde(default)]
    enable_futures: bool,
    #[serde(default)]
    permits_universal_transfer: bool,
    #[serde(default)]
    enable_vanilla_options: bool,
    #[serde(default)]
    enable_fix_api_trade: bool,
    #[serde(default)]
    enable_spot_and_margin_trading: bool,
    #[serde(default)]
    enable_portfolio_margin_trading: bool,
}

fn validate_api_restrictions(value: &BinanceApiRestrictions) -> Result<(), String> {
    if !value.enable_reading {
        return Err("Binance API 키의 읽기 권한이 필요합니다.".to_owned());
    }
    if !value.ip_restrict {
        return Err("Binance API 키에 이 PC의 고정 IP 제한을 먼저 적용해 주세요.".to_owned());
    }
    let risky = [
        (value.enable_withdrawals, "출금"),
        (value.enable_internal_transfer, "내부 이체"),
        (value.enable_margin, "마진"),
        (value.enable_futures, "선물 거래"),
        (value.permits_universal_transfer, "통합 이체"),
        (value.enable_vanilla_options, "옵션 거래"),
        (value.enable_fix_api_trade, "FIX 거래"),
        (value.enable_spot_and_margin_trading, "현물·마진 거래"),
        (
            value.enable_portfolio_margin_trading,
            "포트폴리오 마진 거래",
        ),
    ]
    .into_iter()
    .filter_map(|(enabled, label)| enabled.then_some(label))
    .collect::<Vec<_>>();
    if !risky.is_empty() {
        return Err(format!(
            "Investa는 읽기 전용 Binance 키만 저장합니다. 다음 권한을 끄세요: {}",
            risky.join(", ")
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceAccountSection {
    connected: bool,
    message: String,
    balances: Vec<BinanceBalance>,
    positions: Vec<BinancePosition>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinanceBalance {
    asset: String,
    wallet_balance: String,
    available_balance: String,
    unrealized_profit: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinancePosition {
    symbol: String,
    position_amount: String,
    entry_price: String,
    mark_price: String,
    unrealized_profit: String,
    liquidation_price: String,
    leverage: String,
    margin_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ServerTime {
    server_time: u64,
}

#[derive(Deserialize)]
struct SpotTicker {
    symbol: String,
    price: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarkPrice {
    symbol: String,
    mark_price: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FundingRate {
    symbol: String,
    funding_time: u64,
    funding_rate: String,
}

#[derive(Debug, Clone)]
struct PublicKline {
    period_start_ms: u64,
    period_end_ms: u64,
    open_minor: u64,
    high_minor: u64,
    low_minor: u64,
    close_minor: u64,
    volume: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotAccount {
    balances: Vec<SpotBalance>,
}

#[derive(Deserialize)]
struct SpotBalance {
    asset: String,
    free: String,
    locked: String,
}

#[derive(Deserialize)]
struct FuturesAccount {
    #[serde(default)]
    assets: Vec<FuturesAsset>,
    #[serde(default)]
    positions: Vec<FuturesPosition>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuturesAsset {
    asset: String,
    #[serde(default)]
    wallet_balance: String,
    #[serde(default)]
    available_balance: String,
    #[serde(default)]
    unrealized_profit: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuturesPosition {
    symbol: String,
    #[serde(default)]
    position_amt: String,
    #[serde(default)]
    entry_price: String,
    #[serde(default)]
    mark_price: String,
    #[serde(default)]
    unrealized_profit: String,
    #[serde(default)]
    liquidation_price: String,
    #[serde(default)]
    leverage: String,
    #[serde(default)]
    margin_type: String,
}

fn credential_entry(account: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, account)
        .map_err(|_| "Windows 자격 증명 저장소를 열지 못했습니다.".to_owned())
}

fn optional_password(entry: &Entry) -> Result<Option<String>, String> {
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("Windows 자격 증명 저장소를 읽지 못했습니다.".to_owned()),
    }
}

fn load_credentials() -> Result<Option<BinanceCredentials>, String> {
    let api_key = optional_password(&credential_entry(API_KEY_ACCOUNT)?)?;
    let secret_key = optional_password(&credential_entry(SECRET_KEY_ACCOUNT)?)?;
    match (api_key, secret_key) {
        (None, None) => Ok(None),
        (Some(api_key), Some(secret_key)) => Ok(Some(BinanceCredentials {
            api_key,
            secret_key,
        })),
        _ => Err("Binance 자격정보가 불완전합니다. 삭제 후 다시 등록해 주세요.".to_owned()),
    }
}

fn validate_credentials(request: BinanceCredentialsRequest) -> Result<BinanceCredentials, String> {
    let api_key = request.api_key.trim().to_owned();
    let secret_key = request.secret_key.trim().to_owned();
    let valid = |value: &str| {
        (16..=256).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_graphic())
    };
    if !valid(&api_key) || !valid(&secret_key) {
        return Err("Binance API Key와 Secret Key 형식을 확인해 주세요.".to_owned());
    }
    Ok(BinanceCredentials {
        api_key,
        secret_key,
    })
}

fn store_credentials(credentials: &BinanceCredentials) -> Result<(), String> {
    let key_entry = credential_entry(API_KEY_ACCOUNT)?;
    let secret_entry = credential_entry(SECRET_KEY_ACCOUNT)?;
    key_entry
        .set_password(&credentials.api_key)
        .map_err(|_| "Binance API Key를 저장하지 못했습니다.".to_owned())?;
    if secret_entry.set_password(&credentials.secret_key).is_err() {
        let _ = key_entry.delete_credential();
        return Err("Binance Secret Key를 저장하지 못했습니다.".to_owned());
    }
    Ok(())
}

fn delete_credentials() -> Result<(), String> {
    for account in [API_KEY_ACCOUNT, SECRET_KEY_ACCOUNT] {
        match credential_entry(account)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => {}
            Err(_) => return Err("Binance 연결 정보를 삭제하지 못했습니다.".to_owned()),
        }
    }
    Ok(())
}

fn signature(secret_key: &str, payload: &str) -> Result<String, String> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret_key.as_bytes())
        .map_err(|_| "Binance 서명 키를 처리하지 못했습니다.".to_owned())?;
    mac.update(payload.as_bytes());
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn safe_api_error(status: StatusCode, product: &str) -> String {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            format!("Binance {product} API 키 권한, 허용 IP 또는 상품 활성화 상태를 확인해 주세요.")
        }
        StatusCode::TOO_MANY_REQUESTS => format!("Binance {product} 조회 한도를 초과했습니다."),
        _ => format!("Binance {product} 서버가 요청을 처리하지 못했습니다."),
    }
}

async fn server_time(bridge: &BinanceBridge, base: &str, path: &str) -> Result<u64, String> {
    let response = bridge
        .client
        .get(format!("{base}{path}"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .send()
        .await
        .map_err(|_| "Binance 서버 시각을 확인하지 못했습니다.".to_owned())?;
    if !response.status().is_success() {
        return Err("Binance 서버 시각 응답을 확인하지 못했습니다.".to_owned());
    }
    response
        .json::<ServerTime>()
        .await
        .map(|value| value.server_time)
        .map_err(|_| "Binance 서버 시각 형식이 올바르지 않습니다.".to_owned())
}

async fn signed_get<T: DeserializeOwned>(
    bridge: &BinanceBridge,
    credentials: &BinanceCredentials,
    base: &str,
    time_path: &str,
    path: &str,
    product: &str,
) -> Result<T, String> {
    let timestamp = server_time(bridge, base, time_path).await?;
    let payload = format!("timestamp={timestamp}&recvWindow={RECV_WINDOW_MS}");
    let signature = signature(&credentials.secret_key, &payload)?;
    let response = bridge
        .client
        .get(format!("{base}{path}?{payload}&signature={signature}"))
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .header("X-MBX-APIKEY", &credentials.api_key)
        .send()
        .await
        .map_err(|_| format!("Binance {product} 서버에 연결하지 못했습니다."))?;
    if !response.status().is_success() {
        return Err(safe_api_error(response.status(), product));
    }
    response
        .json::<T>()
        .await
        .map_err(|_| format!("Binance {product} 응답 형식이 올바르지 않습니다."))
}

fn has_non_zero(value: &str) -> bool {
    value.parse::<f64>().is_ok_and(|number| number != 0.0)
}

fn failed_section(message: String) -> BinanceAccountSection {
    BinanceAccountSection {
        connected: false,
        message,
        balances: vec![],
        positions: vec![],
    }
}

async fn spot_section(
    bridge: &BinanceBridge,
    credentials: &BinanceCredentials,
) -> BinanceAccountSection {
    match signed_get::<SpotAccount>(
        bridge,
        credentials,
        SPOT_BASE,
        "/api/v3/time",
        "/api/v3/account",
        "현물",
    )
    .await
    {
        Ok(account) if account.balances.len() <= 5_000 => BinanceAccountSection {
            connected: true,
            message: "현물 계좌 조회 가능".to_owned(),
            balances: account
                .balances
                .into_iter()
                .filter(|balance| has_non_zero(&balance.free) || has_non_zero(&balance.locked))
                .take(500)
                .map(|balance| BinanceBalance {
                    asset: balance.asset,
                    wallet_balance: balance.free,
                    available_balance: "-".to_owned(),
                    unrealized_profit: "0".to_owned(),
                })
                .collect(),
            positions: vec![],
        },
        Ok(_) => failed_section("Binance 현물 잔고 항목이 허용 범위를 초과했습니다.".to_owned()),
        Err(error) => failed_section(error),
    }
}

async fn futures_section(
    bridge: &BinanceBridge,
    credentials: &BinanceCredentials,
    coin_m: bool,
) -> BinanceAccountSection {
    let (base, time_path, account_path, product) = if coin_m {
        (COINM_BASE, "/dapi/v1/time", "/dapi/v1/account", "COIN-M")
    } else {
        (USDM_BASE, "/fapi/v1/time", "/fapi/v3/account", "USDⓈ-M")
    };
    match signed_get::<FuturesAccount>(bridge, credentials, base, time_path, account_path, product)
        .await
    {
        Ok(account) if account.assets.len() <= 5_000 && account.positions.len() <= 20_000 => {
            BinanceAccountSection {
                connected: true,
                message: format!("{product} 계좌 조회 가능"),
                balances: account
                    .assets
                    .into_iter()
                    .filter(|asset| {
                        has_non_zero(&asset.wallet_balance)
                            || has_non_zero(&asset.unrealized_profit)
                    })
                    .take(500)
                    .map(|asset| BinanceBalance {
                        asset: asset.asset,
                        wallet_balance: asset.wallet_balance,
                        available_balance: asset.available_balance,
                        unrealized_profit: asset.unrealized_profit,
                    })
                    .collect(),
                positions: account
                    .positions
                    .into_iter()
                    .filter(|position| has_non_zero(&position.position_amt))
                    .take(500)
                    .map(|position| BinancePosition {
                        symbol: position.symbol,
                        position_amount: position.position_amt,
                        entry_price: position.entry_price,
                        mark_price: position.mark_price,
                        unrealized_profit: position.unrealized_profit,
                        liquidation_price: position.liquidation_price,
                        leverage: position.leverage,
                        margin_type: position.margin_type,
                    })
                    .collect(),
            }
        }
        Ok(_) => failed_section(format!(
            "Binance {product} 응답 항목이 허용 범위를 초과했습니다."
        )),
        Err(error) => failed_section(error),
    }
}

async fn account_snapshot(
    bridge: &BinanceBridge,
    credentials: &BinanceCredentials,
) -> Result<BinanceAccountSnapshot, String> {
    let restrictions = signed_get::<BinanceApiRestrictions>(
        bridge,
        credentials,
        SPOT_BASE,
        "/api/v3/time",
        "/sapi/v1/account/apiRestrictions",
        "API 권한",
    )
    .await?;
    validate_api_restrictions(&restrictions)?;
    let spot = spot_section(bridge, credentials).await;
    let usd_m = futures_section(bridge, credentials, false).await;
    let coin_m = futures_section(bridge, credentials, true).await;
    if !spot.connected && !usd_m.connected && !coin_m.connected {
        return Err("Binance 현물·USDⓈ-M·COIN-M 계좌를 모두 확인하지 못했습니다.".to_owned());
    }
    Ok(BinanceAccountSnapshot {
        provider: "Binance",
        fetched_at_ms: crate::paper_trading::now_ms()?,
        read_only: true,
        permission_verified: true,
        permission_message: "읽기 권한·IP 제한 확인됨 · 거래·출금·이체 권한 비활성".to_owned(),
        spot,
        usd_m,
        coin_m,
    })
}

async fn public_json<T: DeserializeOwned>(
    bridge: &BinanceBridge,
    url: String,
    product: &str,
) -> Result<T, String> {
    let response = bridge
        .client
        .get(url)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
        .send()
        .await
        .map_err(|_| format!("Binance {product} 공개 시세에 연결하지 못했습니다."))?;
    if !response.status().is_success() {
        return Err(format!(
            "Binance {product} 공개 시세를 확인하지 못했습니다."
        ));
    }
    response
        .json::<T>()
        .await
        .map_err(|_| format!("Binance {product} 공개 시세 형식이 올바르지 않습니다."))
}

fn parse_positive_decimal(value: &str, scale: f64, label: &str) -> Result<u64, String> {
    let parsed = value
        .parse::<f64>()
        .map_err(|_| format!("Binance {label} 숫자를 해석하지 못했습니다."))?;
    let scaled = parsed * scale;
    if !scaled.is_finite() || scaled < 0.0 || scaled > u64::MAX as f64 {
        return Err(format!("Binance {label} 값이 지원 범위를 벗어났습니다."));
    }
    Ok(scaled.round() as u64)
}

fn parse_public_klines(
    rows: Vec<Vec<serde_json::Value>>,
    label: &str,
) -> Result<Vec<PublicKline>, String> {
    let mut bars = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() < 7 {
            return Err(format!("Binance {label} 봉 응답 필드가 부족합니다."));
        }
        let text = |index: usize| {
            row[index]
                .as_str()
                .ok_or_else(|| format!("Binance {label} 봉 숫자 형식이 올바르지 않습니다."))
        };
        let period_start_ms = row[0]
            .as_u64()
            .ok_or_else(|| format!("Binance {label} 봉 시작 시각이 올바르지 않습니다."))?;
        let period_end_ms = row[6]
            .as_u64()
            .ok_or_else(|| format!("Binance {label} 봉 종료 시각이 올바르지 않습니다."))?;
        let open_minor = parse_positive_decimal(text(1)?, 100.0, label)?;
        let high_minor = parse_positive_decimal(text(2)?, 100.0, label)?;
        let low_minor = parse_positive_decimal(text(3)?, 100.0, label)?;
        let close_minor = parse_positive_decimal(text(4)?, 100.0, label)?;
        if period_end_ms <= period_start_ms
            || low_minor > open_minor
            || low_minor > close_minor
            || high_minor < open_minor
            || high_minor < close_minor
        {
            return Err(format!(
                "Binance {label} 봉 시간 또는 OHLC 관계가 올바르지 않습니다."
            ));
        }
        bars.push(PublicKline {
            period_start_ms,
            period_end_ms,
            open_minor,
            high_minor,
            low_minor,
            close_minor,
            volume: parse_positive_decimal(text(5)?, 100_000_000.0, "거래량")?,
        });
    }
    bars.sort_by_key(|bar| bar.period_start_ms);
    if bars.windows(2).any(|pair| {
        pair[0].period_start_ms == pair[1].period_start_ms
            || pair[0].period_end_ms >= pair[1].period_end_ms
    }) {
        return Err(format!(
            "Binance {label} 봉 시계열에 중복 또는 역순 데이터가 있습니다."
        ));
    }
    Ok(bars)
}

fn resolve_perpetual_symbol(query: &str) -> Result<String, String> {
    let normalized = query.trim().to_ascii_uppercase();
    for token in normalized.split(|character: char| !character.is_ascii_alphanumeric()) {
        if (7..=20).contains(&token.len()) && token.ends_with("USDT") {
            return Ok(token.to_owned());
        }
    }
    if normalized.contains("비트코인") || normalized.contains("BTC") {
        return Ok("BTCUSDT".to_owned());
    }
    if normalized.contains("이더리움") || normalized.contains("ETH") {
        return Ok("ETHUSDT".to_owned());
    }
    if normalized.contains("리플") || normalized.contains("XRP") {
        return Ok("XRPUSDT".to_owned());
    }
    Err(
        "분석 요청에서 Binance USDⓈ-M 무기한선물 심볼을 확정하지 못했습니다. 예: BTCUSDT"
            .to_owned(),
    )
}

#[tauri::command]
pub fn binance_connection_status() -> Result<BinanceConnectionStatus, String> {
    Ok(match load_credentials()? {
        Some(_) => BinanceConnectionStatus {
            configured: true,
            connected: false,
            message: "자격정보 저장됨 · 계좌 조회로 상품별 상태를 확인해 주세요.".to_owned(),
        },
        None => BinanceConnectionStatus {
            configured: false,
            connected: false,
            message: "Binance 계좌가 연결되지 않았습니다.".to_owned(),
        },
    })
}

async fn fetch_binance_public_snapshot(
    bridge: &BinanceBridge,
) -> Result<BinancePublicSnapshot, String> {
    let spot = public_json::<SpotTicker>(
        &bridge,
        format!("{SPOT_BASE}/api/v3/ticker/price?symbol=BTCUSDT"),
        "현물",
    )
    .await?;
    let usd_m = public_json::<MarkPrice>(
        &bridge,
        format!("{USDM_BASE}/fapi/v1/premiumIndex?symbol=BTCUSDT"),
        "USDⓈ-M",
    )
    .await?;
    let coin_m = public_json::<Vec<MarkPrice>>(
        &bridge,
        format!("{COINM_BASE}/dapi/v1/premiumIndex?symbol=BTCUSD_PERP"),
        "COIN-M",
    )
    .await?
    .into_iter()
    .find(|quote| quote.symbol == "BTCUSD_PERP")
    .ok_or_else(|| "Binance COIN-M 공개 시세에서 BTCUSD_PERP를 찾지 못했습니다.".to_owned())?;
    Ok(BinancePublicSnapshot {
        fetched_at_ms: crate::paper_trading::now_ms()?,
        spot: PublicQuote {
            market: "SPOT",
            symbol: spot.symbol,
            price: spot.price,
        },
        usd_m: PublicQuote {
            market: "USDⓈ-M",
            symbol: usd_m.symbol,
            price: usd_m.mark_price,
        },
        coin_m: PublicQuote {
            market: "COIN-M",
            symbol: coin_m.symbol,
            price: coin_m.mark_price,
        },
    })
}

#[tauri::command]
pub async fn binance_public_snapshot(
    bridge: State<'_, BinanceBridge>,
) -> Result<BinancePublicSnapshot, String> {
    fetch_binance_public_snapshot(&bridge).await
}

async fn fetch_perpetual_analysis_snapshot(
    request: AnalysisSnapshotRequest,
    bridge: &BinanceBridge,
) -> Result<AnalysisSnapshot, String> {
    if request.query.trim().is_empty()
        || request.query.chars().count() > 500
        || request.query.chars().any(char::is_control)
    {
        return Err("Binance 분석 요청은 한 줄 1자 이상 500자 이하여야 합니다.".to_owned());
    }
    if !(60..=500).contains(&request.count) {
        return Err("Binance 분석 스냅샷 봉은 60개에서 500개 사이여야 합니다.".to_owned());
    }
    let symbol = resolve_perpetual_symbol(&request.query)?;
    let limit = request.count.to_string();
    let trade_rows = public_json::<Vec<Vec<serde_json::Value>>>(bridge, format!("{USDM_BASE}/fapi/v1/continuousKlines?pair={symbol}&contractType=PERPETUAL&interval=4h&limit={limit}"), "USDⓈ-M 체결 봉").await?;
    let mark_rows = public_json::<Vec<Vec<serde_json::Value>>>(
        bridge,
        format!("{USDM_BASE}/fapi/v1/markPriceKlines?symbol={symbol}&interval=4h&limit={limit}"),
        "USDⓈ-M 마크가격 봉",
    )
    .await?;
    let index_rows = public_json::<Vec<Vec<serde_json::Value>>>(
        bridge,
        format!("{USDM_BASE}/fapi/v1/indexPriceKlines?pair={symbol}&interval=4h&limit={limit}"),
        "USDⓈ-M 지수가격 봉",
    )
    .await?;
    let funding = public_json::<Vec<FundingRate>>(
        bridge,
        format!("{USDM_BASE}/fapi/v1/fundingRate?symbol={symbol}&limit=1000"),
        "USDⓈ-M 펀딩",
    )
    .await?;
    if funding.iter().any(|item| item.symbol != symbol) {
        return Err("Binance 펀딩 응답에 다른 심볼이 포함되었습니다.".to_owned());
    }
    let fetched_at_ms = crate::paper_trading::now_ms()?;
    let trade = parse_public_klines(trade_rows, "체결")?;
    let mark: HashMap<u64, PublicKline> = parse_public_klines(mark_rows, "마크가격")?
        .into_iter()
        .map(|bar| (bar.period_start_ms, bar))
        .collect();
    let index: HashMap<u64, PublicKline> = parse_public_klines(index_rows, "지수가격")?
        .into_iter()
        .map(|bar| (bar.period_start_ms, bar))
        .collect();
    let funding_by_time: HashMap<u64, f64> = funding
        .into_iter()
        .map(|item| {
            let bps = item
                .funding_rate
                .parse::<f64>()
                .map(|rate| rate * 10_000.0)
                .map_err(|_| "Binance 펀딩 비율을 해석하지 못했습니다.".to_owned())?;
            if !bps.is_finite() {
                return Err("Binance 펀딩 비율이 유효하지 않습니다.".to_owned());
            }
            Ok((item.funding_time, bps))
        })
        .collect::<Result<_, String>>()?;
    let mut bars = Vec::new();
    for bar in trade
        .into_iter()
        .filter(|bar| bar.period_end_ms <= fetched_at_ms)
    {
        let mark_bar = mark
            .get(&bar.period_start_ms)
            .ok_or_else(|| "Binance 마크가격 봉이 체결 봉과 일치하지 않습니다.".to_owned())?;
        let index_bar = index
            .get(&bar.period_start_ms)
            .ok_or_else(|| "Binance 지수가격 봉이 체결 봉과 일치하지 않습니다.".to_owned())?;
        let funding_observation = funding_by_time
            .iter()
            .filter(|(time, _)| **time > bar.period_start_ms && **time <= bar.period_end_ms)
            .max_by_key(|(time, _)| **time);
        bars.push(TossChartBar {
            period_start_ms: bar.period_start_ms,
            period_end_ms: bar.period_end_ms,
            open_minor: bar.open_minor,
            high_minor: bar.high_minor,
            low_minor: bar.low_minor,
            close_minor: bar.close_minor,
            volume: bar.volume,
            completed: true,
            available_at_ms: Some(bar.period_end_ms),
            ingested_at_ms: Some(fetched_at_ms),
            session_id: Some("BINANCE-24H".to_owned()),
            contract_code: Some(symbol.clone()),
            settlement_price_minor: None,
            mark_price_minor: Some(mark_bar.close_minor),
            index_price_minor: Some(index_bar.close_minor),
            funding_rate_bps: funding_observation.map(|(_, rate)| *rate),
            funding_time_ms: funding_observation.map(|(time, _)| *time),
        });
    }
    public_market_analysis_snapshot(
        "BINANCE_USDM_PUBLIC_API", symbol.clone(), symbol.clone(), "crypto_perpetual".to_owned(),
        "crypto_perpetual", "USD".to_owned(), "4h".to_owned(), fetched_at_ms, bars,
        vec!["체결·마크·지수 봉을 동일 시작 시각으로 교차 검증했으며 펀딩은 실제 관측 시각에만 표시합니다.".to_owned()],
    )
}

#[tauri::command]
pub async fn binance_perpetual_analysis_snapshot(
    request: AnalysisSnapshotRequest,
    bridge: State<'_, BinanceBridge>,
) -> Result<AnalysisSnapshot, String> {
    fetch_perpetual_analysis_snapshot(request, &bridge).await
}

#[tauri::command]
pub async fn binance_save_credentials(
    request: BinanceCredentialsRequest,
    bridge: State<'_, BinanceBridge>,
) -> Result<BinanceAccountSnapshot, String> {
    let credentials = validate_credentials(request)?;
    let snapshot = account_snapshot(&bridge, &credentials).await?;
    store_credentials(&credentials)?;
    Ok(snapshot)
}

#[tauri::command]
pub async fn binance_account_snapshot(
    bridge: State<'_, BinanceBridge>,
) -> Result<BinanceAccountSnapshot, String> {
    let credentials =
        load_credentials()?.ok_or_else(|| "Binance 연결 정보가 없습니다.".to_owned())?;
    account_snapshot(&bridge, &credentials).await
}

#[tauri::command]
pub fn binance_delete_credentials() -> Result<BinanceConnectionStatus, String> {
    delete_credentials()?;
    Ok(BinanceConnectionStatus {
        configured: false,
        connected: false,
        message: "Binance 연결 정보를 이 PC에서 삭제했습니다.".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_binance_credentials() {
        assert!(validate_credentials(BinanceCredentialsRequest {
            api_key: "short".to_owned(),
            secret_key: "short".to_owned(),
        })
        .is_err());
    }

    #[test]
    fn hmac_signature_is_deterministic_and_does_not_expose_the_secret() {
        let secret = "test-secret-key-123456";
        let first = signature(secret, "timestamp=1&recvWindow=5000").expect("signature");
        let second = signature(secret, "timestamp=1&recvWindow=5000").expect("signature");
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(!first.contains(secret));
    }

    #[test]
    fn rejects_binance_keys_with_trading_or_missing_ip_restriction() {
        let safe = BinanceApiRestrictions {
            ip_restrict: true,
            enable_reading: true,
            enable_withdrawals: false,
            enable_internal_transfer: false,
            enable_margin: false,
            enable_futures: false,
            permits_universal_transfer: false,
            enable_vanilla_options: false,
            enable_fix_api_trade: false,
            enable_spot_and_margin_trading: false,
            enable_portfolio_margin_trading: false,
        };
        assert!(validate_api_restrictions(&safe).is_ok());

        let no_ip = BinanceApiRestrictions {
            ip_restrict: false,
            ..safe
        };
        assert!(validate_api_restrictions(&no_ip).is_err());

        let trading = BinanceApiRestrictions {
            ip_restrict: true,
            enable_spot_and_margin_trading: true,
            ..no_ip
        };
        assert!(validate_api_restrictions(&trading).is_err());
    }

    #[test]
    fn perpetual_symbol_resolver_accepts_explicit_and_korean_aliases() {
        assert_eq!(
            resolve_perpetual_symbol("BTCUSDT 4시간봉").unwrap(),
            "BTCUSDT"
        );
        assert_eq!(
            resolve_perpetual_symbol("비트코인 무기한 선물").unwrap(),
            "BTCUSDT"
        );
        assert_eq!(
            resolve_perpetual_symbol("이더리움 perp").unwrap(),
            "ETHUSDT"
        );
        assert!(resolve_perpetual_symbol("심볼 없는 요청").is_err());
    }

    #[test]
    fn public_kline_parser_preserves_point_in_time_boundaries() {
        let rows = vec![vec![
            serde_json::json!(1_000_u64),
            serde_json::json!("100.00"),
            serde_json::json!("110.00"),
            serde_json::json!("90.00"),
            serde_json::json!("105.00"),
            serde_json::json!("12.5"),
            serde_json::json!(2_000_u64),
        ]];
        let parsed = parse_public_klines(rows, "테스트").expect("parse");
        assert_eq!(parsed[0].period_start_ms, 1_000);
        assert_eq!(parsed[0].period_end_ms, 2_000);
        assert_eq!(parsed[0].close_minor, 10_500);
        assert_eq!(parsed[0].volume, 1_250_000_000);
    }

    #[test]
    fn public_kline_parser_rejects_invalid_ohlc() {
        let rows = vec![vec![
            serde_json::json!(1_000_u64),
            serde_json::json!("100"),
            serde_json::json!("95"),
            serde_json::json!("90"),
            serde_json::json!("105"),
            serde_json::json!("1"),
            serde_json::json!(2_000_u64),
        ]];
        assert!(parse_public_klines(rows, "테스트").is_err());
    }

    #[test]
    #[ignore = "저장된 Binance 자격정보와 외부 개인계좌 서버를 사용하는 명시적 읽기 전용 검사"]
    fn live_binance_account_snapshot_uses_stored_read_only_credentials() {
        let credentials = load_credentials()
            .expect("credential store")
            .expect("stored Binance credentials");
        let snapshot = tauri::async_runtime::block_on(account_snapshot(
            &BinanceBridge::default(),
            &credentials,
        ))
        .expect("Binance read-only account snapshot");
        assert!(snapshot.read_only);
        assert!(snapshot.permission_verified);
        assert!(snapshot.spot.connected || snapshot.usd_m.connected || snapshot.coin_m.connected);
        eprintln!(
            "BINANCE_ACCOUNT_CONNECTED=true read_only=true spot={} usd_m={} coin_m={} usd_m_status={:?} coin_m_status={:?}",
            snapshot.spot.connected,
            snapshot.usd_m.connected,
            snapshot.coin_m.connected,
            snapshot.usd_m.message,
            snapshot.coin_m.message
        );
    }

    #[test]
    #[ignore = "Binance 공개 USDⓈ-M API와 외부 네트워크를 사용하는 명시적 통합 검사"]
    fn live_binance_perpetual_snapshot_aligns_trade_mark_index_and_funding() {
        let snapshot = tauri::async_runtime::block_on(fetch_perpetual_analysis_snapshot(
            AnalysisSnapshotRequest {
                query: "BTCUSDT 무기한선물".to_owned(),
                count: 60,
            },
            &BinanceBridge::default(),
        ))
        .expect("public perpetual snapshot");
        assert_eq!(snapshot.asset_class, "crypto_perpetual");
        assert!(snapshot.completed_bar_count >= 20);
        assert!(snapshot
            .bars
            .iter()
            .all(|bar| bar.mark_price_minor.is_some() && bar.index_price_minor.is_some()));
    }

    #[test]
    #[ignore = "Binance 공개 현물·USDⓈ-M·COIN-M API를 사용하는 명시적 통합 검사"]
    fn live_binance_public_markets_return_positive_prices() {
        let snapshot = tauri::async_runtime::block_on(fetch_binance_public_snapshot(
            &BinanceBridge::default(),
        ))
        .expect("public Binance markets");
        for quote in [&snapshot.spot, &snapshot.usd_m, &snapshot.coin_m] {
            assert!(quote.price.parse::<f64>().expect("price") > 0.0);
        }
    }
}
