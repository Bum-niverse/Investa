use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    forecast::{
        audit_forecast_dataset, calibration_metrics, validate_forecast_evidence,
        CalibrationMetrics, CalibrationObservation, ForecastAssetClass, ForecastAssetContract,
        ForecastDatasetAuditInput, ForecastDatasetAuditReview, ForecastEvidenceMode,
        ForecastFeature, ForecastSample, ProbabilityForecast, TimeSplit,
    },
    persistence::PersistenceBridge,
};

const MAX_SAMPLES: usize = 20_000;
const MAX_FEATURES: usize = 200_000;
const MAX_CALIBRATION_OBSERVATIONS: usize = 20_000;
const MAX_HISTORY_LIMIT: u16 = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastDatasetAuditRequest {
    pub audit_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub samples: Vec<ForecastSample>,
    pub features: Vec<ForecastFeature>,
    pub split: TimeSplit,
    pub audit: ForecastDatasetAuditInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredForecastDatasetAudit {
    pub audit_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub review: ForecastDatasetAuditReview,
    pub created_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbabilityForecastSaveRequest {
    pub asset_class: ForecastAssetClass,
    pub forecast: ProbabilityForecast,
    pub news_feature_available: bool,
    pub price_feature_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProbabilityForecast {
    pub asset_class: ForecastAssetClass,
    pub evidence_mode: ForecastEvidenceMode,
    pub forecast: ProbabilityForecast,
    pub created_at_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastCalibrationSaveRequest {
    pub calibration_id: String,
    pub asset_class: ForecastAssetClass,
    pub model_id: String,
    pub model_version: String,
    pub dataset_id: String,
    pub horizon_ms: u64,
    pub evaluated_at_ms: u64,
    pub observations: Vec<CalibrationObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredForecastCalibration {
    pub calibration_id: String,
    pub asset_class: ForecastAssetClass,
    pub model_id: String,
    pub model_version: String,
    pub dataset_id: String,
    pub horizon_ms: u64,
    pub metrics: CalibrationMetrics,
    pub created_at_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastTraceSummary {
    pub forecast: StoredProbabilityForecast,
    pub calibration: Option<StoredForecastCalibration>,
}

fn validate_identifier(value: &str, label: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "{label} 식별자는 1~128자의 영문·숫자·-_.:만 사용할 수 있습니다."
        ))
    }
}

fn asset_class_key(asset_class: ForecastAssetClass) -> &'static str {
    match asset_class {
        ForecastAssetClass::KoreaStock => "korea_stock",
        ForecastAssetClass::UnitedStatesStock => "united_states_stock",
        ForecastAssetClass::EquityFuture => "equity_future",
        ForecastAssetClass::IndexFuture => "index_future",
        ForecastAssetClass::CryptoSpot => "crypto_spot",
        ForecastAssetClass::CryptoPerpetual => "crypto_perpetual",
    }
}

fn evidence_mode_key(mode: ForecastEvidenceMode) -> &'static str {
    match mode {
        ForecastEvidenceMode::FullFeatures => "full_features",
        ForecastEvidenceMode::PriceOnlyFallback => "price_only_fallback",
        ForecastEvidenceMode::Unavailable => "unavailable",
    }
}

fn safe_storage_error(label: &str) -> String {
    format!("{label} 로컬 저장소 작업을 완료하지 못했습니다.")
}

struct ImmutableInsert<'a> {
    table: &'a str,
    key_column: &'a str,
    key: &'a str,
    json_column: &'a str,
    expected_json: &'a str,
    insert_sql: &'a str,
    insert_params: &'a [&'a dyn rusqlite::ToSql],
}

fn store_immutable(
    bridge: &PersistenceBridge,
    insert: ImmutableInsert<'_>,
) -> Result<bool, String> {
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| safe_storage_error("예측"))?;
    let existing = connection
        .query_row(
            &format!(
                "SELECT {} FROM {} WHERE {} = ?1",
                insert.json_column, insert.table, insert.key_column
            ),
            params![insert.key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| safe_storage_error("예측"))?;
    match existing {
        Some(existing) if existing == insert.expected_json => Ok(false),
        Some(_) => Err(
            "같은 식별자에 다른 내용이 저장되어 있습니다. 새 식별자를 사용해 주세요.".to_owned(),
        ),
        None => {
            connection
                .execute(insert.insert_sql, insert.insert_params)
                .map_err(|_| safe_storage_error("예측"))?;
            Ok(true)
        }
    }
}

