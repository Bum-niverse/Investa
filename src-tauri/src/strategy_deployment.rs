use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use uuid::Uuid;

use crate::{
    experiments::WalkForwardReport,
    persistence::{now_ms, PersistenceBridge},
    research::{review_strategy_spec, StrategySpec},
    strategy_plugins,
};

const DEPLOYMENT_CONTRACT_VERSION: &str = "strategy-deployment-v1";
const CANARY_APPROVAL: &str = "SHADOW CANARY 배치 승인";
const PAPER_APPROVAL: &str = "내부 모의운용 승격 승인";
const ROLLBACK_APPROVAL: &str = "이전 전략 버전 롤백 승인";
const MAX_CANARY_DRAWDOWN_BPS: u64 = 1_200;
const MAX_CANARY_SLIPPAGE_BPS: u64 = 100;
const MIN_CANARY_OBSERVATIONS: u64 = 20;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryPolicy {
    pub minimum_observation_count: u64,
    pub maximum_drawdown_bps: u64,
    pub maximum_average_slippage_bps: u64,
    pub maximum_error_count: u64,
    pub minimum_net_pnl_minor: i64,
}

impl Default for CanaryPolicy {
    fn default() -> Self {
        Self {
            minimum_observation_count: MIN_CANARY_OBSERVATIONS,
            maximum_drawdown_bps: 600,
            maximum_average_slippage_bps: 30,
            maximum_error_count: 0,
            minimum_net_pnl_minor: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CostStressResult {
    pub multiplier_bps: u64,
    pub aggregate_return_bps: i64,
    pub passed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentEvidence {
    pub contract_version: String,
    pub experiment_id: String,
    pub validation_run_id: String,
    pub strategy_id: String,
    pub strategy_schema_version: String,
    pub plugin_id: String,
    pub plugin_version: u32,
    pub dataset_id: String,
    pub code_version: String,
    pub provider: String,
    pub interval: String,
    pub promotion_policy_version: String,
    pub oos_trade_count: usize,
    pub original_cost_bps_milli: u64,
    pub cost_stress: Vec<CostStressResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyDeployment {
    pub deployment_id: String,
    pub slot_key: String,
    pub experiment_id: String,
    pub validation_run_id: String,
    pub strategy_id: String,
    pub strategy_schema_version: String,
    pub plugin_id: String,
    pub plugin_version: u32,
    pub dataset_id: String,
    pub evidence_sha256: String,
    pub status: String,
    pub revision: u64,
    pub canary_policy: CanaryPolicy,
    pub evidence: DeploymentEvidence,
    pub previous_deployment_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub live_order_allowed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCandidateRequest {
    pub idempotency_key: String,
    pub experiment_id: String,
    pub validation_run_id: String,
    #[serde(default)]
    pub canary_policy: CanaryPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub deployment_id: String,
    pub expected_revision: u64,
    pub approval_text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryObservation {
    pub observation_id: String,
    pub observed_at_ms: u64,
    pub sample_count: u64,
    pub net_pnl_minor: i64,
    pub maximum_drawdown_bps: u64,
    pub average_slippage_bps: u64,
    pub error_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObserveCanaryRequest {
    pub deployment_id: String,
    pub expected_revision: u64,
    pub observation: CanaryObservation,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackRequest {
    pub deployment_id: String,
    pub target_deployment_id: String,
    pub expected_revision: u64,
    pub target_expected_revision: u64,
    pub approval_text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectRequest {
    pub deployment_id: String,
    pub expected_revision: u64,
    pub reason: String,
}

struct StoredSource {
    dataset_id: String,
    symbol: String,
    provider: String,
    interval: String,
    record: Value,
    report: WalkForwardReport,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_policy(policy: &CanaryPolicy) -> Result<(), String> {
    if policy.minimum_observation_count < MIN_CANARY_OBSERVATIONS
        || policy.minimum_observation_count > 100_000
        || policy.maximum_drawdown_bps == 0
        || policy.maximum_drawdown_bps > MAX_CANARY_DRAWDOWN_BPS
        || policy.maximum_average_slippage_bps == 0
        || policy.maximum_average_slippage_bps > MAX_CANARY_SLIPPAGE_BPS
        || policy.maximum_error_count > 10
        || policy.minimum_net_pnl_minor < 0
    {
        return Err("Canary 정책은 최소 20개 관측, 낙폭 1,200bp 이하, 평균 슬리피지 100bp 이하, 비음수 최소손익 범위여야 합니다.".to_owned());
    }
    Ok(())
}

fn as_bps_milli(value: &Value, pointer: &str) -> Result<u64, String> {
    let number = value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("저장 실험의 {pointer} 비용을 확인하지 못했습니다."))?;
    if !number.is_finite() || !(0.0..=10_000.0).contains(&number) {
        return Err("저장 실험 비용이 허용 범위를 벗어났습니다.".to_owned());
    }
    Ok((number * 1_000.0).round() as u64)
}

fn build_cost_stress(
    record: &Value,
    report: &WalkForwardReport,
) -> Result<(u64, Vec<CostStressResult>), String> {
    let buy = as_bps_milli(record, "/config/costs/buyFeeBps")?;
    let sell = as_bps_milli(record, "/config/costs/sellFeeBps")?;
    let tax = as_bps_milli(record, "/config/costs/sellTaxBps")?;
    let slippage = as_bps_milli(record, "/config/costs/slippageBps")?;
    // turnover는 매수·매도 체결 명목가를 각각 합산하므로 왕복 비용의 합이 아니라
    // 더 비싼 한쪽 체결 비용을 보수적으로 모든 turnover에 적용한다.
    let original = buy
        .checked_add(slippage)
        .zip(
            sell.checked_add(tax)
                .and_then(|value| value.checked_add(slippage)),
        )
        .map(|(buy_side, sell_side)| buy_side.max(sell_side))
        .ok_or_else(|| "거래 비용 합계가 범위를 초과했습니다.".to_owned())?;
    if original == 0 {
        return Err("비용이 0인 검증은 배치 근거로 사용할 수 없습니다.".to_owned());
    }
    let mut scenarios = Vec::new();
    for multiplier_bps in [15_000_u64, 20_000_u64] {
        let extra_cost_milli = u128::from(original)
            .saturating_mul(u128::from(multiplier_bps.saturating_sub(10_000)))
            / 10_000;
        let stressed_sum = report.folds.iter().try_fold(0_i128, |sum, fold| {
            let penalty_bps = u128::from(fold.out_of_sample.turnover_bps)
                .saturating_mul(extra_cost_milli)
                / 10_000
                / 1_000;
            let penalty = i128::try_from(penalty_bps)
                .map_err(|_| "비용 스트레스 계산이 범위를 초과했습니다.".to_owned())?;
            Ok::<_, String>(sum.saturating_add(
                i128::from(fold.out_of_sample.total_return_bps).saturating_sub(penalty),
            ))
        })?;
        let aggregate = stressed_sum
            .checked_div(report.folds.len() as i128)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or_else(|| "비용 스트레스 평균을 계산하지 못했습니다.".to_owned())?;
        scenarios.push(CostStressResult {
            multiplier_bps,
            aggregate_return_bps: aggregate,
            passed: aggregate > 0,
        });
    }
    Ok((original, scenarios))
}

fn load_source(
    bridge: &PersistenceBridge,
    request: &CreateCandidateRequest,
) -> Result<StoredSource, String> {
    if !valid_id(&request.idempotency_key)
        || !valid_id(&request.experiment_id)
        || !valid_id(&request.validation_run_id)
    {
        return Err("멱등 키·실험 ID·Walk-forward ID 형식을 확인해 주세요.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 근거 조회 잠금을 획득하지 못했습니다.".to_owned())?;
    let row = connection
        .query_row(
            "SELECT b.dataset_id,b.symbol,b.provider,b.interval,b.record_json,w.report_json
             FROM backtest_runs b JOIN walk_forward_runs w ON w.source_experiment_id=b.experiment_id
             WHERE b.experiment_id=?1 AND w.validation_run_id=?2",
            params![request.experiment_id, request.validation_run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| format!("전략 배치 근거를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| {
            "서로 연결된 저장 백테스트와 Walk-forward 결과를 찾지 못했습니다.".to_owned()
        })?;
    Ok(StoredSource {
        dataset_id: row.0,
        symbol: row.1,
        provider: row.2,
        interval: row.3,
        record: serde_json::from_str(&row.4)
            .map_err(|error| format!("백테스트 기록을 해석하지 못했습니다: {error}"))?,
        report: serde_json::from_str(&row.5)
            .map_err(|error| format!("Walk-forward 기록을 해석하지 못했습니다: {error}"))?,
    })
}

fn evidence(
    source: &StoredSource,
    experiment_id: &str,
) -> Result<(String, DeploymentEvidence), String> {
    if source.report.source_experiment_id != experiment_id
        || source.report.folds.is_empty()
        || !source.report.promotion_evaluation.eligible_for_paper_review
        || !source.report.promotion_blockers.is_empty()
    {
        return Err(
            "OOS 승격 게이트를 모두 통과한 Walk-forward 결과만 배치할 수 있습니다.".to_owned(),
        );
    }
    let strategy: StrategySpec = serde_json::from_value(
        source
            .record
            .pointer("/report/strategyCandidate")
            .cloned()
            .ok_or_else(|| "저장 실험에 전략 계약이 없습니다.".to_owned())?,
    )
    .map_err(|error| format!("전략 계약을 해석하지 못했습니다: {error}"))?;
    if !review_strategy_spec(&strategy).executable {
        return Err("실행 가능 검토를 통과한 전략 계약만 배치할 수 있습니다.".to_owned());
    }
    let interval_seconds = match source.interval.as_str() {
        "1m" => 60,
        "3m" => 180,
        "5m" => 300,
        "15m" => 900,
        "30m" => 1_800,
        "1h" | "60m" => 3_600,
        "4h" | "240m" => 14_400,
        "1d" => 86_400,
        _ => return Err("배치할 전략의 봉 주기가 지원 목록에 없습니다.".to_owned()),
    };
    strategy_plugins::strategy_plugin_validate(
        strategy_plugins::StrategyPluginValidationRequest {
            strategy: strategy.clone(),
            interval_seconds,
            available_bar_fields: vec![
                "open".to_owned(),
                "high".to_owned(),
                "low".to_owned(),
                "close".to_owned(),
                "volume".to_owned(),
                "availableAt".to_owned(),
                "ingestedAt".to_owned(),
            ],
        },
    )?;
    let plugin = strategy_plugins::descriptor(&strategy.entry_signal);
    let exit_plugin = strategy_plugins::descriptor(&strategy.exit_signal);
    if plugin.plugin_id != exit_plugin.plugin_id || plugin.version != exit_plugin.version {
        return Err("진입·청산 전략 플러그인 버전이 일치하지 않습니다.".to_owned());
    }
    let (original_cost_bps_milli, cost_stress) = build_cost_stress(&source.record, &source.report)?;
    if cost_stress.iter().any(|scenario| !scenario.passed) {
        return Err(
            "1.5배·2배 거래 비용 스트레스에서 모두 양의 OOS 수익을 유지해야 합니다.".to_owned(),
        );
    }
    let code_version = source
        .record
        .pointer("/config/codeVersion")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let evidence = DeploymentEvidence {
        contract_version: DEPLOYMENT_CONTRACT_VERSION.to_owned(),
        experiment_id: experiment_id.to_owned(),
        validation_run_id: source.report.validation_run_id.clone(),
        strategy_id: strategy.strategy_id,
        strategy_schema_version: strategy.schema_version,
        plugin_id: plugin.plugin_id.to_owned(),
        plugin_version: plugin.version,
        dataset_id: source.dataset_id.clone(),
        code_version,
        provider: source.provider.clone(),
        interval: source.interval.clone(),
        promotion_policy_version: source.report.promotion_evaluation.policy_version.clone(),
        oos_trade_count: source.report.total_oos_trade_count,
        original_cost_bps_milli,
        cost_stress,
    };
    let json = serde_json::to_vec(&evidence)
        .map_err(|error| format!("배치 근거를 직렬화하지 못했습니다: {error}"))?;
    Ok((format!("{:x}", Sha256::digest(json)), evidence))
}

fn append_event(
    transaction: &Transaction<'_>,
    deployment_id: &str,
    event_type: &str,
    payload: &Value,
    occurred_at_ms: u64,
) -> Result<(), String> {
    let next_index: u64 = transaction
        .query_row("SELECT COALESCE(MAX(event_index)+1,0) FROM strategy_deployment_events WHERE deployment_id=?1", params![deployment_id], |row| row.get(0))
        .map_err(|error| format!("배치 사건 순번을 조회하지 못했습니다: {error}"))?;
    let event_id = format!("deployment-event-{}", Uuid::new_v4().simple());
    transaction.execute(
        "INSERT INTO strategy_deployment_events(event_id,deployment_id,event_index,event_type,event_json,occurred_at_ms) VALUES(?1,?2,?3,?4,?5,?6)",
        params![event_id,deployment_id,next_index,event_type,payload.to_string(),occurred_at_ms],
    ).map_err(|error| format!("전략 배치 사건을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

fn decode_deployment(row: &rusqlite::Row<'_>) -> rusqlite::Result<StrategyDeployment> {
    let policy_json: String = row.get(12)?;
    let evidence_json: String = row.get(13)?;
    Ok(StrategyDeployment {
        deployment_id: row.get(0)?,
        slot_key: row.get(1)?,
        experiment_id: row.get(2)?,
        validation_run_id: row.get(3)?,
        strategy_id: row.get(4)?,
        strategy_schema_version: row.get(5)?,
        plugin_id: row.get(6)?,
        plugin_version: row.get(7)?,
        dataset_id: row.get(8)?,
        evidence_sha256: row.get(9)?,
        status: row.get(10)?,
        revision: row.get(11)?,
        canary_policy: serde_json::from_str(&policy_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        evidence: serde_json::from_str(&evidence_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        previous_deployment_id: row.get(14)?,
        created_at_ms: row.get(15)?,
        updated_at_ms: row.get(16)?,
        live_order_allowed: false,
    })
}

const DEPLOYMENT_SELECT: &str = "SELECT deployment_id,slot_key,experiment_id,validation_run_id,strategy_id,strategy_schema_version,plugin_id,plugin_version,dataset_id,evidence_sha256,status,revision,canary_policy_json,evidence_json,previous_deployment_id,created_at_ms,updated_at_ms FROM strategy_deployments";

fn load_deployment(
    transaction: &Transaction<'_>,
    deployment_id: &str,
) -> Result<StrategyDeployment, String> {
    transaction
        .query_row(
            &format!("{DEPLOYMENT_SELECT} WHERE deployment_id=?1"),
            params![deployment_id],
            decode_deployment,
        )
        .optional()
        .map_err(|error| format!("전략 배치를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "전략 배치를 찾지 못했습니다.".to_owned())
}

fn require_revision(deployment: &StrategyDeployment, expected: u64) -> Result<(), String> {
    if deployment.revision != expected {
        return Err(
            "전략 배치가 다른 작업에서 변경되었습니다. 최신 상태를 다시 확인해 주세요.".to_owned(),
        );
    }
    Ok(())
}

fn arm_watch(
    transaction: &Transaction<'_>,
    deployment: &StrategyDeployment,
    now: u64,
) -> Result<(), String> {
    let watch_id = format!("watch-{}", deployment.experiment_id);
    transaction.execute("INSERT INTO shadow_watches(watch_id,experiment_id,enabled,interval_seconds,status,created_at_ms,updated_at_ms) VALUES(?1,?2,1,60,'watching',?3,?3) ON CONFLICT(watch_id) DO UPDATE SET enabled=1,status='watching',last_error=NULL,updated_at_ms=excluded.updated_at_ms",params![watch_id,deployment.experiment_id,now])
        .map_err(|error| format!("Canary 섀도우 감시를 시작하지 못했습니다: {error}"))?;
    Ok(())
}

fn stop_watch(transaction: &Transaction<'_>, experiment_id: &str, now: u64) -> Result<(), String> {
    transaction.execute("UPDATE shadow_watches SET enabled=0,status='stopped',updated_at_ms=?2 WHERE experiment_id=?1",params![experiment_id,now])
        .map_err(|error| format!("섀도우 감시를 중지하지 못했습니다: {error}"))?;
    Ok(())
}

pub fn create_candidate(
    bridge: &PersistenceBridge,
    request: CreateCandidateRequest,
) -> Result<StrategyDeployment, String> {
    validate_policy(&request.canary_policy)?;
    let source = load_source(bridge, &request)?;
    let (evidence_hash, evidence) = evidence(&source, &request.experiment_id)?;
    let slot_key = format!(
        "{}:{}:{}",
        source.symbol, evidence.plugin_id, source.interval
    );
    let now = now_ms()?;
    let deployment_id = format!(
        "strategy-deployment-{}",
        &Uuid::new_v4().simple().to_string()[..16]
    );
    let policy_json = serde_json::to_string(&request.canary_policy)
        .map_err(|error| format!("Canary 정책을 직렬화하지 못했습니다: {error}"))?;
    let evidence_json = serde_json::to_string(&evidence)
        .map_err(|error| format!("배치 근거를 직렬화하지 못했습니다: {error}"))?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("전략 배치 트랜잭션을 시작하지 못했습니다: {error}"))?;
    if let Some(existing_id) = transaction
        .query_row(
            "SELECT deployment_id FROM strategy_deployments WHERE idempotency_key=?1",
            params![request.idempotency_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("전략 배치 멱등 기록을 확인하지 못했습니다: {error}"))?
    {
        let existing = load_deployment(&transaction, &existing_id)?;
        if existing.experiment_id != request.experiment_id
            || existing.validation_run_id != request.validation_run_id
            || existing.evidence_sha256 != evidence_hash
        {
            return Err("같은 멱등 키가 다른 배치 근거에 사용되었습니다.".to_owned());
        }
        return Ok(existing);
    }
    transaction.execute("INSERT INTO strategy_deployments(deployment_id,idempotency_key,slot_key,experiment_id,validation_run_id,strategy_id,strategy_schema_version,plugin_id,plugin_version,dataset_id,evidence_sha256,status,revision,canary_policy_json,evidence_json,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'awaiting_approval',1,?12,?13,?14,?14)",params![deployment_id,request.idempotency_key,slot_key,request.experiment_id,request.validation_run_id,evidence.strategy_id,evidence.strategy_schema_version,evidence.plugin_id,evidence.plugin_version,evidence.dataset_id,evidence_hash,policy_json,evidence_json,now]).map_err(|error|format!("전략 배치 후보를 저장하지 못했습니다: {error}"))?;
    append_event(
        &transaction,
        &deployment_id,
        "candidate_created",
        &serde_json::json!({"evidenceSha256":evidence_hash,"liveOrderAllowed":false}),
        now,
    )?;
    let result = load_deployment(&transaction, &deployment_id)?;
    transaction
        .commit()
        .map_err(|error| format!("전략 배치 후보를 확정하지 못했습니다: {error}"))?;
    Ok(result)
}

fn approve_canary(
    bridge: &PersistenceBridge,
    request: ApprovalRequest,
) -> Result<StrategyDeployment, String> {
    if request.approval_text.trim() != CANARY_APPROVAL {
        return Err(format!(
            "Canary 배치를 시작하려면 '{CANARY_APPROVAL}' 문구가 필요합니다."
        ));
    }
    transition(
        bridge,
        &request.deployment_id,
        request.expected_revision,
        "awaiting_approval",
        "canary",
        "canary_approved",
        true,
    )
}

fn transition(
    bridge: &PersistenceBridge,
    deployment_id: &str,
    expected_revision: u64,
    from: &str,
    to: &str,
    event: &str,
    watch_enabled: bool,
) -> Result<StrategyDeployment, String> {
    let now = now_ms()?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("전략 배치 트랜잭션을 시작하지 못했습니다: {error}"))?;
    let current = load_deployment(&transaction, deployment_id)?;
    require_revision(&current, expected_revision)?;
    if current.status != from {
        return Err(format!("{} 상태에서만 이 전환을 실행할 수 있습니다.", from));
    }
    if watch_enabled {
        arm_watch(&transaction, &current, now)?;
    } else {
        stop_watch(&transaction, &current.experiment_id, now)?;
    }
    let changed=transaction.execute("UPDATE strategy_deployments SET status=?2,revision=revision+1,updated_at_ms=?3 WHERE deployment_id=?1 AND revision=?4",params![deployment_id,to,now,expected_revision]).map_err(|error|format!("전략 배치 상태를 변경하지 못했습니다: {error}"))?;
    if changed != 1 {
        return Err("전략 배치가 동시에 변경되었습니다.".to_owned());
    }
    append_event(
        &transaction,
        deployment_id,
        event,
        &serde_json::json!({"from":from,"to":to,"liveOrderAllowed":false}),
        now,
    )?;
    let result = load_deployment(&transaction, deployment_id)?;
    transaction
        .commit()
        .map_err(|error| format!("전략 배치 상태를 확정하지 못했습니다: {error}"))?;
    Ok(result)
}

fn observe_canary(
    bridge: &PersistenceBridge,
    request: ObserveCanaryRequest,
) -> Result<StrategyDeployment, String> {
    if !valid_id(&request.observation.observation_id) || request.observation.observed_at_ms == 0 {
        return Err("Canary 관측 ID와 시각을 확인해 주세요.".to_owned());
    }
    if request.observation.sample_count > 1_000_000_000
        || request.observation.maximum_drawdown_bps > 10_000
        || request.observation.average_slippage_bps > 10_000
        || request.observation.error_count > 1_000_000
    {
        return Err("Canary 관측 수치가 허용 범위를 벗어났습니다.".to_owned());
    }
    let now = now_ms()?;
    if request.observation.observed_at_ms > now.saturating_add(60_000) {
        return Err("Canary 관측 시각이 미래입니다.".to_owned());
    }
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("Canary 관측 트랜잭션을 시작하지 못했습니다: {error}"))?;
    let current = load_deployment(&transaction, &request.deployment_id)?;
    require_revision(&current, request.expected_revision)?;
    if !matches!(current.status.as_str(), "canary" | "paper_active") {
        return Err("실행 중인 Canary 또는 내부 모의운용 배치만 관측할 수 있습니다.".to_owned());
    }
    if request.observation.observed_at_ms < current.created_at_ms {
        return Err("Canary 관측 시각은 배치 생성 시각보다 이를 수 없습니다.".to_owned());
    }
    let policy = &current.canary_policy;
    let hard_failure = request.observation.error_count > policy.maximum_error_count
        || request.observation.maximum_drawdown_bps > policy.maximum_drawdown_bps
        || request.observation.average_slippage_bps > policy.maximum_average_slippage_bps;
    let enough = request.observation.sample_count >= policy.minimum_observation_count;
    let performance_failure =
        enough && request.observation.net_pnl_minor < policy.minimum_net_pnl_minor;
    let (status, event) = if hard_failure || performance_failure {
        ("stopped", "auto_stopped")
    } else if enough && current.status == "canary" {
        ("canary_passed", "canary_passed")
    } else if current.status == "paper_active" {
        ("paper_active", "performance_observed")
    } else {
        ("canary", "canary_observed")
    };
    if status != "canary" {
        stop_watch(&transaction, &current.experiment_id, now)?;
    }
    let payload = serde_json::to_value(&request.observation)
        .map_err(|error| format!("Canary 관측을 직렬화하지 못했습니다: {error}"))?;
    append_event(
        &transaction,
        &current.deployment_id,
        event,
        &payload,
        request.observation.observed_at_ms,
    )?;
    transaction.execute("UPDATE strategy_deployments SET status=?2,revision=revision+1,updated_at_ms=?3 WHERE deployment_id=?1 AND revision=?4",params![current.deployment_id,status,now,request.expected_revision]).map_err(|error|format!("Canary 관측 상태를 저장하지 못했습니다: {error}"))?;
    let result = load_deployment(&transaction, &current.deployment_id)?;
    transaction
        .commit()
        .map_err(|error| format!("Canary 관측을 확정하지 못했습니다: {error}"))?;
    Ok(result)
}

fn approve_paper(
    bridge: &PersistenceBridge,
    request: ApprovalRequest,
) -> Result<StrategyDeployment, String> {
    if request.approval_text.trim() != PAPER_APPROVAL {
        return Err(format!(
            "내부 모의운용으로 승격하려면 '{PAPER_APPROVAL}' 문구가 필요합니다."
        ));
    }
    let now = now_ms()?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("내부 모의운용 승격 트랜잭션을 시작하지 못했습니다: {error}"))?;
    let current = load_deployment(&transaction, &request.deployment_id)?;
    require_revision(&current, request.expected_revision)?;
    if current.status != "canary_passed" {
        return Err("Canary 검증을 통과한 배치만 내부 모의운용으로 승격할 수 있습니다.".to_owned());
    }
    let previous: Option<String>=transaction.query_row("SELECT deployment_id FROM strategy_deployments WHERE slot_key=?1 AND status='paper_active'",params![current.slot_key],|row|row.get(0)).optional().map_err(|error|format!("기존 활성 전략을 조회하지 못했습니다: {error}"))?;
    if let Some(previous_id) = &previous {
        transaction.execute("UPDATE strategy_deployments SET status='superseded',revision=revision+1,updated_at_ms=?2 WHERE deployment_id=?1",params![previous_id,now]).map_err(|error|format!("기존 전략을 보존 상태로 전환하지 못했습니다: {error}"))?;
        append_event(
            &transaction,
            previous_id,
            "superseded",
            &serde_json::json!({"replacementDeploymentId":current.deployment_id}),
            now,
        )?;
    }
    transaction.execute("UPDATE strategy_deployments SET status='paper_active',previous_deployment_id=?2,revision=revision+1,updated_at_ms=?3 WHERE deployment_id=?1 AND revision=?4",params![current.deployment_id,previous,now,request.expected_revision]).map_err(|error|format!("내부 모의운용 승격을 저장하지 못했습니다: {error}"))?;
    arm_watch(&transaction, &current, now)?;
    append_event(
        &transaction,
        &current.deployment_id,
        "paper_approved",
        &serde_json::json!({"previousDeploymentId":previous,"liveOrderAllowed":false}),
        now,
    )?;
    let result = load_deployment(&transaction, &current.deployment_id)?;
    transaction
        .commit()
        .map_err(|error| format!("내부 모의운용 승격을 확정하지 못했습니다: {error}"))?;
    Ok(result)
}

fn rollback(
    bridge: &PersistenceBridge,
    request: RollbackRequest,
) -> Result<StrategyDeployment, String> {
    if request.approval_text.trim() != ROLLBACK_APPROVAL {
        return Err(format!(
            "롤백하려면 '{ROLLBACK_APPROVAL}' 문구가 필요합니다."
        ));
    }
    let now = now_ms()?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("전략 롤백 트랜잭션을 시작하지 못했습니다: {error}"))?;
    let current = load_deployment(&transaction, &request.deployment_id)?;
    let target = load_deployment(&transaction, &request.target_deployment_id)?;
    require_revision(&current, request.expected_revision)?;
    require_revision(&target, request.target_expected_revision)?;
    if !matches!(current.status.as_str(), "paper_active" | "stopped")
        || target.status != "superseded"
        || current.slot_key != target.slot_key
        || current.previous_deployment_id.as_deref() != Some(target.deployment_id.as_str())
    {
        return Err("같은 전략 슬롯의 직전 보존 버전으로만 롤백할 수 있습니다.".to_owned());
    }
    stop_watch(&transaction, &current.experiment_id, now)?;
    transaction.execute("UPDATE strategy_deployments SET status='rolled_back',revision=revision+1,updated_at_ms=?2 WHERE deployment_id=?1 AND revision=?3",params![current.deployment_id,now,request.expected_revision]).map_err(|error|format!("현재 전략 롤백 상태를 저장하지 못했습니다: {error}"))?;
    transaction.execute("UPDATE strategy_deployments SET status='paper_active',revision=revision+1,updated_at_ms=?2 WHERE deployment_id=?1 AND revision=?3",params![target.deployment_id,now,request.target_expected_revision]).map_err(|error|format!("이전 전략을 복구하지 못했습니다: {error}"))?;
    arm_watch(&transaction, &target, now)?;
    append_event(
        &transaction,
        &current.deployment_id,
        "rollback_approved",
        &serde_json::json!({"targetDeploymentId":target.deployment_id,"liveOrderAllowed":false}),
        now,
    )?;
    append_event(
        &transaction,
        &target.deployment_id,
        "rollback_approved",
        &serde_json::json!({"replacedDeploymentId":current.deployment_id,"liveOrderAllowed":false}),
        now,
    )?;
    let result = load_deployment(&transaction, &target.deployment_id)?;
    transaction
        .commit()
        .map_err(|error| format!("전략 롤백을 확정하지 못했습니다: {error}"))?;
    Ok(result)
}

#[tauri::command]
pub fn strategy_deployment_candidate_create(
    request: CreateCandidateRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StrategyDeployment, String> {
    create_candidate(&bridge, request)
}
#[tauri::command]
pub fn strategy_deployment_canary_approve(
    request: ApprovalRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StrategyDeployment, String> {
    approve_canary(&bridge, request)
}
#[tauri::command]
pub fn strategy_deployment_canary_observe(
    request: ObserveCanaryRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StrategyDeployment, String> {
    observe_canary(&bridge, request)
}
#[tauri::command]
pub fn strategy_deployment_paper_approve(
    request: ApprovalRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StrategyDeployment, String> {
    approve_paper(&bridge, request)
}
#[tauri::command]
pub fn strategy_deployment_rollback(
    request: RollbackRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StrategyDeployment, String> {
    rollback(&bridge, request)
}
#[tauri::command]
pub fn strategy_deployment_reject(
    request: RejectRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StrategyDeployment, String> {
    if request.reason.trim().len() < 3 || request.reason.len() > 500 {
        return Err("기각 사유를 3~500자로 입력해 주세요.".to_owned());
    }
    transition(
        &bridge,
        &request.deployment_id,
        request.expected_revision,
        "awaiting_approval",
        "rejected",
        "rejected",
        false,
    )
}
#[tauri::command]
pub fn strategy_deployment_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<StrategyDeployment>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "전략 배치 이력 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare(&format!(
            "{DEPLOYMENT_SELECT} ORDER BY updated_at_ms DESC,deployment_id DESC LIMIT ?1"
        ))
        .map_err(|error| format!("전략 배치 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit.clamp(1, 200)], decode_deployment)
        .map_err(|error| format!("전략 배치 이력을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| row.map_err(|error| format!("전략 배치 이력을 읽지 못했습니다: {error}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::research::{CrossDirection, Market, SignalSpec};

    fn seed_source(bridge: &PersistenceBridge, suffix: &str) -> (String, String) {
        let experiment_id = format!("experiment-{suffix}");
        let validation_run_id = format!("walk-forward-{suffix}");
        let trace_id = format!("trace-{suffix}");
        let dataset_id = format!("dataset-{suffix}");
        let strategy = StrategySpec {
            schema_version: "1".to_owned(),
            strategy_id: format!("strategy-{suffix}"),
            name: "배치 수명주기 테스트 전략".to_owned(),
            market: Market::Korea,
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            hypothesis: "완료 봉 이동평균 교차 신호를 다음 봉에서 체결한다.".to_owned(),
            source_evidence_ids: vec!["repo-1".to_owned()],
            entry_signal: SignalSpec::MovingAverageCross {
                fast_window: 2,
                slow_window: 3,
                direction: CrossDirection::Above,
            },
            exit_signal: SignalSpec::MovingAverageCross {
                fast_window: 2,
                slow_window: 3,
                direction: CrossDirection::Below,
            },
            limitations: vec!["고정 테스트 데이터만 사용한다.".to_owned()],
            unknowns: vec![],
        };
        let metrics = crate::experiments::WalkForwardMetrics {
            total_return_bps: 300,
            max_drawdown_bps: 50,
            completed_trade_count: 100,
            win_rate_bps: Some(6_000),
            profit_factor_milli: Some(1_500),
            expected_trade_pnl_minor: Some(1),
            realized_pnl_minor: 100,
            gross_profit_minor: 200,
            gross_loss_minor: 100,
            alpha_vs_price_benchmark_bps: Some(100),
            periods_per_year: Some(252),
            period_returns_ppm: vec![],
            turnover_bps: 20_000,
            exposure_bps: 5_000,
        };
        let report = WalkForwardReport {
            validation_run_id: validation_run_id.clone(),
            created_at_ms: 1,
            source_experiment_id: experiment_id.clone(),
            strategy_trial_count: 1,
            fold_count: 2,
            initial_training_bar_count: 10,
            positive_oos_fold_count: 2,
            largest_absolute_oos_return_share_bps: 5_000,
            oos_return_spread_bps: 0,
            total_oos_trade_count: 200,
            minimum_oos_trade_count: 200,
            meets_research_sample_minimum: true,
            promotion_blockers: vec![],
            promotion_evaluation: crate::experiments::PromotionEvaluation {
                policy_version: "paper-review-v1".to_owned(),
                eligible_for_paper_review: true,
                checks: vec![],
                warning: String::new(),
            },
            overfit_diagnostics: Default::default(),
            folds: (1..=2)
                .map(|fold_number| crate::experiments::WalkForwardFold {
                    fold_number,
                    training_bar_count: 10,
                    oos_bar_count: 10,
                    training_end_ms: 10,
                    oos_start_ms: 11,
                    oos_end_ms: 20,
                    training: metrics.clone(),
                    out_of_sample: metrics.clone(),
                    regimes: vec![],
                    unclassified_trade_count: 0,
                    state_model: Default::default(),
                })
                .collect(),
            warnings: vec![],
        };
        let record = serde_json::json!({
            "report":{"strategyCandidate":strategy},
            "config":{"codeVersion":"test-v1","costs":{"buyFeeBps":10.0,"sellFeeBps":10.0,"sellTaxBps":0.0,"slippageBps":5.0}}
        });
        let connection = bridge.connection.lock().expect("database lock");
        connection.execute("INSERT INTO research_reports(trace_id,strategy_id,symbol,currency,report_json,review_json,created_at_ms) VALUES(?1,?2,'005930','KRW','{}','{}',1)",params![trace_id,format!("strategy-{suffix}")]).expect("research report");
        connection.execute("INSERT INTO datasets(dataset_id,provider,symbol,currency,interval,adjusted,bar_count,first_period_start_ms,last_available_at_ms,ingested_at_ms,bars_json,created_at_ms) VALUES(?1,'fixture','005930','KRW','1d',1,2,1,2,2,'{}',1)",params![dataset_id]).expect("dataset");
        connection.execute("INSERT INTO backtest_runs(experiment_id,trace_id,dataset_id,strategy_id,strategy_name,symbol,currency,provider,interval,adjusted,bar_count,total_return_bps,max_drawdown_bps,win_rate_bps,completed_trade_count,classification,record_json,created_at_ms) VALUES(?1,?2,?3,?4,'fixture','005930','KRW','fixture','1d',1,2,300,50,6000,200,'promotion_candidate',?5,1)",params![experiment_id,trace_id,dataset_id,format!("strategy-{suffix}"),record.to_string()]).expect("backtest");
        connection.execute("INSERT INTO walk_forward_runs(validation_run_id,source_experiment_id,fold_count,strategy_trial_count,report_json,created_at_ms) VALUES(?1,?2,2,1,?3,1)",params![validation_run_id,experiment_id,serde_json::to_string(&report).expect("report json")]).expect("walk-forward");
        (experiment_id, validation_run_id)
    }

    fn candidate(bridge: &PersistenceBridge, suffix: &str) -> StrategyDeployment {
        let (experiment_id, validation_run_id) = seed_source(bridge, suffix);
        create_candidate(
            bridge,
            CreateCandidateRequest {
                idempotency_key: format!("candidate-{suffix}"),
                experiment_id,
                validation_run_id,
                canary_policy: CanaryPolicy::default(),
            },
        )
        .expect("candidate")
    }

    #[test]
    fn conservative_canary_policy_rejects_weak_limits() {
        let weak = CanaryPolicy {
            minimum_observation_count: 1,
            maximum_drawdown_bps: 5_000,
            maximum_average_slippage_bps: 500,
            maximum_error_count: 99,
            minimum_net_pnl_minor: -1,
        };
        assert!(validate_policy(&weak).is_err());
        assert!(validate_policy(&CanaryPolicy::default()).is_ok());
    }

    #[test]
    fn cost_stress_uses_walk_forward_turnover_without_future_data() {
        let record = serde_json::json!({"config":{"costs":{"buyFeeBps":10.0,"sellFeeBps":10.0,"sellTaxBps":0.0,"slippageBps":5.0}}});
        let metrics = crate::experiments::WalkForwardMetrics {
            total_return_bps: 300,
            max_drawdown_bps: 50,
            completed_trade_count: 100,
            win_rate_bps: Some(6000),
            profit_factor_milli: Some(1500),
            expected_trade_pnl_minor: Some(1),
            realized_pnl_minor: 1,
            gross_profit_minor: 2,
            gross_loss_minor: 1,
            alpha_vs_price_benchmark_bps: Some(100),
            periods_per_year: Some(252),
            period_returns_ppm: vec![],
            turnover_bps: 20_000,
            exposure_bps: 5_000,
        };
        let report = WalkForwardReport {
            validation_run_id: "wf".to_owned(),
            created_at_ms: 1,
            source_experiment_id: "exp".to_owned(),
            strategy_trial_count: 1,
            fold_count: 2,
            initial_training_bar_count: 10,
            positive_oos_fold_count: 2,
            largest_absolute_oos_return_share_bps: 5_000,
            oos_return_spread_bps: 0,
            total_oos_trade_count: 200,
            minimum_oos_trade_count: 200,
            meets_research_sample_minimum: true,
            promotion_blockers: vec![],
            promotion_evaluation: crate::experiments::PromotionEvaluation {
                policy_version: "paper-review-v1".to_owned(),
                eligible_for_paper_review: true,
                checks: vec![],
                warning: String::new(),
            },
            overfit_diagnostics: Default::default(),
            folds: (1..=2)
                .map(|fold_number| crate::experiments::WalkForwardFold {
                    fold_number,
                    training_bar_count: 10,
                    oos_bar_count: 10,
                    training_end_ms: 10,
                    oos_start_ms: 11,
                    oos_end_ms: 20,
                    training: metrics.clone(),
                    out_of_sample: metrics.clone(),
                    regimes: vec![],
                    unclassified_trade_count: 0,
                    state_model: Default::default(),
                })
                .collect(),
            warnings: vec![],
        };
        let (original, stress) = build_cost_stress(&record, &report).expect("stress");
        assert_eq!(original, 15_000);
        assert_eq!(stress.len(), 2);
        assert!(stress[0].aggregate_return_bps < 300);
        assert!(stress.iter().all(|item| item.passed));
    }

    #[test]
    fn canary_requires_approval_and_auto_stops_on_degradation() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let deployment = candidate(&bridge, "degraded");
        assert!(approve_canary(
            &bridge,
            ApprovalRequest {
                deployment_id: deployment.deployment_id.clone(),
                expected_revision: 1,
                approval_text: "승인".to_owned(),
            }
        )
        .is_err());
        let canary = approve_canary(
            &bridge,
            ApprovalRequest {
                deployment_id: deployment.deployment_id,
                expected_revision: 1,
                approval_text: CANARY_APPROVAL.to_owned(),
            },
        )
        .expect("canary approval");
        let stopped = observe_canary(
            &bridge,
            ObserveCanaryRequest {
                deployment_id: canary.deployment_id.clone(),
                expected_revision: canary.revision,
                observation: CanaryObservation {
                    observation_id: "observation-error".to_owned(),
                    observed_at_ms: now_ms().expect("clock"),
                    sample_count: 1,
                    net_pnl_minor: 0,
                    maximum_drawdown_bps: 0,
                    average_slippage_bps: 0,
                    error_count: 1,
                },
            },
        )
        .expect("automatic stop");
        assert_eq!(stopped.status, "stopped");
        assert!(!stopped.live_order_allowed);
        let connection = bridge.connection.lock().expect("database lock");
        let enabled: i64 = connection
            .query_row(
                "SELECT enabled FROM shadow_watches WHERE experiment_id=?1",
                params![stopped.experiment_id],
                |row| row.get(0),
            )
            .expect("watch");
        assert_eq!(enabled, 0);
    }

    #[test]
    fn paper_promotion_preserves_previous_version_for_explicit_rollback() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let activate = |bridge: &PersistenceBridge, suffix: &str| {
            let created = candidate(bridge, suffix);
            let canary = approve_canary(
                bridge,
                ApprovalRequest {
                    deployment_id: created.deployment_id,
                    expected_revision: 1,
                    approval_text: CANARY_APPROVAL.to_owned(),
                },
            )
            .expect("canary");
            let passed = observe_canary(
                bridge,
                ObserveCanaryRequest {
                    deployment_id: canary.deployment_id,
                    expected_revision: canary.revision,
                    observation: CanaryObservation {
                        observation_id: format!("observation-{suffix}"),
                        observed_at_ms: now_ms().expect("clock"),
                        sample_count: 20,
                        net_pnl_minor: 1,
                        maximum_drawdown_bps: 10,
                        average_slippage_bps: 1,
                        error_count: 0,
                    },
                },
            )
            .expect("canary pass");
            approve_paper(
                bridge,
                ApprovalRequest {
                    deployment_id: passed.deployment_id,
                    expected_revision: passed.revision,
                    approval_text: PAPER_APPROVAL.to_owned(),
                },
            )
            .expect("paper promotion")
        };
        let first = activate(&bridge, "v1");
        let second = activate(&bridge, "v2");
        let first_after_supersede = {
            let history = {
                let connection = bridge.connection.lock().expect("database lock");
                let mut statement = connection
                    .prepare(&format!("{DEPLOYMENT_SELECT} WHERE deployment_id=?1"))
                    .expect("history query");
                statement
                    .query_row(params![first.deployment_id], decode_deployment)
                    .expect("previous deployment")
            };
            assert_eq!(history.status, "superseded");
            history
        };
        let restored = rollback(
            &bridge,
            RollbackRequest {
                deployment_id: second.deployment_id,
                target_deployment_id: first_after_supersede.deployment_id.clone(),
                expected_revision: second.revision,
                target_expected_revision: first_after_supersede.revision,
                approval_text: ROLLBACK_APPROVAL.to_owned(),
            },
        )
        .expect("rollback");
        assert_eq!(restored.deployment_id, first_after_supersede.deployment_id);
        assert_eq!(restored.status, "paper_active");
        assert!(!restored.live_order_allowed);
    }
}
