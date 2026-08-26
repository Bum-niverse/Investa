use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    backtest::{BacktestTrade, PriceBar},
    paper_account::{replay_ledger, AppendOnlyLedger},
    paper_trading::ledger_id_for_currency,
    persistence::{self, PersistenceBridge},
};

const MIN_ROBUSTNESS_TRADES: usize = 5;
const MIN_RETURN_OBSERVATIONS: usize = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestRobustnessReport {
    pub method: String,
    pub seed: u64,
    pub iteration_count: usize,
    pub trade_sample_count: usize,
    pub computed: bool,
    pub median_return_bps: Option<i64>,
    pub lower_return_bps: Option<i64>,
    pub upper_return_bps: Option<i64>,
    pub probability_of_loss_bps: Option<u64>,
    pub probability_of_ruin_bps: Option<u64>,
    pub worst_path_drawdown_bps: Option<u64>,
    pub warning: String,
}

#[derive(Debug, Clone, Copy)]
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state = value;
        value
    }
}

fn percentile(sorted: &[i64], numerator: usize, denominator: usize) -> i64 {
    let index = (sorted.len().saturating_sub(1) * numerator) / denominator;
    sorted[index]
}

fn stable_seed(parts: &[&str]) -> u64 {
    parts
        .iter()
        .flat_map(|part| part.as_bytes().iter().copied().chain([0xff]))
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
        })
}