pub(crate) fn save_dataset_audit(
    bridge: &PersistenceBridge,
    request: ForecastDatasetAuditRequest,
) -> Result<StoredForecastDatasetAudit, String> {
    validate_identifier(&request.audit_id, "감사")?;
    validate_identifier(&request.dataset_id, "데이터셋")?;
    if request.samples.is_empty()
        || request.samples.len() > MAX_SAMPLES
        || request.features.is_empty()
        || request.features.len() > MAX_FEATURES
    {
        return Err(format!(
            "감사 입력은 표본 1~{MAX_SAMPLES}개, 피처 1~{MAX_FEATURES}개 범위여야 합니다."
        ));
    }
    let review = audit_forecast_dataset(
        &request.asset,
        &request.dataset_id,
        &request.samples,
        &request.features,
        &request.split,
        &request.audit,
    );
    let created_at_ms = request
        .features
        .iter()
        .map(|feature| feature.metadata.ingested_at_ms)
        .max()
        .ok_or_else(|| "감사할 피처의 수집 시각이 필요합니다.".to_owned())?;
    let stored = StoredForecastDatasetAudit {
        audit_id: request.audit_id,
        dataset_id: request.dataset_id,
        asset: request.asset,
        review,
        created_at_ms,
    };
    let json = serde_json::to_string(&stored)
        .map_err(|_| "예측 데이터 감사를 직렬화하지 못했습니다.".to_owned())?;
    let asset_class = asset_class_key(stored.asset.asset_class);
    store_immutable(
        bridge,
        ImmutableInsert {
            table: "forecast_dataset_audits",
            key_column: "audit_id",
            key: &stored.audit_id,
            json_column: "audit_json",
            expected_json: &json,
            insert_sql: "INSERT INTO forecast_dataset_audits
         (audit_id, dataset_id, asset_contract_id, asset_class, valid, audit_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            insert_params: &[
                &stored.audit_id,
                &stored.dataset_id,
                &stored.asset.contract_id,
                &asset_class,
                &stored.review.valid,
                &json,
                &stored.created_at_ms,
            ],
        },
    )?;
    Ok(stored)
}

pub(crate) fn save_probability_forecast(
    bridge: &PersistenceBridge,
    request: ProbabilityForecastSaveRequest,
) -> Result<StoredProbabilityForecast, String> {
    for (value, label) in [
        (&request.forecast.forecast_id, "예측"),
        (&request.forecast.model_id, "모델"),
        (&request.forecast.model_version, "모델 버전"),
        (&request.forecast.dataset_id, "데이터셋"),
        (&request.forecast.asset_contract_id, "자산 계약"),
    ] {
        validate_identifier(value, label)?;
    }
    let evidence_mode = validate_forecast_evidence(
        &request.forecast,
        request.news_feature_available,
        request.price_feature_available,
    )?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| safe_storage_error("데이터셋 감사"))?;
    let audit = connection
        .query_row(
            "SELECT asset_contract_id, asset_class, valid FROM forecast_dataset_audits
             WHERE dataset_id = ?1 ORDER BY created_at_ms DESC LIMIT 1",
            params![request.forecast.dataset_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| safe_storage_error("데이터셋 감사"))?;
    drop(connection);
    let Some((asset_contract_id, asset_class, dataset_valid)) = audit else {
        return Err("먼저 동일 데이터셋의 품질 감사를 저장해야 합니다.".to_owned());
    };
    if asset_contract_id != request.forecast.asset_contract_id
        || asset_class != asset_class_key(request.asset_class)
    {
        return Err("예측의 자산군·자산 계약이 데이터셋 감사와 일치하지 않습니다.".to_owned());
    }
    if !dataset_valid && request.forecast.up_probability_bps.is_some() {
        return Err("품질 감사에 실패한 데이터셋으로 숫자 확률을 저장할 수 없습니다.".to_owned());
    }
    let generated_at_ms = request.forecast.generated_at_ms;
    let stored = StoredProbabilityForecast {
        asset_class: request.asset_class,
        evidence_mode,
        forecast: request.forecast,
        created_at_ms: generated_at_ms,
    };
    let json = serde_json::to_string(&stored)
        .map_err(|_| "예측 결과를 직렬화하지 못했습니다.".to_owned())?;
    let asset_class = asset_class_key(stored.asset_class);
    let evidence_mode = evidence_mode_key(stored.evidence_mode);
    store_immutable(bridge, ImmutableInsert {
        table: "probability_forecasts",
        key_column: "forecast_id",
        key: &stored.forecast.forecast_id,
        json_column: "forecast_json",
        expected_json: &json,
        insert_sql: "INSERT INTO probability_forecasts
         (forecast_id, model_id, model_version, dataset_id, asset_contract_id, asset_class, horizon_ms, evidence_mode, forecast_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        insert_params: &[
            &stored.forecast.forecast_id,
            &stored.forecast.model_id,
            &stored.forecast.model_version,
            &stored.forecast.dataset_id,
            &stored.forecast.asset_contract_id,
            &asset_class,
            &stored.forecast.horizon_ms,
            &evidence_mode,
            &json,
            &stored.created_at_ms,
        ],
    })?;
    Ok(stored)
}

pub(crate) fn save_calibration(
    bridge: &PersistenceBridge,
    request: ForecastCalibrationSaveRequest,
) -> Result<StoredForecastCalibration, String> {
    for (value, label) in [
        (&request.calibration_id, "보정"),
        (&request.model_id, "모델"),
        (&request.model_version, "모델 버전"),
        (&request.dataset_id, "데이터셋"),
    ] {
        validate_identifier(value, label)?;
    }
    if request.horizon_ms == 0
        || request.evaluated_at_ms == 0
        || request.observations.is_empty()
        || request.observations.len() > MAX_CALIBRATION_OBSERVATIONS
    {
        return Err(format!(
            "보정 horizon과 1~{MAX_CALIBRATION_OBSERVATIONS}개의 관측값이 필요합니다."
        ));
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| safe_storage_error("예측 보정"))?;
    let mut statement = connection
        .prepare(
            "SELECT forecast_json FROM probability_forecasts
             WHERE asset_class = ?1 AND model_id = ?2 AND model_version = ?3
               AND dataset_id = ?4 AND horizon_ms = ?5",
        )
        .map_err(|_| safe_storage_error("예측 보정"))?;
    let matching_forecasts = statement
        .query_map(
            params![
                asset_class_key(request.asset_class),
                request.model_id,
                request.model_version,
                request.dataset_id,
                request.horizon_ms
            ],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| safe_storage_error("예측 보정"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| safe_storage_error("예측 보정"))?;
    drop(statement);
    drop(connection);
    let has_numeric_forecast = matching_forecasts.iter().try_fold(false, |found, json| {
        let forecast: StoredProbabilityForecast = serde_json::from_str(json)
            .map_err(|_| "저장된 예측 결과를 해석하지 못했습니다.".to_owned())?;
        Ok::<_, String>(found || forecast.forecast.up_probability_bps.is_some())
    })?;
    if !has_numeric_forecast {
        return Err("동일 자산군·모델·데이터셋·horizon의 숫자 예측 기록이 없습니다.".to_owned());
    }
    let metrics = calibration_metrics(&request.observations)?;
    let stored = StoredForecastCalibration {
        calibration_id: request.calibration_id,
        asset_class: request.asset_class,
        model_id: request.model_id,
        model_version: request.model_version,
        dataset_id: request.dataset_id,
        horizon_ms: request.horizon_ms,
        metrics,
        created_at_ms: request.evaluated_at_ms,
    };
    let json = serde_json::to_string(&stored)
        .map_err(|_| "예측 보정 결과를 직렬화하지 못했습니다.".to_owned())?;
    let asset_class = asset_class_key(stored.asset_class);
    let sample_count = i64::try_from(stored.metrics.sample_count)
        .map_err(|_| "예측 보정 표본 수가 지원 범위를 초과했습니다.".to_owned())?;
    store_immutable(bridge, ImmutableInsert {
        table: "forecast_calibration_runs",
        key_column: "calibration_id",
        key: &stored.calibration_id,
        json_column: "metrics_json",
        expected_json: &json,
        insert_sql: "INSERT INTO forecast_calibration_runs
         (calibration_id, asset_class, model_id, model_version, dataset_id, horizon_ms, sample_count, metrics_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        insert_params: &[
            &stored.calibration_id,
            &asset_class,
            &stored.model_id,
            &stored.model_version,
            &stored.dataset_id,
            &stored.horizon_ms,
            &sample_count,
            &json,
            &stored.created_at_ms,
        ],
    })?;
    Ok(stored)
}

pub(crate) fn forecast_history(
    bridge: &PersistenceBridge,
    limit: u16,
) -> Result<Vec<ForecastTraceSummary>, String> {
    let limit = limit.clamp(1, MAX_HISTORY_LIMIT);
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| safe_storage_error("예측 이력"))?;
    let mut statement = connection
        .prepare(
            "SELECT forecast_json FROM probability_forecasts
             ORDER BY created_at_ms DESC, forecast_id DESC LIMIT ?1",
        )
        .map_err(|_| safe_storage_error("예측 이력"))?;
    let forecasts = statement
        .query_map(params![limit], |row| row.get::<_, String>(0))
        .map_err(|_| safe_storage_error("예측 이력"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| safe_storage_error("예측 이력"))?;
    let mut result = Vec::with_capacity(forecasts.len());
    for json in forecasts {
        let forecast: StoredProbabilityForecast = serde_json::from_str(&json)
            .map_err(|_| "저장된 예측 결과를 해석하지 못했습니다.".to_owned())?;
        let calibration_json = connection
            .query_row(
                "SELECT metrics_json FROM forecast_calibration_runs
                 WHERE asset_class = ?1 AND model_id = ?2 AND model_version = ?3
                   AND dataset_id = ?4 AND horizon_ms = ?5
                 ORDER BY created_at_ms DESC LIMIT 1",
                params![
                    asset_class_key(forecast.asset_class),
                    forecast.forecast.model_id,
                    forecast.forecast.model_version,
                    forecast.forecast.dataset_id,
                    forecast.forecast.horizon_ms
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| safe_storage_error("예측 보정 이력"))?;
        let calibration = calibration_json
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|_| "저장된 예측 보정 결과를 해석하지 못했습니다.".to_owned())
            })
            .transpose()?;
        result.push(ForecastTraceSummary {
            forecast,
            calibration,
        });
    }
    Ok(result)
}

#[tauri::command]
pub fn forecast_dataset_audit_save(
    state: State<'_, PersistenceBridge>,
    request: ForecastDatasetAuditRequest,
) -> Result<StoredForecastDatasetAudit, String> {
    save_dataset_audit(&state, request)
}

#[tauri::command]
pub fn probability_forecast_save(
    state: State<'_, PersistenceBridge>,
    request: ProbabilityForecastSaveRequest,
) -> Result<StoredProbabilityForecast, String> {
    save_probability_forecast(&state, request)
}

#[tauri::command]
pub fn forecast_calibration_save(
    state: State<'_, PersistenceBridge>,
    request: ForecastCalibrationSaveRequest,
) -> Result<StoredForecastCalibration, String> {
    save_calibration(&state, request)
}

#[tauri::command]
pub fn probability_forecast_history(
    state: State<'_, PersistenceBridge>,
    limit: u16,
) -> Result<Vec<ForecastTraceSummary>, String> {
    forecast_history(&state, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_quality::TemporalMetadata;

    fn stock_contract() -> ForecastAssetContract {
        ForecastAssetContract {
            contract_id: "kr-stock-v1".to_owned(),
            asset_class: ForecastAssetClass::KoreaStock,
            exchange: "KRX".to_owned(),
            symbol: "005930".to_owned(),
            currency: "KRW".to_owned(),
            timezone: "Asia/Seoul".to_owned(),
            adjusted_price_policy: Some("point_in_time".to_owned()),
            corporate_action_policy: Some("effective_at".to_owned()),
            contract_multiplier: None,
            expiry_policy: None,
            rollover_policy: None,
            price_basis: None,
            funding_policy: None,
            leverage_policy: None,
        }
    }

    fn audit_request(audit_id: &str) -> ForecastDatasetAuditRequest {
        ForecastDatasetAuditRequest {
            audit_id: audit_id.to_owned(),
            dataset_id: "dataset-v1".to_owned(),
            asset: stock_contract(),
            samples: vec![ForecastSample {
                sample_id: "sample-1".to_owned(),
                decision_time_ms: 100,
                target_observed_at_ms: 200,
                target_class: 1,
            }],
            features: vec![ForecastFeature {
                feature_id: "price.close".to_owned(),
                sample_id: "sample-1".to_owned(),
                source_record_id: "bar-1".to_owned(),
                dataset_version: "dataset-v1".to_owned(),
                metadata: TemporalMetadata {
                    event_time_ms: 90,
                    available_at_ms: 95,
                    ingested_at_ms: 96,
                    source: "price".to_owned(),
                    source_revision: "v1".to_owned(),
                },
                value_scaled: 70_000,
                value_scale: 1,
                quality_flags: vec![],
            }],
            split: TimeSplit {
                train_end_ms: 10,
                validation_start_ms: 11,
                validation_end_ms: 20,
                test_start_ms: 21,
            },
            audit: ForecastDatasetAuditInput {
                expected_feature_ids: vec!["price.close".to_owned()],
                corporate_action_coverage_confirmed: true,
                trading_session_coverage_confirmed: true,
                listing_history_checked: true,
            },
        }
    }

    fn forecast_request(asset_class: ForecastAssetClass) -> ProbabilityForecastSaveRequest {
        ProbabilityForecastSaveRequest {
            asset_class,
            forecast: ProbabilityForecast {
                forecast_id: "forecast-v1".to_owned(),
                model_id: "model-v1".to_owned(),
                model_version: "1.0".to_owned(),
                dataset_id: "dataset-v1".to_owned(),
                asset_contract_id: "kr-stock-v1".to_owned(),
                horizon_ms: 86_400_000,
                generated_at_ms: 100,
                up_probability_bps: Some(5_000),
                down_probability_bps: Some(3_000),
                flat_probability_bps: Some(2_000),
                recommendation_confidence_bps: Some(4_000),
                model_reliability_bps: Some(6_000),
                unavailable_reason: None,
                price_only_fallback: true,
            },
            news_feature_available: false,
            price_feature_available: true,
        }
    }

    #[test]
    fn immutable_forecast_replay_is_idempotent_and_conflicts_are_rejected() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        save_dataset_audit(&bridge, audit_request("audit-v1")).expect("audit");
        let first =
            save_probability_forecast(&bridge, forecast_request(ForecastAssetClass::KoreaStock))
                .expect("forecast");
        let second =
            save_probability_forecast(&bridge, forecast_request(ForecastAssetClass::KoreaStock))
                .expect("idempotent replay");
        assert_eq!(first.forecast.forecast_id, second.forecast.forecast_id);
        let mut conflicting = forecast_request(ForecastAssetClass::KoreaStock);
        conflicting.forecast.model_reliability_bps = Some(5_000);
        assert!(save_probability_forecast(&bridge, conflicting).is_err());
    }

    #[test]
    fn calibration_cannot_reuse_another_asset_class_forecast() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        save_dataset_audit(&bridge, audit_request("audit-v1")).expect("audit");
        save_probability_forecast(&bridge, forecast_request(ForecastAssetClass::KoreaStock))
            .expect("forecast");
        let request = ForecastCalibrationSaveRequest {
            calibration_id: "calibration-v1".to_owned(),
            asset_class: ForecastAssetClass::UnitedStatesStock,
            model_id: "model-v1".to_owned(),
            model_version: "1.0".to_owned(),
            dataset_id: "dataset-v1".to_owned(),
            horizon_ms: 86_400_000,
            evaluated_at_ms: 300,
            observations: vec![CalibrationObservation {
                predicted_up_bps: 5_000,
                actual_up: true,
            }],
        };
        assert!(save_calibration(&bridge, request).is_err());
    }

    #[test]
    fn calibration_requires_a_matching_numeric_forecast() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        save_dataset_audit(&bridge, audit_request("audit-v1")).expect("audit");
        let mut unavailable = forecast_request(ForecastAssetClass::KoreaStock);
        unavailable.forecast.up_probability_bps = None;
        unavailable.forecast.down_probability_bps = None;
        unavailable.forecast.flat_probability_bps = None;
        unavailable.forecast.recommendation_confidence_bps = None;
        unavailable.forecast.model_reliability_bps = None;
        unavailable.forecast.unavailable_reason = Some("가격 피처 누락".to_owned());
        unavailable.forecast.price_only_fallback = false;
        unavailable.news_feature_available = false;
        unavailable.price_feature_available = false;
        save_probability_forecast(&bridge, unavailable).expect("unavailable trace");
        let request = ForecastCalibrationSaveRequest {
            calibration_id: "calibration-v1".to_owned(),
            asset_class: ForecastAssetClass::KoreaStock,
            model_id: "model-v1".to_owned(),
            model_version: "1.0".to_owned(),
            dataset_id: "dataset-v1".to_owned(),
            horizon_ms: 86_400_000,
            evaluated_at_ms: 300,
            observations: vec![CalibrationObservation {
                predicted_up_bps: 5_000,
                actual_up: true,
            }],
        };
        assert!(save_calibration(&bridge, request).is_err());
    }
}
