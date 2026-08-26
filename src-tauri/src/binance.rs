use std::time::Duration;

use hmac::{Hmac, Mac};
use keyring::{Entry, Error as KeyringError};
use reqwest::{Client, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::Sha256;
use tauri::State;

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
    spot: BinanceAccountSection,
    usd_m: BinanceAccountSection,
    coin_m: BinanceAccountSection,
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

#[tauri::command]
pub async fn binance_public_snapshot(
    bridge: State<'_, BinanceBridge>,
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
}
