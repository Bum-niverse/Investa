use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::{
    crypto_market::CryptoMarketBridge,
    market_data::MarketDataBridge,
    operations::ShadowEngineRuntime,
    paper_account::AppendOnlyLedger,
    persistence::{now_ms, PersistenceBridge},
};

const MAX_HANDOFFS: u16 = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPaperHandoffPrepareRequest {
    pub workflow_job_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPaperHandoffFinalizeRequest {
    pub workflow_job_id: String,
    pub experiment_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingPaperHandoff {
    pub handoff_id: String,
    pub workflow_job_id: String,
    pub analysis_record_id: String,
    pub symbol: String,
    pub strategy: String,
    pub experiment_id: Option<String>,
    pub paper_candidate_id: Option<String>,
    pub engine_run_id: Option<String>,
    pub status: String,
    pub blocker: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub live_order_enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GoldenPathStage {
    pub id: &'static str,
    pub label: &'static str,
    pub status: &'static str,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoldenPathAudit {
    pub workflow_job_id: String,
    pub handoff_id: String,
    pub status: String,
    pub stages: Vec<GoldenPathStage>,
    pub checked_at_ms: u64,
    pub live_order_enabled: bool,
}

struct StoredHandoff {
    handoff_id: String,
    workflow_job_id: String,
    analysis_record_id: String,
    symbol: String,
    strategy: String,
    experiment_id: Option<String>,
    paper_candidate_id: Option<String>,
    engine_run_id: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
}

fn stored_handoff(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredHandoff> {
    Ok(StoredHandoff {
        handoff_id: row.get(0)?,
        workflow_job_id: row.get(1)?,
        analysis_record_id: row.get(2)?,
        symbol: row.get(3)?,
        strategy: row.get(4)?,
        experiment_id: row.get(5)?,
        paper_candidate_id: row.get(6)?,
        engine_run_id: row.get(7)?,
        created_at_ms: row.get(8)?,
        updated_at_ms: row.get(9)?,
    })
}

fn reconcile_paper_candidate_links(
    bridge: &PersistenceBridge,
    observed_at_ms: u64,
) -> Result<(), String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "회의-모의후보 계보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "UPDATE meeting_paper_handoffs
             SET paper_candidate_id=(
                 SELECT candidate_id FROM paper_order_candidates candidate
                 WHERE candidate.experiment_id=meeting_paper_handoffs.experiment_id
                 ORDER BY candidate.updated_at_ms DESC,candidate.candidate_id DESC LIMIT 1
             ),updated_at_ms=?1
             WHERE experiment_id IS NOT NULL AND paper_candidate_id IS NULL
               AND EXISTS(SELECT 1 FROM paper_order_candidates candidate
                          WHERE candidate.experiment_id=meeting_paper_handoffs.experiment_id)",
            params![observed_at_ms],
        )
        .map_err(|error| format!("회의-모의후보 계보를 연결하지 못했습니다: {error}"))?;
    Ok(())
}

fn reconcile_engine_links(bridge: &PersistenceBridge, observed_at_ms: u64) -> Result<(), String> {
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "회의-엔진 계보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let pending = {
        let mut statement = connection
            .prepare(
                "SELECT handoff_id,analysis_record_id,symbol FROM meeting_paper_handoffs
                 WHERE engine_run_id IS NULL ORDER BY created_at_ms ASC,handoff_id ASC",
            )
            .map_err(|error| format!("미연결 회의 인계 조회를 준비하지 못했습니다: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| format!("미연결 회의 인계를 조회하지 못했습니다: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("미연결 회의 인계를 읽지 못했습니다: {error}"))?
    };
    let runs = {
        let mut statement = connection
            .prepare(
                "SELECT run_id,input_json FROM engine_runs
                 WHERE status='completed' AND candidate_ready=1
                 ORDER BY updated_at_ms DESC,run_id DESC",
            )
            .map_err(|error| format!("후보 준비 엔진 실행 조회를 준비하지 못했습니다: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| format!("후보 준비 엔진 실행을 조회하지 못했습니다: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("후보 준비 엔진 실행을 읽지 못했습니다: {error}"))?
    };

    let transaction = connection
        .transaction()
        .map_err(|error| format!("회의-엔진 계보 대사를 시작하지 못했습니다: {error}"))?;
    for (handoff_id, analysis_record_id, symbol) in pending {
        let matched = runs.iter().find_map(|(run_id, input_json)| {
            let request = serde_json::from_str::<Value>(input_json).ok()?;
            let same_symbol = request.pointer("/tradePlan/symbol").and_then(Value::as_str)
                == Some(symbol.as_str());
            let contains_analysis = request
                .get("analysisIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| {
                    ids.iter()
                        .any(|id| id.as_str() == Some(&analysis_record_id))
                });
            (same_symbol && contains_analysis).then_some(run_id)
        });
        if let Some(run_id) = matched {
            transaction
                .execute(
                    "UPDATE meeting_paper_handoffs SET engine_run_id=?1,updated_at_ms=?2
                     WHERE handoff_id=?3 AND engine_run_id IS NULL
                       AND NOT EXISTS(SELECT 1 FROM meeting_paper_handoffs WHERE engine_run_id=?1)",
                    params![run_id, observed_at_ms, handoff_id],
                )
                .map_err(|error| format!("회의-엔진 계보를 연결하지 못했습니다: {error}"))?;
        }
    }
    transaction
        .commit()
        .map_err(|error| format!("회의-엔진 계보 대사를 확정하지 못했습니다: {error}"))
}

fn effective_status(
    bridge: &PersistenceBridge,
    stored: StoredHandoff,
) -> Result<MeetingPaperHandoff, String> {
    let (status, blocker) = if let Some(candidate_id) = &stored.paper_candidate_id {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의-모의후보 상태 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let status = connection
            .query_row(
                "SELECT status FROM paper_order_candidates WHERE candidate_id=?1",
                params![candidate_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| format!("연결된 모의주문 후보를 조회하지 못했습니다: {error}"))?;
        match status {
            Some(status) => (status, None),
            None => (
                "interrupted".to_owned(),
                Some("연결된 모의주문 후보를 찾지 못해 복구가 필요합니다.".to_owned()),
            ),
        }
    } else if let Some(experiment_id) = &stored.experiment_id {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의-섀도우 감시 상태 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let watch = connection
            .query_row(
                "SELECT status,last_error FROM shadow_watches WHERE experiment_id=?1",
                params![experiment_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|error| format!("연결된 섀도우 감시를 조회하지 못했습니다: {error}"))?;
        match watch {
            Some((status, _last_error)) if status == "watching" => {
                ("watching_signal".to_owned(), None)
            }
            Some((_status, Some(error))) => ("blocked".to_owned(), Some(error)),
            Some((status, None)) => (status, None),
            None => (
                "backtest_completed".to_owned(),
                Some("백테스트는 저장됐지만 섀도우 감시가 시작되지 않았습니다.".to_owned()),
            ),
        }
    } else if let Some(run_id) = &stored.engine_run_id {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의-엔진 상태 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let run: Option<(String, bool, String)> = connection
            .query_row(
                "SELECT status,candidate_ready,report_json FROM engine_runs WHERE run_id=?1",
                params![run_id],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? == 1, row.get(2)?)),
            )
            .optional()
            .map_err(|error| format!("연결된 엔진 실행을 조회하지 못했습니다: {error}"))?;
        match run {
            None => (
                "interrupted".to_owned(),
                Some("연결된 엔진 실행을 찾지 못해 복구가 필요합니다.".to_owned()),
            ),
            Some((run_status, false, report_json)) => {
                let blockers = serde_json::from_str::<Value>(&report_json)
                    .ok()
                    .and_then(|report| report.get("blockers").cloned())
                    .and_then(|value| value.as_array().cloned())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                    .collect::<Vec<_>>();
                (
                    run_status,
                    Some(if blockers.is_empty() {
                        "결정론적 엔진 게이트를 통과하지 못했습니다.".to_owned()
                    } else {
                        blockers.join(" · ")
                    }),
                )
            }
            Some((_run_status, true, _)) => {
                let candidate_status: Option<String> = connection
                    .query_row(
                        "SELECT status FROM engine_order_candidates WHERE run_id=?1",
                        params![run_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|error| {
                        format!("연결된 모의주문 후보를 조회하지 못했습니다: {error}")
                    })?;
                (
                    candidate_status.unwrap_or_else(|| "candidate_ready".to_owned()),
                    None,
                )
            }
        }
    } else {
        (
            "awaiting_backtest".to_owned(),
            Some(
                "회의 분석에서 지원 전략 계약을 만들고 탐색 백테스트를 실행해야 합니다.".to_owned(),
            ),
        )
    };
    Ok(MeetingPaperHandoff {
        handoff_id: stored.handoff_id,
        workflow_job_id: stored.workflow_job_id,
        analysis_record_id: stored.analysis_record_id,
        symbol: stored.symbol,
        strategy: stored.strategy,
        experiment_id: stored.experiment_id,
        paper_candidate_id: stored.paper_candidate_id,
        engine_run_id: stored.engine_run_id,
        status,
        blocker,
        created_at_ms: stored.created_at_ms,
        updated_at_ms: stored.updated_at_ms,
        live_order_enabled: false,
    })
}

fn load_handoff(
    bridge: &PersistenceBridge,
    handoff_id: &str,
) -> Result<MeetingPaperHandoff, String> {
    let stored = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의 인계 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT handoff_id,workflow_job_id,analysis_record_id,symbol,strategy,experiment_id,paper_candidate_id,engine_run_id,created_at_ms,updated_at_ms
                 FROM meeting_paper_handoffs WHERE handoff_id=?1",
                params![handoff_id],
                stored_handoff,
            )
            .optional()
            .map_err(|error| format!("회의 인계를 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| "회의 인계를 찾지 못했습니다.".to_owned())?
    };
    effective_status(bridge, stored)
}

fn prepare(
    bridge: &PersistenceBridge,
    request: MeetingPaperHandoffPrepareRequest,
    created_at_ms: u64,
) -> Result<MeetingPaperHandoff, String> {
    if !valid_id(&request.workflow_job_id) {
        return Err("유효한 회의 작업 ID가 필요합니다.".to_owned());
    }
    let analysis_record_id = format!("analysis-{}", request.workflow_job_id);
    let (status, synthesis_json) = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의 작업 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT status,synthesis_json FROM workflow_jobs WHERE job_id=?1",
                params![request.workflow_job_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(|error| format!("회의 작업을 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| "회의 작업을 찾지 못했습니다.".to_owned())?
    };
    if status != "completed" {
        return Err("완료된 회의만 모의투자 검증으로 인계할 수 있습니다.".to_owned());
    }
    let synthesis: Value = serde_json::from_str(
        synthesis_json
            .as_deref()
            .ok_or_else(|| "회의 종합 보고가 저장되지 않았습니다.".to_owned())?,
    )
    .map_err(|error| format!("회의 종합 보고를 해석하지 못했습니다: {error}"))?;
    if synthesis.get("decision").and_then(Value::as_str) != Some("paper_candidate") {
        return Err("모의투자 후보로 판정된 회의만 검증 단계로 인계할 수 있습니다.".to_owned());
    }
    let recommendation = synthesis
        .get("backtestRecommendation")
        .ok_or_else(|| "회의 종합 보고에 백테스트 권고가 없습니다.".to_owned())?;
    if recommendation.get("required").and_then(Value::as_bool) != Some(true) {
        return Err(
            "필수 백테스트가 지정되지 않은 회의는 주문 후보로 인계할 수 없습니다.".to_owned(),
        );
    }
    let symbol = recommendation
        .get("symbol")
        .and_then(Value::as_str)
        .filter(|value| valid_symbol(value))
        .ok_or_else(|| "검증 가능한 단일 종목 코드가 필요합니다.".to_owned())?;
    let strategy = recommendation
        .get("strategy")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.chars().count() <= 500)
        .ok_or_else(|| "검증 가능한 전략 설명이 필요합니다.".to_owned())?;
    let handoff_id = format!("meeting-handoff:{}", request.workflow_job_id);
    {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의 인계 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let analysis_exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM analysis_notes WHERE record_id=?1 AND kind='meeting' AND status='completed')",
                params![analysis_record_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("회의 분석 기록을 확인하지 못했습니다: {error}"))?;
        if !analysis_exists {
            return Err("완료된 회의 분석 기록이 저장된 뒤 다시 시도해 주세요.".to_owned());
        }
        connection
            .execute(
                "INSERT INTO meeting_paper_handoffs(handoff_id,workflow_job_id,analysis_record_id,symbol,strategy,experiment_id,paper_candidate_id,engine_run_id,created_at_ms,updated_at_ms)
                 VALUES(?1,?2,?3,?4,?5,NULL,NULL,NULL,?6,?6)
                 ON CONFLICT(workflow_job_id) DO NOTHING",
                params![handoff_id, request.workflow_job_id, analysis_record_id, symbol, strategy, created_at_ms],
            )
            .map_err(|error| format!("회의 분석을 모의투자 검증으로 인계하지 못했습니다: {error}"))?;
    }
    reconcile_engine_links(bridge, created_at_ms)?;
    load_handoff(bridge, &handoff_id)
}

fn link_backtest(
    bridge: &PersistenceBridge,
    request: &MeetingPaperHandoffFinalizeRequest,
    linked_at_ms: u64,
) -> Result<String, String> {
    if !valid_id(&request.workflow_job_id) || !valid_id(&request.experiment_id) {
        return Err("유효한 회의 작업 ID와 백테스트 실험 ID가 필요합니다.".to_owned());
    }
    let stored: (String, String, String, u64, Option<String>) = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의 백테스트 계보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT handoff_id,analysis_record_id,symbol,created_at_ms,experiment_id
                 FROM meeting_paper_handoffs WHERE workflow_job_id=?1",
                params![request.workflow_job_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("회의 인계를 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| "먼저 회의 분석을 백테스트 단계로 인계해 주세요.".to_owned())?
    };
    if stored
        .4
        .as_deref()
        .is_some_and(|id| id != request.experiment_id)
    {
        return Err("하나의 회의 인계에 다른 백테스트 실험을 덮어쓸 수 없습니다.".to_owned());
    }
    let (symbol, currency, record_json, created_at_ms): (String, String, String, u64) = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "백테스트 계보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT symbol,currency,record_json,created_at_ms FROM backtest_runs WHERE experiment_id=?1",
                params![request.experiment_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| format!("백테스트 기록을 조회하지 못했습니다: {error}"))?
            .ok_or_else(|| "완료된 백테스트 기록을 찾지 못했습니다.".to_owned())?
    };
    if currency != "KRW" {
        return Err("현재 회의 자동 섀도우 후보 연결은 KRW 내부 모의원장만 지원합니다.".to_owned());
    }
    if symbol != stored.2 || created_at_ms < stored.3 {
        return Err(
            "회의 인계 이후 같은 종목으로 실행한 백테스트만 연결할 수 있습니다.".to_owned(),
        );
    }
    let record: Value = serde_json::from_str(&record_json)
        .map_err(|error| format!("백테스트 기록을 해석하지 못했습니다: {error}"))?;
    if record
        .pointer("/review/executable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err("실행 가능 계약 검증을 통과한 백테스트만 연결할 수 있습니다.".to_owned());
    }
    let expected_source = format!("investa://analysis/{}", stored.1);
    let has_analysis_lineage = record
        .pointer("/report/evidence")
        .and_then(Value::as_array)
        .is_some_and(|evidence| {
            evidence.iter().any(|item| {
                item.get("kind").and_then(Value::as_str) == Some("local_analysis")
                    && item.get("sourceUrl").and_then(Value::as_str)
                        == Some(expected_source.as_str())
            })
        });
    if !has_analysis_lineage {
        return Err("백테스트가 이 회의의 불변 분석 기록을 근거로 참조하지 않습니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "회의 백테스트 계보 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    connection
        .execute(
            "UPDATE meeting_paper_handoffs SET experiment_id=?1,updated_at_ms=?2
             WHERE handoff_id=?3 AND (experiment_id IS NULL OR experiment_id=?1)",
            params![request.experiment_id, linked_at_ms, stored.0],
        )
        .map_err(|error| format!("회의와 백테스트 계보를 연결하지 못했습니다: {error}"))?;
    Ok(stored.0)
}

async fn finalize(
    market: &MarketDataBridge,
    crypto: &CryptoMarketBridge,
    bridge: &PersistenceBridge,
    runtime: &ShadowEngineRuntime,
    request: MeetingPaperHandoffFinalizeRequest,
) -> Result<MeetingPaperHandoff, String> {
    let linked_at_ms = now_ms()?;
    let handoff_id = link_backtest(bridge, &request, linked_at_ms)?;
    crate::operations::arm_shadow_watch(bridge, &request.experiment_id, Some(60))?;
    crate::operations::run_shadow_engine_once(market, crypto, bridge, runtime).await?;
    reconcile_paper_candidate_links(bridge, now_ms()?)?;
    load_handoff(bridge, &handoff_id)
}

fn history(bridge: &PersistenceBridge, limit: u16) -> Result<Vec<MeetingPaperHandoff>, String> {
    if limit == 0 || limit > MAX_HANDOFFS {
        return Err(format!(
            "회의 인계 조회 개수는 1~{MAX_HANDOFFS}개여야 합니다."
        ));
    }
    reconcile_paper_candidate_links(bridge, now_ms()?)?;
    reconcile_engine_links(bridge, now_ms()?)?;
    let stored = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "회의 인계 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT handoff_id,workflow_job_id,analysis_record_id,symbol,strategy,experiment_id,paper_candidate_id,engine_run_id,created_at_ms,updated_at_ms
                 FROM meeting_paper_handoffs ORDER BY updated_at_ms DESC,handoff_id DESC LIMIT ?1",
            )
            .map_err(|error| format!("회의 인계 목록 조회를 준비하지 못했습니다: {error}"))?;
        let rows = statement
            .query_map(params![limit], stored_handoff)
            .map_err(|error| format!("회의 인계 목록을 조회하지 못했습니다: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("회의 인계 목록을 읽지 못했습니다: {error}"))?
    };
    stored
        .into_iter()
        .map(|item| effective_status(bridge, item))
        .collect()
}

fn audit_stage(
    id: &'static str,
    label: &'static str,
    status: &'static str,
    detail: impl Into<String>,
) -> GoldenPathStage {
    GoldenPathStage {
        id,
        label,
        status,
        detail: detail.into(),
    }
}

fn golden_path_audit(
    bridge: &PersistenceBridge,
    workflow_job_id: &str,
    checked_at_ms: u64,
) -> Result<GoldenPathAudit, String> {
    if !valid_id(workflow_job_id) {
        return Err("유효한 회의 작업 ID가 필요합니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "골든패스 감사 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let stored = connection
        .query_row(
            "SELECT handoff_id,workflow_job_id,analysis_record_id,symbol,strategy,experiment_id,paper_candidate_id,engine_run_id,created_at_ms,updated_at_ms
             FROM meeting_paper_handoffs WHERE workflow_job_id=?1",
            params![workflow_job_id],
            stored_handoff,
        )
        .optional()
        .map_err(|error| format!("골든패스 회의 인계를 조회하지 못했습니다: {error}"))?
        .ok_or_else(|| "분석→모의원장 인계 기록이 없습니다.".to_owned())?;

    let analysis_ok: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM analysis_notes WHERE record_id=?1 AND kind='meeting' AND status='completed')",
            params![stored.analysis_record_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("불변 분석 기록을 확인하지 못했습니다: {error}"))?;
    let mut stages = vec![audit_stage(
        "analysis",
        "불변 분석 기록",
        if analysis_ok { "passed" } else { "failed" },
        if analysis_ok {
            format!(
                "{} 기록이 완료 상태로 보존됩니다.",
                stored.analysis_record_id
            )
        } else {
            "완료된 회의 분석 기록이 없거나 종류가 일치하지 않습니다.".to_owned()
        },
    )];

    let backtest = stored
        .experiment_id
        .as_deref()
        .map_or(Ok(None), |experiment_id| {
            connection
                .query_row(
                    "SELECT symbol,record_json FROM backtest_runs WHERE experiment_id=?1",
                    params![experiment_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|error| format!("백테스트 계보를 확인하지 못했습니다: {error}"))
        })?;
    let expected_source = format!("investa://analysis/{}", stored.analysis_record_id);
    let backtest_ok = backtest.as_ref().is_some_and(|(symbol, record_json)| {
        symbol == &stored.symbol
            && serde_json::from_str::<Value>(record_json)
                .ok()
                .is_some_and(|record| {
                    record
                        .pointer("/report/evidence")
                        .and_then(Value::as_array)
                        .is_some_and(|items| {
                            items.iter().any(|item| {
                                item.get("kind").and_then(Value::as_str) == Some("local_analysis")
                                    && item.get("sourceUrl").and_then(Value::as_str)
                                        == Some(expected_source.as_str())
                            })
                        })
                })
    });
    stages.push(audit_stage(
        "backtest",
        "시점 정합 백테스트",
        if backtest_ok {
            "passed"
        } else if stored.experiment_id.is_some() {
            "failed"
        } else {
            "pending"
        },
        if backtest_ok {
            format!(
                "{} 실험이 분석 근거 ID와 종목을 정확히 참조합니다.",
                stored.experiment_id.as_deref().unwrap_or_default()
            )
        } else if stored.experiment_id.is_some() {
            "연결된 백테스트의 종목 또는 불변 분석 근거 계보가 일치하지 않습니다.".to_owned()
        } else {
            "회의 이후 실행한 백테스트 연결을 기다립니다.".to_owned()
        },
    ));

    let candidate = if let Some(candidate_id) = &stored.paper_candidate_id {
        connection
            .query_row(
                "SELECT status,currency FROM paper_order_candidates WHERE candidate_id=?1",
                params![candidate_id],
                |row| {
                    Ok((
                        candidate_id.clone(),
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        "paper",
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("섀도우 모의후보를 확인하지 못했습니다: {error}"))?
    } else if let Some(run_id) = &stored.engine_run_id {
        connection
            .query_row(
                "SELECT candidate_id,status,currency FROM engine_order_candidates WHERE run_id=?1",
                params![run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        "engine",
                    ))
                },
            )
            .optional()
            .map_err(|error| format!("엔진 모의후보를 확인하지 못했습니다: {error}"))?
    } else {
        None
    };
    let candidate_status = candidate.as_ref().map(|item| item.1.as_str());
    let safety_ok = candidate_status.is_some_and(|status| {
        matches!(
            status,
            "safety_approved" | "user_approved" | "submitted" | "partially_filled" | "filled"
        )
    });
    stages.push(audit_stage(
        "safety",
        "결정론적 안전 후보",
        if safety_ok {
            "passed"
        } else if candidate.is_some() {
            "failed"
        } else {
            "pending"
        },
        candidate.as_ref().map_or_else(
            || "백테스트 통과 후 안전 게이트가 만든 모의후보를 기다립니다.".to_owned(),
            |(id, status, _, _)| format!("{id} · 현재 상태 {status}"),
        ),
    ));

    let user_approved = candidate_status.is_some_and(|status| {
        matches!(
            status,
            "user_approved" | "submitted" | "partially_filled" | "filled"
        )
    });
    stages.push(audit_stage(
        "user_approval",
        "사용자 명시 승인",
        if user_approved {
            "passed"
        } else if safety_ok {
            "pending"
        } else {
            "blocked"
        },
        if user_approved {
            "로컬 사용자의 승인 상태 전이가 확인됐습니다.".to_owned()
        } else {
            "AI 판단만으로 체결하지 않으며 로컬 사용자 승인을 기다립니다.".to_owned()
        },
    ));

    drop(connection);
    let ledger_filled = if let Some((candidate_id, status, currency, _kind)) = &candidate {
        if status == "filled" {
            let ledger_id = crate::paper_trading::ledger_id_for_currency(currency)?;
            let ledger = bridge.paper_ledger(ledger_id)?;
            ledger.events().iter().any(|event| {
                matches!(
                    event,
                    crate::paper_account::LedgerEvent::OrderFilled { idempotency_key, .. }
                        if idempotency_key == candidate_id.as_str()
                )
            })
        } else {
            false
        }
    } else {
        false
    };
    stages.push(audit_stage(
        "ledger",
        "내부 불변 모의원장",
        if ledger_filled {
            "passed"
        } else if user_approved {
            "pending"
        } else {
            "blocked"
        },
        if ledger_filled {
            "후보 ID와 같은 멱등키의 내부 모의체결이 불변 원장에서 재생됩니다.".to_owned()
        } else {
            "실전 전송 없이 내부 모의체결 및 원장 재생 확인을 기다립니다.".to_owned()
        },
    ));

    let status = if stages.iter().any(|stage| stage.status == "failed") {
        "failed"
    } else if stages.iter().all(|stage| stage.status == "passed") {
        "passed"
    } else {
        "pending"
    };
    Ok(GoldenPathAudit {
        workflow_job_id: stored.workflow_job_id,
        handoff_id: stored.handoff_id,
        status: status.to_owned(),
        stages,
        checked_at_ms,
        live_order_enabled: false,
    })
}

#[tauri::command]
pub fn meeting_paper_handoff_prepare(
    request: MeetingPaperHandoffPrepareRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<MeetingPaperHandoff, String> {
    prepare(&bridge, request, now_ms()?)
}

#[tauri::command]
pub async fn meeting_paper_handoff_finalize(
    request: MeetingPaperHandoffFinalizeRequest,
    market: State<'_, MarketDataBridge>,
    crypto: State<'_, CryptoMarketBridge>,
    bridge: State<'_, PersistenceBridge>,
    runtime: State<'_, ShadowEngineRuntime>,
) -> Result<MeetingPaperHandoff, String> {
    finalize(&market, &crypto, &bridge, &runtime, request).await
}

#[tauri::command]
pub fn meeting_paper_handoff_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<MeetingPaperHandoff>, String> {
    history(&bridge, limit)
}

#[tauri::command]
pub fn meeting_paper_golden_path_audit(
    workflow_job_id: String,
    bridge: State<'_, PersistenceBridge>,
) -> Result<GoldenPathAudit, String> {
    golden_path_audit(&bridge, &workflow_job_id, now_ms()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seed_completed_meeting(bridge: &PersistenceBridge, job_id: &str, decision: &str) {
        let analysis_id = format!("analysis-{job_id}");
        let connection = bridge.connection.lock().expect("database");
        connection.execute("INSERT INTO workflow_jobs(job_id,topic,importance,stage,status,selected_departments_json,reports_json,synthesis_json,created_at_ms,updated_at_ms) VALUES(?1,'한화 분석','normal','results','completed','[]','{}',?2,1,1)",params![job_id,json!({"decision":decision,"summary":"요약","consensus":[],"disagreements":[],"conditions":[],"backtestRecommendation":{"required":true,"symbol":"000880","strategy":"5/20 이동평균 교차","reason":"검증 필요"}}).to_string()]).expect("workflow");
        connection.execute("INSERT INTO analysis_notes(record_id,kind,status,market,title,symbol,currency,requested_at_ms,content_json,created_at_ms) VALUES(?1,'meeting','completed','kr','한화 분석','000880',NULL,NULL,'{}',1)",params![analysis_id]).expect("analysis");
    }

    fn seed_backtest(bridge: &PersistenceBridge, experiment_id: &str, analysis_id: &str) {
        let connection = bridge.connection.lock().expect("database");
        let trace_id = format!("trace-{experiment_id}");
        let dataset_id = format!("dataset-{experiment_id}");
        connection.execute("INSERT INTO research_reports(trace_id,strategy_id,symbol,currency,report_json,review_json,created_at_ms) VALUES(?1,'strategy-1','000880','KRW','{}','{}',11)",params![trace_id]).expect("report");
        connection.execute("INSERT INTO datasets(dataset_id,provider,symbol,currency,interval,adjusted,bar_count,first_period_start_ms,last_available_at_ms,ingested_at_ms,bars_json,created_at_ms) VALUES(?1,'fixture','000880','KRW','1d',1,1,1,2,2,'{\"bars\":[]}',11)",params![dataset_id]).expect("dataset");
        let record = json!({
            "review": {"executable": true},
            "report": {"evidence": [{"kind":"local_analysis","sourceUrl":format!("investa://analysis/{analysis_id}")}]},
            "result": {"maxDrawdownBps": 10},
            "config": {"orderQuantity": 1}
        });
        connection.execute("INSERT INTO backtest_runs(experiment_id,trace_id,dataset_id,strategy_id,strategy_name,symbol,currency,provider,interval,adjusted,bar_count,total_return_bps,max_drawdown_bps,completed_trade_count,classification,record_json,created_at_ms) VALUES(?1,?2,?3,'strategy-1','5/20 이동평균 교차','000880','KRW','fixture','1d',1,1,10,10,1,'research_experiment',?4,11)",params![experiment_id,trace_id,dataset_id,record.to_string()]).expect("backtest");
    }

    #[test]
    fn prepares_idempotent_handoff_without_creating_an_order() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_meeting(&bridge, "job-1", "paper_candidate");
        let first = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-1".to_owned(),
            },
            10,
        )
        .expect("handoff");
        let replay = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-1".to_owned(),
            },
            20,
        )
        .expect("replay");
        assert_eq!(first.handoff_id, replay.handoff_id);
        assert_eq!(replay.status, "awaiting_backtest");
        assert!(!replay.live_order_enabled);
        let candidate_count: u64 = bridge
            .connection
            .lock()
            .expect("database")
            .query_row("SELECT COUNT(*) FROM engine_order_candidates", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(candidate_count, 0);
        let paper_candidate_count: u64 = bridge
            .connection
            .lock()
            .expect("database")
            .query_row("SELECT COUNT(*) FROM paper_order_candidates", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(paper_candidate_count, 0);
    }

    #[test]
    fn refuses_non_candidate_meeting() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_meeting(&bridge, "job-2", "hold");
        let error = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-2".to_owned(),
            },
            10,
        )
        .expect_err("blocked");
        assert!(error.contains("모의투자 후보"));
    }

    #[test]
    fn restart_reconciliation_links_only_matching_analysis_and_symbol() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_meeting(&bridge, "job-3", "paper_candidate");
        let handoff = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-3".to_owned(),
            },
            10,
        )
        .expect("handoff");
        {
            let connection = bridge.connection.lock().expect("database");
            connection.execute("INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms) VALUES('wrong-run','wrong-key','completed','000880','korea',1,?1,'{\"blockers\":[]}',11,11)", params![json!({"analysisIds":[handoff.analysis_record_id],"tradePlan":{"symbol":"005930"}}).to_string()]).expect("wrong run");
            connection.execute("INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms) VALUES('matching-run','matching-key','completed','000880','korea',1,?1,'{\"blockers\":[]}',12,12)", params![json!({"analysisIds":[handoff.analysis_record_id],"tradePlan":{"symbol":"000880"}}).to_string()]).expect("matching run");
        }
        let recovered = history(&bridge, 10).expect("history");
        assert_eq!(recovered[0].engine_run_id.as_deref(), Some("matching-run"));
        assert_eq!(recovered[0].status, "candidate_ready");
        assert!(!recovered[0].live_order_enabled);
    }

    #[test]
    fn links_only_a_post_handoff_backtest_with_exact_local_analysis_lineage() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_meeting(&bridge, "job-4", "paper_candidate");
        let handoff = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-4".to_owned(),
            },
            10,
        )
        .expect("handoff");
        seed_backtest(&bridge, "experiment-4", &handoff.analysis_record_id);
        let handoff_id = link_backtest(
            &bridge,
            &MeetingPaperHandoffFinalizeRequest {
                workflow_job_id: "job-4".to_owned(),
                experiment_id: "experiment-4".to_owned(),
            },
            12,
        )
        .expect("linked");
        let linked = load_handoff(&bridge, &handoff_id).expect("status");
        assert_eq!(linked.experiment_id.as_deref(), Some("experiment-4"));
        assert_eq!(linked.status, "backtest_completed");
        assert!(!linked.live_order_enabled);

        seed_backtest(&bridge, "experiment-other", "analysis-someone-else");
        let error = link_backtest(
            &bridge,
            &MeetingPaperHandoffFinalizeRequest {
                workflow_job_id: "job-4".to_owned(),
                experiment_id: "experiment-other".to_owned(),
            },
            13,
        )
        .expect_err("immutable lineage");
        assert!(error.contains("덮어쓸 수 없습니다"));
    }

    #[test]
    fn golden_path_audit_reports_lineage_and_never_enables_live_orders() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_meeting(&bridge, "job-golden", "paper_candidate");
        let handoff = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-golden".to_owned(),
            },
            10,
        )
        .expect("handoff");
        seed_backtest(&bridge, "experiment-golden", &handoff.analysis_record_id);
        link_backtest(
            &bridge,
            &MeetingPaperHandoffFinalizeRequest {
                workflow_job_id: "job-golden".to_owned(),
                experiment_id: "experiment-golden".to_owned(),
            },
            12,
        )
        .expect("link");
        let audit = golden_path_audit(&bridge, "job-golden", 20).expect("audit");
        assert_eq!(audit.status, "pending");
        assert!(!audit.live_order_enabled);
        assert_eq!(audit.stages[0].status, "passed");
        assert_eq!(audit.stages[1].status, "passed");
        assert_eq!(audit.stages[2].status, "pending");
        let candidate_count: u64 = bridge
            .connection
            .lock()
            .expect("database")
            .query_row("SELECT COUNT(*) FROM engine_order_candidates", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(candidate_count, 0);
    }

    #[test]
    fn golden_path_audit_passes_only_after_user_approved_internal_fill() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_meeting(&bridge, "job-complete", "paper_candidate");
        let handoff = prepare(
            &bridge,
            MeetingPaperHandoffPrepareRequest {
                workflow_job_id: "job-complete".to_owned(),
            },
            10,
        )
        .expect("handoff");
        seed_backtest(&bridge, "experiment-complete", &handoff.analysis_record_id);
        link_backtest(
            &bridge,
            &MeetingPaperHandoffFinalizeRequest {
                workflow_job_id: "job-complete".to_owned(),
                experiment_id: "experiment-complete".to_owned(),
            },
            12,
        )
        .expect("link");

        {
            let connection = bridge.connection.lock().expect("database");
            let input = json!({
                "analysisIds": [handoff.analysis_record_id],
                "tradePlan": {"symbol": "000880"}
            });
            connection.execute(
                "INSERT INTO engine_runs(run_id,idempotency_key,status,symbol,market,candidate_ready,input_json,report_json,created_at_ms,updated_at_ms)
                 VALUES('run-complete','key-complete','completed','000880','korea',1,?1,'{\"blockers\":[]}',13,13)",
                params![input.to_string()],
            ).expect("engine run");
            connection.execute(
                "INSERT INTO engine_order_candidates(candidate_id,run_id,symbol,market,currency,side,quantity,quantity_scale,reference_price_minor,valid_until_ms,status,safety_json,created_at_ms,updated_at_ms)
                 VALUES('engine-cand-complete','run-complete','000880','korea','KRW','buy',1,1,70000,60000,'filled','{}',14,18)",
                [],
            ).expect("candidate");
            for (index, event_type) in [
                "candidate_created",
                "safety_approved",
                "user_approved",
                "submitted",
                "filled",
            ]
            .iter()
            .enumerate()
            {
                connection.execute(
                    "INSERT INTO engine_order_events(candidate_id,event_index,event_type,event_json,occurred_at_ms)
                     VALUES('engine-cand-complete',?1,?2,'{}',?3)",
                    params![index as u64, event_type, 14 + index as u64],
                ).expect("candidate event");
            }
        }
        history(&bridge, 10).expect("reconcile engine lineage");
        crate::paper_trading::load_or_open_account_for_currency(&bridge, "KRW")
            .expect("paper account");
        let mut ledger = bridge
            .paper_ledger(crate::paper_trading::PAPER_LEDGER_ID)
            .expect("ledger");
        let filled_at_ms = ledger
            .events()
            .last()
            .map(crate::paper_account::LedgerEvent::occurred_at_ms)
            .unwrap_or(19)
            + 1;
        ledger
            .append(crate::paper_account::LedgerEvent::OrderFilled {
                account_id: crate::paper_trading::PAPER_ACCOUNT_ID.to_owned(),
                order_id: "shadow-order-complete".to_owned(),
                idempotency_key: "engine-cand-complete".to_owned(),
                symbol: "000880".to_owned(),
                side: crate::trading::TradeSide::Buy,
                quantity: 1,
                quantity_scale: 1,
                reference_price_minor: 70_000,
                execution_price_minor: 70_000,
                notional_minor: 70_000,
                fee_minor: 0,
                tax_minor: 0,
                costs: crate::simulation::TradingCosts {
                    buy_fee_bps: 0.0,
                    sell_fee_bps: 0.0,
                    sell_tax_bps: 0.0,
                    slippage_bps: 0.0,
                },
                exit_reason: None,
                cause_event_id: None,
                occurred_at_ms: filled_at_ms,
            })
            .expect("internal fill");

        let audit = golden_path_audit(&bridge, "job-complete", 20).expect("audit");
        assert_eq!(audit.status, "passed");
        assert!(audit.stages.iter().all(|stage| stage.status == "passed"));
        assert!(!audit.live_order_enabled);
    }
}
