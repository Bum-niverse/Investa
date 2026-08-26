use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    simulation::{quote_execution_scaled, CostError, TradingCosts},
    trading::TradeSide,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LedgerEvent {
    AccountOpened {
        account_id: String,
        currency: String,
        initial_cash_minor: u64,
        occurred_at_ms: u64,
    },
    OrderFilled {
        account_id: String,
        order_id: String,
        idempotency_key: String,
        symbol: String,
        side: TradeSide,
        quantity: u64,
        #[serde(default = "unit_scale")]
        quantity_scale: u64,
        reference_price_minor: u64,
        execution_price_minor: u64,
        notional_minor: u64,
        fee_minor: u64,
        tax_minor: u64,
        costs: TradingCosts,
        #[serde(default)]
        exit_reason: Option<String>,
        #[serde(default)]
        cause_event_id: Option<String>,
        occurred_at_ms: u64,
    },
}

impl LedgerEvent {
    pub(crate) fn occurred_at_ms(&self) -> u64 {
        match self {
            LedgerEvent::AccountOpened { occurred_at_ms, .. }
            | LedgerEvent::OrderFilled { occurred_at_ms, .. } => *occurred_at_ms,
        }
    }
}

pub trait AppendOnlyLedger {
    fn events(&self) -> &[LedgerEvent];
    fn append(&mut self, event: LedgerEvent) -> Result<(), LedgerError>;
}

#[derive(Debug, Default)]
pub struct InMemoryLedger {
    events: Vec<LedgerEvent>,
}

impl InMemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AppendOnlyLedger for InMemoryLedger {
    fn events(&self) -> &[LedgerEvent] {
        &self.events
    }

