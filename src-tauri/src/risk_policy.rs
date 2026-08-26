use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::persistence::{self, PersistenceBridge};
use crate::{
    backtest::{
        run_backtest_with_risk, BacktestConfig, BacktestResult, BacktestRiskLimits, PriceBar,
    },
    research::ResearchReport,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPolicy {
    pub policy_id: String,
    pub max_order_notional_minor: u64,
    pub max_backtest_drawdown_bps: u64,
    pub stop_loss_bps: u64,
    pub take_profit_bps: u64,
    pub daily_loss_limit_minor: u64,
    #[serde(default)]
    pub protection: Option<crate::strategy_protection::StrategyProtectionPolicy>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPolicyEvaluateRequest {
    pub policy: RiskPolicy,
    pub experiment_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskExperimentEvaluation {
    experiment_id: String,
    max_drawdown_bps: u64,
    baseline_max_drawdown_bps: u64,
    total_return_bps: i64,
    completed_trade_count: usize,
    passed: bool,
    reasons: Vec<String>,
    protection_would_lock: bool,
    protection_trigger_codes: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPolicyEvaluation {
    policy: RiskPolicy,
    experiments: Vec<RiskExperimentEvaluation>,
    unsupported_checks: Vec<String>,
    can_recommend: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RiskPolicySaveRequest {
    pub policy: RiskPolicy,
    pub experiment_ids: Vec<String>,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_policy(policy: &RiskPolicy) -> Result<(), String> {
    if !valid_id(&policy.policy_id)
        || policy.max_order_notional_minor == 0
        || !(1..=10_000).contains(&policy.max_backtest_drawdown_bps)
        || !(1..=9_999).contains(&policy.stop_loss_bps)
        || !(1..=50_000).contains(&policy.take_profit_bps)
        || policy.daily_loss_limit_minor == 0
    {
        return Err("위험 정책 ID와 주문·낙폭·손절·익절·일일손실 한도를 확인해 주세요.".to_owned());
    }
    if let Some(protection) = &policy.protection {
        crate::strategy_protection::validate_strategy_protection_policy(protection)?;
    }
    Ok(())
}

fn evaluate(
    bridge: &PersistenceBridge,
    request: RiskPolicyEvaluateRequest,
) -> Result<RiskPolicyEvaluation, String> {
    validate_policy(&request.policy)?;
    if request.experiment_ids.is_empty()
        || request.experiment_ids.len() > 100
        || request.experiment_ids.iter().any(|id| !valid_id(id))
    {
        return Err("비교할 백테스트 기록을 1~100개 선택해 주세요.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "위험 정책 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut experiments = Vec::new();
    for id in &request.experiment_ids {
        let stored: (String, String) = connection
            .query_row(
                "SELECT b.record_json, d.bars_json FROM backtest_runs b JOIN datasets d ON d.dataset_id = b.dataset_id WHERE b.experiment_id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("백테스트 위험 지표를 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| format!("백테스트 기록을 찾지 못했습니다: {id}"))?;
        let record: serde_json::Value = serde_json::from_str(&stored.0)
            .map_err(|_| format!("백테스트 기록을 해석하지 못했습니다: {id}"))?;
        let dataset: serde_json::Value = serde_json::from_str(&stored.1)
            .map_err(|_| format!("가격 데이터셋을 해석하지 못했습니다: {id}"))?;
        let report: ResearchReport = serde_json::from_value(
            record
                .get("report")
                .cloned()
                .ok_or_else(|| format!("연구 계약이 없습니다: {id}"))?,
        )
        .map_err(|_| format!("연구 계약을 해석하지 못했습니다: {id}"))?;
        let config: BacktestConfig = serde_json::from_value(
            record
                .get("config")
                .cloned()
                .ok_or_else(|| format!("백테스트 설정이 없습니다: {id}"))?,
        )
        .map_err(|_| format!("백테스트 설정을 해석하지 못했습니다: {id}"))?;
        let baseline: BacktestResult = serde_json::from_value(
            record
                .get("result")
                .cloned()
                .ok_or_else(|| format!("기준 결과가 없습니다: {id}"))?,
        )
        .map_err(|_| format!("기준 결과를 해석하지 못했습니다: {id}"))?;
        let mut bars: Vec<PriceBar> = serde_json::from_value(
            dataset
                .get("bars")
                .cloned()
                .ok_or_else(|| format!("가격 봉이 없습니다: {id}"))?,
        )
        .map_err(|_| format!("가격 봉을 해석하지 못했습니다: {id}"))?;
        for bar in &mut bars {
            // schema v6 이전 데이터는 고가·저가를 보존하지 않았다. 임의 범위를 만들지 않고
            // 관측된 시가·종가 범위만 사용해 이전 기록도 결정론적으로 재생한다.
            if bar.high_minor == 0 {
                bar.high_minor = bar.open_minor.max(bar.close_minor);
            }
            if bar.low_minor == 0 {
                bar.low_minor = bar.open_minor.min(bar.close_minor);
            }
        }
        let result = run_backtest_with_risk(
            &report.strategy_candidate,
            &bars,
            &config,
            Some(BacktestRiskLimits {
                stop_loss_bps: request.policy.stop_loss_bps,
                take_profit_bps: request.policy.take_profit_bps,
                daily_loss_limit_minor: request.policy.daily_loss_limit_minor,
            }),
        )
        .map_err(|error| format!("위험 정책 백테스트에 실패했습니다: {}", error.message))?;
        let passed = result.max_drawdown_bps <= request.policy.max_backtest_drawdown_bps;
        let protection = request
            .policy
            .protection
            .clone()
            .map(|policy| {
                let now_ms = result
                    .trades
                    .last()
                    .map_or(result.last_available_at_ms.max(1), |trade| {
                        trade.closed_at_ms
                    });
                crate::strategy_protection::evaluate_strategy_protection(
                    crate::strategy_protection::StrategyProtectionRequest {
                        target_symbol: report.strategy_candidate.symbol.clone(),
                        now_ms,
                        initial_equity_minor: result.initial_cash_minor,
                        policy,
                        closed_trades: result
                            .trades
                            .iter()
                            .enumerate()
                            .map(|(index, trade)| {
                                crate::strategy_protection::ClosedTradeObservation {
                                    trade_id: format!("{id}-{index}"),
                                    symbol: report.strategy_candidate.symbol.clone(),
                                    closed_at_ms: trade.closed_at_ms,
                                    net_pnl_minor: trade.pnl_minor,
                                    exit_kind: match trade.exit_kind {
                                        crate::backtest::BacktestExitKind::StopLoss => {
                                            crate::strategy_protection::TradeExitKind::StopLoss
                                        }
                                        crate::backtest::BacktestExitKind::TakeProfit => {
                                            crate::strategy_protection::TradeExitKind::TakeProfit
                                        }
                                        crate::backtest::BacktestExitKind::Signal
                                        | crate::backtest::BacktestExitKind::EndOfPeriod => {
                                            crate::strategy_protection::TradeExitKind::Signal
                                        }
                                    },
                                }
                            })
                            .collect(),
                    },
                )
            })
            .transpose()?;
        experiments.push(RiskExperimentEvaluation {
            experiment_id: id.clone(),
            max_drawdown_bps: result.max_drawdown_bps,
            baseline_max_drawdown_bps: baseline.max_drawdown_bps,
            total_return_bps: result.total_return_bps,
            completed_trade_count: result.completed_trade_count,
            passed,
            reasons: vec![if passed {
                "백테스트 최대 낙폭 한도 통과".to_owned()
            } else {
                "백테스트 최대 낙폭 한도 초과".to_owned()
            }],
            protection_would_lock: protection
                .as_ref()
                .is_some_and(|decision| !decision.can_open_new_position),
            protection_trigger_codes: protection
                .map(|decision| {
                    decision
                        .triggers
                        .into_iter()
                        .map(|trigger| trigger.code)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    Ok(RiskPolicyEvaluation {
        policy: request.policy,
        can_recommend: !experiments.is_empty(),
        experiments,
        unsupported_checks: vec![],
    })
}

#[tauri::command]
pub fn risk_policy_evaluate(
    request: RiskPolicyEvaluateRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RiskPolicyEvaluation, String> {
    evaluate(&bridge, request)
}

#[tauri::command]
pub fn risk_policy_save_recommendation(
    request: RiskPolicySaveRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    let evaluation = evaluate(
        &bridge,
        RiskPolicyEvaluateRequest {
            policy: request.policy.clone(),
            experiment_ids: request.experiment_ids,
        },
    )?;
    let policy_json = serde_json::to_string(&request.policy)
        .map_err(|_| "위험 정책을 직렬화하지 못했습니다.".to_owned())?;
    let evidence_json = serde_json::to_string(&evaluation)
        .map_err(|_| "위험 정책 근거를 직렬화하지 못했습니다.".to_owned())?;
    let now = persistence::now_ms()?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "위험 정책 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "INSERT INTO risk_policy_versions (policy_id, status, policy_json, evidence_json, created_at_ms) VALUES (?1, 'recommended', ?2, ?3, ?4)",
            params![request.policy.policy_id, policy_json, evidence_json, now],
        )
        .map_err(|error| format!("위험 정책 추천을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn risk_policy_approve(
    policy_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<RiskPolicy, String> {
    if !valid_id(&policy_id) {
        return Err("승인할 위험 정책 ID가 올바르지 않습니다.".to_owned());
    }
    let now = persistence::now_ms()?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "위험 정책 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("위험 정책 승인 트랜잭션을 시작하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "UPDATE risk_policy_versions SET status = 'retired' WHERE status = 'active'",
            [],
        )
        .map_err(|error| format!("기존 위험 정책을 종료하지 못했습니다: {error}"))?;
    let changed = transaction
        .execute(
            "UPDATE risk_policy_versions SET status = 'active', approved_at_ms = ?2 WHERE policy_id = ?1 AND status = 'recommended'",
            params![policy_id, now],
        )
        .map_err(|error| format!("위험 정책을 승인하지 못했습니다: {error}"))?;
    if changed == 0 {
        return Err("승인 대기 중인 위험 정책을 찾지 못했습니다.".to_owned());
    }
    let json: String = transaction
        .query_row(
            "SELECT policy_json FROM risk_policy_versions WHERE policy_id = ?1",
            params![policy_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("승인 정책을 읽지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("위험 정책 승인을 확정하지 못했습니다: {error}"))?;
    serde_json::from_str(&json).map_err(|_| "승인된 위험 정책을 해석하지 못했습니다.".to_owned())
}

pub(crate) fn active_policy(bridge: &PersistenceBridge) -> Result<Option<RiskPolicy>, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "위험 정책 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let json: Option<String> = connection
        .query_row(
            "SELECT policy_json FROM risk_policy_versions WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| format!("활성 위험 정책을 조회하지 못했습니다: {error}"))?;
    json.map(|value| {
        serde_json::from_str(&value).map_err(|_| "활성 위험 정책을 해석하지 못했습니다.".to_owned())
    })
    .transpose()
}

#[tauri::command]
pub fn risk_policy_status(
    bridge: State<'_, PersistenceBridge>,
) -> Result<Option<RiskPolicy>, String> {
    active_policy(&bridge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_or_unbacked_policy_recommendations() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let policy = RiskPolicy {
            policy_id: "risk-v1".to_owned(),
            max_order_notional_minor: 1_000_000,
            max_backtest_drawdown_bps: 2_000,
            stop_loss_bps: 500,
            take_profit_bps: 1_000,
            daily_loss_limit_minor: 500_000,
            protection: None,
        };
        assert!(evaluate(
            &bridge,
            RiskPolicyEvaluateRequest {
                policy: policy.clone(),
                experiment_ids: vec![]
            }
        )
        .is_err());
        assert!(evaluate(
            &bridge,
            RiskPolicyEvaluateRequest {
                policy,
                experiment_ids: vec!["missing".to_owned()]
            }
        )
        .is_err());
    }
}
