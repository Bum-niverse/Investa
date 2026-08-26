use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::persistence::{now_ms, PersistenceBridge};

const LEDGER_ID: &str = "paper-futures-krw";
const INITIAL_CASH_MINOR: u64 = 100_000_000;
const MAX_CONTRACTS_PER_ORDER: u32 = 100;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FuturesKind {
    Stock,
    Index,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FuturesSide {
    Long,
    Short,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenFuturesPositionRequest {
    pub request_id: String,
    pub symbol: String,
    pub name: String,
    pub kind: FuturesKind,
    pub side: FuturesSide,
    pub contracts: u32,
    pub entry_price_minor: u64,
    pub price_scale: u64,
    pub contract_multiplier: u64,
    pub initial_margin_bps: u32,
    pub maintenance_margin_bps: u32,
    pub fee_minor: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkFuturesPositionRequest {
    pub request_id: String,
    pub position_id: String,
    pub mark_price_minor: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseFuturesPositionRequest {
    pub request_id: String,
    pub position_id: String,
    pub exit_price_minor: u64,
    pub fee_minor: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FuturesLifecycleKind {
    DailySettlement,
    ExpiryClose,
    ManualRollover,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesLifecycleRequest {
    pub event_id: String,
    pub request_id: String,
    pub contract_symbol: String,
    pub kind: FuturesLifecycleKind,
    pub previous_settlement_price_minor: u64,
    pub settlement_price_minor: u64,
    pub contracts: u32,
    pub contract_multiplier: u64,
    pub price_scale: u64,
    pub rollover_to_symbol: Option<String>,
    pub automatic: bool,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FuturesLifecycleEvent {
    pub event_id: String,
    pub request_id: String,
    pub contract_symbol: String,
    pub kind: FuturesLifecycleKind,
    pub previous_settlement_price_minor: u64,
    pub settlement_price_minor: u64,
    pub variation_margin_minor: i64,
    pub rollover_to_symbol: Option<String>,
    pub automatic: bool,
    pub occurred_at_ms: u64,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum FuturesEvent {
    AccountOpened {
        initial_cash_minor: u64,
        occurred_at_ms: u64,
    },
    PositionOpened {
        request_id: String,
        position: FuturesPosition,
        fee_minor: u64,
        occurred_at_ms: u64,
    },
    PositionMarked {
        request_id: String,
        position_id: String,
        mark_price_minor: u64,
        occurred_at_ms: u64,
    },
    PositionClosed {
        request_id: String,
        position_id: String,
        exit_price_minor: u64,
        fee_minor: u64,
        occurred_at_ms: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesPosition {
    pub position_id: String,
    pub symbol: String,
    pub name: String,
    pub kind: FuturesKind,
    pub side: FuturesSide,
    pub contracts: u32,
    pub entry_price_minor: u64,
    pub mark_price_minor: u64,
    pub contract_multiplier: u64,
    pub price_scale: u64,
    pub initial_margin_bps: u32,
    pub maintenance_margin_bps: u32,
    pub reserved_margin_minor: u64,
    pub unrealized_pnl_minor: i64,
    pub maintenance_required_minor: u64,
    pub liquidation_warning: bool,
    pub opened_at_ms: u64,
    pub marked_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesPaperSnapshot {
    pub currency: &'static str,
    pub initial_cash_minor: u64,
    pub available_cash_minor: i64,
    pub equity_minor: i64,
    pub reserved_margin_minor: u64,
    pub unrealized_pnl_minor: i64,
    pub realized_pnl_minor: i64,
    pub positions: Vec<FuturesPosition>,
    pub event_count: usize,
    pub live_order_enabled: bool,
    pub warning: &'static str,
}

#[derive(Default)]
struct ReplayState {
    available_cash_minor: i128,
    realized_pnl_minor: i128,
    positions: Vec<FuturesPosition>,
}

fn checked_notional(
    price: u64,
    multiplier: u64,
    contracts: u32,
    price_scale: u64,
) -> Result<u64, String> {
    if price_scale == 0 {
        return Err("가격 배율은 0일 수 없습니다.".to_owned());
    }
    let numerator = u128::from(price)
        .checked_mul(u128::from(multiplier))
        .and_then(|value| value.checked_mul(u128::from(contracts)))
        .ok_or_else(|| "계약 명목금액이 지원 범위를 초과했습니다.".to_owned())?;
    u64::try_from(numerator.div_ceil(u128::from(price_scale)))
        .map_err(|_| "계약 명목금액이 지원 범위를 초과했습니다.".to_owned())
}

fn margin(notional: u64, bps: u32) -> Result<u64, String> {
    u64::try_from((u128::from(notional) * u128::from(bps)).div_ceil(10_000))
        .map_err(|_| "증거금 계산 결과가 지원 범위를 초과했습니다.".to_owned())
}

fn pnl(position: &FuturesPosition, price: u64) -> Result<i64, String> {
    let price_delta = i128::from(price) - i128::from(position.entry_price_minor);
    let signed_delta = match position.side {
        FuturesSide::Long => price_delta,
        FuturesSide::Short => -price_delta,
    };
    i64::try_from(
        signed_delta * i128::from(position.contract_multiplier) * i128::from(position.contracts)
            / i128::from(position.price_scale),
    )
    .map_err(|_| "평가손익이 지원 범위를 초과했습니다.".to_owned())
}

fn valid_identifier(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn build_lifecycle_event(
    request: FuturesLifecycleRequest,
) -> Result<FuturesLifecycleEvent, String> {
    if !valid_identifier(request.event_id.trim(), 128)
        || !valid_identifier(request.request_id.trim(), 128)
        || !valid_identifier(request.contract_symbol.trim(), 64)
        || request.previous_settlement_price_minor == 0
        || request.settlement_price_minor == 0
        || request.contracts == 0
        || request.contract_multiplier == 0
        || request.price_scale == 0
        || request.occurred_at_ms == 0
    {
        return Err("선물 수명주기 사건의 식별자·가격·계약·시각을 확인해 주세요.".to_owned());
    }
    if request.automatic {
        return Err(
            "선물 만기 연장과 롤오버는 자동 실행할 수 없습니다. 새 수동 요청으로 승인해야 합니다."
                .to_owned(),
        );
    }
    match request.kind {
        FuturesLifecycleKind::ManualRollover => {
            let target = request.rollover_to_symbol.as_deref().unwrap_or_default();
            if !valid_identifier(target, 64) || target == request.contract_symbol {
                return Err("롤오버는 기존 계약과 다른 신규 계약코드가 필요합니다.".to_owned());
            }
        }
        _ if request.rollover_to_symbol.is_some() => {
            return Err("신규 계약코드는 수동 롤오버 사건에서만 사용할 수 있습니다.".to_owned());
        }
        _ => {}
    }
    let delta = i128::from(request.settlement_price_minor)
        - i128::from(request.previous_settlement_price_minor);
    let variation = delta * i128::from(request.contracts) * i128::from(request.contract_multiplier)
        / i128::from(request.price_scale);
    Ok(FuturesLifecycleEvent {
        event_id: request.event_id,
        request_id: request.request_id,
        contract_symbol: request.contract_symbol.to_uppercase(),
        kind: request.kind,
        previous_settlement_price_minor: request.previous_settlement_price_minor,
        settlement_price_minor: request.settlement_price_minor,
        variation_margin_minor: i64::try_from(variation)
            .map_err(|_| "일일정산 변동증거금이 지원 범위를 초과했습니다.".to_owned())?,
        rollover_to_symbol: request.rollover_to_symbol.map(|value| value.to_uppercase()),
        automatic: false,
        occurred_at_ms: request.occurred_at_ms,
        live_order_allowed: false,
    })
}

fn lifecycle_kind_db(kind: &FuturesLifecycleKind) -> &'static str {
    match kind {
        FuturesLifecycleKind::DailySettlement => "daily_settlement",
        FuturesLifecycleKind::ExpiryClose => "expiry_close",
        FuturesLifecycleKind::ManualRollover => "manual_rollover",
    }
}

fn lifecycle_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<FuturesLifecycleEvent> {
    let json: String = row.get(0)?;
    serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}

fn validate_open(request: &OpenFuturesPositionRequest) -> Result<(), String> {
    if !valid_identifier(request.request_id.trim(), 96)
        || !valid_identifier(request.symbol.trim(), 24)
    {
        return Err("요청 ID와 선물 종목코드는 영문·숫자·-_.만 사용할 수 있습니다.".to_owned());
    }
    if request.name.trim().is_empty()
        || request.name.chars().count() > 60
        || request.contracts == 0
        || request.contracts > MAX_CONTRACTS_PER_ORDER
        || request.entry_price_minor == 0
        || request.price_scale == 0
        || request.contract_multiplier == 0
    {
        return Err("상품명·계약 수·가격·계약승수의 허용 범위를 확인해 주세요.".to_owned());
    }
    if request.initial_margin_bps == 0
        || request.initial_margin_bps > 10_000
        || request.maintenance_margin_bps == 0
        || request.maintenance_margin_bps > request.initial_margin_bps
    {
        return Err("유지증거금률은 0보다 크고 개시증거금률 이하여야 합니다.".to_owned());
    }
    Ok(())
}

fn event_request_id(event: &FuturesEvent) -> Option<&str> {
    match event {
        FuturesEvent::AccountOpened { .. } => None,
        FuturesEvent::PositionOpened { request_id, .. }
        | FuturesEvent::PositionMarked { request_id, .. }
        | FuturesEvent::PositionClosed { request_id, .. } => Some(request_id),
    }
}

fn replay(events: &[FuturesEvent]) -> Result<ReplayState, String> {
    let mut state = ReplayState::default();
    for (index, event) in events.iter().enumerate() {
        match event {
            FuturesEvent::AccountOpened {
                initial_cash_minor, ..
            } if index == 0 => state.available_cash_minor = i128::from(*initial_cash_minor),
            FuturesEvent::AccountOpened { .. } => {
                return Err("선물 모의계좌 개설 사건이 중복됐습니다.".to_owned())
            }
            FuturesEvent::PositionOpened {
                position,
                fee_minor,
                ..
            } => {
                if state
                    .positions
                    .iter()
                    .any(|item| item.position_id == position.position_id)
                {
                    return Err("선물 포지션 식별자가 중복됐습니다.".to_owned());
                }
                state.available_cash_minor -=
                    i128::from(position.reserved_margin_minor) + i128::from(*fee_minor);
                state.realized_pnl_minor -= i128::from(*fee_minor);
                state.positions.push(position.clone());
            }
            FuturesEvent::PositionMarked {
                position_id,
                mark_price_minor,
                occurred_at_ms,
                ..
            } => {
                let position = state
                    .positions
                    .iter_mut()
                    .find(|item| item.position_id == *position_id)
                    .ok_or_else(|| "존재하지 않는 선물 포지션의 시가평가 사건입니다.".to_owned())?;
                position.mark_price_minor = *mark_price_minor;
                position.unrealized_pnl_minor = pnl(position, *mark_price_minor)?;
                position.maintenance_required_minor = margin(
                    checked_notional(
                        *mark_price_minor,
                        position.contract_multiplier,
                        position.contracts,
                        position.price_scale,
                    )?,
                    position.maintenance_margin_bps,
                )?;
                let margin_balance = i128::from(position.reserved_margin_minor)
                    + i128::from(position.unrealized_pnl_minor);
                position.liquidation_warning =
                    margin_balance < i128::from(position.maintenance_required_minor);
                position.marked_at_ms = *occurred_at_ms;
            }
            FuturesEvent::PositionClosed {
                position_id,
                exit_price_minor,
                fee_minor,
                ..
            } => {
                let index = state
                    .positions
                    .iter()
                    .position(|item| item.position_id == *position_id)
                    .ok_or_else(|| "존재하지 않는 선물 포지션의 청산 사건입니다.".to_owned())?;
                let position = state.positions.remove(index);
                let realized =
                    i128::from(pnl(&position, *exit_price_minor)?) - i128::from(*fee_minor);
                state.available_cash_minor += i128::from(position.reserved_margin_minor) + realized;
                state.realized_pnl_minor += realized;
            }
        }
    }
    Ok(state)
}

fn load_events(bridge: &PersistenceBridge) -> Result<Vec<FuturesEvent>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "선물 모의원장 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare("SELECT event_json FROM futures_paper_events WHERE ledger_id = ?1 ORDER BY event_index ASC").map_err(|error| format!("선물 모의원장을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![LEDGER_ID], |row| row.get::<_, String>(0))
        .map_err(|error| format!("선물 모의원장을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        let json = row.map_err(|error| format!("선물 모의원장을 읽지 못했습니다: {error}"))?;
        serde_json::from_str(&json)
            .map_err(|error| format!("선물 모의원장이 손상됐습니다: {error}"))
    })
    .collect()
}

fn append_event(bridge: &PersistenceBridge, event: &FuturesEvent) -> Result<(), String> {
    let json = serde_json::to_string(event)
        .map_err(|_| "선물 모의사건을 직렬화하지 못했습니다.".to_owned())?;
    let occurred_at_ms = match event {
        FuturesEvent::AccountOpened { occurred_at_ms, .. }
        | FuturesEvent::PositionOpened { occurred_at_ms, .. }
        | FuturesEvent::PositionMarked { occurred_at_ms, .. }
        | FuturesEvent::PositionClosed { occurred_at_ms, .. } => *occurred_at_ms,
    };
    let event_type = match event {
        FuturesEvent::AccountOpened { .. } => "account_opened",
        FuturesEvent::PositionOpened { .. } => "position_opened",
        FuturesEvent::PositionMarked { .. } => "position_marked",
        FuturesEvent::PositionClosed { .. } => "position_closed",
    };
    let request_id = match event {
        FuturesEvent::AccountOpened { .. } => None,
        FuturesEvent::PositionOpened { request_id, .. }
        | FuturesEvent::PositionMarked { request_id, .. }
        | FuturesEvent::PositionClosed { request_id, .. } => Some(request_id.as_str()),
    };
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "선물 모의원장 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("선물 모의사건 저장을 시작하지 못했습니다: {error}"))?;
    if let Some(request_id) = request_id {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT event_json FROM futures_paper_events WHERE request_id = ?1",
                params![request_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| format!("선물 중복 요청을 확인하지 못했습니다: {error}"))?;
        if let Some(existing) = existing {
            return if existing == json {
                Ok(())
            } else {
                Err("같은 요청 ID에 다른 선물 모의주문이 이미 저장돼 있습니다.".to_owned())
            };
        }
    }
    let index: u64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM futures_paper_events WHERE ledger_id = ?1",
            params![LEDGER_ID],
            |row| row.get(0),
        )
        .map_err(|error| format!("선물 사건 순번을 확인하지 못했습니다: {error}"))?;
    transaction.execute("INSERT INTO futures_paper_events (ledger_id,event_index,event_type,request_id,event_json,occurred_at_ms,created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?6)", params![LEDGER_ID,index,event_type,request_id,json,occurred_at_ms]).map_err(|error| format!("선물 모의사건을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("선물 모의사건을 확정하지 못했습니다: {error}"))
}

fn ensure_account(bridge: &PersistenceBridge) -> Result<Vec<FuturesEvent>, String> {
    let mut events = load_events(bridge)?;
    if events.is_empty() {
        append_event(
            bridge,
            &FuturesEvent::AccountOpened {
                initial_cash_minor: INITIAL_CASH_MINOR,
                occurred_at_ms: now_ms()?,
            },
        )?;
        events = load_events(bridge)?;
    }
    Ok(events)
}

fn snapshot(events: &[FuturesEvent]) -> Result<FuturesPaperSnapshot, String> {
    let state = replay(events)?;
    let reserved = state.positions.iter().try_fold(0_u64, |sum, item| {
        sum.checked_add(item.reserved_margin_minor)
            .ok_or_else(|| "예약 증거금 합계가 범위를 초과했습니다.".to_owned())
    })?;
    let unrealized = state.positions.iter().try_fold(0_i64, |sum, item| {
        sum.checked_add(item.unrealized_pnl_minor)
            .ok_or_else(|| "미실현손익 합계가 범위를 초과했습니다.".to_owned())
    })?;
    let equity = state.available_cash_minor + i128::from(reserved) + i128::from(unrealized);
    Ok(FuturesPaperSnapshot {
        currency: "KRW",
        initial_cash_minor: INITIAL_CASH_MINOR,
        available_cash_minor: i64::try_from(state.available_cash_minor)
            .map_err(|_| "가용현금이 범위를 초과했습니다.".to_owned())?,
        equity_minor: i64::try_from(equity)
            .map_err(|_| "계좌자산이 범위를 초과했습니다.".to_owned())?,
        reserved_margin_minor: reserved,
        unrealized_pnl_minor: unrealized,
        realized_pnl_minor: i64::try_from(state.realized_pnl_minor)
            .map_err(|_| "실현손익이 범위를 초과했습니다.".to_owned())?,
        positions: state.positions,
        event_count: events.len(),
        live_order_enabled: false,
        warning: "Investa 내부 선물 sandbox입니다. 실제 거래소·증권사 주문은 전송되지 않습니다.",
    })
}

#[tauri::command]
pub fn futures_paper_status(
    persistence: State<'_, PersistenceBridge>,
) -> Result<FuturesPaperSnapshot, String> {
    let events = ensure_account(persistence.inner())?;
    snapshot(&events)
}

#[tauri::command]
pub fn futures_paper_open(
    request: OpenFuturesPositionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<FuturesPaperSnapshot, String> {
    validate_open(&request)?;
    let events = ensure_account(persistence.inner())?;
    if events
        .iter()
        .any(|event| event_request_id(event) == Some(request.request_id.as_str()))
    {
        return snapshot(&events);
    }
    let state = replay(&events)?;
    let notional = checked_notional(
        request.entry_price_minor,
        request.contract_multiplier,
        request.contracts,
        request.price_scale,
    )?;
    let required_margin = margin(notional, request.initial_margin_bps)?;
    let required_cash = i128::from(required_margin) + i128::from(request.fee_minor);
    if state.available_cash_minor < required_cash {
        return Err("선물 모의계좌의 가용현금이 개시증거금과 수수료보다 부족합니다.".to_owned());
    }
    let occurred_at_ms = now_ms()?;
    append_event(
        persistence.inner(),
        &FuturesEvent::PositionOpened {
            request_id: request.request_id.clone(),
            fee_minor: request.fee_minor,
            occurred_at_ms,
            position: FuturesPosition {
                position_id: format!("futures-{}", request.request_id),
                symbol: request.symbol.trim().to_uppercase(),
                name: request.name.trim().to_owned(),
                kind: request.kind,
                side: request.side,
                contracts: request.contracts,
                entry_price_minor: request.entry_price_minor,
                mark_price_minor: request.entry_price_minor,
                contract_multiplier: request.contract_multiplier,
                price_scale: request.price_scale,
                initial_margin_bps: request.initial_margin_bps,
                maintenance_margin_bps: request.maintenance_margin_bps,
                reserved_margin_minor: required_margin,
                unrealized_pnl_minor: 0,
                maintenance_required_minor: margin(notional, request.maintenance_margin_bps)?,
                liquidation_warning: false,
                opened_at_ms: occurred_at_ms,
                marked_at_ms: occurred_at_ms,
            },
        },
    )?;
    snapshot(&load_events(persistence.inner())?)
}

#[tauri::command]
pub fn futures_paper_mark(
    request: MarkFuturesPositionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<FuturesPaperSnapshot, String> {
    if !valid_identifier(request.request_id.trim(), 96)
        || !valid_identifier(request.position_id.trim(), 120)
        || request.mark_price_minor == 0
    {
        return Err("시가평가 요청 ID와 가격을 확인해 주세요.".to_owned());
    }
    let events = ensure_account(persistence.inner())?;
    if events
        .iter()
        .any(|event| event_request_id(event) == Some(request.request_id.as_str()))
    {
        return snapshot(&events);
    }
    if !replay(&events)?
        .positions
        .iter()
        .any(|item| item.position_id == request.position_id)
    {
        return Err("시가평가할 선물 포지션을 찾지 못했습니다.".to_owned());
    }
    append_event(
        persistence.inner(),
        &FuturesEvent::PositionMarked {
            request_id: request.request_id,
            position_id: request.position_id,
            mark_price_minor: request.mark_price_minor,
            occurred_at_ms: now_ms()?,
        },
    )?;
    snapshot(&load_events(persistence.inner())?)
}

#[tauri::command]
pub fn futures_paper_close(
    request: CloseFuturesPositionRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<FuturesPaperSnapshot, String> {
    if !valid_identifier(request.request_id.trim(), 96)
        || !valid_identifier(request.position_id.trim(), 120)
        || request.exit_price_minor == 0
    {
        return Err("청산 요청 ID와 가격을 확인해 주세요.".to_owned());
    }
    let events = ensure_account(persistence.inner())?;
    if events
        .iter()
        .any(|event| event_request_id(event) == Some(request.request_id.as_str()))
    {
        return snapshot(&events);
    }
    if !replay(&events)?
        .positions
        .iter()
        .any(|item| item.position_id == request.position_id)
    {
        return Err("청산할 선물 포지션을 찾지 못했습니다.".to_owned());
    }
    append_event(
        persistence.inner(),
        &FuturesEvent::PositionClosed {
            request_id: request.request_id,
            position_id: request.position_id,
            exit_price_minor: request.exit_price_minor,
            fee_minor: request.fee_minor,
            occurred_at_ms: now_ms()?,
        },
    )?;
    snapshot(&load_events(persistence.inner())?)
}

#[tauri::command]
pub fn futures_lifecycle_record(
    request: FuturesLifecycleRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<FuturesLifecycleEvent, String> {
    let event = build_lifecycle_event(request)?;
    let json = serde_json::to_string(&event)
        .map_err(|_| "선물 수명주기 사건을 직렬화하지 못했습니다.".to_owned())?;
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "선물 수명주기 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing = connection.query_row(
        "SELECT event_json FROM futures_lifecycle_events WHERE event_id=?1 OR request_id=?2 LIMIT 1",
        params![event.event_id,event.request_id], lifecycle_row,
    ).optional().map_err(|error| format!("기존 선물 수명주기 사건을 확인하지 못했습니다: {error}"))?;
    if let Some(existing) = existing {
        if existing == event {
            return Ok(existing);
        }
        return Err("같은 사건 또는 요청 식별자에 다른 선물 수명주기 기록이 있습니다.".to_owned());
    }
    connection.execute(
        "INSERT INTO futures_lifecycle_events(event_id,request_id,contract_symbol,event_type,event_json,occurred_at_ms) VALUES(?1,?2,?3,?4,?5,?6)",
        params![event.event_id,event.request_id,event.contract_symbol,lifecycle_kind_db(&event.kind),json,event.occurred_at_ms],
    ).map_err(|error| format!("선물 수명주기 사건을 저장하지 못했습니다: {error}"))?;
    Ok(event)
}

#[tauri::command]
pub fn futures_lifecycle_history(
    limit: u16,
    persistence: State<'_, PersistenceBridge>,
) -> Result<Vec<FuturesLifecycleEvent>, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "선물 수명주기 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare("SELECT event_json FROM futures_lifecycle_events ORDER BY occurred_at_ms DESC,event_id DESC LIMIT ?1")
        .map_err(|error| format!("선물 수명주기 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 500)], lifecycle_row)
        .map_err(|error| format!("선물 수명주기 이력을 조회하지 못했습니다: {error}"))?
        .map(|row| row.map_err(|error| format!("선물 수명주기 이력을 읽지 못했습니다: {error}")))
        .collect();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(side: FuturesSide) -> FuturesPosition {
        FuturesPosition {
            position_id: "p1".into(),
            symbol: "TEST".into(),
            name: "테스트".into(),
            kind: FuturesKind::Index,
            side,
            contracts: 2,
            entry_price_minor: 1000,
            mark_price_minor: 1000,
            contract_multiplier: 250,
            price_scale: 1,
            initial_margin_bps: 1500,
            maintenance_margin_bps: 1000,
            reserved_margin_minor: 75_000,
            unrealized_pnl_minor: 0,
            maintenance_required_minor: 50_000,
            liquidation_warning: false,
            opened_at_ms: 1,
            marked_at_ms: 1,
        }
    }

    #[test]
    fn lifecycle_records_settlement_and_rejects_automatic_rollover() {
        let settlement = build_lifecycle_event(FuturesLifecycleRequest {
            event_id: "settle-1".into(),
            request_id: "request-1".into(),
            contract_symbol: "KOSPI200M6".into(),
            kind: FuturesLifecycleKind::DailySettlement,
            previous_settlement_price_minor: 1000,
            settlement_price_minor: 1010,
            contracts: 2,
            contract_multiplier: 250,
            price_scale: 1,
            rollover_to_symbol: None,
            automatic: false,
            occurred_at_ms: 1,
        })
        .expect("settlement");
        assert_eq!(settlement.variation_margin_minor, 5_000);
        assert!(!settlement.live_order_allowed);
        let error = build_lifecycle_event(FuturesLifecycleRequest {
            event_id: "roll-1".into(),
            request_id: "request-2".into(),
            contract_symbol: "OLD".into(),
            kind: FuturesLifecycleKind::ManualRollover,
            previous_settlement_price_minor: 1000,
            settlement_price_minor: 1000,
            contracts: 1,
            contract_multiplier: 1,
            price_scale: 1,
            rollover_to_symbol: Some("NEW".into()),
            automatic: true,
            occurred_at_ms: 2,
        })
        .unwrap_err();
        assert!(error.contains("자동 실행"));
    }

    #[test]
    fn long_and_short_pnl_are_symmetric() {
        assert_eq!(pnl(&position(FuturesSide::Long), 1100), Ok(50_000));
        assert_eq!(pnl(&position(FuturesSide::Short), 1100), Ok(-50_000));
    }

    #[test]
    fn marks_liquidation_warning_below_maintenance_margin() {
        let events = vec![
            FuturesEvent::AccountOpened {
                initial_cash_minor: 100_000,
                occurred_at_ms: 1,
            },
            FuturesEvent::PositionOpened {
                request_id: "o1".into(),
                position: position(FuturesSide::Long),
                fee_minor: 0,
                occurred_at_ms: 2,
            },
            FuturesEvent::PositionMarked {
                request_id: "m1".into(),
                position_id: "p1".into(),
                mark_price_minor: 900,
                occurred_at_ms: 3,
            },
        ];
        let state = replay(&events).unwrap();
        assert!(state.positions[0].liquidation_warning);
        assert_eq!(state.positions[0].unrealized_pnl_minor, -50_000);
    }

    #[test]
    fn validates_margin_ordering_and_contract_limit() {
        let request = OpenFuturesPositionRequest {
            request_id: "r1".into(),
            symbol: "KOSPI200".into(),
            name: "코스피200 선물".into(),
            kind: FuturesKind::Index,
            side: FuturesSide::Long,
            contracts: 101,
            entry_price_minor: 35000,
            price_scale: 100,
            contract_multiplier: 250_000,
            initial_margin_bps: 1000,
            maintenance_margin_bps: 1200,
            fee_minor: 0,
        };
        assert!(validate_open(&request).is_err());
    }

    #[test]
    fn persists_and_replays_the_append_only_futures_ledger() {
        let bridge = PersistenceBridge::in_memory().unwrap();
        let opened_at = 10;
        append_event(
            &bridge,
            &FuturesEvent::AccountOpened {
                initial_cash_minor: 100_000,
                occurred_at_ms: 1,
            },
        )
        .unwrap();
        append_event(
            &bridge,
            &FuturesEvent::PositionOpened {
                request_id: "open-1".into(),
                position: FuturesPosition {
                    opened_at_ms: opened_at,
                    marked_at_ms: opened_at,
                    ..position(FuturesSide::Long)
                },
                fee_minor: 100,
                occurred_at_ms: opened_at,
            },
        )
        .unwrap();
        append_event(
            &bridge,
            &FuturesEvent::PositionMarked {
                request_id: "mark-1".into(),
                position_id: "p1".into(),
                mark_price_minor: 1_100,
                occurred_at_ms: 20,
            },
        )
        .unwrap();

        let events = load_events(&bridge).unwrap();
        let result = snapshot(&events).unwrap();
        assert_eq!(result.event_count, 3);
        assert_eq!(result.available_cash_minor, 24_900);
        assert_eq!(result.unrealized_pnl_minor, 50_000);
        assert_eq!(result.equity_minor, 149_900);
    }
}
