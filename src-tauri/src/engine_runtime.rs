use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::State;

use crate::{
    data_quality::{
        build_point_in_time_snapshot, summarize_community_sentiment, CommunityObservation,
        PointInTimeRecord, SourceFreshnessPolicy,
    },
    decision::{
        decide_portfolio_proposal, review_debate, review_trade_plan, DebateTurn, RiskPanelVote,
        SpecialistReport, StructuredTradePlan,
    },
    execution_control::{evaluate_pretrade, PreTradeInput, PreTradePolicy},
    persistence::{now_ms, PersistenceBridge},
    screening::{screen_candidates, ScreeningObservation, ScreeningStrategy, UniverseVersion},
};

const MAX_RUNS_LIMIT: u16 = 200;
const MAX_REQUEST_BYTES: usize = 2_000_000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRunRequest {
    pub run_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub restarted_from_run_id: Option<String>,
    pub as_of_ms: u64,
    pub records: Vec<PointInTimeRecord>,
    pub freshness_policies: Vec<SourceFreshnessPolicy>,
    pub community_observations: Vec<CommunityObservation>,
    pub universe: UniverseVersion,
    pub screening_strategy: ScreeningStrategy,
    pub screening_observations: Vec<ScreeningObservation>,
    pub specialist_reports: Vec<SpecialistReport>,
    pub debate_turns: Vec<DebateTurn>,
    pub maximum_debate_rounds: usize,
    pub maximum_debate_tokens: usize,
    pub trade_plan: StructuredTradePlan,
    pub maximum_loss_minor: u64,
    pub analysis_ids: Vec<String>,
    pub strategy_version: String,
    pub risk_policy_version: String,
    pub risk_votes: Vec<RiskPanelVote>,
    pub pretrade_policy: PreTradePolicy,
    pub pretrade_input: PreTradeInput,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRunReport {
    pub run_id: String,
    pub status: String,
    pub symbol: String,
    pub market: String,
    pub candidate_ready: bool,
    pub blockers: Vec<String>,
    pub stages: Value,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRunSummary {
    pub run_id: String,
    pub status: String,
    pub symbol: String,
    pub market: String,
    pub candidate_ready: bool,
    pub updated_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRuntimeOverview {
    pub total_runs: u64,
    pub candidate_ready_runs: u64,
    pub blocked_runs: u64,
    pub interrupted_runs: u64,
    pub latest_run: Option<EngineRunSummary>,
    pub live_order_enabled: bool,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn contains_secret_marker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "client_secret",
        "api_secret",
        "access_token",
        "authorization:",
        "password=",
        "private_key",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn market_key(request: &EngineRunRequest) -> String {
    request
        .universe
        .entries
        .iter()
        .find(|entry| entry.symbol == request.trade_plan.symbol)
        .map(|entry| format!("{:?}", entry.market).to_ascii_lowercase())
        .unwrap_or_else(|| "unknown".to_owned())
}

fn evaluate(request: &EngineRunRequest, created_at_ms: u64) -> Result<EngineRunReport, String> {
    if !valid_id(&request.run_id)
        || !valid_id(&request.idempotency_key)
        || request.as_of_ms == 0
        || request.trade_plan.symbol != request.pretrade_input.symbol
        || request.trade_plan.side != request.pretrade_input.side
        || request.trade_plan.entry_price_minor != request.pretrade_input.entry_price_minor
        || request.trade_plan.stop_price_minor != request.pretrade_input.stop_price_minor
    {
        return Err(
            "실행 ID·기준 시각 또는 거래 계획과 사전 주문 입력이 일치하지 않습니다.".to_owned(),
        );
    }
    let serialized = serde_json::to_string(request)
        .map_err(|error| format!("엔진 요청을 직렬화하지 못했습니다: {error}"))?;
    if serialized.len() > MAX_REQUEST_BYTES || contains_secret_marker(&serialized) {
        return Err(
            "엔진 요청 크기가 너무 크거나 비밀정보로 의심되는 값이 포함되었습니다.".to_owned(),
        );
    }

    let snapshot = build_point_in_time_snapshot(
        &format!("snapshot:{}", request.run_id),
        request.as_of_ms,
        &request.records,
        &request.freshness_policies,
    )?;
    let community =
        summarize_community_sentiment(request.as_of_ms, &request.community_observations)?;
    let screening = screen_candidates(
        &request.universe,
        &request.screening_strategy,
        request.as_of_ms,
        &request.screening_observations,
    )?;
    let debate = if request.specialist_reports.len() >= 2 {
        review_debate(
            &request.specialist_reports[0],
            &request.specialist_reports[1],
            &request.debate_turns,
            request.maximum_debate_rounds,
            request.maximum_debate_tokens,
        )
    } else {
        return Err("서로 다른 관점의 전문 보고서가 2개 이상 필요합니다.".to_owned());
    };
    let trade_plan_review = review_trade_plan(
        &request.trade_plan,
        request.as_of_ms,
        request.maximum_loss_minor,
    );
    let portfolio = decide_portfolio_proposal(
        &request.analysis_ids,
        &request.strategy_version,
        &request.risk_policy_version,
        &request.risk_votes,
    );
    let pretrade = evaluate_pretrade(
        &request.pretrade_policy,
        &request.pretrade_input,
        request.as_of_ms,
    );

    let mut blockers = Vec::new();
    if !snapshot.order_allowed {
        blockers.push("시점 정합 데이터 품질 게이트 미통과".to_owned());
    }
    if !screening
        .candidates
        .iter()
        .any(|candidate| candidate.symbol == request.trade_plan.symbol)
    {
        blockers.push("거래 계획 종목이 스크리닝 후보에 없음".to_owned());
    }
    blockers.extend(debate.issues.iter().cloned());
    blockers.extend(trade_plan_review.issues.iter().cloned());
    if !portfolio.approved {
        blockers.extend(portfolio.reasons.iter().cloned());
    }
    blockers.extend(
        pretrade
            .checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| format!("{}: {}", check.rule_id, check.message)),
    );
    blockers.sort();
    blockers.dedup();
    let candidate_ready = blockers.is_empty();
    Ok(EngineRunReport {
        run_id: request.run_id.clone(),
        status: if candidate_ready {
            "completed"
        } else {
            "blocked"
        }
        .to_owned(),
        symbol: request.trade_plan.symbol.clone(),
        market: market_key(request),
        candidate_ready,
        blockers,
        stages: json!({
            "restartedFromRunId": request.restarted_from_run_id,
            "snapshot": snapshot,
            "community": community,
            "screening": screening,
            "debate": debate,
            "tradePlanReview": trade_plan_review,
            "portfolio": portfolio,
            "pretrade": pretrade,
        }),
        created_at_ms,
    })
}

fn execute(
    bridge: &PersistenceBridge,
    request: EngineRunRequest,
) -> Result<EngineRunReport, String> {
    let input_json = serde_json::to_string(&request)
        .map_err(|error| format!("엔진 요청을 직렬화하지 못했습니다: {error}"))?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing = connection
        .query_row(
            "SELECT input_json,report_json FROM engine_runs WHERE idempotency_key=?1",
            params![request.idempotency_key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("중복 엔진 실행을 확인하지 못했습니다: {error}"))?;
    if let Some((stored_input, stored_report)) = existing {
        if stored_input != input_json {
            return Err("같은 멱등성 키를 다른 엔진 입력에 재사용할 수 없습니다.".to_owned());
        }
        return serde_json::from_str(&stored_report)
            .map_err(|error| format!("저장된 엔진 보고서를 해석하지 못했습니다: {error}"));
    }
    let created_at_ms = now_ms()?;
    let report = evaluate(&request, created_at_ms)?;
    let report_json = serde_json::to_string(&report)
        .map_err(|error| format!("엔진 보고서를 직렬화하지 못했습니다: {error}"))?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("엔진 기록 트랜잭션을 시작하지 못했습니다: {error}"))?;
    transaction
        .execute(
            "INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
            params![&report.run_id,&request.idempotency_key,&report.status,&report.symbol,&report.market,i64::from(report.candidate_ready),&input_json,&report_json,created_at_ms],
        )
        .map_err(|error| format!("엔진 실행 기록을 저장하지 못했습니다: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("엔진 실행 기록을 확정하지 못했습니다: {error}"))?;
    Ok(report)
}

fn history(bridge: &PersistenceBridge, limit: u16) -> Result<Vec<EngineRunSummary>, String> {
    if limit == 0 || limit > MAX_RUNS_LIMIT {
        return Err("엔진 실행 조회 개수는 1~200이어야 합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection.prepare("SELECT r.run_id,COALESCE((SELECT e.status FROM engine_run_status_events e WHERE e.run_id=r.run_id ORDER BY e.occurred_at_ms DESC,e.event_id DESC LIMIT 1),r.status),r.symbol,r.market,r.candidate_ready,MAX(r.updated_at_ms,COALESCE((SELECT MAX(e.occurred_at_ms) FROM engine_run_status_events e WHERE e.run_id=r.run_id),0)) FROM engine_runs r ORDER BY 6 DESC,r.run_id DESC LIMIT ?1").map_err(|error| format!("엔진 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit], |row| {
            Ok(EngineRunSummary {
                run_id: row.get(0)?,
                status: row.get(1)?,
                symbol: row.get(2)?,
                market: row.get(3)?,
                candidate_ready: row.get::<_, i64>(4)? == 1,
                updated_at_ms: row.get(5)?,
            })
        })
        .map_err(|error| format!("엔진 이력을 조회하지 못했습니다: {error}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("엔진 이력을 읽지 못했습니다: {error}"))
}

#[tauri::command]
pub fn engine_run_execute(
    request: EngineRunRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<EngineRunReport, String> {
    execute(&bridge, request)
}

#[tauri::command]
pub fn engine_run_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<EngineRunSummary>, String> {
    history(&bridge, limit)
}

fn detail(bridge: &PersistenceBridge, run_id: &str) -> Result<EngineRunReport, String> {
    if !valid_id(run_id) {
        return Err("유효한 엔진 실행 ID가 필요합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let (serialized, effective_status) = connection
        .query_row(
            "SELECT r.report_json,COALESCE((SELECT e.status FROM engine_run_status_events e WHERE e.run_id=r.run_id ORDER BY e.occurred_at_ms DESC,e.event_id DESC LIMIT 1),r.status) FROM engine_runs r WHERE r.run_id=?1",
            params![run_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("엔진 실행 상세를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "엔진 실행 기록을 찾지 못했습니다.".to_owned())?;
    let mut report: EngineRunReport = serde_json::from_str(&serialized)
        .map_err(|error| format!("엔진 실행 상세를 해석하지 못했습니다: {error}"))?;
    report.status = effective_status;
    Ok(report)
}

#[tauri::command]
pub fn engine_run_detail(
    run_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<EngineRunReport, String> {
    detail(&bridge, &run_id)
}

fn cancel(bridge: &PersistenceBridge, run_id: &str) -> Result<(), String> {
    if !valid_id(run_id) {
        return Err("유효한 엔진 실행 ID가 필요합니다.".to_owned());
    }
    let updated_at_ms = now_ms()?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let effective_status = connection.query_row("SELECT COALESCE((SELECT e.status FROM engine_run_status_events e WHERE e.run_id=r.run_id ORDER BY e.occurred_at_ms DESC,e.event_id DESC LIMIT 1),r.status) FROM engine_runs r WHERE r.run_id=?1", params![run_id], |row| row.get::<_,String>(0)).optional().map_err(|error| format!("취소할 엔진 실행을 조회하지 못했습니다: {error}"))?.ok_or_else(|| "완료된 후보 또는 존재하지 않는 실행은 취소할 수 없습니다.".to_owned())?;
    if !matches!(effective_status.as_str(), "blocked" | "interrupted") {
        return Err("완료·취소된 실행은 취소할 수 없습니다.".to_owned());
    }
    connection.execute("INSERT INTO engine_run_status_events(event_id,run_id,status,reason,occurred_at_ms) VALUES(?1,?2,'cancelled','사용자 취소',?3)", params![format!("cancel:{run_id}:{updated_at_ms}"),run_id,updated_at_ms]).map_err(|error| format!("엔진 실행 취소 사건을 저장하지 못했습니다: {error}"))?;
    Ok(())
}

fn restart(
    bridge: &PersistenceBridge,
    source_run_id: &str,
    new_run_id: &str,
    idempotency_key: &str,
) -> Result<EngineRunReport, String> {
    if !valid_id(source_run_id)
        || !valid_id(new_run_id)
        || !valid_id(idempotency_key)
        || source_run_id == new_run_id
    {
        return Err("원본·신규 실행 ID와 새 멱등성 키가 필요합니다.".to_owned());
    }
    let stored_input = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT r.input_json FROM engine_runs r WHERE r.run_id=?1 AND COALESCE((SELECT e.status FROM engine_run_status_events e WHERE e.run_id=r.run_id ORDER BY e.occurred_at_ms DESC,e.event_id DESC LIMIT 1),r.status) IN ('blocked','cancelled','interrupted')",
                params![source_run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("재시작할 엔진 입력을 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| "완료된 후보 또는 존재하지 않는 실행은 재시작할 수 없습니다.".to_owned())?
    };
    let mut request: EngineRunRequest = serde_json::from_str(&stored_input)
        .map_err(|error| format!("재시작 입력을 해석하지 못했습니다: {error}"))?;
    request.run_id = new_run_id.to_owned();
    request.idempotency_key = idempotency_key.to_owned();
    request.restarted_from_run_id = Some(source_run_id.to_owned());
    let snapshot_id = format!("snapshot:{new_run_id}");
    for record in &mut request.records {
        record.snapshot_id.clone_from(&snapshot_id);
    }
    execute(bridge, request)
}

#[tauri::command]
pub fn engine_run_cancel(
    run_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<(), String> {
    cancel(&bridge, &run_id)
}

#[tauri::command]
pub fn engine_run_restart(
    source_run_id: String,
    new_run_id: String,
    idempotency_key: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<EngineRunReport, String> {
    restart(&bridge, &source_run_id, &new_run_id, &idempotency_key)
}

#[tauri::command]
pub fn engine_runtime_overview(
    bridge: State<'_, PersistenceBridge>,
) -> Result<EngineRuntimeOverview, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "엔진 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let counts = connection.query_row("WITH effective AS (SELECT r.candidate_ready,COALESCE((SELECT e.status FROM engine_run_status_events e WHERE e.run_id=r.run_id ORDER BY e.occurred_at_ms DESC,e.event_id DESC LIMIT 1),r.status) status FROM engine_runs r) SELECT COUNT(*),COALESCE(SUM(candidate_ready),0),COALESCE(SUM(status='blocked'),0),COALESCE(SUM(status='interrupted'),0) FROM effective", [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).map_err(|error| format!("엔진 상태를 집계하지 못했습니다: {error}"))?;
    drop(connection);
    Ok(EngineRuntimeOverview {
        total_runs: counts.0,
        candidate_ready_runs: counts.1,
        blocked_runs: counts.2,
        interrupted_runs: counts.3,
        latest_run: history(&bridge, 1)?.into_iter().next(),
        live_order_enabled: false,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        data_quality::TemporalMetadata,
        decision::{EvidenceClaim, RiskPanelDecision},
        research::Market,
        screening::{InstrumentStatus, RuleOperator, ScreeningRule, UniverseEntry},
        simulation::TradingCosts,
        trading::TradeSide,
    };

    fn request(run_id: &str) -> EngineRunRequest {
        let snapshot_id = format!("snapshot:{run_id}");
        let claim = |claim_id: &str| EvidenceClaim {
            claim_id: claim_id.to_owned(),
            statement: "검증 가능한 주장".to_owned(),
            evidence_ids: vec!["evidence-1".to_owned()],
            counter_evidence_ids: vec![],
            confidence_bps: 6_000,
        };
        EngineRunRequest {
            run_id: run_id.to_owned(),
            idempotency_key: format!("idempotency-{run_id}"),
            restarted_from_run_id: None,
            as_of_ms: 100,
            records: vec![PointInTimeRecord {
                record_id: "price-005930".to_owned(),
                snapshot_id,
                symbol: "005930".to_owned(),
                metadata: TemporalMetadata {
                    event_time_ms: 90,
                    available_at_ms: 95,
                    ingested_at_ms: 95,
                    source: "market".to_owned(),
                    source_revision: "rev-1".to_owned(),
                },
                quality_flags: vec![],
                payload_hash: "0123456789abcdef".to_owned(),
            }],
            freshness_policies: vec![SourceFreshnessPolicy {
                source: "market".to_owned(),
                required: true,
                maximum_age_ms: 10,
            }],
            community_observations: vec![],
            universe: UniverseVersion {
                universe_id: "kr-liquid".to_owned(),
                version: 1,
                active_markets: vec![Market::Korea],
                entries: vec![UniverseEntry {
                    symbol: "005930".to_owned(),
                    market: Market::Korea,
                    status: InstrumentStatus::Tradable,
                    effective_from_ms: 1,
                    effective_to_ms: None,
                    spread_bps: Some(2),
                    abnormal_spread_bps: 20,
                }],
            },
            screening_strategy: ScreeningStrategy {
                strategy_id: "liquidity".to_owned(),
                version: 1,
                rules: vec![ScreeningRule {
                    rule_id: "volume-min".to_owned(),
                    metric: "volume".to_owned(),
                    operator: RuleOperator::GreaterOrEqual,
                    threshold: 1_000,
                    score_weight_bps: 10_000,
                    description: "유동성 기준 통과".to_owned(),
                }],
                maximum_candidates_per_market: 10,
                analysis_budget: 10,
            },
            screening_observations: vec![ScreeningObservation {
                symbol: "005930".to_owned(),
                market: Market::Korea,
                observed_at_ms: 99,
                metrics: BTreeMap::from([("volume".to_owned(), 10_000)]),
            }],
            specialist_reports: vec![
                SpecialistReport {
                    report_id: "bull-report".to_owned(),
                    role_id: "bull".to_owned(),
                    trace_id: "trace-1".to_owned(),
                    data_as_of_ms: 100,
                    claims: vec![claim("bull-claim")],
                    incomplete_reasons: vec![],
                },
                SpecialistReport {
                    report_id: "bear-report".to_owned(),
                    role_id: "bear".to_owned(),
                    trace_id: "trace-1".to_owned(),
                    data_as_of_ms: 100,
                    claims: vec![claim("bear-claim")],
                    incomplete_reasons: vec![],
                },
            ],
            debate_turns: vec![
                DebateTurn {
                    side: "bull".to_owned(),
                    addressed_claim_ids: vec!["bear-claim".to_owned()],
                    response_claims: vec![],
                    token_count: 100,
                },
                DebateTurn {
                    side: "bear".to_owned(),
                    addressed_claim_ids: vec!["bull-claim".to_owned()],
                    response_claims: vec![],
                    token_count: 100,
                },
            ],
            maximum_debate_rounds: 2,
            maximum_debate_tokens: 500,
            trade_plan: StructuredTradePlan {
                schema_version: 1,
                plan_id: "plan-1".to_owned(),
                symbol: "005930".to_owned(),
                side: TradeSide::Buy,
                entry_price_minor: 70_000,
                stop_price_minor: 68_000,
                target_price_minor: 76_000,
                valid_until_ms: 200,
                suggested_quantity: 5,
                evidence_ids: vec!["evidence-1".to_owned()],
            },
            maximum_loss_minor: 20_000,
            analysis_ids: vec!["analysis-1".to_owned()],
            strategy_version: "strategy-v1".to_owned(),
            risk_policy_version: "risk-v1".to_owned(),
            risk_votes: ["conservative", "balanced", "aggressive"]
                .into_iter()
                .map(|perspective| RiskPanelVote {
                    perspective: perspective.to_owned(),
                    decision: RiskPanelDecision::Approve,
                    measured_value: 1,
                    limit_value: 2,
                    reason: "한도 이내".to_owned(),
                })
                .collect(),
            pretrade_policy: PreTradePolicy {
                maximum_loss_minor: 20_000,
                maximum_order_notional_minor: 1_000_000,
                maximum_participation_bps: 100,
                maximum_quote_age_ms: 10,
                maximum_price_deviation_bps: 20,
            },
            pretrade_input: PreTradeInput {
                symbol: "005930".to_owned(),
                side: TradeSide::Buy,
                suggested_quantity: 5,
                entry_price_minor: 70_000,
                stop_price_minor: 68_000,
                quote_price_minor: 70_000,
                quote_observed_at_ms: 99,
                average_period_volume: 100_000,
                current_gross_exposure_minor: 0,
                maximum_gross_exposure_minor: 1_000_000,
                costs: TradingCosts {
                    buy_fee_bps: 1.5,
                    sell_fee_bps: 1.5,
                    sell_tax_bps: 20.0,
                    slippage_bps: 0.0,
                },
            },
        }
    }

    #[test]
    fn completed_run_is_persisted_once_and_replayed_idempotently() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let first = execute(&bridge, request("run-1")).expect("run");
        let replay = execute(&bridge, request("run-1")).expect("replay");
        assert!(first.candidate_ready);
        assert_eq!(first.created_at_ms, replay.created_at_ms);
        assert_eq!(history(&bridge, 10).expect("history").len(), 1);
        assert!(cancel(&bridge, "run-1").is_err());
    }

    #[test]
    fn completed_run_promotes_to_a_traced_internal_candidate() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let current = now_ms().expect("time");
        let mut input = request("run-candidate");
        input.as_of_ms = current;
        input.records[0].metadata.event_time_ms = current - 10;
        input.records[0].metadata.available_at_ms = current - 5;
        input.records[0].metadata.ingested_at_ms = current - 5;
        input.screening_observations[0].observed_at_ms = current - 1;
        input
            .specialist_reports
            .iter_mut()
            .for_each(|report| report.data_as_of_ms = current);
        input.trade_plan.valid_until_ms = current + 60_000;
        input.pretrade_input.quote_observed_at_ms = current - 1;
        execute(&bridge, input).expect("run");
        let candidate =
            crate::runtime_ops::create_engine_candidate(&bridge, "run-candidate", current + 1)
                .expect("candidate");
        assert_eq!(candidate.run_id, "run-candidate");
        assert_eq!(
            candidate.status,
            crate::runtime_ops::EngineCandidateStatus::SafetyApproved
        );
        assert_eq!(candidate.quantity, 5);
        assert_eq!(candidate.safety["liveOrderEnabled"], false);
    }

    #[test]
    fn idempotency_key_rejects_changed_input() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        execute(&bridge, request("run-2")).expect("run");
        let mut changed = request("run-2");
        changed.pretrade_input.suggested_quantity = 4;
        assert!(execute(&bridge, changed)
            .expect_err("changed input")
            .contains("멱등성 키"));
    }

    #[test]
    fn stale_required_data_blocks_then_allows_cancellation() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let mut stale = request("run-3");
        stale.freshness_policies[0].maximum_age_ms = 2;
        let report = execute(&bridge, stale).expect("blocked run");
        assert_eq!(report.status, "blocked");
        assert!(!report.candidate_ready);
        cancel(&bridge, "run-3").expect("cancel");
        assert_eq!(
            detail(&bridge, "run-3").expect("detail").status,
            "cancelled"
        );
        let (base_status, base_report): (String, String) = bridge
            .connection
            .lock()
            .expect("lock")
            .query_row(
                "SELECT status,report_json FROM engine_runs WHERE run_id='run-3'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("base run");
        assert_eq!(base_status, "blocked");
        assert_eq!(
            serde_json::from_str::<EngineRunReport>(&base_report)
                .expect("base report")
                .status,
            "blocked"
        );
        let replay =
            restart(&bridge, "run-3", "run-3-retry", "idempotency-run-3-retry").expect("restart");
        assert_eq!(replay.status, "blocked");
        assert_eq!(history(&bridge, 10).expect("history").len(), 2);
    }
}