pub fn backtest_robustness(
    experiment_id: &str,
    dataset_id: &str,
    initial_cash_minor: u64,
    trades: &[BacktestTrade],
) -> BacktestRobustnessReport {
    let seed = stable_seed(&[experiment_id, dataset_id]);
    let iteration_count = 2_000;
    if initial_cash_minor == 0 || trades.len() < MIN_ROBUSTNESS_TRADES {
        return BacktestRobustnessReport {
            method: "empirical_trade_bootstrap_v1".to_owned(),
            seed,
            iteration_count,
            trade_sample_count: trades.len(),
            computed: false,
            median_return_bps: None,
            lower_return_bps: None,
            upper_return_bps: None,
            probability_of_loss_bps: None,
            probability_of_ruin_bps: None,
            worst_path_drawdown_bps: None,
            warning: format!(
                "완료 거래가 {MIN_ROBUSTNESS_TRADES}건 미만이어서 경험적 부트스트랩을 계산하지 않았습니다."
            ),
        };
    }

    let initial = i128::from(initial_cash_minor);
    let ruin_floor = initial / 2;
    let mut rng = XorShift64::new(seed);
    let mut returns = Vec::with_capacity(iteration_count);
    let mut loss_count = 0usize;
    let mut ruin_count = 0usize;
    let mut worst_drawdown_bps = 0u64;

    for _ in 0..iteration_count {
        let mut equity = initial;
        let mut peak = initial;
        let mut path_ruined = false;
        let mut path_drawdown = 0u64;
        for _ in 0..trades.len() {
            let index = (rng.next() as usize) % trades.len();
            equity = equity.saturating_add(i128::from(trades[index].pnl_minor));
            peak = peak.max(equity);
            if equity <= ruin_floor {
                path_ruined = true;
            }
            if peak > 0 && equity < peak {
                let drawdown = ((peak - equity).saturating_mul(10_000) / peak).clamp(0, 10_000);
                path_drawdown = path_drawdown.max(drawdown as u64);
            }
        }
        let return_bps = ((equity - initial).saturating_mul(10_000) / initial)
            .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64;
        loss_count += usize::from(return_bps < 0);
        ruin_count += usize::from(path_ruined);
        worst_drawdown_bps = worst_drawdown_bps.max(path_drawdown);
        returns.push(return_bps);
    }
    returns.sort_unstable();

    BacktestRobustnessReport {
        method: "empirical_trade_bootstrap_v1".to_owned(),
        seed,
        iteration_count,
        trade_sample_count: trades.len(),
        computed: true,
        median_return_bps: Some(percentile(&returns, 50, 100)),
        lower_return_bps: Some(percentile(&returns, 5, 100)),
        upper_return_bps: Some(percentile(&returns, 95, 100)),
        probability_of_loss_bps: Some((loss_count as u64 * 10_000) / iteration_count as u64),
        probability_of_ruin_bps: Some((ruin_count as u64 * 10_000) / iteration_count as u64),
        worst_path_drawdown_bps: Some(worst_drawdown_bps),
        warning: "거래 손익을 독립·동일분포로 재표집한 민감도 검사이며 시장 레짐·유동성 변화나 미래 성과를 보장하지 않습니다.".to_owned(),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioReturnPoint {
    pub period_end_ms: u64,
    pub available_at_ms: u64,
    pub return_ppm: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRiskPosition {
    pub symbol: String,
    pub currency: String,
    pub weight_bps: u64,
    pub returns: Vec<PortfolioReturnPoint>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRiskRequest {
    pub as_of_ms: u64,
    pub positions: Vec<PortfolioRiskPosition>,
    #[serde(default)]
    pub stress_shocks_bps: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelationCell {
    pub left_symbol: String,
    pub right_symbol: String,
    pub correlation_milli: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRiskReport {
    pub as_of_ms: u64,
    pub currency: String,
    pub position_count: usize,
    pub observation_count: usize,
    pub historical_var_95_bps: Option<u64>,
    pub historical_cvar_95_bps: Option<u64>,
    pub historical_var_99_bps: Option<u64>,
    pub concentration_hhi_bps: u64,
    pub stressed_portfolio_return_bps: Option<i64>,
    pub correlations: Vec<CorrelationCell>,
    pub warnings: Vec<String>,
}

fn validate_position(position: &PortfolioRiskPosition, as_of_ms: u64) -> Result<(), String> {
    if position.symbol.trim().is_empty()
        || position.symbol.len() > 32
        || position.currency.len() < 3
        || position.currency.len() > 8
        || position.weight_bps == 0
        || position.weight_bps > 10_000
        || position.returns.is_empty()
        || position.returns.len() > 20_000
    {
        return Err("포트폴리오 종목·통화·비중을 확인해 주세요.".to_owned());
    }
    let mut previous = 0u64;
    for point in &position.returns {
        if point.period_end_ms == 0
            || point.period_end_ms <= previous
            || point.available_at_ms < point.period_end_ms
            || point.available_at_ms > as_of_ms
            || point.return_ppm.unsigned_abs() > 10_000_000
        {
            return Err("수익률 관측의 시점 순서·가용 시각·값 범위를 확인해 주세요.".to_owned());
        }
        previous = point.period_end_ms;
    }
    Ok(())
}

fn historical_loss_bps(sorted_returns_ppm: &[i64], tail_percent: usize) -> (u64, u64) {
    let tail_count = (sorted_returns_ppm.len() * tail_percent)
        .div_ceil(100)
        .max(1);
    let tail = &sorted_returns_ppm[..tail_count];
    let threshold = tail[tail.len() - 1];
    let mean = tail.iter().map(|value| i128::from(*value)).sum::<i128>() / tail.len() as i128;
    (
        ((-threshold).max(0) as u64) / 100,
        ((-mean).max(0) as u64) / 100,
    )
}

fn correlation(left: &[i64], right: &[i64]) -> Option<i64> {
    if left.len() != right.len() || left.len() < 2 {
        return None;
    }
    let left_mean = left.iter().map(|v| *v as f64).sum::<f64>() / left.len() as f64;
    let right_mean = right.iter().map(|v| *v as f64).sum::<f64>() / right.len() as f64;
    let mut covariance = 0.0;
    let mut left_variance = 0.0;
    let mut right_variance = 0.0;
    for (left_value, right_value) in left.iter().zip(right) {
        let left_delta = *left_value as f64 - left_mean;
        let right_delta = *right_value as f64 - right_mean;
        covariance += left_delta * right_delta;
        left_variance += left_delta * left_delta;
        right_variance += right_delta * right_delta;
    }
    let denominator = (left_variance * right_variance).sqrt();
    (denominator > 0.0).then(|| ((covariance / denominator) * 1_000.0).round() as i64)
}

pub fn analyze_portfolio_risk(
    request: PortfolioRiskRequest,
) -> Result<PortfolioRiskReport, String> {
    if request.as_of_ms == 0 || request.positions.is_empty() || request.positions.len() > 100 {
        return Err("기준 시각과 1~100개 포트폴리오 종목이 필요합니다.".to_owned());
    }
    for position in &request.positions {
        validate_position(position, request.as_of_ms)?;
    }
    if request.stress_shocks_bps.len() > request.positions.len()
        || request.stress_shocks_bps.iter().any(|(symbol, shock)| {
            !request
                .positions
                .iter()
                .any(|position| position.symbol == *symbol)
                || shock.unsigned_abs() > 100_000
        })
    {
        return Err(
            "스트레스 충격은 포트폴리오 종목에만 -100,000~100,000bp 범위로 지정해 주세요."
                .to_owned(),
        );
    }
    let currency = request.positions[0].currency.trim().to_ascii_uppercase();
    if request
        .positions
        .iter()
        .any(|position| position.currency.trim().to_ascii_uppercase() != currency)
    {
        return Err("시점 정합 환율 없이 서로 다른 통화의 위험을 합산할 수 없습니다.".to_owned());
    }
    let weight_sum = request
        .positions
        .iter()
        .try_fold(0u64, |sum, position| sum.checked_add(position.weight_bps))
        .ok_or_else(|| "포트폴리오 비중 합계가 범위를 초과했습니다.".to_owned())?;
    if weight_sum != 10_000 {
        return Err("포트폴리오 비중 합계는 정확히 10,000bp여야 합니다.".to_owned());
    }
    let timestamps: Vec<u64> = request.positions[0]
        .returns
        .iter()
        .map(|point| point.period_end_ms)
        .collect();
    if request.positions.iter().any(|position| {
        position.returns.len() != timestamps.len()
            || position
                .returns
                .iter()
                .zip(&timestamps)
                .any(|(point, timestamp)| point.period_end_ms != *timestamp)
    }) {
        return Err("종목별 수익률은 동일한 기간 끝 시각으로 정렬되어야 합니다.".to_owned());
    }

    let mut portfolio_returns = Vec::with_capacity(timestamps.len());
    for index in 0..timestamps.len() {
        let weighted = request
            .positions
            .iter()
            .try_fold(0i128, |sum, position| {
                sum.checked_add(
                    i128::from(position.returns[index].return_ppm)
                        .saturating_mul(i128::from(position.weight_bps)),
                )
            })
            .ok_or_else(|| "가중 포트폴리오 수익률이 범위를 초과했습니다.".to_owned())?;
        portfolio_returns.push((weighted / 10_000) as i64);
    }
    let concentration_hhi_bps = request.positions.iter().fold(0u128, |sum, position| {
        sum + u128::from(position.weight_bps).pow(2)
    }) / 10_000;
    let stressed = if request.stress_shocks_bps.is_empty() {
        None
    } else {
        Some(
            request
                .positions
                .iter()
                .try_fold(0i128, |sum, position| {
                    let shock = request
                        .stress_shocks_bps
                        .get(&position.symbol)
                        .copied()
                        .unwrap_or_default();
                    sum.checked_add(i128::from(shock) * i128::from(position.weight_bps))
                })
                .ok_or_else(|| "스트레스 충격 합계가 범위를 초과했습니다.".to_owned())?
                / 10_000,
        )
    };
    let mut correlations = Vec::new();
    for left in 0..request.positions.len() {
        for right in left + 1..request.positions.len() {
            correlations.push(CorrelationCell {
                left_symbol: request.positions[left].symbol.clone(),
                right_symbol: request.positions[right].symbol.clone(),
                correlation_milli: correlation(
                    &request.positions[left]
                        .returns
                        .iter()
                        .map(|point| point.return_ppm)
                        .collect::<Vec<_>>(),
                    &request.positions[right]
                        .returns
                        .iter()
                        .map(|point| point.return_ppm)
                        .collect::<Vec<_>>(),
                ),
            });
        }
    }
    let mut warnings = vec![
        "역사적 VaR·CVaR는 관측된 과거 분포를 재사용하며 급격한 구조 변화와 유동성 손실을 보장하지 않습니다.".to_owned(),
    ];
    let (var_95, cvar_95, var_99) = if portfolio_returns.len() >= MIN_RETURN_OBSERVATIONS {
        let mut sorted = portfolio_returns.clone();
        sorted.sort_unstable();
        let (var_95, cvar_95) = historical_loss_bps(&sorted, 5);
        let var_99 = (sorted.len() >= 100).then(|| historical_loss_bps(&sorted, 1).0);
        if var_99.is_none() {
            warnings
                .push("99% 역사적 VaR는 최소 100개 관측이 필요해 표시하지 않았습니다.".to_owned());
        }
        (Some(var_95), Some(cvar_95), var_99)
    } else {
        warnings.push(format!(
            "수익률 관측이 {MIN_RETURN_OBSERVATIONS}개 미만이어서 VaR·CVaR를 표시하지 않았습니다."
        ));
        (None, None, None)
    };

    Ok(PortfolioRiskReport {
        as_of_ms: request.as_of_ms,
        currency,
        position_count: request.positions.len(),
        observation_count: timestamps.len(),
        historical_var_95_bps: var_95,
        historical_cvar_95_bps: cvar_95,
        historical_var_99_bps: var_99,
        concentration_hhi_bps: concentration_hhi_bps as u64,
        stressed_portfolio_return_bps: stressed.map(|value| value as i64),
        correlations,
        warnings,
    })
}

#[tauri::command]
pub fn portfolio_risk_analyze(
    request: PortfolioRiskRequest,
) -> Result<PortfolioRiskReport, String> {
    analyze_portfolio_risk(request)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRiskSnapshotSaveRequest {
    pub snapshot_id: String,
    pub request: PortfolioRiskRequest,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredPortfolioRiskSnapshot {
    pub snapshot_id: String,
    pub request: PortfolioRiskRequest,
    pub report: PortfolioRiskReport,
    pub created_at_ms: u64,
}

fn valid_snapshot_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

pub(crate) fn save_portfolio_risk_snapshot(
    bridge: &PersistenceBridge,
    request: PortfolioRiskSnapshotSaveRequest,
) -> Result<StoredPortfolioRiskSnapshot, String> {
    if !valid_snapshot_id(&request.snapshot_id) {
        return Err("위험 스냅샷 ID는 1~128자의 영문·숫자·-_.:만 사용할 수 있습니다.".to_owned());
    }
    let report = analyze_portfolio_risk(request.request.clone())?;
    let created_at_ms = persistence::now_ms()?;
    if request.request.as_of_ms > created_at_ms {
        return Err("미래 시각의 포트폴리오 위험 스냅샷을 저장할 수 없습니다.".to_owned());
    }
    let stored = StoredPortfolioRiskSnapshot {
        snapshot_id: request.snapshot_id,
        request: request.request,
        report,
        created_at_ms,
    };
    let request_json = serde_json::to_string(&stored.request)
        .map_err(|_| "포트폴리오 위험 입력을 직렬화하지 못했습니다.".to_owned())?;
    let report_json = serde_json::to_string(&stored.report)
        .map_err(|_| "포트폴리오 위험 결과를 직렬화하지 못했습니다.".to_owned())?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "포트폴리오 위험 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let existing = connection
        .query_row(
            "SELECT request_json, report_json FROM portfolio_risk_snapshots WHERE snapshot_id = ?1",
            params![stored.snapshot_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("포트폴리오 위험 스냅샷을 확인하지 못했습니다: {error}"))?;
    match existing {
        Some((existing_request, existing_report))
            if existing_request == request_json && existing_report == report_json => {}
        Some(_) => {
            return Err("같은 스냅샷 ID에 다른 위험 결과가 저장되어 있습니다.".to_owned());
        }
        None => {
            connection
                .execute(
                    "INSERT INTO portfolio_risk_snapshots
                     (snapshot_id, as_of_ms, currency, request_json, report_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        stored.snapshot_id,
                        stored.report.as_of_ms,
                        stored.report.currency,
                        request_json,
                        report_json,
                        stored.created_at_ms
                    ],
                )
                .map_err(|error| {
                    format!("포트폴리오 위험 스냅샷을 저장하지 못했습니다: {error}")
                })?;
        }
    }
    Ok(stored)
}

#[tauri::command]
pub fn portfolio_risk_snapshot_save(
    request: PortfolioRiskSnapshotSaveRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StoredPortfolioRiskSnapshot, String> {
    save_portfolio_risk_snapshot(&bridge, request)
}

#[tauri::command]
pub fn portfolio_risk_snapshot_history(
    limit: u16,
    bridge: State<'_, PersistenceBridge>,
) -> Result<Vec<StoredPortfolioRiskSnapshot>, String> {
    let limit = limit.clamp(1, 100);
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "포트폴리오 위험 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut statement = connection
        .prepare(
            "SELECT snapshot_id, request_json, report_json, created_at_ms
             FROM portfolio_risk_snapshots ORDER BY created_at_ms DESC, snapshot_id DESC LIMIT ?1",
        )
        .map_err(|error| format!("포트폴리오 위험 이력을 준비하지 못했습니다: {error}"))?;
    let rows = statement
        .query_map(params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u64>(3)?,
            ))
        })
        .map_err(|error| format!("포트폴리오 위험 이력을 조회하지 못했습니다: {error}"))?;
    rows.map(|row| {
        let (snapshot_id, request_json, report_json, created_at_ms) =
            row.map_err(|error| format!("포트폴리오 위험 이력을 읽지 못했습니다: {error}"))?;
        Ok(StoredPortfolioRiskSnapshot {
            snapshot_id,
            request: serde_json::from_str(&request_json)
                .map_err(|_| "저장된 포트폴리오 위험 입력을 해석하지 못했습니다.".to_owned())?,
            report: serde_json::from_str(&report_json)
                .map_err(|_| "저장된 포트폴리오 위험 결과를 해석하지 못했습니다.".to_owned())?,
            created_at_ms,
        })
    })
    .collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerPortfolioRiskRequest {
    pub snapshot_id: String,
    pub currency: String,
    #[serde(default)]
    pub stress_shocks_bps: BTreeMap<String, i64>,
}

#[tauri::command]
pub fn portfolio_risk_from_ledger(
    request: LedgerPortfolioRiskRequest,
    bridge: State<'_, PersistenceBridge>,
) -> Result<StoredPortfolioRiskSnapshot, String> {
    let currency = request.currency.trim().to_ascii_uppercase();
    let ledger = bridge.paper_ledger(ledger_id_for_currency(&currency)?)?;
    let account = replay_ledger(ledger.events()).map_err(|error| error.message)?;
    if account.positions.is_empty() {
        return Err("선택 통화의 내부 모의원장에 포지션이 없습니다.".to_owned());
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "가격 데이터 저장소 잠금을 획득하지 못했습니다.".to_owned())?;
    let mut series = Vec::<(String, u64, Vec<PriceBar>)>::new();
    for position in account.positions.values() {
        let raw: String = connection.query_row(
            "SELECT bars_json FROM datasets WHERE symbol=?1 AND currency=?2 ORDER BY last_available_at_ms DESC, created_at_ms DESC LIMIT 1",
            params![position.symbol, currency], |row| row.get(0),
        ).map_err(|_| format!("{}의 저장된 시점 정합 가격 데이터가 없습니다.", position.symbol))?;
        let value: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|_| "저장 가격 데이터 형식이 올바르지 않습니다.".to_owned())?;
        let bars: Vec<PriceBar> =
            serde_json::from_value(value.get("bars").cloned().unwrap_or_default())
                .map_err(|_| "저장 가격봉을 해석하지 못했습니다.".to_owned())?;
        if bars.len() < 31 {
            return Err(format!(
                "{}의 위험 계산 가격봉이 31개 미만입니다.",
                position.symbol
            ));
        }
        series.push((position.symbol.clone(), position.cost_basis_minor, bars));
    }
    drop(connection);
    let common = series
        .iter()
        .map(|(_, _, bars)| {
            bars.iter()
                .map(|bar| bar.period_end_ms)
                .collect::<BTreeSet<_>>()
        })
        .reduce(|left, right| left.intersection(&right).copied().collect())
        .unwrap_or_default();
    let timestamps = common.into_iter().collect::<Vec<_>>();
    if timestamps.len() < MIN_RETURN_OBSERVATIONS + 1 {
        return Err("모든 보유종목에 공통인 수익률 관측이 30개 미만입니다.".to_owned());
    }
    let total_cost = series
        .iter()
        .try_fold(0u64, |sum, (_, cost, _)| sum.checked_add(*cost))
        .ok_or_else(|| "포지션 원가 합계가 범위를 초과했습니다.".to_owned())?;
    if total_cost == 0 {
        return Err("포트폴리오 원가 합계는 0보다 커야 합니다.".to_owned());
    }
    let mut positions = Vec::new();
    let mut allocated = 0u64;
    for (index, (symbol, cost, bars)) in series.iter().enumerate() {
        let map = bars
            .iter()
            .map(|bar| (bar.period_end_ms, bar))
            .collect::<BTreeMap<_, _>>();
        let selected = timestamps
            .iter()
            .filter_map(|timestamp| map.get(timestamp).copied())
            .collect::<Vec<_>>();
        if selected.windows(2).any(|pair| pair[0].close_minor == 0) {
            return Err(format!(
                "{}의 저장 가격 데이터에 0 종가가 있습니다.",
                symbol
            ));
        }
        let returns = selected
            .windows(2)
            .map(|pair| PortfolioReturnPoint {
                period_end_ms: pair[1].period_end_ms,
                available_at_ms: pair[1].available_at_ms,
                return_ppm: ((i128::from(pair[1].close_minor) - i128::from(pair[0].close_minor))
                    * 1_000_000
                    / i128::from(pair[0].close_minor)) as i64,
            })
            .collect();
        let weight_bps = if index + 1 == series.len() {
            10_000 - allocated
        } else {
            u64::try_from(u128::from(*cost) * 10_000 / u128::from(total_cost))
                .map_err(|_| "비중 계산이 범위를 초과했습니다.".to_owned())?
        };
        allocated += weight_bps;
        positions.push(PortfolioRiskPosition {
            symbol: symbol.clone(),
            currency: currency.clone(),
            weight_bps,
            returns,
        });
    }
    save_portfolio_risk_snapshot(
        &bridge,
        PortfolioRiskSnapshotSaveRequest {
            snapshot_id: request.snapshot_id,
            request: PortfolioRiskRequest {
                as_of_ms: persistence::now_ms()?,
                positions,
                stress_shocks_bps: request.stress_shocks_bps,
            },
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(pnl_minor: i64, index: u64) -> BacktestTrade {
        BacktestTrade {
            opened_at_ms: index * 10,
            closed_at_ms: index * 10 + 5,
            quantity: 1,
            entry_price_minor: 100,
            exit_price_minor: 100,
            pnl_minor,
            exit_kind: crate::backtest::BacktestExitKind::Signal,
        }
    }

    #[test]
    fn bootstrap_is_reproducible_and_exposes_downside_probability() {
        let trades = [-100, 150, -80, 220, -40, 90, -130, 210]
            .into_iter()
            .enumerate()
            .map(|(index, pnl)| trade(pnl, index as u64))
            .collect::<Vec<_>>();
        let first = backtest_robustness("experiment", "dataset", 1_000, &trades);
        let second = backtest_robustness("experiment", "dataset", 1_000, &trades);
        assert!(first.computed);
        assert_eq!(first.seed, second.seed);
        assert_eq!(first.lower_return_bps, second.lower_return_bps);
        assert!(first.probability_of_loss_bps.is_some());
        assert!(first.worst_path_drawdown_bps.unwrap() > 0);
    }

    #[test]
    fn portfolio_risk_requires_point_in_time_aligned_single_currency_inputs() {
        let points = (1..=100)
            .map(|index| PortfolioReturnPoint {
                period_end_ms: index,
                available_at_ms: index + 1,
                return_ppm: if index % 4 == 0 { -20_000 } else { 10_000 },
            })
            .collect::<Vec<_>>();
        let report = analyze_portfolio_risk(PortfolioRiskRequest {
            as_of_ms: 200,
            positions: vec![
                PortfolioRiskPosition {
                    symbol: "A".to_owned(),
                    currency: "KRW".to_owned(),
                    weight_bps: 6_000,
                    returns: points.clone(),
                },
                PortfolioRiskPosition {
                    symbol: "B".to_owned(),
                    currency: "KRW".to_owned(),
                    weight_bps: 4_000,
                    returns: points,
                },
            ],
            stress_shocks_bps: BTreeMap::from([("A".to_owned(), -1_000), ("B".to_owned(), -500)]),
        })
        .expect("portfolio risk");
        assert_eq!(report.historical_var_95_bps, Some(200));
        assert_eq!(report.stressed_portfolio_return_bps, Some(-800));
        assert_eq!(report.concentration_hhi_bps, 5_200);
        assert_eq!(report.correlations[0].correlation_milli, Some(1_000));
    }

    #[test]
    fn portfolio_risk_rejects_implicit_currency_conversion() {
        let points = vec![PortfolioReturnPoint {
            period_end_ms: 1,
            available_at_ms: 2,
            return_ppm: 0,
        }];
        let error = analyze_portfolio_risk(PortfolioRiskRequest {
            as_of_ms: 3,
            positions: vec![
                PortfolioRiskPosition {
                    symbol: "KR".to_owned(),
                    currency: "KRW".to_owned(),
                    weight_bps: 5_000,
                    returns: points.clone(),
                },
                PortfolioRiskPosition {
                    symbol: "US".to_owned(),
                    currency: "USD".to_owned(),
                    weight_bps: 5_000,
                    returns: points,
                },
            ],
            stress_shocks_bps: BTreeMap::new(),
        })
        .unwrap_err();
        assert!(error.contains("환율"));
    }

    #[test]
    fn portfolio_risk_snapshot_is_immutable_and_replayable() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let as_of_ms = persistence::now_ms().expect("now").saturating_sub(1);
        let points = (1..=30)
            .map(|index| PortfolioReturnPoint {
                period_end_ms: index,
                available_at_ms: index,
                return_ppm: if index % 2 == 0 { 10_000 } else { -10_000 },
            })
            .collect();
        let stored = save_portfolio_risk_snapshot(
            &bridge,
            PortfolioRiskSnapshotSaveRequest {
                snapshot_id: "portfolio-risk-1".to_owned(),
                request: PortfolioRiskRequest {
                    as_of_ms,
                    positions: vec![PortfolioRiskPosition {
                        symbol: "005930".to_owned(),
                        currency: "KRW".to_owned(),
                        weight_bps: 10_000,
                        returns: points,
                    }],
                    stress_shocks_bps: BTreeMap::from([("005930".to_owned(), -1_000)]),
                },
            },
        )
        .expect("snapshot");
        assert_eq!(stored.report.stressed_portfolio_return_bps, Some(-1_000));
        let connection = bridge.connection.lock().expect("connection");
        let count: u64 = connection
            .query_row("SELECT COUNT(*) FROM portfolio_risk_snapshots", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(count, 1);
    }
}
