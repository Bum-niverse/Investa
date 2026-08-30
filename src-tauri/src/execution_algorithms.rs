use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{persistence::PersistenceBridge, trading::TradeSide};

const CONTRACT_VERSION: &str = "internal-execution-v1";
const MAX_CHILDREN: u16 = 100;
const MAX_SQLITE_INTEGER: u64 = i64::MAX as u64;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAssetClass {
    StockSpot,
    CryptoSpot,
    SecuritiesFuture,
    CryptoPerpetual,
}

impl ExecutionAssetClass {
    fn is_derivative(self) -> bool {
        matches!(self, Self::SecuritiesFuture | Self::CryptoPerpetual)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionIntent {
    Open,
    Reduce,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Working,
    PartiallyFilled,
    Filled,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPolicy {
    pub quantity_step: u64,
    pub maximum_child_quantity: u64,
    pub maximum_child_count: u16,
    pub maximum_reprices: u16,
    pub maximum_slippage_bps: u64,
    pub minimum_liquidation_buffer_bps: u64,
    pub maximum_leverage_milli: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DerivativeExecutionBoundary {
    pub intent: ExecutionIntent,
    pub reduce_only: bool,
    pub signed_position_quantity: i64,
    pub leverage_milli: u64,
    pub isolated_margin: bool,
    pub available_margin_minor: u64,
    pub initial_margin_required_minor: u64,
    pub maintenance_margin_minor: u64,
    pub mark_price_minor: u64,
    pub liquidation_price_minor: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionPlanRequest {
    pub execution_id: String,
    pub idempotency_key: String,
    pub asset_class: ExecutionAssetClass,
    pub market: String,
    pub symbol: String,
    pub currency: String,
    pub side: TradeSide,
    pub total_quantity: u64,
    pub quantity_scale: u64,
    pub reference_price_minor: u64,
    pub initial_limit_price_minor: u64,
    pub expires_at_ms: u64,
    pub policy: ExecutionPolicy,
    pub derivative: Option<DerivativeExecutionBoundary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionChildOrder {
    pub child_index: u16,
    pub quantity: u64,
    pub filled_quantity: u64,
    pub status: ExecutionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InternalExecutionState {
    pub contract_version: String,
    pub execution_id: String,
    pub idempotency_key: String,
    pub asset_class: ExecutionAssetClass,
    pub market: String,
    pub symbol: String,
    pub currency: String,
    pub side: TradeSide,
    pub total_quantity: u64,
    pub quantity_scale: u64,
    pub cumulative_filled_quantity: u64,
    pub weighted_fill_notional: u128,
    pub average_fill_price_minor: Option<u64>,
    pub reference_price_minor: u64,
    pub current_limit_price_minor: u64,
    pub reprice_count: u16,
    pub expires_at_ms: u64,
    pub status: ExecutionStatus,
    pub children: Vec<ExecutionChildOrder>,
    pub derivative: Option<DerivativeExecutionBoundary>,
    pub live_order_allowed: bool,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionFillRequest {
    pub execution_id: String,
    pub event_id: String,
    pub fill_quantity: u64,
    pub fill_price_minor: u64,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionRepriceRequest {
    pub execution_id: String,
    pub event_id: String,
    pub new_limit_price_minor: u64,
    pub quote_price_minor: u64,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionCloseRequest {
    pub execution_id: String,
    pub event_id: String,
    pub reason: String,
    pub occurred_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredExecutionPlan {
    request: ExecutionPlanRequest,
    state: InternalExecutionState,
}

fn valid_id(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn slippage_bps(reference: u64, price: u64) -> u64 {
    if reference == 0 {
        return u64::MAX;
    }
    u64::try_from((u128::from(reference.abs_diff(price)) * 10_000).div_ceil(u128::from(reference)))
        .unwrap_or(u64::MAX)
}

fn validate_derivative(
    request: &ExecutionPlanRequest,
    boundary: &DerivativeExecutionBoundary,
) -> Result<(), String> {
    if !request.asset_class.is_derivative() {
        return Err("현물 실행 계획에 파생상품 증거금 경계를 넣을 수 없습니다.".to_owned());
    }
    if boundary.leverage_milli == 0
        || boundary.leverage_milli > request.policy.maximum_leverage_milli.min(2_000)
        || !boundary.isolated_margin
        || boundary.mark_price_minor == 0
        || boundary.liquidation_price_minor == 0
        || boundary.maintenance_margin_minor > boundary.available_margin_minor
    {
        return Err(
            "파생상품은 최대 2배 격리증거금·유지증거금·mark·청산가 경계를 충족해야 합니다."
                .to_owned(),
        );
    }
    let liquidation_buffer =
        slippage_bps(boundary.mark_price_minor, boundary.liquidation_price_minor);
    if liquidation_buffer < request.policy.minimum_liquidation_buffer_bps {
        return Err("청산가 완충거리가 실행 정책의 최소 한도보다 작습니다.".to_owned());
    }
    match boundary.intent {
        ExecutionIntent::Open => {
            if boundary.reduce_only
                || boundary.initial_margin_required_minor == 0
                || boundary.initial_margin_required_minor > boundary.available_margin_minor
            {
                return Err("신규 파생 포지션은 reduce-only가 아니어야 하며 충분한 초기증거금이 필요합니다.".to_owned());
            }
        }
        ExecutionIntent::Reduce => {
            let reduces_position = match request.side {
                TradeSide::Buy => boundary.signed_position_quantity < 0,
                TradeSide::Sell => boundary.signed_position_quantity > 0,
            };
            let position_quantity = boundary.signed_position_quantity.unsigned_abs();
            if !boundary.reduce_only
                || !reduces_position
                || request.total_quantity > position_quantity
            {
                return Err("감축 주문은 reduce-only여야 하며 기존 포지션을 넘거나 방향을 뒤집을 수 없습니다.".to_owned());
            }
        }
    }
    Ok(())
}

fn split_children(request: &ExecutionPlanRequest) -> Result<Vec<ExecutionChildOrder>, String> {
    let step = request.policy.quantity_step;
    let child_cap = request.policy.maximum_child_quantity / step * step;
    if step == 0 || child_cap == 0 || !request.total_quantity.is_multiple_of(step) {
        return Err("총 수량과 분할 수량은 최소 주문 단위의 배수여야 합니다.".to_owned());
    }
    let mut remaining = request.total_quantity;
    let mut children = Vec::new();
    while remaining > 0 {
        let quantity = remaining.min(child_cap);
        children.push(ExecutionChildOrder {
            child_index: u16::try_from(children.len())
                .map_err(|_| "분할 주문 수가 너무 많습니다.")?,
            quantity,
            filled_quantity: 0,
            status: ExecutionStatus::Working,
        });
        remaining -= quantity;
    }
    let maximum = request.policy.maximum_child_count.min(MAX_CHILDREN);
    if children.len() > usize::from(maximum) {
        return Err("분할 주문 수가 정책 한도를 초과합니다.".to_owned());
    }
    Ok(children)
}

fn build_plan(request: ExecutionPlanRequest, now_ms: u64) -> Result<StoredExecutionPlan, String> {
    if now_ms == 0
        || now_ms > MAX_SQLITE_INTEGER
        || !valid_id(&request.execution_id, 128)
        || !valid_id(&request.idempotency_key, 128)
        || !valid_id(&request.symbol, 32)
        || request.market.trim().is_empty()
        || request.market.len() > 32
        || request.currency.len() < 3
        || request.currency.len() > 8
        || request.total_quantity == 0
        || request.quantity_scale == 0
        || request.reference_price_minor == 0
        || request.initial_limit_price_minor == 0
        || request.expires_at_ms <= now_ms
        || request.expires_at_ms > MAX_SQLITE_INTEGER
        || request.policy.maximum_child_count == 0
        || request.policy.maximum_child_count > MAX_CHILDREN
        || request.policy.maximum_reprices > 100
        || request.policy.maximum_slippage_bps > 10_000
        || request.policy.minimum_liquidation_buffer_bps > 10_000
        || request.policy.maximum_leverage_milli == 0
        || request.policy.maximum_leverage_milli > 2_000
        || slippage_bps(
            request.reference_price_minor,
            request.initial_limit_price_minor,
        ) > request.policy.maximum_slippage_bps
    {
        return Err("내부 실행 계획의 식별자·수량·가격·만료·정책을 확인해 주세요.".to_owned());
    }
    let market_currency_valid = match request.asset_class {
        ExecutionAssetClass::StockSpot => matches!(
            (request.market.as_str(), request.currency.as_str()),
            ("kr", "KRW") | ("us", "USD")
        ),
        ExecutionAssetClass::CryptoSpot => {
            request.market == "coin" && matches!(request.currency.as_str(), "KRW" | "USD" | "USDT")
        }
        ExecutionAssetClass::SecuritiesFuture => {
            request.market == "securities_futures"
                && matches!(request.currency.as_str(), "KRW" | "USD")
        }
        ExecutionAssetClass::CryptoPerpetual => {
            request.market == "crypto_futures"
                && request
                    .currency
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase())
        }
    };
    if !market_currency_valid {
        return Err("자산군과 시장·결제 통화 조합이 일치하지 않습니다.".to_owned());
    }
    match (&request.derivative, request.asset_class.is_derivative()) {
        (Some(boundary), true) => validate_derivative(&request, boundary)?,
        (None, true) => {
            return Err("파생상품 실행에는 증거금·reduce-only·청산 경계가 필요합니다.".to_owned())
        }
        (Some(_), false) => {
            return Err("현물 실행에 파생상품 경계를 사용할 수 없습니다.".to_owned())
        }
        (None, false) => {}
    }
    let children = split_children(&request)?;
    let state = InternalExecutionState {
        contract_version: CONTRACT_VERSION.to_owned(),
        execution_id: request.execution_id.clone(),
        idempotency_key: request.idempotency_key.clone(),
        asset_class: request.asset_class,
        market: request.market.clone(),
        symbol: request.symbol.clone(),
        currency: request.currency.clone(),
        side: request.side,
        total_quantity: request.total_quantity,
        quantity_scale: request.quantity_scale,
        cumulative_filled_quantity: 0,
        weighted_fill_notional: 0,
        average_fill_price_minor: None,
        reference_price_minor: request.reference_price_minor,
        current_limit_price_minor: request.initial_limit_price_minor,
        reprice_count: 0,
        expires_at_ms: request.expires_at_ms,
        status: ExecutionStatus::Working,
        children,
        derivative: request.derivative.clone(),
        live_order_allowed: false,
        updated_at_ms: now_ms,
    };
    Ok(StoredExecutionPlan { request, state })
}

fn ensure_working(state: &InternalExecutionState, now_ms: u64) -> Result<(), String> {
    if now_ms <= state.updated_at_ms {
        return Err("실행 사건 시각은 직전 상태 변경보다 뒤여야 합니다.".to_owned());
    }
    if now_ms >= state.expires_at_ms {
        return Err("실행 계획이 만료되었습니다. 만료 사건을 기록해 주세요.".to_owned());
    }
    if !matches!(
        state.status,
        ExecutionStatus::Working | ExecutionStatus::PartiallyFilled
    ) {
        return Err("종료된 실행 계획은 변경할 수 없습니다.".to_owned());
    }
    Ok(())
}

fn apply_fill(
    plan: &mut StoredExecutionPlan,
    request: &ExecutionFillRequest,
) -> Result<(), String> {
    ensure_working(&plan.state, request.occurred_at_ms)?;
    if request.fill_quantity == 0
        || request.fill_price_minor == 0
        || slippage_bps(plan.state.reference_price_minor, request.fill_price_minor)
            > plan.request.policy.maximum_slippage_bps
    {
        return Err("명시적 체결 수량·가격이 없거나 최대 슬리피지 한도를 초과했습니다.".to_owned());
    }
    let child = plan
        .state
        .children
        .iter_mut()
        .find(|child| child.filled_quantity < child.quantity)
        .ok_or_else(|| "남은 분할 주문이 없습니다.".to_owned())?;
    let child_remaining = child.quantity - child.filled_quantity;
    if request.fill_quantity > child_remaining {
        return Err("한 체결 사건이 현재 분할 주문의 잔여 수량을 초과했습니다.".to_owned());
    }
    child.filled_quantity += request.fill_quantity;
    child.status = if child.filled_quantity == child.quantity {
        ExecutionStatus::Filled
    } else {
        ExecutionStatus::PartiallyFilled
    };
    plan.state.cumulative_filled_quantity += request.fill_quantity;
    plan.state.weighted_fill_notional = plan
        .state
        .weighted_fill_notional
        .checked_add(u128::from(request.fill_quantity) * u128::from(request.fill_price_minor))
        .ok_or_else(|| "누적 체결 금액이 지원 범위를 초과했습니다.".to_owned())?;
    plan.state.average_fill_price_minor = Some(
        u64::try_from(
            plan.state.weighted_fill_notional / u128::from(plan.state.cumulative_filled_quantity),
        )
        .map_err(|_| "평균 체결가가 지원 범위를 초과했습니다.".to_owned())?,
    );
    plan.state.status = if plan.state.cumulative_filled_quantity == plan.state.total_quantity {
        ExecutionStatus::Filled
    } else {
        ExecutionStatus::PartiallyFilled
    };
    plan.state.updated_at_ms = request.occurred_at_ms;
    Ok(())
}

fn apply_reprice(
    plan: &mut StoredExecutionPlan,
    request: &ExecutionRepriceRequest,
) -> Result<(), String> {
    ensure_working(&plan.state, request.occurred_at_ms)?;
    if request.new_limit_price_minor == 0
        || request.quote_price_minor == 0
        || plan.state.reprice_count >= plan.request.policy.maximum_reprices
        || slippage_bps(
            plan.state.reference_price_minor,
            request.new_limit_price_minor,
        ) > plan.request.policy.maximum_slippage_bps
        || slippage_bps(plan.state.reference_price_minor, request.quote_price_minor)
            > plan.request.policy.maximum_slippage_bps
    {
        return Err(
            "재호가 횟수·가격 또는 최신 시세가 최대 슬리피지 계약을 벗어났습니다.".to_owned(),
        );
    }
    plan.state.current_limit_price_minor = request.new_limit_price_minor;
    plan.state.reprice_count += 1;
    plan.state.updated_at_ms = request.occurred_at_ms;
    Ok(())
}

fn load_plan(
    persistence: &PersistenceBridge,
    execution_id: &str,
) -> Result<StoredExecutionPlan, String> {
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "내부 실행 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let json: Option<String> = connection
        .query_row(
            "SELECT state_json FROM internal_execution_plans WHERE execution_id=?1",
            params![execution_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("내부 실행 계획을 조회하지 못했습니다: {error}"))?;
    let json = json.ok_or_else(|| "내부 실행 계획을 찾지 못했습니다.".to_owned())?;
    serde_json::from_str(&json)
        .map_err(|_| "저장된 내부 실행 계획을 해석하지 못했습니다.".to_owned())
}

fn save_event(
    persistence: &PersistenceBridge,
    plan: &StoredExecutionPlan,
    expected_updated_at_ms: u64,
    event_id: &str,
    event_type: &str,
    event_json: &str,
    occurred_at_ms: u64,
) -> Result<InternalExecutionState, String> {
    if !valid_id(event_id, 128) || occurred_at_ms == 0 || occurred_at_ms > MAX_SQLITE_INTEGER {
        return Err("유효한 실행 사건 ID와 시각이 필요합니다.".to_owned());
    }
    let state_json = serde_json::to_string(plan)
        .map_err(|_| "내부 실행 상태를 직렬화하지 못했습니다.".to_owned())?;
    let mut connection = persistence
        .connection
        .lock()
        .map_err(|_| "내부 실행 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("내부 실행 트랜잭션을 시작하지 못했습니다: {error}"))?;
    let event_index: u64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM internal_execution_events WHERE execution_id=?1",
            params![plan.state.execution_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("실행 사건 순번을 확인하지 못했습니다: {error}"))?;
    let updated = transaction
        .execute(
            "UPDATE internal_execution_plans SET status=?2,state_json=?3,updated_at_ms=?4
         WHERE execution_id=?1 AND updated_at_ms=?5",
            params![
                plan.state.execution_id,
                status_db(plan.state.status),
                state_json,
                occurred_at_ms,
                expected_updated_at_ms
            ],
        )
        .map_err(|error| format!("내부 실행 상태를 저장하지 못했습니다: {error}"))?;
    if updated != 1 {
        return Err(
            "내부 실행 상태가 다른 작업에서 변경되었습니다. 다시 불러온 뒤 재시도해 주세요."
                .to_owned(),
        );
    }
    transaction.execute(
        "INSERT INTO internal_execution_events(event_id,execution_id,event_index,event_type,event_json,occurred_at_ms) VALUES(?1,?2,?3,?4,?5,?6)",
        params![event_id,plan.state.execution_id,event_index,event_type,event_json,occurred_at_ms],
    ).map_err(|error| format!("내부 실행 사건을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("내부 실행 변경을 확정하지 못했습니다: {error}"))?;
    Ok(plan.state.clone())
}

fn existing_event_result(
    persistence: &PersistenceBridge,
    execution_id: &str,
    event_id: &str,
    event_json: &str,
) -> Result<Option<InternalExecutionState>, String> {
    if !valid_id(event_id, 128) {
        return Err("유효한 실행 사건 ID가 필요합니다.".to_owned());
    }
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "내부 실행 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT event_json,state_json FROM internal_execution_events AS events
             JOIN internal_execution_plans AS plans USING(execution_id)
             WHERE events.event_id=?1 AND events.execution_id=?2",
            params![event_id, execution_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| format!("중복 실행 사건을 확인하지 못했습니다: {error}"))?;
    match existing {
        None => Ok(None),
        Some((stored_event, state_json)) if stored_event == event_json => {
            let stored: StoredExecutionPlan = serde_json::from_str(&state_json)
                .map_err(|_| "저장된 내부 실행 상태를 해석하지 못했습니다.".to_owned())?;
            Ok(Some(stored.state))
        }
        Some(_) => Err("같은 사건 ID에 다른 실행 내용이 있습니다.".to_owned()),
    }
}

fn status_db(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Working => "working",
        ExecutionStatus::PartiallyFilled => "partially_filled",
        ExecutionStatus::Filled => "filled",
        ExecutionStatus::Cancelled => "cancelled",
        ExecutionStatus::Expired => "expired",
    }
}

#[tauri::command]
pub fn internal_execution_create(
    request: ExecutionPlanRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<InternalExecutionState, String> {
    create(request, &persistence, crate::persistence::now_ms()?)
}

fn create(
    request: ExecutionPlanRequest,
    persistence: &PersistenceBridge,
    now_ms: u64,
) -> Result<InternalExecutionState, String> {
    let plan = build_plan(request, now_ms)?;
    let request_json = serde_json::to_string(&plan.request)
        .map_err(|_| "실행 계획을 직렬화하지 못했습니다.".to_owned())?;
    let state_json = serde_json::to_string(&plan)
        .map_err(|_| "실행 상태를 직렬화하지 못했습니다.".to_owned())?;
    let connection = persistence
        .connection
        .lock()
        .map_err(|_| "내부 실행 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing: Option<(String, String)> = connection.query_row(
        "SELECT request_json,state_json FROM internal_execution_plans WHERE execution_id=?1 OR idempotency_key=?2 LIMIT 1",
        params![plan.state.execution_id, plan.state.idempotency_key],
        |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional().map_err(|error| format!("기존 실행 계획을 확인하지 못했습니다: {error}"))?;
    if let Some((existing_request, existing_state)) = existing {
        if existing_request == request_json {
            let existing: StoredExecutionPlan = serde_json::from_str(&existing_state)
                .map_err(|_| "기존 실행 상태를 해석하지 못했습니다.".to_owned())?;
            return Ok(existing.state);
        }
        return Err("같은 실행 ID 또는 멱등 키에 다른 계획이 있습니다.".to_owned());
    }
    connection.execute(
        "INSERT INTO internal_execution_plans(execution_id,idempotency_key,status,request_json,state_json,created_at_ms,updated_at_ms) VALUES(?1,?2,'working',?3,?4,?5,?5)",
        params![plan.state.execution_id,plan.state.idempotency_key,request_json,state_json,now_ms],
    ).map_err(|error| format!("내부 실행 계획을 저장하지 못했습니다: {error}"))?;
    Ok(plan.state)
}

#[tauri::command]
pub fn internal_execution_fill(
    request: ExecutionFillRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<InternalExecutionState, String> {
    let event_json = serde_json::to_string(&request)
        .map_err(|_| "체결 사건을 직렬화하지 못했습니다.".to_owned())?;
    if let Some(state) = existing_event_result(
        &persistence,
        &request.execution_id,
        &request.event_id,
        &event_json,
    )? {
        return Ok(state);
    }
    let mut plan = load_plan(&persistence, &request.execution_id)?;
    let expected_updated_at_ms = plan.state.updated_at_ms;
    apply_fill(&mut plan, &request)?;
    save_event(
        &persistence,
        &plan,
        expected_updated_at_ms,
        &request.event_id,
        "fill",
        &event_json,
        request.occurred_at_ms,
    )
}

#[tauri::command]
pub fn internal_execution_reprice(
    request: ExecutionRepriceRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<InternalExecutionState, String> {
    let event_json = serde_json::to_string(&request)
        .map_err(|_| "재호가 사건을 직렬화하지 못했습니다.".to_owned())?;
    if let Some(state) = existing_event_result(
        &persistence,
        &request.execution_id,
        &request.event_id,
        &event_json,
    )? {
        return Ok(state);
    }
    let mut plan = load_plan(&persistence, &request.execution_id)?;
    let expected_updated_at_ms = plan.state.updated_at_ms;
    apply_reprice(&mut plan, &request)?;
    save_event(
        &persistence,
        &plan,
        expected_updated_at_ms,
        &request.event_id,
        "reprice",
        &event_json,
        request.occurred_at_ms,
    )
}

fn close(
    request: ExecutionCloseRequest,
    persistence: &PersistenceBridge,
    expired: bool,
) -> Result<InternalExecutionState, String> {
    if request.reason.trim().is_empty() || request.reason.len() > 500 {
        return Err("취소·만료 원인이 필요합니다.".to_owned());
    }
    let event_json = serde_json::to_string(&request)
        .map_err(|_| "종료 사건을 직렬화하지 못했습니다.".to_owned())?;
    if let Some(state) = existing_event_result(
        persistence,
        &request.execution_id,
        &request.event_id,
        &event_json,
    )? {
        return Ok(state);
    }
    let mut plan = load_plan(persistence, &request.execution_id)?;
    let expected_updated_at_ms = plan.state.updated_at_ms;
    if expired {
        if request.occurred_at_ms < plan.state.expires_at_ms {
            return Err("만료 시각 전에는 실행 계획을 만료시킬 수 없습니다.".to_owned());
        }
    } else {
        ensure_working(&plan.state, request.occurred_at_ms)?;
    }
    plan.state.status = if expired {
        ExecutionStatus::Expired
    } else {
        ExecutionStatus::Cancelled
    };
    plan.state.updated_at_ms = request.occurred_at_ms;
    for child in &mut plan.state.children {
        if child.filled_quantity < child.quantity {
            child.status = plan.state.status;
        }
    }
    save_event(
        persistence,
        &plan,
        expected_updated_at_ms,
        &request.event_id,
        if expired { "expire" } else { "cancel" },
        &event_json,
        request.occurred_at_ms,
    )
}

#[tauri::command]
pub fn internal_execution_cancel(
    request: ExecutionCloseRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<InternalExecutionState, String> {
    close(request, &persistence, false)
}

#[tauri::command]
pub fn internal_execution_expire(
    request: ExecutionCloseRequest,
    persistence: State<'_, PersistenceBridge>,
) -> Result<InternalExecutionState, String> {
    close(request, &persistence, true)
}

#[tauri::command]
pub fn internal_execution_get(
    execution_id: String,
    persistence: State<'_, PersistenceBridge>,
) -> Result<InternalExecutionState, String> {
    if !valid_id(&execution_id, 128) {
        return Err("유효한 내부 실행 ID가 필요합니다.".to_owned());
    }
    Ok(load_plan(&persistence, &execution_id)?.state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(asset_class: ExecutionAssetClass) -> ExecutionPlanRequest {
        ExecutionPlanRequest {
            execution_id: "exec-1".into(),
            idempotency_key: "idem-1".into(),
            asset_class,
            market: "us".into(),
            symbol: "AAPL".into(),
            currency: "USD".into(),
            side: TradeSide::Buy,
            total_quantity: 25,
            quantity_scale: 1,
            reference_price_minor: 10_000,
            initial_limit_price_minor: 10_010,
            expires_at_ms: 10_000,
            policy: ExecutionPolicy {
                quantity_step: 1,
                maximum_child_quantity: 10,
                maximum_child_count: 3,
                maximum_reprices: 2,
                maximum_slippage_bps: 50,
                minimum_liquidation_buffer_bps: 500,
                maximum_leverage_milli: 2_000,
            },
            derivative: None,
        }
    }

    #[test]
    fn splits_reprices_and_records_partial_fills_without_inventing_volume() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let state = create(request(ExecutionAssetClass::StockSpot), &bridge, 1).expect("create");
        assert_eq!(
            state
                .children
                .iter()
                .map(|item| item.quantity)
                .collect::<Vec<_>>(),
            vec![10, 10, 5]
        );
        let mut plan = load_plan(&bridge, "exec-1").expect("plan");
        apply_reprice(
            &mut plan,
            &ExecutionRepriceRequest {
                execution_id: "exec-1".into(),
                event_id: "rp-1".into(),
                new_limit_price_minor: 10_020,
                quote_price_minor: 10_015,
                occurred_at_ms: 2,
            },
        )
        .expect("reprice");
        apply_fill(
            &mut plan,
            &ExecutionFillRequest {
                execution_id: "exec-1".into(),
                event_id: "fill-1".into(),
                fill_quantity: 4,
                fill_price_minor: 10_020,
                occurred_at_ms: 3,
            },
        )
        .expect("partial");
        assert_eq!(plan.state.status, ExecutionStatus::PartiallyFilled);
        assert_eq!(plan.state.cumulative_filled_quantity, 4);
        assert!(apply_fill(
            &mut plan,
            &ExecutionFillRequest {
                execution_id: "exec-1".into(),
                event_id: "fill-too-large".into(),
                fill_quantity: 7,
                fill_price_minor: 10_020,
                occurred_at_ms: 4
            }
        )
        .is_err());
    }

    #[test]
    fn rejects_slippage_and_changed_idempotent_requests() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        create(request(ExecutionAssetClass::StockSpot), &bridge, 1).expect("create");
        let mut changed = request(ExecutionAssetClass::StockSpot);
        changed.total_quantity = 20;
        assert!(create(changed, &bridge, 1).is_err());
        let mut plan = load_plan(&bridge, "exec-1").expect("plan");
        assert!(apply_fill(
            &mut plan,
            &ExecutionFillRequest {
                execution_id: "exec-1".into(),
                event_id: "bad-fill".into(),
                fill_quantity: 1,
                fill_price_minor: 11_000,
                occurred_at_ms: 2
            }
        )
        .is_err());
    }

    #[test]
    fn derivative_reduce_only_cannot_flip_a_position() {
        let mut reduce = request(ExecutionAssetClass::CryptoPerpetual);
        reduce.market = "crypto_futures".into();
        reduce.currency = "USDT".into();
        reduce.side = TradeSide::Sell;
        reduce.total_quantity = 5;
        reduce.policy.maximum_child_quantity = 5;
        reduce.derivative = Some(DerivativeExecutionBoundary {
            intent: ExecutionIntent::Reduce,
            reduce_only: true,
            signed_position_quantity: 5,
            leverage_milli: 1_000,
            isolated_margin: true,
            available_margin_minor: 100_000,
            initial_margin_required_minor: 0,
            maintenance_margin_minor: 10_000,
            mark_price_minor: 10_000,
            liquidation_price_minor: 9_000,
        });
        assert!(build_plan(reduce.clone(), 1).is_ok());
        reduce.total_quantity = 6;
        assert!(build_plan(reduce, 1).is_err());
    }

    #[test]
    fn derivative_open_requires_isolated_margin_and_liquidation_buffer() {
        let mut open = request(ExecutionAssetClass::SecuritiesFuture);
        open.market = "securities_futures".into();
        open.derivative = Some(DerivativeExecutionBoundary {
            intent: ExecutionIntent::Open,
            reduce_only: false,
            signed_position_quantity: 0,
            leverage_milli: 2_000,
            isolated_margin: true,
            available_margin_minor: 100_000,
            initial_margin_required_minor: 50_000,
            maintenance_margin_minor: 20_000,
            mark_price_minor: 10_000,
            liquidation_price_minor: 9_000,
        });
        assert!(build_plan(open.clone(), 1).is_ok());
        open.derivative.as_mut().expect("boundary").isolated_margin = false;
        assert!(build_plan(open, 1).is_err());
    }

    #[test]
    fn expiry_preserves_partial_fill_and_closes_the_remainder() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        create(request(ExecutionAssetClass::StockSpot), &bridge, 1).expect("create");
        let partial = ExecutionFillRequest {
            execution_id: "exec-1".into(),
            event_id: "fill-1".into(),
            fill_quantity: 4,
            fill_price_minor: 10_010,
            occurred_at_ms: 2,
        };
        let mut plan = load_plan(&bridge, "exec-1").expect("plan");
        apply_fill(&mut plan, &partial).expect("fill");
        save_event(
            &bridge,
            &plan,
            1,
            "fill-1",
            "fill",
            &serde_json::to_string(&partial).unwrap(),
            2,
        )
        .expect("save");
        let expired = close(
            ExecutionCloseRequest {
                execution_id: "exec-1".into(),
                event_id: "expire-1".into(),
                reason: "time_in_force".into(),
                occurred_at_ms: 10_000,
            },
            &bridge,
            true,
        )
        .expect("expire");
        assert_eq!(expired.status, ExecutionStatus::Expired);
        assert_eq!(expired.cumulative_filled_quantity, 4);
        let repeated = close(
            ExecutionCloseRequest {
                execution_id: "exec-1".into(),
                event_id: "expire-1".into(),
                reason: "time_in_force".into(),
                occurred_at_ms: 10_000,
            },
            &bridge,
            true,
        )
        .expect("idempotent expiry");
        assert_eq!(repeated, expired);
    }
}