    fn append(&mut self, event: LedgerEvent) -> Result<(), LedgerError> {
        self.events.push(event);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperPosition {
    pub symbol: String,
    pub quantity: u64,
    pub quantity_scale: u64,
    pub cost_basis_minor: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperAccountState {
    pub account_id: String,
    pub currency: String,
    pub cash_minor: u64,
    pub realized_pnl_minor: i64,
    pub positions: BTreeMap<String, PaperPosition>,
    pub event_count: usize,
    pub last_event_at_ms: u64,
    #[serde(skip)]
    seen_order_ids: BTreeSet<String>,
    #[serde(skip)]
    seen_idempotency_keys: BTreeSet<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowOrderRequest {
    pub account_id: String,
    pub order_id: String,
    pub idempotency_key: String,
    pub symbol: String,
    pub currency: String,
    pub side: TradeSide,
    pub quantity: u64,
    #[serde(default = "unit_scale")]
    pub quantity_scale: u64,
    pub reference_price_minor: u64,
    pub occurred_at_ms: u64,
}

const fn unit_scale() -> u64 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerErrorCode {
    InvalidEvent,
    AccountAlreadyOpened,
    AccountNotOpened,
    AccountMismatch,
    DuplicateOrder,
    DuplicateIdempotencyKey,
    EventTimeRegression,
    InsufficientCash,
    InsufficientPosition,
    ArithmeticOverflow,
    AppendFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerError {
    pub code: LedgerErrorCode,
    pub message: String,
}

fn error(code: LedgerErrorCode, message: &str) -> LedgerError {
    LedgerError {
        code,
        message: message.to_owned(),
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_currency(value: &str) -> bool {
    value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_uppercase())
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 24
        && value.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
}

fn checked_signed_difference(left: u64, right: u64) -> Result<i64, LedgerError> {
    i64::try_from(i128::from(left) - i128::from(right)).map_err(|_| {
        error(
            LedgerErrorCode::ArithmeticOverflow,
            "실현손익이 지원 범위를 초과했습니다.",
        )
    })
}

fn proportional_cost_basis(position: &PaperPosition, quantity: u64) -> Result<u64, LedgerError> {
    let value = u128::from(position.cost_basis_minor)
        .checked_mul(u128::from(quantity))
        .ok_or_else(|| {
            error(
                LedgerErrorCode::ArithmeticOverflow,
                "포지션 원가 배분 값이 범위를 초과했습니다.",
            )
        })?
        / u128::from(position.quantity);
    u64::try_from(value).map_err(|_| {
        error(
            LedgerErrorCode::ArithmeticOverflow,
            "포지션 원가 배분 결과가 범위를 초과했습니다.",
        )
    })
}

pub fn open_paper_account<L: AppendOnlyLedger>(
    ledger: &mut L,
    account_id: String,
    currency: String,
    initial_cash_minor: u64,
    occurred_at_ms: u64,
) -> Result<PaperAccountState, LedgerError> {
    if !ledger.events().is_empty() {
        return Err(error(
            LedgerErrorCode::AccountAlreadyOpened,
            "하나의 원장에는 계좌 개설 사건을 한 번만 기록할 수 있습니다.",
        ));
    }
    if !valid_identifier(&account_id) || !valid_currency(&currency) || initial_cash_minor == 0 {
        return Err(error(
            LedgerErrorCode::InvalidEvent,
            "계좌 식별자, 통화와 초기 가상 예수금이 유효해야 합니다.",
        ));
    }

    ledger
        .append(LedgerEvent::AccountOpened {
            account_id,
            currency,
            initial_cash_minor,
            occurred_at_ms,
        })
        .map_err(|_| {
            error(
                LedgerErrorCode::AppendFailed,
                "계좌 개설 사건을 원장에 추가하지 못했습니다.",
            )
        })?;
    replay_ledger(ledger.events())
}

pub fn execute_shadow_order<L: AppendOnlyLedger>(
    ledger: &mut L,
    request: ShadowOrderRequest,
    costs: TradingCosts,
) -> Result<PaperAccountState, LedgerError> {
    let state = replay_ledger(ledger.events())?;
    if request.account_id != state.account_id {
        return Err(error(
            LedgerErrorCode::AccountMismatch,
            "주문과 모의계좌 식별자가 일치하지 않습니다.",
        ));
    }
    if !valid_identifier(&request.order_id)
        || !valid_identifier(&request.idempotency_key)
        || !valid_symbol(&request.symbol)
        || !valid_currency(&request.currency)
        || request.quantity == 0
        || request.quantity_scale == 0
        || request.reference_price_minor == 0
    {
        return Err(error(
            LedgerErrorCode::InvalidEvent,
            "주문 식별자, 종목, 수량과 기준 가격이 유효해야 합니다.",
        ));
    }
    if request.occurred_at_ms < state.last_event_at_ms {
        return Err(error(
            LedgerErrorCode::EventTimeRegression,
            "이전 원장 사건보다 과거 시각의 주문은 추가할 수 없습니다.",
        ));
    }
    if request.currency != state.currency {
        return Err(error(
            LedgerErrorCode::AccountMismatch,
            "주문 통화와 모의계좌 기준 통화가 일치하지 않습니다.",
        ));
    }
    if state.seen_order_ids.contains(&request.order_id) {
        return Err(error(
            LedgerErrorCode::DuplicateOrder,
            "이미 처리된 주문 ID입니다.",
        ));
    }
    if state
        .seen_idempotency_keys
        .contains(&request.idempotency_key)
    {
        return Err(error(
            LedgerErrorCode::DuplicateIdempotencyKey,
            "같은 멱등성 키로 주문을 다시 처리할 수 없습니다.",
        ));
    }

    let quote = quote_execution_scaled(
        request.side,
        request.reference_price_minor,
        request.quantity,
        request.quantity_scale,
        costs,
    )
    .map_err(|cost_error| match cost_error {
        CostError::InvalidCosts | CostError::InvalidOrder => error(
            LedgerErrorCode::InvalidEvent,
            "모의 체결 비용 또는 주문 값이 유효하지 않습니다.",
        ),
        CostError::Overflow => error(
            LedgerErrorCode::ArithmeticOverflow,
            "모의 체결 금액이 지원 범위를 초과했습니다.",
        ),
    })?;

    match request.side {
        TradeSide::Buy => {
            let debit = quote
                .notional_minor
                .checked_add(quote.fee_minor)
                .and_then(|value| value.checked_add(quote.tax_minor))
                .ok_or_else(|| {
                    error(
                        LedgerErrorCode::ArithmeticOverflow,
                        "매수 결제 금액이 범위를 초과했습니다.",
                    )
                })?;
            if state.cash_minor < debit {
                return Err(error(
                    LedgerErrorCode::InsufficientCash,
                    "가상 예수금이 부족합니다.",
                ));
            }
        }
        TradeSide::Sell => {
            if state.positions.get(&request.symbol).is_none_or(|position| {
                position.quantity_scale != request.quantity_scale
                    || position.quantity < request.quantity
            }) {
                return Err(error(
                    LedgerErrorCode::InsufficientPosition,
                    "보유한 가상 포지션보다 많은 수량을 매도할 수 없습니다.",
                ));
            }
        }
    }

    ledger
        .append(LedgerEvent::OrderFilled {
            account_id: request.account_id,
            order_id: request.order_id,
            idempotency_key: request.idempotency_key,
            symbol: request.symbol,
            side: request.side,
            quantity: request.quantity,
            quantity_scale: request.quantity_scale,
            reference_price_minor: request.reference_price_minor,
            execution_price_minor: quote.execution_price_minor,
            notional_minor: quote.notional_minor,
            fee_minor: quote.fee_minor,
            tax_minor: quote.tax_minor,
            costs,
            exit_reason: matches!(request.side, TradeSide::Sell).then(|| "user_manual".to_owned()),
            cause_event_id: None,
            occurred_at_ms: request.occurred_at_ms,
        })
        .map_err(|_| {
            error(
                LedgerErrorCode::AppendFailed,
                "모의 체결 사건을 원장에 추가하지 못했습니다.",
            )
        })?;

    replay_ledger(ledger.events())
}

pub fn replay_ledger(events: &[LedgerEvent]) -> Result<PaperAccountState, LedgerError> {
    let Some(LedgerEvent::AccountOpened {
        account_id,
        currency,
        initial_cash_minor,
        occurred_at_ms,
    }) = events.first()
    else {
        return Err(error(
            LedgerErrorCode::AccountNotOpened,
            "원장의 첫 사건은 모의계좌 개설이어야 합니다.",
        ));
    };
    if !valid_identifier(account_id) || !valid_currency(currency) || *initial_cash_minor == 0 {
        return Err(error(
            LedgerErrorCode::InvalidEvent,
            "모의계좌 개설 사건이 유효하지 않습니다.",
        ));
    }

    let mut state = PaperAccountState {
        account_id: account_id.clone(),
        currency: currency.clone(),
        cash_minor: *initial_cash_minor,
        realized_pnl_minor: 0,
        positions: BTreeMap::new(),
        event_count: 1,
        last_event_at_ms: *occurred_at_ms,
        seen_order_ids: BTreeSet::new(),
        seen_idempotency_keys: BTreeSet::new(),
    };

    for event in &events[1..] {
        if event.occurred_at_ms() < state.last_event_at_ms {
            return Err(error(
                LedgerErrorCode::EventTimeRegression,
                "원장 사건의 시각이 이전 사건보다 과거입니다.",
            ));
        }
        match event {
            LedgerEvent::AccountOpened { .. } => {
                return Err(error(
                    LedgerErrorCode::AccountAlreadyOpened,
                    "원장 중간에 계좌 개설 사건이 다시 나타났습니다.",
                ));
            }
            LedgerEvent::OrderFilled {
                account_id,
                order_id,
                idempotency_key,
                symbol,
                side,
                quantity,
                quantity_scale,
                reference_price_minor,
                execution_price_minor,
                notional_minor,
                fee_minor,
                tax_minor,
                costs,
                occurred_at_ms,
                ..
            } => {
                if account_id != &state.account_id {
                    return Err(error(
                        LedgerErrorCode::AccountMismatch,
                        "다른 계좌의 사건이 원장에 섞여 있습니다.",
                    ));
                }
                if !valid_identifier(order_id)
                    || !valid_identifier(idempotency_key)
                    || !valid_symbol(symbol)
                    || *quantity == 0
                    || *quantity_scale == 0
                    || *reference_price_minor == 0
                    || *execution_price_minor == 0
                {
                    return Err(error(
                        LedgerErrorCode::InvalidEvent,
                        "체결 사건의 식별자, 가격, 수량 또는 명목 금액이 유효하지 않습니다.",
                    ));
                }
                let expected_quote = quote_execution_scaled(
                    *side,
                    *reference_price_minor,
                    *quantity,
                    *quantity_scale,
                    *costs,
                )
                .map_err(|_| {
                    error(
                        LedgerErrorCode::InvalidEvent,
                        "원장에 기록된 비용 모델로 체결 값을 재현할 수 없습니다.",
                    )
                })?;
                if expected_quote.execution_price_minor != *execution_price_minor
                    || expected_quote.notional_minor != *notional_minor
                    || expected_quote.fee_minor != *fee_minor
                    || expected_quote.tax_minor != *tax_minor
                {
                    return Err(error(
                        LedgerErrorCode::InvalidEvent,
                        "체결 가격·명목 금액·수수료·세금이 기록된 비용 모델과 일치하지 않습니다.",
                    ));
                }
                if !state.seen_order_ids.insert(order_id.clone()) {
                    return Err(error(
                        LedgerErrorCode::DuplicateOrder,
                        "원장에 중복 주문 ID가 있습니다.",
                    ));
                }
                if !state.seen_idempotency_keys.insert(idempotency_key.clone()) {
                    return Err(error(
                        LedgerErrorCode::DuplicateIdempotencyKey,
                        "원장에 중복 멱등성 키가 있습니다.",
                    ));
                }

                match side {
                    TradeSide::Buy => {
                        let debit = notional_minor
                            .checked_add(*fee_minor)
                            .and_then(|value| value.checked_add(*tax_minor))
                            .ok_or_else(|| {
                                error(
                                    LedgerErrorCode::ArithmeticOverflow,
                                    "원장의 매수 결제 금액이 범위를 초과했습니다.",
                                )
                            })?;
                        state.cash_minor =
                            state.cash_minor.checked_sub(debit).ok_or_else(|| {
                                error(
                                    LedgerErrorCode::InsufficientCash,
                                    "원장 재생 중 가상 예수금이 음수가 되었습니다.",
                                )
                            })?;
                        let position =
                            state
                                .positions
                                .entry(symbol.clone())
                                .or_insert(PaperPosition {
                                    symbol: symbol.clone(),
                                    quantity: 0,
                                    quantity_scale: *quantity_scale,
                                    cost_basis_minor: 0,
                                });
                        if position.quantity_scale != *quantity_scale {
                            return Err(error(
                                LedgerErrorCode::InvalidEvent,
                                "같은 종목의 수량 정밀도가 원장 안에서 달라졌습니다.",
                            ));
                        }
                        position.quantity =
                            position.quantity.checked_add(*quantity).ok_or_else(|| {
                                error(
                                    LedgerErrorCode::ArithmeticOverflow,
                                    "포지션 수량이 범위를 초과했습니다.",
                                )
                            })?;
                        position.cost_basis_minor = position
                            .cost_basis_minor
                            .checked_add(debit)
                            .ok_or_else(|| {
                                error(
                                    LedgerErrorCode::ArithmeticOverflow,
                                    "포지션 원가가 범위를 초과했습니다.",
                                )
                            })?;
                    }
                    TradeSide::Sell => {
                        let position = state.positions.get(symbol).cloned().ok_or_else(|| {
                            error(
                                LedgerErrorCode::InsufficientPosition,
                                "원장 재생 중 보유하지 않은 종목의 매도가 발견되었습니다.",
                            )
                        })?;
                        if position.quantity < *quantity {
                            return Err(error(
                                LedgerErrorCode::InsufficientPosition,
                                "원장 재생 중 보유 수량을 초과한 매도가 발견되었습니다.",
                            ));
                        }
                        let credit = notional_minor
                            .checked_sub(*fee_minor)
                            .and_then(|value| value.checked_sub(*tax_minor))
                            .ok_or_else(|| {
                                error(
                                    LedgerErrorCode::InvalidEvent,
                                    "수수료와 세금이 매도 명목 금액을 초과했습니다.",
                                )
                            })?;
                        let released_cost = proportional_cost_basis(&position, *quantity)?;
                        state.cash_minor =
                            state.cash_minor.checked_add(credit).ok_or_else(|| {
                                error(
                                    LedgerErrorCode::ArithmeticOverflow,
                                    "가상 예수금이 범위를 초과했습니다.",
                                )
                            })?;
                        state.realized_pnl_minor = state
                            .realized_pnl_minor
                            .checked_add(checked_signed_difference(credit, released_cost)?)
                            .ok_or_else(|| {
                                error(
                                    LedgerErrorCode::ArithmeticOverflow,
                                    "누적 실현손익이 범위를 초과했습니다.",
                                )
                            })?;

                        if position.quantity == *quantity {
                            state.positions.remove(symbol);
                        } else {
                            let remaining = state.positions.get_mut(symbol).ok_or_else(|| {
                                error(
                                    LedgerErrorCode::InvalidEvent,
                                    "부분 매도 후 갱신할 포지션을 찾지 못했습니다.",
                                )
                            })?;
                            remaining.quantity -= *quantity;
                            remaining.cost_basis_minor -= released_cost;
                        }
                    }
                }
                state.last_event_at_ms = *occurred_at_ms;
                state.event_count += 1;
            }
        }
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zero_costs() -> TradingCosts {
        TradingCosts {
            buy_fee_bps: 0.0,
            sell_fee_bps: 0.0,
            sell_tax_bps: 0.0,
            slippage_bps: 0.0,
        }
    }

    fn order(
        order_id: &str,
        idempotency_key: &str,
        side: TradeSide,
        quantity: u64,
        price: u64,
        occurred_at_ms: u64,
    ) -> ShadowOrderRequest {
        ShadowOrderRequest {
            account_id: "paper-001".to_owned(),
            order_id: order_id.to_owned(),
            idempotency_key: idempotency_key.to_owned(),
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            side,
            quantity,
            quantity_scale: 1,
            reference_price_minor: price,
            occurred_at_ms,
        }
    }

    fn opened_ledger() -> InMemoryLedger {
        let mut ledger = InMemoryLedger::new();
        open_paper_account(
            &mut ledger,
            "paper-001".to_owned(),
            "KRW".to_owned(),
            1_000_000,
            1_000,
        )
        .unwrap();
        ledger
    }

    #[test]
    fn replays_partial_sales_into_cash_position_and_realized_pnl() {
        let mut ledger = opened_ledger();
        execute_shadow_order(
            &mut ledger,
            order("order-1", "idem-1", TradeSide::Buy, 10, 10_000, 2_000),
            zero_costs(),
        )
        .unwrap();
        let state = execute_shadow_order(
            &mut ledger,
            order("order-2", "idem-2", TradeSide::Sell, 4, 12_000, 3_000),
            zero_costs(),
        )
        .unwrap();

        assert_eq!(state.cash_minor, 948_000);
        assert_eq!(state.realized_pnl_minor, 8_000);
        assert_eq!(state.positions["005930"].quantity, 6);
        assert_eq!(state.positions["005930"].cost_basis_minor, 60_000);
        assert_eq!(state.event_count, 3);
    }

    #[test]
    fn replays_fractional_crypto_base_units_without_float_rounding() {
        let mut ledger = opened_ledger();
        let mut buy = order(
            "coin-buy",
            "coin-buy",
            TradeSide::Buy,
            10_000_000,
            1_000_000,
            2_000,
        );
        buy.symbol = "KRW-BTC".to_owned();
        buy.quantity_scale = 100_000_000;
        let state = execute_shadow_order(&mut ledger, buy, zero_costs()).expect("fractional buy");
        assert_eq!(state.cash_minor, 900_000);
        assert_eq!(state.positions["KRW-BTC"].quantity, 10_000_000);
        assert_eq!(state.positions["KRW-BTC"].quantity_scale, 100_000_000);
    }

    #[test]
    fn rejects_duplicate_idempotency_without_appending_an_event() {
        let mut ledger = opened_ledger();
        execute_shadow_order(
            &mut ledger,
            order("order-1", "same-key", TradeSide::Buy, 1, 10_000, 2_000),
            zero_costs(),
        )
        .unwrap();
        let before = ledger.events().len();

        let result = execute_shadow_order(
            &mut ledger,
            order("order-2", "same-key", TradeSide::Buy, 1, 10_000, 3_000),
            zero_costs(),
        );

        assert_eq!(
            result.unwrap_err().code,
            LedgerErrorCode::DuplicateIdempotencyKey
        );
        assert_eq!(ledger.events().len(), before);
    }

    #[test]
    fn rejects_insufficient_cash_and_naked_sales() {
        let mut ledger = opened_ledger();
        assert_eq!(
            execute_shadow_order(
                &mut ledger,
                order("large-buy", "large-buy", TradeSide::Buy, 101, 10_000, 2_000),
                zero_costs(),
            )
            .unwrap_err()
            .code,
            LedgerErrorCode::InsufficientCash
        );
        assert_eq!(
            execute_shadow_order(
                &mut ledger,
                order(
                    "naked-sell",
                    "naked-sell",
                    TradeSide::Sell,
                    1,
                    10_000,
                    2_000
                ),
                zero_costs(),
            )
            .unwrap_err()
            .code,
            LedgerErrorCode::InsufficientPosition
        );
        assert_eq!(ledger.events().len(), 1);
    }

    #[test]
    fn rejects_an_order_in_a_different_currency() {
        let mut ledger = opened_ledger();
        let mut usd_order = order("usd-order", "usd-order", TradeSide::Buy, 1, 10_000, 2_000);
        usd_order.currency = "USD".to_owned();

        let result = execute_shadow_order(&mut ledger, usd_order, zero_costs());

        assert_eq!(result.unwrap_err().code, LedgerErrorCode::AccountMismatch);
        assert_eq!(ledger.events().len(), 1);
    }

    #[test]
    fn detects_tampered_or_out_of_order_ledger_events() {
        let mut ledger = opened_ledger();
        ledger.events.push(LedgerEvent::OrderFilled {
            account_id: "paper-001".to_owned(),
            order_id: "tampered".to_owned(),
            idempotency_key: "tampered".to_owned(),
            symbol: "005930".to_owned(),
            side: TradeSide::Buy,
            quantity: 10,
            quantity_scale: 1,
            reference_price_minor: 10_000,
            execution_price_minor: 10_000,
            notional_minor: 99_999,
            fee_minor: 0,
            tax_minor: 0,
            costs: zero_costs(),
            exit_reason: None,
            cause_event_id: None,
            occurred_at_ms: 2_000,
        });
        assert_eq!(
            replay_ledger(ledger.events()).unwrap_err().code,
            LedgerErrorCode::InvalidEvent
        );

        let mut out_of_order = opened_ledger();
        out_of_order.events.push(LedgerEvent::OrderFilled {
            account_id: "paper-001".to_owned(),
            order_id: "past".to_owned(),
            idempotency_key: "past".to_owned(),
            symbol: "005930".to_owned(),
            side: TradeSide::Buy,
            quantity: 1,
            quantity_scale: 1,
            reference_price_minor: 10_000,
            execution_price_minor: 10_000,
            notional_minor: 10_000,
            fee_minor: 0,
            tax_minor: 0,
            costs: zero_costs(),
            exit_reason: None,
            cause_event_id: None,
            occurred_at_ms: 999,
        });
        assert_eq!(
            replay_ledger(out_of_order.events()).unwrap_err().code,
            LedgerErrorCode::EventTimeRegression
        );
    }
}
