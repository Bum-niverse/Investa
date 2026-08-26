use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{paper_trading, persistence::PersistenceBridge, trading::TradeSide};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualLimitOrderRequest {
    order_id: String,
    market: String,
    symbol: String,
    currency: String,
    side: TradeSide,
    quantity: u64,
    limit_price_minor: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualPaperOrder {
    order_id: String,
    market: String,
    symbol: String,
    currency: String,
    side: String,
    order_type: &'static str,
    quantity: u64,
    quantity_scale: u64,
    limit_price_minor: u64,
    status: String,
    created_at_ms: u64,
    updated_at_ms: u64,
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn row_to_order(row: &rusqlite::Row<'_>) -> rusqlite::Result<ManualPaperOrder> {
    Ok(ManualPaperOrder {
        order_id: row.get(0)?,
        market: row.get(1)?,
        symbol: row.get(2)?,
        currency: row.get(3)?,
        side: row.get(4)?,
        order_type: "limit",
        quantity: row.get(5)?,
        quantity_scale: if row.get::<_, String>(1)? == "coin" {
            100_000_000
        } else {
            1
        },
        limit_price_minor: row.get(6)?,
        status: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
    })
}

#[tauri::command]
pub fn manual_paper_limit_order_submit(
    request: ManualLimitOrderRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<ManualPaperOrder, String> {
    submit_limit_order(request, &persistence)
}

fn submit_limit_order(
    request: ManualLimitOrderRequest,
    persistence: &PersistenceBridge,
) -> Result<ManualPaperOrder, String> {
    let symbol = request.symbol.trim().to_ascii_uppercase();
    if !valid_identifier(&request.order_id, 128)
        || !valid_identifier(&symbol, 24)
        || !matches!(request.market.as_str(), "kr" | "us" | "coin")
        || !matches!(request.currency.as_str(), "KRW" | "USD")
        || request.quantity == 0
        || request.limit_price_minor == 0
        || (matches!(request.market.as_str(), "kr" | "coin") && request.currency != "KRW")
        || (request.market == "us" && request.currency != "USD")
    {
        return Err("지정가 모의주문의 시장·종목·통화·수량·가격을 확인해 주세요.".to_owned());
    }
    let account = paper_trading::load_or_open_account_for_currency(persistence, &request.currency)?;
    match request.side {
        TradeSide::Buy => {
            let scale: u64 = if request.market == "coin" {
                100_000_000
            } else {
                1
            };
            let required = u64::try_from(
                u128::from(request.limit_price_minor) * u128::from(request.quantity)
                    / u128::from(scale),
            )
            .map_err(|_| "지정가 모의주문 금액이 지원 범위를 초과했습니다.".to_owned())?;
            if account.cash_minor < required {
                return Err("내부 모의계좌 예수금이 부족합니다.".to_owned());
            }
        }
        TradeSide::Sell => {
            if account.positions.get(&symbol).is_none_or(|position| {
                position.quantity_scale
                    != if request.market == "coin" {
                        100_000_000
                    } else {
                        1
                    }
                    || position.quantity < request.quantity
            }) {
                return Err(
                    "내부 모의계좌 보유 수량보다 많은 지정가 매도를 대기시킬 수 없습니다."
                        .to_owned(),
                );
            }
        }
    }
    let now = crate::persistence::now_ms()?;
    let side = match request.side {
        TradeSide::Buy => "buy",
        TradeSide::Sell => "sell",
    };
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 주문 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "INSERT INTO manual_paper_orders
             (order_id, market, symbol, currency, side, order_type, quantity,
              limit_price_minor, status, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 'limit', ?6, ?7, 'pending', ?8, ?8)",
            params![
                request.order_id,
                request.market,
                symbol,
                request.currency,
                side,
                request.quantity,
                request.limit_price_minor,
                now
            ],
        )
        .map_err(|_| {
            "같은 주문 ID가 이미 존재하거나 지정가 주문을 저장하지 못했습니다.".to_owned()
        })?;
    drop(connection);
    list_orders(persistence)?
        .into_iter()
        .find(|order| order.order_id == request.order_id)
        .ok_or_else(|| "저장한 지정가 모의주문을 다시 찾지 못했습니다.".to_owned())
}

#[tauri::command]
pub fn manual_paper_orders(
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<ManualPaperOrder>, String> {
    list_orders(&persistence)
}

fn list_orders(persistence: &PersistenceBridge) -> Result<Vec<ManualPaperOrder>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 주문 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT order_id, market, symbol, currency, side, quantity,
                    limit_price_minor, status, created_at_ms, updated_at_ms
             FROM manual_paper_orders ORDER BY updated_at_ms DESC, order_id DESC LIMIT 100",
        )
        .map_err(|error| format!("지정가 모의주문 조회를 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map([], row_to_order)
        .map_err(|error| format!("지정가 모의주문을 조회하지 못했습니다: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("지정가 모의주문 결과를 읽지 못했습니다: {error}"))
}

#[tauri::command]
pub fn manual_paper_order_cancel(
    order_id: String,
    persistence: State<'_, PersistenceBridge>,
) -> Result<ManualPaperOrder, String> {
    cancel_order(&order_id, &persistence)
}

fn cancel_order(
    order_id: &str,
    persistence: &PersistenceBridge,
) -> Result<ManualPaperOrder, String> {
    if !valid_identifier(&order_id, 128) {
        return Err("취소할 주문 ID가 올바르지 않습니다.".to_owned());
    }
    let now = crate::persistence::now_ms()?;
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "로컬 주문 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM manual_paper_orders WHERE order_id = ?1",
            params![order_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("취소할 주문을 확인하지 못했습니다: {error}"))?;
    match status.as_deref() {
        None => return Err("취소할 지정가 모의주문을 찾지 못했습니다.".to_owned()),
        Some("pending") => {}
        _ => return Err("이미 취소되었거나 체결된 주문은 취소할 수 없습니다.".to_owned()),
    }
    connection
        .execute(
            "UPDATE manual_paper_orders SET status = 'cancelled', updated_at_ms = ?2
             WHERE order_id = ?1 AND status = 'pending'",
            params![order_id, now],
        )
        .map_err(|error| format!("지정가 모의주문을 취소하지 못했습니다: {error}"))?;
    drop(connection);
    list_orders(persistence)?
        .into_iter()
        .find(|order| order.order_id == order_id)
        .ok_or_else(|| "취소한 주문을 다시 찾지 못했습니다.".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_and_cancels_pending_limit_orders_only() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let request = ManualLimitOrderRequest {
            order_id: "limit-1".to_owned(),
            market: "kr".to_owned(),
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            side: TradeSide::Buy,
            quantity: 1,
            limit_price_minor: 70_000,
        };
        let order = submit_limit_order(request, &persistence).expect("submit");
        assert_eq!(order.status, "pending");
        let cancelled = cancel_order("limit-1", &persistence).expect("cancel");
        assert_eq!(cancelled.status, "cancelled");
    }

    #[test]
    fn accepts_krw_coin_limits_but_rejects_a_mismatched_currency() {
        let persistence = PersistenceBridge::in_memory().expect("database");
        let coin = ManualLimitOrderRequest {
            order_id: "coin-limit-1".to_owned(),
            market: "coin".to_owned(),
            symbol: "KRW-XRP".to_owned(),
            currency: "KRW".to_owned(),
            side: TradeSide::Buy,
            quantity: 10,
            limit_price_minor: 4_000,
        };
        assert_eq!(
            submit_limit_order(coin, &persistence)
                .expect("coin limit")
                .market,
            "coin"
        );

        let mismatched = ManualLimitOrderRequest {
            order_id: "coin-limit-usd".to_owned(),
            market: "coin".to_owned(),
            symbol: "KRW-XRP".to_owned(),
            currency: "USD".to_owned(),
            side: TradeSide::Buy,
            quantity: 1,
            limit_price_minor: 4_000,
        };
        assert!(submit_limit_order(mismatched, &persistence).is_err());
    }
}
