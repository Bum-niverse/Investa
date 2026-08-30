use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::{
    forecast::{
        audit_forecast_dataset, ForecastAssetClass, ForecastAssetContract,
        ForecastDatasetAuditInput, ForecastFeature, ForecastSample, TimeSplit,
    },
    forecast_runtime::StoredForecastDatasetAudit,
    persistence::PersistenceBridge,
};

const WORKER_CONTRACT_VERSION: &str = "investa-ml-worker-v1";
const SHARD_WORKER_CONTRACT_VERSION: &str = "investa-ml-worker-sharded-v1";
const MAX_SAMPLES: usize = 20_000;
const MAX_FEATURES: usize = 200_000;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_HYPERPARAMETERS: usize = 64;
const MAX_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_HISTORY_LIMIT: u16 = 100;
const MAX_DATASET_SHARDS: usize = 64;
const MAX_SHARDED_SAMPLES: usize = 1_000_000;
const MAX_SHARDED_FEATURES: usize = 10_000_000;
const PROBABILITY_SCALE: u32 = 1_000_000;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MlAlgorithm {
    Lightgbm,
    Xgboost,
    Chronos,
    Timesfm,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MlArtifactFormat {
    LightgbmText,
    XgboostJson,
    Safetensors,
    Onnx,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MlDatasetSourceKind {
    #[default]
    Manifest,
    ShardSet,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlDatasetManifestCreateRequest {
    pub manifest_id: String,
    pub audit_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub samples: Vec<ForecastSample>,
    pub features: Vec<ForecastFeature>,
    pub split: TimeSplit,
    pub audit: ForecastDatasetAuditInput,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalDatasetPayload {
    dataset_id: String,
    asset: ForecastAssetContract,
    samples: Vec<ForecastSample>,
    features: Vec<ForecastFeature>,
    split: TimeSplit,
    expected_feature_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMlDatasetManifest {
    pub manifest_id: String,
    pub audit_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub content_sha256: String,
    pub feature_schema_sha256: String,
    pub sample_count: usize,
    pub feature_count: usize,
    pub first_decision_time_ms: u64,
    pub last_decision_time_ms: u64,
    pub split: TimeSplit,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlTrainingJobPrepareRequest {
    pub job_id: String,
    pub manifest_id: String,
    pub algorithm: MlAlgorithm,
    pub code_version: String,
    pub random_seed: u64,
    pub horizon_ms: u64,
    pub timeout_seconds: u32,
    pub memory_limit_mb: u32,
    pub max_threads: u8,
    pub hyperparameters: BTreeMap<String, Value>,
    pub requested_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMlTrainingJob {
    pub job_id: String,
    pub manifest_id: String,
    #[serde(default)]
    pub dataset_source_kind: MlDatasetSourceKind,
    pub algorithm: MlAlgorithm,
    pub contract_version: String,
    pub dataset_content_sha256: String,
    pub feature_schema_sha256: String,
    pub input_sha256: String,
    pub code_version: String,
    pub random_seed: u64,
    pub horizon_ms: u64,
    pub timeout_seconds: u32,
    pub memory_limit_mb: u32,
    pub max_threads: u8,
    pub hyperparameters: BTreeMap<String, Value>,
    pub status: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlOosMetrics {
    pub sample_count: u64,
    pub fold_count: u32,
    pub log_loss_millionths: u64,
    pub brier_score_millionths: u64,
    pub expected_calibration_error_bps: u64,
    pub balanced_accuracy_bps: u64,
    pub evaluated_at_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlOosPrediction {
    pub sample_id: String,
    pub fold_index: u32,
    pub target_class: u8,
    pub probability_down_millionths: u32,
    pub probability_flat_millionths: u32,
    pub probability_up_millionths: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlArtifactDescriptor {
    pub file_name: String,
    pub format: MlArtifactFormat,
    pub sha256: String,
    pub byte_size: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlTrainingJobCompleteRequest {
    pub job_id: String,
    pub input_sha256: String,
    pub completed_at_ms: u64,
    pub succeeded: bool,
    pub failure_code: Option<String>,
    pub model_id: Option<String>,
    pub model_version: Option<String>,
    pub artifact: Option<MlArtifactDescriptor>,
    pub metrics: Option<MlOosMetrics>,
    pub predictions: Option<Vec<MlOosPrediction>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMlModelVersion {
    pub model_id: String,
    pub model_version: String,
    pub job_id: String,
    pub manifest_id: String,
    #[serde(default)]
    pub dataset_source_kind: MlDatasetSourceKind,
    pub asset_class: ForecastAssetClass,
    pub algorithm: MlAlgorithm,
    pub artifact: MlArtifactDescriptor,
    pub metrics: MlOosMetrics,
    pub status: String,
    pub created_at_ms: u64,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlTrainingJobCompletion {
    pub job: StoredMlTrainingJob,
    pub model: Option<StoredMlModelVersion>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlPipelineHistory {
    pub manifests: Vec<StoredMlDatasetManifest>,
    pub jobs: Vec<StoredMlTrainingJob>,
    pub models: Vec<StoredMlModelVersion>,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlDatasetShardSetCreateRequest {
    pub shard_set_id: String,
    pub dataset_id: String,
    pub manifest_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlDatasetShardExtent {
    pub sample_count: usize,
    pub first_decision_time_ms: u64,
    pub last_decision_time_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlDatasetShardDescriptor {
    pub manifest_id: String,
    pub content_sha256: String,
    pub sample_count: usize,
    pub feature_count: usize,
    pub train: MlDatasetShardExtent,
    pub validation: MlDatasetShardExtent,
    pub test: MlDatasetShardExtent,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredMlDatasetShardSet {
    pub shard_set_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub split: TimeSplit,
    pub feature_schema_sha256: String,
    pub combined_content_sha256: String,
    pub shard_count: usize,
    pub sample_count: usize,
    pub feature_count: usize,
    pub shards: Vec<MlDatasetShardDescriptor>,
    pub created_at_ms: u64,
    pub worker_ready: bool,
    pub live_order_allowed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlDatasetShardSetHistory {
    pub shard_sets: Vec<StoredMlDatasetShardSet>,
    pub worker_ready: bool,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MlWorkerBundle {
    pub contract_version: String,
    pub job: StoredMlTrainingJob,
    pub manifest: StoredMlDatasetManifest,
    pub dataset_payload_json: String,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum MlTrainingJobSource {
    Manifest(MlWorkerBundle),
    ShardSet {
        job: StoredMlTrainingJob,
        shard_set: StoredMlDatasetShardSet,
    },
}

impl MlTrainingJobSource {
    pub(crate) fn job(&self) -> &StoredMlTrainingJob {
        match self {
            Self::Manifest(bundle) => &bundle.job,
            Self::ShardSet { job, .. } => job,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MlShardWorkerFile {
    pub manifest: StoredMlDatasetManifest,
    pub file_name: String,
    pub byte_size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MlShardWorkerBundle {
    pub contract_version: String,
    pub job: StoredMlTrainingJob,
    pub shard_set: StoredMlDatasetShardSet,
    pub dataset_shards: Vec<MlShardWorkerFile>,
    pub live_order_allowed: bool,
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

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
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

fn algorithm_key(algorithm: MlAlgorithm) -> &'static str {
    match algorithm {
        MlAlgorithm::Lightgbm => "lightgbm",
        MlAlgorithm::Xgboost => "xgboost",
        MlAlgorithm::Chronos => "chronos",
        MlAlgorithm::Timesfm => "timesfm",
    }
}

fn dataset_source_kind_key(kind: MlDatasetSourceKind) -> &'static str {
    match kind {
        MlDatasetSourceKind::Manifest => "manifest",
        MlDatasetSourceKind::ShardSet => "shard_set",
    }
}

fn artifact_format_key(format: MlArtifactFormat) -> &'static str {
    match format {
        MlArtifactFormat::LightgbmText => "lightgbm_text",
        MlArtifactFormat::XgboostJson => "xgboost_json",
        MlArtifactFormat::Safetensors => "safetensors",
        MlArtifactFormat::Onnx => "onnx",
    }
}

fn same_split(left: &TimeSplit, right: &TimeSplit) -> bool {
    left.train_end_ms == right.train_end_ms
        && left.validation_start_ms == right.validation_start_ms
        && left.validation_end_ms == right.validation_end_ms
        && left.test_start_ms == right.test_start_ms
}

fn shard_extent(samples: &[&ForecastSample], label: &str) -> Result<MlDatasetShardExtent, String> {
    let first = samples
        .first()
        .ok_or_else(|| format!("각 shard의 {label} 구간에는 최소 한 표본이 필요합니다."))?;
    let last = samples.last().expect("non-empty shard extent");
    Ok(MlDatasetShardExtent {
        sample_count: samples.len(),
        first_decision_time_ms: first.decision_time_ms,
        last_decision_time_ms: last.decision_time_ms,
    })
}

fn shard_descriptor(
    manifest: &StoredMlDatasetManifest,
    payload: &CanonicalDatasetPayload,
) -> Result<MlDatasetShardDescriptor, String> {
    let train = payload
        .samples
        .iter()
        .filter(|sample| sample.decision_time_ms <= payload.split.train_end_ms)
        .collect::<Vec<_>>();
    let validation = payload
        .samples
        .iter()
        .filter(|sample| {
            sample.decision_time_ms >= payload.split.validation_start_ms
                && sample.decision_time_ms <= payload.split.validation_end_ms
        })
        .collect::<Vec<_>>();
    let test = payload
        .samples
        .iter()
        .filter(|sample| sample.decision_time_ms >= payload.split.test_start_ms)
        .collect::<Vec<_>>();
    Ok(MlDatasetShardDescriptor {
        manifest_id: manifest.manifest_id.clone(),
        content_sha256: manifest.content_sha256.clone(),
        sample_count: manifest.sample_count,
        feature_count: manifest.feature_count,
        train: shard_extent(&train, "train")?,
        validation: shard_extent(&validation, "validation")?,
        test: shard_extent(&test, "test")?,
    })
}

fn shard_set_record(
    bridge: &PersistenceBridge,
    request: MlDatasetShardSetCreateRequest,
) -> Result<StoredMlDatasetShardSet, String> {
    validate_identifier(&request.shard_set_id, "shard set")?;
    validate_identifier(&request.dataset_id, "논리 데이터셋")?;
    if !(2..=MAX_DATASET_SHARDS).contains(&request.manifest_ids.len()) {
        return Err(format!(
            "shard set은 2~{MAX_DATASET_SHARDS}개 매니페스트가 필요합니다."
        ));
    }
    let unique = request.manifest_ids.iter().collect::<BTreeSet<_>>();
    if unique.len() != request.manifest_ids.len() {
        return Err("shard set 매니페스트 ID는 중복될 수 없습니다.".to_owned());
    }
    for manifest_id in &request.manifest_ids {
        validate_identifier(manifest_id, "shard 매니페스트")?;
    }

    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML shard 저장소를 사용할 수 없습니다.".to_owned())?;
    let mut asset: Option<ForecastAssetContract> = None;
    let mut split: Option<TimeSplit> = None;
    let mut schema_hash: Option<String> = None;
    let mut descriptors = Vec::with_capacity(request.manifest_ids.len());
    let mut sample_ids = BTreeSet::new();
    let mut sample_count = 0_usize;
    let mut feature_count = 0_usize;
    let mut created_at_ms = 0_u64;

    for manifest_id in &request.manifest_ids {
        let (manifest_json, payload_json) = connection
            .query_row(
                "SELECT manifest_json, payload_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
                params![manifest_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| "ML shard 매니페스트를 조회하지 못했습니다.".to_owned())?
            .ok_or_else(|| format!("ML shard 매니페스트가 없습니다: {manifest_id}"))?;
        let manifest: StoredMlDatasetManifest = serde_json::from_str(&manifest_json)
            .map_err(|_| "저장된 ML shard 매니페스트를 해석하지 못했습니다.".to_owned())?;
        let payload: CanonicalDatasetPayload = serde_json::from_str(&payload_json)
            .map_err(|_| "저장된 ML shard payload를 해석하지 못했습니다.".to_owned())?;
        if sha256_hex(payload_json.as_bytes()) != manifest.content_sha256
            || payload.samples.len() != manifest.sample_count
            || payload.features.len() != manifest.feature_count
        {
            return Err("ML shard payload의 해시 또는 행 수가 매니페스트와 다릅니다.".to_owned());
        }
        if let Some(expected) = &asset {
            let expected_json = serde_json::to_vec(expected)
                .map_err(|_| "자산 계약을 직렬화하지 못했습니다.".to_owned())?;
            let actual_json = serde_json::to_vec(&manifest.asset)
                .map_err(|_| "자산 계약을 직렬화하지 못했습니다.".to_owned())?;
            if expected_json != actual_json {
                return Err("모든 shard는 동일한 자산 계약이어야 합니다.".to_owned());
            }
        } else {
            asset = Some(manifest.asset.clone());
        }
        if let Some(expected) = &split {
            if !same_split(expected, &manifest.split) {
                return Err(
                    "모든 shard는 동일한 train·validation·test 경계를 사용해야 합니다.".to_owned(),
                );
            }
        } else {
            split = Some(manifest.split.clone());
        }
        if schema_hash
            .as_ref()
            .is_some_and(|value| value != &manifest.feature_schema_sha256)
        {
            return Err("모든 shard는 동일한 피처 스키마를 사용해야 합니다.".to_owned());
        }
        schema_hash.get_or_insert_with(|| manifest.feature_schema_sha256.clone());
        for sample in &payload.samples {
            if !sample_ids.insert(sample.sample_id.clone()) {
                return Err("shard 사이에 중복된 표본 ID가 있습니다.".to_owned());
            }
        }
        sample_count = sample_count
            .checked_add(manifest.sample_count)
            .ok_or_else(|| "shard 표본 수가 범위를 초과했습니다.".to_owned())?;
        feature_count = feature_count
            .checked_add(manifest.feature_count)
            .ok_or_else(|| "shard 피처 수가 범위를 초과했습니다.".to_owned())?;
        created_at_ms = created_at_ms.max(manifest.created_at_ms);
        descriptors.push(shard_descriptor(&manifest, &payload)?);
    }
    if sample_count > MAX_SHARDED_SAMPLES || feature_count > MAX_SHARDED_FEATURES {
        return Err(format!(
            "논리 shard set은 표본 {MAX_SHARDED_SAMPLES}개·피처 {MAX_SHARDED_FEATURES}개를 넘을 수 없습니다."
        ));
    }
    for pair in descriptors.windows(2) {
        for (left, right, label) in [
            (&pair[0].train, &pair[1].train, "train"),
            (&pair[0].validation, &pair[1].validation, "validation"),
            (&pair[0].test, &pair[1].test, "test"),
        ] {
            if left.last_decision_time_ms >= right.first_decision_time_ms {
                return Err(format!(
                    "shard {label} 구간이 겹치거나 순서가 역전됐습니다."
                ));
            }
        }
    }
    let asset = asset.expect("validated shard assets");
    let split = split.expect("validated shard splits");
    let feature_schema_sha256 = schema_hash.expect("validated shard schema");
    let combined_hash_input = serde_json::to_vec(&(
        &request.dataset_id,
        &asset,
        &split,
        &feature_schema_sha256,
        &descriptors,
    ))
    .map_err(|_| "shard set 해시 입력을 직렬화하지 못했습니다.".to_owned())?;
    let stored = StoredMlDatasetShardSet {
        shard_set_id: request.shard_set_id,
        dataset_id: request.dataset_id,
        asset,
        split,
        feature_schema_sha256,
        combined_content_sha256: sha256_hex(&combined_hash_input),
        shard_count: descriptors.len(),
        sample_count,
        feature_count,
        shards: descriptors,
        created_at_ms,
        worker_ready: false,
        live_order_allowed: false,
    };
    let record_json = serde_json::to_string(&stored)
        .map_err(|_| "ML shard set을 직렬화하지 못했습니다.".to_owned())?;
    let existing = connection
        .query_row(
            "SELECT record_json FROM ml_dataset_shard_sets WHERE shard_set_id = ?1",
            params![stored.shard_set_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "ML shard set을 조회하지 못했습니다.".to_owned())?;
    match existing {
        Some(existing) if existing == record_json => {}
        Some(_) => {
            return Err("같은 shard set ID에 다른 데이터가 이미 저장되어 있습니다.".to_owned())
        }
        None => {
            connection
                .execute(
                    "INSERT INTO ml_dataset_shard_sets
                     (shard_set_id, dataset_id, asset_class, feature_schema_sha256,
                      combined_content_sha256, shard_count, sample_count, feature_count,
                      record_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        stored.shard_set_id,
                        stored.dataset_id,
                        asset_class_key(stored.asset.asset_class),
                        stored.feature_schema_sha256,
                        stored.combined_content_sha256,
                        stored.shard_count,
                        stored.sample_count,
                        stored.feature_count,
                        record_json,
                        stored.created_at_ms,
                    ],
                )
                .map_err(|_| "ML shard set을 저장하지 못했습니다.".to_owned())?;
        }
    }
    Ok(stored)
}

fn canonicalize_dataset(
    request: &MlDatasetManifestCreateRequest,
) -> Result<CanonicalDatasetPayload, String> {
    if request.samples.is_empty()
        || request.samples.len() > MAX_SAMPLES
        || request.features.is_empty()
        || request.features.len() > MAX_FEATURES
    {
        return Err(format!(
            "ML 데이터셋은 표본 1~{MAX_SAMPLES}개, 피처 1~{MAX_FEATURES}개 범위여야 합니다."
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
    if !review.valid {
        return Err(format!(
            "ML 데이터셋 품질 감사를 통과하지 못했습니다: {}",
            review.issues.join(" ")
        ));
    }

    let mut samples = request.samples.clone();
    samples.sort_by_key(|sample| (sample.decision_time_ms, sample.sample_id.clone()));
    if samples
        .windows(2)
        .any(|window| window[0].decision_time_ms >= window[1].decision_time_ms)
    {
        return Err("한 자산의 표본 결정 시각은 중복 없이 증가해야 합니다.".to_owned());
    }
    let train_count = samples
        .iter()
        .filter(|sample| sample.decision_time_ms <= request.split.train_end_ms)
        .count();
    let validation_count = samples
        .iter()
        .filter(|sample| {
            sample.decision_time_ms >= request.split.validation_start_ms
                && sample.decision_time_ms <= request.split.validation_end_ms
        })
        .count();
    let test_count = samples
        .iter()
        .filter(|sample| sample.decision_time_ms >= request.split.test_start_ms)
        .count();
    if train_count == 0 || validation_count == 0 || test_count == 0 {
        return Err("train·validation·test 각 구간에 최소 한 표본이 필요합니다.".to_owned());
    }
    if samples.iter().any(|sample| {
        (sample.decision_time_ms <= request.split.train_end_ms
            && sample.target_observed_at_ms >= request.split.validation_start_ms)
            || (sample.decision_time_ms >= request.split.validation_start_ms
                && sample.decision_time_ms <= request.split.validation_end_ms
                && sample.target_observed_at_ms >= request.split.test_start_ms)
    }) {
        return Err("학습·검증 타깃이 다음 시간 구간으로 넘어가 누수될 수 있습니다.".to_owned());
    }

    let mut features = request.features.clone();
    features.sort_by(|left, right| {
        (&left.sample_id, &left.feature_id, &left.source_record_id).cmp(&(
            &right.sample_id,
            &right.feature_id,
            &right.source_record_id,
        ))
    });
    let mut expected_feature_ids = request.audit.expected_feature_ids.clone();
    expected_feature_ids.sort();
    expected_feature_ids.dedup();
    Ok(CanonicalDatasetPayload {
        dataset_id: request.dataset_id.clone(),
        asset: request.asset.clone(),
        samples,
        features,
        split: request.split.clone(),
        expected_feature_ids,
    })
}

pub(crate) fn create_dataset_manifest(
    bridge: &PersistenceBridge,
    request: MlDatasetManifestCreateRequest,
) -> Result<StoredMlDatasetManifest, String> {
    for (value, label) in [
        (&request.manifest_id, "매니페스트"),
        (&request.audit_id, "감사"),
        (&request.dataset_id, "데이터셋"),
    ] {
        validate_identifier(value, label)?;
    }
    let payload = canonicalize_dataset(&request)?;
    let payload_json = serde_json::to_string(&payload)
        .map_err(|_| "ML 데이터셋을 직렬화하지 못했습니다.".to_owned())?;
    if payload_json.len() > MAX_PAYLOAD_BYTES {
        return Err("ML 데이터셋 직렬화 크기가 64MiB 제한을 초과했습니다.".to_owned());
    }
    let feature_schema_json = serde_json::to_vec(&payload.expected_feature_ids)
        .map_err(|_| "피처 스키마를 직렬화하지 못했습니다.".to_owned())?;
    let content_sha256 = sha256_hex(payload_json.as_bytes());
    let feature_schema_sha256 = sha256_hex(&feature_schema_json);
    let created_at_ms = payload
        .features
        .iter()
        .map(|feature| feature.metadata.ingested_at_ms)
        .max()
        .ok_or_else(|| "피처 수집 시각이 필요합니다.".to_owned())?;
    let stored = StoredMlDatasetManifest {
        manifest_id: request.manifest_id,
        audit_id: request.audit_id,
        dataset_id: request.dataset_id,
        asset: request.asset,
        content_sha256,
        feature_schema_sha256,
        sample_count: payload.samples.len(),
        feature_count: payload.features.len(),
        first_decision_time_ms: payload.samples[0].decision_time_ms,
        last_decision_time_ms: payload.samples[payload.samples.len() - 1].decision_time_ms,
        split: payload.split.clone(),
        created_at_ms,
    };
    let manifest_json = serde_json::to_string(&stored)
        .map_err(|_| "ML 매니페스트를 직렬화하지 못했습니다.".to_owned())?;
    let sample_count = i64::try_from(stored.sample_count)
        .map_err(|_| "표본 수가 지원 범위를 초과했습니다.".to_owned())?;
    let feature_count = i64::try_from(stored.feature_count)
        .map_err(|_| "피처 수가 지원 범위를 초과했습니다.".to_owned())?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let audit_json = connection
        .query_row(
            "SELECT audit_json FROM forecast_dataset_audits WHERE audit_id = ?1",
            params![stored.audit_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "ML 데이터셋 감사를 조회하지 못했습니다.".to_owned())?
        .ok_or_else(|| "먼저 동일한 품질 감사를 저장해야 합니다.".to_owned())?;
    let saved_audit: StoredForecastDatasetAudit = serde_json::from_str(&audit_json)
        .map_err(|_| "저장된 데이터셋 감사를 해석하지 못했습니다.".to_owned())?;
    if !saved_audit.review.valid
        || saved_audit.dataset_id != stored.dataset_id
        || saved_audit.asset.contract_id != stored.asset.contract_id
        || saved_audit.asset.asset_class != stored.asset.asset_class
    {
        return Err("저장된 품질 감사와 ML 데이터셋 계약이 일치하지 않습니다.".to_owned());
    }
    let existing = connection
        .query_row(
            "SELECT manifest_json, payload_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
            params![stored.manifest_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| "ML 매니페스트를 조회하지 못했습니다.".to_owned())?;
    match existing {
        Some((existing_manifest, existing_payload))
            if existing_manifest == manifest_json && existing_payload == payload_json => {}
        Some(_) => {
            return Err("같은 매니페스트 ID에 다른 데이터가 이미 저장되어 있습니다.".to_owned())
        }
        None => {
            connection
                .execute(
                    "INSERT INTO ml_dataset_manifests
                     (manifest_id, audit_id, dataset_id, asset_class, content_sha256,
                      feature_schema_sha256, sample_count, feature_count, manifest_json,
                      payload_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        stored.manifest_id,
                        stored.audit_id,
                        stored.dataset_id,
                        asset_class_key(stored.asset.asset_class),
                        stored.content_sha256,
                        stored.feature_schema_sha256,
                        sample_count,
                        feature_count,
                        manifest_json,
                        payload_json,
                        stored.created_at_ms,
                    ],
                )
                .map_err(|_| "ML 매니페스트를 저장하지 못했습니다.".to_owned())?;
        }
    }
    Ok(stored)
}

fn validate_hyperparameters(values: &BTreeMap<String, Value>) -> Result<(), String> {
    if values.len() > MAX_HYPERPARAMETERS {
        return Err(format!(
            "하이퍼파라미터는 최대 {MAX_HYPERPARAMETERS}개입니다."
        ));
    }
    for (key, value) in values {
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            || !matches!(value, Value::Bool(_) | Value::Number(_) | Value::String(_))
            || value.as_str().is_some_and(|item| item.len() > 128)
        {
            return Err("하이퍼파라미터는 제한된 이름과 짧은 scalar 값만 허용합니다.".to_owned());
        }
    }
    Ok(())
}

fn prepare_training_job(
    bridge: &PersistenceBridge,
    request: MlTrainingJobPrepareRequest,
) -> Result<StoredMlTrainingJob, String> {
    for (value, label) in [
        (&request.job_id, "학습 작업"),
        (&request.manifest_id, "매니페스트"),
        (&request.code_version, "코드 버전"),
    ] {
        validate_identifier(value, label)?;
    }
    if request.horizon_ms == 0
        || !(60..=21_600).contains(&request.timeout_seconds)
        || !(512..=16_384).contains(&request.memory_limit_mb)
        || !(1..=16).contains(&request.max_threads)
        || request.requested_at_ms == 0
    {
        return Err("horizon·timeout·메모리·스레드·요청 시각 제한이 올바르지 않습니다.".to_owned());
    }
    validate_hyperparameters(&request.hyperparameters)?;
    let manifest = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT manifest_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
                params![request.manifest_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| "ML 매니페스트를 조회하지 못했습니다.".to_owned())?
            .map(|json| {
                serde_json::from_str::<StoredMlDatasetManifest>(&json)
                    .map_err(|_| "저장된 ML 매니페스트를 해석하지 못했습니다.".to_owned())
            })
            .transpose()?
    };
    let (
        dataset_source_kind,
        dataset_content_sha256,
        feature_schema_sha256,
        anchor_manifest_id,
        contract_version,
    ) = if let Some(manifest) = manifest {
        (
            MlDatasetSourceKind::Manifest,
            manifest.content_sha256,
            manifest.feature_schema_sha256,
            request.manifest_id.clone(),
            WORKER_CONTRACT_VERSION,
        )
    } else {
        let shard_set = shard_set_detail(bridge, &request.manifest_id).map_err(|error| {
            if error.contains("없습니다") {
                "학습 전에 ML 데이터셋 매니페스트 또는 shard set이 필요합니다.".to_owned()
            } else {
                error
            }
        })?;
        if request.algorithm != MlAlgorithm::Xgboost {
            return Err(
                "현재 shard-aware worker는 XGBoost hist 외부 메모리 학습만 허용합니다.".to_owned(),
            );
        }
        let anchor_manifest_id = shard_set
            .shards
            .first()
            .map(|shard| shard.manifest_id.clone())
            .ok_or_else(|| "shard set에 기준 매니페스트가 없습니다.".to_owned())?;
        (
            MlDatasetSourceKind::ShardSet,
            shard_set.combined_content_sha256,
            shard_set.feature_schema_sha256,
            anchor_manifest_id,
            SHARD_WORKER_CONTRACT_VERSION,
        )
    };
    let mut stored = StoredMlTrainingJob {
        job_id: request.job_id,
        manifest_id: request.manifest_id,
        dataset_source_kind,
        algorithm: request.algorithm,
        contract_version: contract_version.to_owned(),
        dataset_content_sha256,
        feature_schema_sha256,
        input_sha256: String::new(),
        code_version: request.code_version,
        random_seed: request.random_seed,
        horizon_ms: request.horizon_ms,
        timeout_seconds: request.timeout_seconds,
        memory_limit_mb: request.memory_limit_mb,
        max_threads: request.max_threads,
        hyperparameters: request.hyperparameters,
        status: "prepared".to_owned(),
        created_at_ms: request.requested_at_ms,
        updated_at_ms: request.requested_at_ms,
        live_order_allowed: false,
    };
    let hash_input = serde_json::to_vec(&stored)
        .map_err(|_| "ML 작업 입력을 직렬화하지 못했습니다.".to_owned())?;
    stored.input_sha256 = sha256_hex(&hash_input);
    let json = serde_json::to_string(&stored)
        .map_err(|_| "ML 작업을 직렬화하지 못했습니다.".to_owned())?;
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let existing = connection
        .query_row(
            "SELECT request_json, status, updated_at_ms FROM ml_training_jobs WHERE job_id = ?1",
            params![stored.job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "ML 작업을 조회하지 못했습니다.".to_owned())?;
    match existing {
        Some((existing, status, updated_at_ms)) if existing == json => {
            let source = connection
                .query_row(
                    "SELECT source_kind, source_id, source_content_sha256
                     FROM ml_training_job_sources WHERE job_id = ?1",
                    params![stored.job_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| "ML 작업 데이터 원천을 조회하지 못했습니다.".to_owned())?;
            let expected = (
                dataset_source_kind_key(stored.dataset_source_kind).to_owned(),
                stored.manifest_id.clone(),
                stored.dataset_content_sha256.clone(),
            );
            if source.as_ref() != Some(&expected) {
                return Err("저장된 ML 작업의 데이터 원천 계보가 다릅니다.".to_owned());
            }
            stored.status = status;
            stored.updated_at_ms = updated_at_ms;
        }
        Some(_) => return Err("같은 학습 작업 ID에 다른 입력이 이미 저장되어 있습니다.".to_owned()),
        None => {
            let transaction = connection
                .transaction()
                .map_err(|_| "ML 작업 저장 트랜잭션을 시작하지 못했습니다.".to_owned())?;
            transaction
                .execute(
                    "INSERT INTO ml_training_jobs
                     (job_id, manifest_id, algorithm, contract_version, input_sha256, status,
                      request_json, result_json, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'prepared', ?6, NULL, ?7, ?7)",
                    params![
                        stored.job_id,
                        anchor_manifest_id,
                        algorithm_key(stored.algorithm),
                        stored.contract_version,
                        stored.input_sha256,
                        json,
                        stored.created_at_ms,
                    ],
                )
                .map_err(|_| "ML 작업을 저장하지 못했습니다.".to_owned())?;
            transaction
                .execute(
                    "INSERT INTO ml_training_job_sources
                     (job_id, source_kind, source_id, source_content_sha256, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        stored.job_id,
                        dataset_source_kind_key(stored.dataset_source_kind),
                        stored.manifest_id,
                        stored.dataset_content_sha256,
                        stored.created_at_ms,
                    ],
                )
                .map_err(|_| "ML 작업 데이터 원천을 저장하지 못했습니다.".to_owned())?;
            transaction
                .commit()
                .map_err(|_| "ML 작업 저장 트랜잭션을 완료하지 못했습니다.".to_owned())?;
        }
    }
    Ok(stored)
}

fn prepared_training_job(
    bridge: &PersistenceBridge,
    job_id: &str,
) -> Result<StoredMlTrainingJob, String> {
    validate_identifier(job_id, "학습 작업")?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let (job_json, stored_status, updated_at_ms) = connection
        .query_row(
            "SELECT request_json, status, updated_at_ms FROM ml_training_jobs WHERE job_id = ?1",
            params![job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "ML 작업을 조회하지 못했습니다.".to_owned())?
        .ok_or_else(|| "준비된 ML 작업이 없습니다.".to_owned())?;
    let mut job: StoredMlTrainingJob = serde_json::from_str(&job_json)
        .map_err(|_| "저장된 ML 작업을 해석하지 못했습니다.".to_owned())?;
    job.status = stored_status;
    job.updated_at_ms = updated_at_ms;
    if job.contract_version != WORKER_CONTRACT_VERSION || job.status != "prepared" {
        if job.contract_version != SHARD_WORKER_CONTRACT_VERSION || job.status != "prepared" {
            return Err("현재 worker가 실행할 수 있는 준비 상태의 작업이 아닙니다.".to_owned());
        }
    }
    Ok(job)
}

pub(crate) fn training_job_source(
    bridge: &PersistenceBridge,
    job_id: &str,
) -> Result<MlTrainingJobSource, String> {
    let job = prepared_training_job(bridge, job_id)?;
    let source_lineage = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
        connection
            .query_row(
                "SELECT source_kind, source_id, source_content_sha256
                 FROM ml_training_job_sources WHERE job_id = ?1",
                params![job.job_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| "ML 작업 데이터 원천 계보를 조회하지 못했습니다.".to_owned())?
            .ok_or_else(|| "ML 작업 데이터 원천 계보가 없습니다.".to_owned())?
    };
    let expected_lineage = (
        dataset_source_kind_key(job.dataset_source_kind).to_owned(),
        job.manifest_id.clone(),
        job.dataset_content_sha256.clone(),
    );
    if source_lineage != expected_lineage {
        return Err("ML 작업 데이터 원천 계보가 준비 시점과 다릅니다.".to_owned());
    }
    if job.dataset_source_kind == MlDatasetSourceKind::ShardSet {
        let shard_set = shard_set_detail(bridge, &job.manifest_id)?;
        if job.algorithm != MlAlgorithm::Xgboost
            || job.contract_version != SHARD_WORKER_CONTRACT_VERSION
            || shard_set.combined_content_sha256 != job.dataset_content_sha256
            || shard_set.feature_schema_sha256 != job.feature_schema_sha256
        {
            return Err("ML shard set 또는 작업 계보가 준비 시점과 다릅니다.".to_owned());
        }
        return Ok(MlTrainingJobSource::ShardSet { job, shard_set });
    }
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let (manifest_json, payload_json) = connection
        .query_row(
            "SELECT manifest_json, payload_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
            params![job.manifest_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|_| "ML 작업 데이터셋을 조회하지 못했습니다.".to_owned())?;
    let manifest: StoredMlDatasetManifest = serde_json::from_str(&manifest_json)
        .map_err(|_| "저장된 ML 매니페스트를 해석하지 못했습니다.".to_owned())?;
    if sha256_hex(payload_json.as_bytes()) != manifest.content_sha256
        || manifest.content_sha256 != job.dataset_content_sha256
        || manifest.feature_schema_sha256 != job.feature_schema_sha256
    {
        return Err("ML 데이터셋 또는 피처 스키마 해시가 준비 시점과 다릅니다.".to_owned());
    }
    Ok(MlTrainingJobSource::Manifest(MlWorkerBundle {
        contract_version: WORKER_CONTRACT_VERSION.to_owned(),
        job,
        manifest,
        dataset_payload_json: payload_json,
        live_order_allowed: false,
    }))
}

pub(crate) fn training_job_bundle(
    bridge: &PersistenceBridge,
    job_id: &str,
) -> Result<MlWorkerBundle, String> {
    match training_job_source(bridge, job_id)? {
        MlTrainingJobSource::Manifest(bundle) => Ok(bundle),
        MlTrainingJobSource::ShardSet { .. } => Err(
            "shard 학습 입력은 ML runner의 격리 파일 staging 경로에서만 열 수 있습니다.".to_owned(),
        ),
    }
}

fn validate_artifact(
    algorithm: MlAlgorithm,
    artifact: &MlArtifactDescriptor,
) -> Result<(), String> {
    let safe_name = !artifact.file_name.is_empty()
        && artifact.file_name.len() <= 128
        && !artifact.file_name.contains(['/', '\\'])
        && !artifact.file_name.contains("..")
        && artifact
            .file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    let extension_matches = match artifact.format {
        MlArtifactFormat::LightgbmText => artifact.file_name.ends_with(".txt"),
        MlArtifactFormat::XgboostJson => artifact.file_name.ends_with(".json"),
        MlArtifactFormat::Safetensors => artifact.file_name.ends_with(".safetensors"),
        MlArtifactFormat::Onnx => artifact.file_name.ends_with(".onnx"),
    };
    let algorithm_matches = match algorithm {
        MlAlgorithm::Lightgbm => matches!(
            artifact.format,
            MlArtifactFormat::LightgbmText | MlArtifactFormat::Onnx
        ),
        MlAlgorithm::Xgboost => matches!(
            artifact.format,
            MlArtifactFormat::XgboostJson | MlArtifactFormat::Onnx
        ),
        MlAlgorithm::Chronos | MlAlgorithm::Timesfm => {
            matches!(
                artifact.format,
                MlArtifactFormat::Safetensors | MlArtifactFormat::Onnx
            )
        }
    };
    if !safe_name
        || !extension_matches
        || !algorithm_matches
        || !is_sha256(&artifact.sha256)
        || artifact.byte_size == 0
        || artifact.byte_size > MAX_ARTIFACT_BYTES
    {
        return Err("모델 아티팩트의 이름·포맷·해시·크기 계약이 올바르지 않습니다.".to_owned());
    }
    Ok(())
}

fn validate_metrics(metrics: &MlOosMetrics, completed_at_ms: u64) -> Result<(), String> {
    if metrics.sample_count == 0
        || metrics.fold_count == 0
        || metrics.log_loss_millionths > 100_000_000
        || metrics.brier_score_millionths > 1_000_000
        || metrics.expected_calibration_error_bps > 10_000
        || metrics.balanced_accuracy_bps > 10_000
        || metrics.evaluated_at_ms == 0
        || metrics.evaluated_at_ms > completed_at_ms
    {
        return Err("OOS 지표의 표본·범위·평가 시각이 올바르지 않습니다.".to_owned());
    }
    Ok(())
}

fn recompute_oos_metrics(
    predictions: &[MlOosPrediction],
    evaluated_at_ms: u64,
) -> Result<MlOosMetrics, String> {
    if predictions.is_empty() || predictions.len() > MAX_SAMPLES || evaluated_at_ms == 0 {
        return Err("OOS 원시 예측의 표본 수와 평가 시각이 올바르지 않습니다.".to_owned());
    }
    let mut sample_ids = BTreeSet::new();
    let mut fold_ids = BTreeSet::new();
    let mut class_total = [0_u64; 3];
    let mut class_correct = [0_u64; 3];
    let mut log_loss = 0.0_f64;
    let mut brier_numerator = 0_u128;
    let mut ece_bins = [(0_u64, 0_u64, 0_u64); 10];

    for prediction in predictions {
        validate_identifier(&prediction.sample_id, "OOS 표본")?;
        if !sample_ids.insert(prediction.sample_id.as_str()) || prediction.target_class > 2 {
            return Err("OOS 표본 ID가 중복되었거나 클래스가 올바르지 않습니다.".to_owned());
        }
        fold_ids.insert(prediction.fold_index);
        let probabilities = [
            prediction.probability_down_millionths,
            prediction.probability_flat_millionths,
            prediction.probability_up_millionths,
        ];
        if probabilities
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>()
            != u64::from(PROBABILITY_SCALE)
        {
            return Err("OOS 방향 확률 합계는 정확히 1,000,000이어야 합니다.".to_owned());
        }
        let target = usize::from(prediction.target_class);
        let target_probability = probabilities[target];
        if target_probability == 0 {
            return Err("OOS 정답 클래스 확률은 0일 수 없습니다.".to_owned());
        }
        log_loss -= (f64::from(target_probability) / f64::from(PROBABILITY_SCALE)).ln();
        for (class, probability) in probabilities.iter().enumerate() {
            let expected = if class == target {
                PROBABILITY_SCALE
            } else {
                0
            };
            let difference = i64::from(*probability) - i64::from(expected);
            brier_numerator += u128::from(difference.unsigned_abs()).pow(2);
        }
        let predicted = probabilities
            .iter()
            .enumerate()
            .max_by_key(|(index, probability)| (**probability, std::cmp::Reverse(*index)))
            .map(|(index, _)| index)
            .ok_or_else(|| "OOS 확률을 판정하지 못했습니다.".to_owned())?;
        class_total[target] += 1;
        if predicted == target {
            class_correct[target] += 1;
        }
        let confidence = probabilities[predicted];
        let bin = usize::try_from((confidence / 100_000).min(9))
            .map_err(|_| "OOS 보정 구간을 계산하지 못했습니다.".to_owned())?;
        ece_bins[bin].0 += 1;
        ece_bins[bin].1 += u64::from(confidence);
        ece_bins[bin].2 += u64::from(predicted == target);
    }
    if fold_ids
        .iter()
        .copied()
        .enumerate()
        .any(|(expected, actual)| u32::try_from(expected).ok() != Some(actual))
    {
        return Err("OOS fold index는 0부터 빠짐없이 이어져야 합니다.".to_owned());
    }
    let sample_count = u64::try_from(predictions.len())
        .map_err(|_| "OOS 표본 수가 지원 범위를 초과했습니다.".to_owned())?;
    let fold_count = u32::try_from(fold_ids.len())
        .map_err(|_| "OOS fold 수가 지원 범위를 초과했습니다.".to_owned())?;
    let observed_classes = class_total.iter().filter(|count| **count > 0).count();
    if observed_classes == 0 {
        return Err("OOS 평가 클래스가 없습니다.".to_owned());
    }
    let balanced_accuracy = class_total
        .iter()
        .zip(class_correct.iter())
        .filter(|(total, _)| **total > 0)
        .map(|(total, correct)| *correct as f64 / *total as f64)
        .sum::<f64>()
        / observed_classes as f64;
    let brier_denominator = u128::from(sample_count) * 3 * u128::from(PROBABILITY_SCALE);
    let brier_score_millionths =
        u64::try_from((brier_numerator + brier_denominator / 2) / brier_denominator)
            .map_err(|_| "OOS Brier score가 범위를 초과했습니다.".to_owned())?;
    let ece_numerator = ece_bins
        .iter()
        .map(|(count, confidence_sum, correct)| {
            if *count == 0 {
                0_u64
            } else {
                (correct * u64::from(PROBABILITY_SCALE)).abs_diff(*confidence_sum)
            }
        })
        .sum::<u64>();
    let ece_denominator = u128::from(sample_count) * u128::from(PROBABILITY_SCALE);
    let expected_calibration_error_bps =
        u64::try_from((u128::from(ece_numerator) * 10_000 + ece_denominator / 2) / ece_denominator)
            .map_err(|_| "OOS ECE가 범위를 초과했습니다.".to_owned())?;
    Ok(MlOosMetrics {
        sample_count,
        fold_count,
        log_loss_millionths: (log_loss / sample_count as f64 * 1_000_000.0).round() as u64,
        brier_score_millionths,
        expected_calibration_error_bps,
        balanced_accuracy_bps: (balanced_accuracy * 10_000.0).round() as u64,
        evaluated_at_ms,
    })
}

fn read_model_for_job(
    connection: &rusqlite::Connection,
    job_id: &str,
) -> Result<Option<StoredMlModelVersion>, String> {
    let json = connection
        .query_row(
            "SELECT record_json FROM ml_model_versions WHERE job_id = ?1",
            params![job_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "ML 모델 이력을 조회하지 못했습니다.".to_owned())?;
    json.map(|value| {
        serde_json::from_str(&value)
            .map_err(|_| "저장된 ML 모델 이력을 해석하지 못했습니다.".to_owned())
    })
    .transpose()
}

pub(crate) fn complete_training_job(
    bridge: &PersistenceBridge,
    request: MlTrainingJobCompleteRequest,
) -> Result<MlTrainingJobCompletion, String> {
    validate_identifier(&request.job_id, "학습 작업")?;
    if !is_sha256(&request.input_sha256) || request.completed_at_ms == 0 {
        return Err("학습 입력 해시와 완료 시각이 필요합니다.".to_owned());
    }
    let mut connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let row = connection
        .query_row(
            "SELECT request_json, status, result_json, updated_at_ms
             FROM ml_training_jobs WHERE job_id = ?1",
            params![request.job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, u64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|_| "ML 작업을 조회하지 못했습니다.".to_owned())?
        .ok_or_else(|| "준비된 ML 작업이 없습니다.".to_owned())?;
    let mut job: StoredMlTrainingJob = serde_json::from_str(&row.0)
        .map_err(|_| "저장된 ML 작업을 해석하지 못했습니다.".to_owned())?;
    job.status = row.1.clone();
    job.updated_at_ms = row.3;
    if job.input_sha256 != request.input_sha256 {
        return Err("완료 결과의 입력 해시가 준비된 작업과 다릅니다.".to_owned());
    }
    if request.completed_at_ms < job.created_at_ms {
        return Err("완료 시각은 작업 요청 시각보다 빠를 수 없습니다.".to_owned());
    }
    let result_json = serde_json::to_string(&request)
        .map_err(|_| "ML 결과를 직렬화하지 못했습니다.".to_owned())?;
    if row.1 != "prepared" {
        if row.2.as_deref() != Some(result_json.as_str()) {
            return Err("이미 종료된 ML 작업에 다른 결과를 저장할 수 없습니다.".to_owned());
        }
        return Ok(MlTrainingJobCompletion {
            model: read_model_for_job(&connection, &job.job_id)?,
            message: "동일한 완료 결과를 재확인했습니다.".to_owned(),
            job,
        });
    }

    if !request.succeeded {
        if request
            .failure_code
            .as_deref()
            .is_none_or(|value| validate_identifier(value, "실패 코드").is_err())
            || request.model_id.is_some()
            || request.model_version.is_some()
            || request.artifact.is_some()
            || request.metrics.is_some()
            || request.predictions.is_some()
        {
            return Err("실패 결과에는 제한된 실패 코드만 기록할 수 있습니다.".to_owned());
        }
        job.status = "failed".to_owned();
        job.updated_at_ms = request.completed_at_ms;
        connection
            .execute(
                "UPDATE ml_training_jobs
                 SET status='failed', result_json=?2, updated_at_ms=?3
                 WHERE job_id=?1 AND status='prepared'",
                params![request.job_id, result_json, request.completed_at_ms],
            )
            .map_err(|_| "ML 실패 결과를 저장하지 못했습니다.".to_owned())?;
        return Ok(MlTrainingJobCompletion {
            job,
            model: None,
            message: "학습 실패를 기록했으며 모델은 등록하지 않았습니다.".to_owned(),
        });
    }

    if request.failure_code.is_some() {
        return Err("성공 결과에는 실패 코드를 포함할 수 없습니다.".to_owned());
    }
    let model_id = request
        .model_id
        .as_deref()
        .ok_or_else(|| "성공 결과에는 모델 ID가 필요합니다.".to_owned())?;
    let model_version = request
        .model_version
        .as_deref()
        .ok_or_else(|| "성공 결과에는 모델 버전이 필요합니다.".to_owned())?;
    validate_identifier(model_id, "모델")?;
    validate_identifier(model_version, "모델 버전")?;
    let artifact = request
        .artifact
        .as_ref()
        .ok_or_else(|| "성공 결과에는 모델 아티팩트가 필요합니다.".to_owned())?;
    validate_artifact(job.algorithm, artifact)?;
    let metrics = request
        .metrics
        .as_ref()
        .ok_or_else(|| "성공 결과에는 OOS 지표가 필요합니다.".to_owned())?;
    validate_metrics(metrics, request.completed_at_ms)?;
    let predictions = request
        .predictions
        .as_deref()
        .ok_or_else(|| "성공 결과에는 OOS 원시 예측이 필요합니다.".to_owned())?;
    let recomputed_metrics = recompute_oos_metrics(predictions, metrics.evaluated_at_ms)?;
    if &recomputed_metrics != metrics {
        return Err("worker OOS 지표가 Rust 재계산 결과와 일치하지 않습니다.".to_owned());
    }

    let asset_class = match job.dataset_source_kind {
        MlDatasetSourceKind::Manifest => {
            let manifest_json = connection
                .query_row(
                    "SELECT manifest_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
                    params![job.manifest_id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|_| "ML 매니페스트를 조회하지 못했습니다.".to_owned())?;
            let manifest: StoredMlDatasetManifest = serde_json::from_str(&manifest_json)
                .map_err(|_| "저장된 ML 매니페스트를 해석하지 못했습니다.".to_owned())?;
            if manifest.content_sha256 != job.dataset_content_sha256 {
                return Err("완료 시점 ML 매니페스트 해시가 준비 시점과 다릅니다.".to_owned());
            }
            manifest.asset.asset_class
        }
        MlDatasetSourceKind::ShardSet => {
            let shard_set = verified_shard_set(&connection, &job.manifest_id)?;
            if shard_set.combined_content_sha256 != job.dataset_content_sha256
                || shard_set.feature_schema_sha256 != job.feature_schema_sha256
            {
                return Err("완료 시점 ML shard set 계보가 준비 시점과 다릅니다.".to_owned());
            }
            shard_set.asset.asset_class
        }
    };
    let model = StoredMlModelVersion {
        model_id: model_id.to_owned(),
        model_version: model_version.to_owned(),
        job_id: job.job_id.clone(),
        manifest_id: job.manifest_id.clone(),
        dataset_source_kind: job.dataset_source_kind,
        asset_class,
        algorithm: job.algorithm,
        artifact: artifact.clone(),
        metrics: metrics.clone(),
        status: "candidate_review".to_owned(),
        created_at_ms: request.completed_at_ms,
        live_order_allowed: false,
    };
    let model_json = serde_json::to_string(&model)
        .map_err(|_| "모델 버전을 직렬화하지 못했습니다.".to_owned())?;
    let metrics_json = serde_json::to_string(&model.metrics)
        .map_err(|_| "모델 지표를 직렬화하지 못했습니다.".to_owned())?;
    job.status = "completed".to_owned();
    job.updated_at_ms = request.completed_at_ms;
    let transaction = connection
        .transaction()
        .map_err(|_| "ML 결과 트랜잭션을 시작하지 못했습니다.".to_owned())?;
    transaction
        .execute(
            "INSERT INTO ml_model_versions
             (model_id, model_version, job_id, asset_class, algorithm, artifact_format,
              artifact_sha256, status, metrics_json, record_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'candidate_review', ?8, ?9, ?10)",
            params![
                model.model_id,
                model.model_version,
                model.job_id,
                asset_class_key(model.asset_class),
                algorithm_key(model.algorithm),
                artifact_format_key(model.artifact.format),
                model.artifact.sha256,
                metrics_json,
                model_json,
                model.created_at_ms,
            ],
        )
        .map_err(|_| "동일 모델 버전이 이미 존재하거나 모델 등록에 실패했습니다.".to_owned())?;
    transaction
        .execute(
            "UPDATE ml_training_jobs
             SET status='completed', result_json=?2, updated_at_ms=?3
             WHERE job_id=?1 AND status='prepared'",
            params![request.job_id, result_json, request.completed_at_ms],
        )
        .map_err(|_| "ML 작업 완료 상태를 저장하지 못했습니다.".to_owned())?;
    transaction
        .commit()
        .map_err(|_| "ML 결과 트랜잭션을 완료하지 못했습니다.".to_owned())?;
    Ok(MlTrainingJobCompletion {
        job,
        model: Some(model),
        message: "모델을 검토 후보로 등록했습니다. 자동 배치와 실주문 권한은 없습니다.".to_owned(),
    })
}

fn pipeline_history(bridge: &PersistenceBridge, limit: u16) -> Result<MlPipelineHistory, String> {
    let limit = limit.clamp(1, MAX_HISTORY_LIMIT);
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML 로컬 저장소를 사용할 수 없습니다.".to_owned())?;
    let manifests = read_json_column::<StoredMlDatasetManifest>(
        &connection,
        "SELECT manifest_json FROM ml_dataset_manifests ORDER BY created_at_ms DESC LIMIT ?1",
        limit,
    )?;
    let jobs = read_training_jobs(&connection, limit)?;
    let models = read_json_column::<StoredMlModelVersion>(
        &connection,
        "SELECT record_json FROM ml_model_versions ORDER BY created_at_ms DESC LIMIT ?1",
        limit,
    )?;
    Ok(MlPipelineHistory {
        manifests,
        jobs,
        models,
        live_order_allowed: false,
    })
}

fn read_training_jobs(
    connection: &rusqlite::Connection,
    limit: u16,
) -> Result<Vec<StoredMlTrainingJob>, String> {
    let mut statement = connection
        .prepare(
            "SELECT request_json, status, updated_at_ms FROM ml_training_jobs
             ORDER BY created_at_ms DESC LIMIT ?1",
        )
        .map_err(|_| "ML 작업 이력 조회를 준비하지 못했습니다.".to_owned())?;
    let rows = statement
        .query_map(params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })
        .map_err(|_| "ML 작업 이력을 조회하지 못했습니다.".to_owned())?;
    let mut jobs = Vec::new();
    for row in rows {
        let (json, status, updated_at_ms) =
            row.map_err(|_| "ML 작업 이력을 읽지 못했습니다.".to_owned())?;
        let mut job: StoredMlTrainingJob = serde_json::from_str(&json)
            .map_err(|_| "저장된 ML 작업을 해석하지 못했습니다.".to_owned())?;
        job.status = status;
        job.updated_at_ms = updated_at_ms;
        jobs.push(job);
    }
    Ok(jobs)
}

fn read_json_column<T: for<'de> Deserialize<'de>>(
    connection: &rusqlite::Connection,
    sql: &str,
    limit: u16,
) -> Result<Vec<T>, String> {
    let mut statement = connection
        .prepare(sql)
        .map_err(|_| "ML 이력 조회를 준비하지 못했습니다.".to_owned())?;
    let records = statement
        .query_map(params![limit], |row| row.get::<_, String>(0))
        .map_err(|_| "ML 이력을 조회하지 못했습니다.".to_owned())?
        .map(|row| {
            row.map_err(|_| "ML 이력을 읽지 못했습니다.".to_owned())
                .and_then(|json| {
                    serde_json::from_str(&json)
                        .map_err(|_| "저장된 ML 이력을 해석하지 못했습니다.".to_owned())
                })
        })
        .collect();
    records
}

fn verified_shard_set(
    connection: &rusqlite::Connection,
    shard_set_id: &str,
) -> Result<StoredMlDatasetShardSet, String> {
    let record_json = connection
        .query_row(
            "SELECT record_json FROM ml_dataset_shard_sets WHERE shard_set_id = ?1",
            params![shard_set_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| "ML shard set을 조회하지 못했습니다.".to_owned())?
        .ok_or_else(|| "ML shard set이 없습니다.".to_owned())?;
    let stored: StoredMlDatasetShardSet = serde_json::from_str(&record_json)
        .map_err(|_| "저장된 ML shard set을 해석하지 못했습니다.".to_owned())?;
    let expected_hash = serde_json::to_vec(&(
        &stored.dataset_id,
        &stored.asset,
        &stored.split,
        &stored.feature_schema_sha256,
        &stored.shards,
    ))
    .map_err(|_| "ML shard set 해시를 재계산하지 못했습니다.".to_owned())?;
    if sha256_hex(&expected_hash) != stored.combined_content_sha256 {
        return Err("저장된 ML shard set 해시가 일치하지 않습니다.".to_owned());
    }
    for shard in &stored.shards {
        let (content_sha256, payload_json) = connection
            .query_row(
                "SELECT content_sha256, payload_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
                params![shard.manifest_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| "ML shard 매니페스트를 재검증하지 못했습니다.".to_owned())?
            .ok_or_else(|| format!("ML shard 매니페스트가 없습니다: {}", shard.manifest_id))?;
        if content_sha256 != shard.content_sha256
            || sha256_hex(payload_json.as_bytes()) != shard.content_sha256
        {
            return Err(format!(
                "ML shard payload 해시가 일치하지 않습니다: {}",
                shard.manifest_id
            ));
        }
    }
    Ok(stored)
}

fn shard_set_detail(
    bridge: &PersistenceBridge,
    shard_set_id: &str,
) -> Result<StoredMlDatasetShardSet, String> {
    validate_identifier(shard_set_id, "shard set")?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML shard 저장소를 사용할 수 없습니다.".to_owned())?;
    verified_shard_set(&connection, shard_set_id)
}

pub(crate) fn shard_payload(
    bridge: &PersistenceBridge,
    manifest_id: &str,
) -> Result<(StoredMlDatasetManifest, String), String> {
    validate_identifier(manifest_id, "shard 매니페스트")?;
    let connection = bridge
        .connection
        .lock()
        .map_err(|_| "ML shard 저장소를 사용할 수 없습니다.".to_owned())?;
    let (manifest_json, payload_json) = connection
        .query_row(
            "SELECT manifest_json, payload_json FROM ml_dataset_manifests WHERE manifest_id = ?1",
            params![manifest_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| "ML shard payload를 조회하지 못했습니다.".to_owned())?
        .ok_or_else(|| "ML shard payload가 없습니다.".to_owned())?;
    let manifest: StoredMlDatasetManifest = serde_json::from_str(&manifest_json)
        .map_err(|_| "저장된 ML shard 매니페스트를 해석하지 못했습니다.".to_owned())?;
    if sha256_hex(payload_json.as_bytes()) != manifest.content_sha256 {
        return Err("ML shard payload 해시가 매니페스트와 다릅니다.".to_owned());
    }
    Ok((manifest, payload_json))
}

fn shard_set_history(
    bridge: &PersistenceBridge,
    limit: u16,
) -> Result<MlDatasetShardSetHistory, String> {
    if limit == 0 || limit > MAX_HISTORY_LIMIT {
        return Err(format!(
            "ML shard set 이력은 1~{MAX_HISTORY_LIMIT}개까지 조회할 수 있습니다."
        ));
    }
    let shard_set_ids = {
        let connection = bridge
            .connection
            .lock()
            .map_err(|_| "ML shard 저장소를 사용할 수 없습니다.".to_owned())?;
        let mut statement = connection
            .prepare(
                "SELECT shard_set_id FROM ml_dataset_shard_sets ORDER BY created_at_ms DESC LIMIT ?1",
            )
            .map_err(|_| "ML shard set 이력을 준비하지 못했습니다.".to_owned())?;
        let rows = statement
            .query_map(params![limit], |row| row.get::<_, String>(0))
            .map_err(|_| "ML shard set 이력을 조회하지 못했습니다.".to_owned())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| "ML shard set 이력을 읽지 못했습니다.".to_owned())?
    };
    let shard_sets = shard_set_ids
        .iter()
        .map(|shard_set_id| shard_set_detail(bridge, shard_set_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(MlDatasetShardSetHistory {
        shard_sets,
        worker_ready: false,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn ml_dataset_manifest_create(
    state: State<'_, PersistenceBridge>,
    request: MlDatasetManifestCreateRequest,
) -> Result<StoredMlDatasetManifest, String> {
    create_dataset_manifest(&state, request)
}

#[tauri::command]
pub fn ml_dataset_shard_set_create(
    state: State<'_, PersistenceBridge>,
    request: MlDatasetShardSetCreateRequest,
) -> Result<StoredMlDatasetShardSet, String> {
    shard_set_record(&state, request)
}

#[tauri::command]
pub fn ml_dataset_shard_set_detail(
    state: State<'_, PersistenceBridge>,
    shard_set_id: String,
) -> Result<StoredMlDatasetShardSet, String> {
    shard_set_detail(&state, &shard_set_id)
}

#[tauri::command]
pub fn ml_dataset_shard_set_history(
    state: State<'_, PersistenceBridge>,
    limit: u16,
) -> Result<MlDatasetShardSetHistory, String> {
    shard_set_history(&state, limit)
}

#[tauri::command]
pub fn ml_training_job_prepare(
    state: State<'_, PersistenceBridge>,
    request: MlTrainingJobPrepareRequest,
) -> Result<StoredMlTrainingJob, String> {
    prepare_training_job(&state, request)
}

#[tauri::command]
pub fn ml_training_job_bundle(
    state: State<'_, PersistenceBridge>,
    job_id: String,
) -> Result<MlWorkerBundle, String> {
    training_job_bundle(&state, &job_id)
}

#[tauri::command]
pub fn ml_training_job_complete(
    state: State<'_, PersistenceBridge>,
    request: MlTrainingJobCompleteRequest,
) -> Result<MlTrainingJobCompletion, String> {
    complete_training_job(&state, request)
}

#[tauri::command]
pub fn ml_pipeline_history(
    state: State<'_, PersistenceBridge>,
    limit: u16,
) -> Result<MlPipelineHistory, String> {
    pipeline_history(&state, limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data_quality::TemporalMetadata,
        forecast_runtime::{save_dataset_audit, ForecastDatasetAuditRequest},
    };

    fn contract() -> ForecastAssetContract {
        ForecastAssetContract {
            contract_id: "kr-stock-005930-v1".to_owned(),
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

    fn manifest_request() -> MlDatasetManifestCreateRequest {
        let decisions = [100, 200, 300, 400, 500, 600];
        let samples = decisions
            .iter()
            .enumerate()
            .map(|(index, decision)| ForecastSample {
                sample_id: format!("sample-{index}"),
                decision_time_ms: *decision,
                target_observed_at_ms: decision + 20,
                target_class: (index % 3) as u8,
            })
            .collect::<Vec<_>>();
        let features = samples
            .iter()
            .map(|sample| ForecastFeature {
                feature_id: "price.close".to_owned(),
                sample_id: sample.sample_id.clone(),
                source_record_id: format!("bar-{}", sample.sample_id),
                dataset_version: "dataset-ml-v1".to_owned(),
                metadata: TemporalMetadata {
                    event_time_ms: sample.decision_time_ms - 10,
                    available_at_ms: sample.decision_time_ms,
                    ingested_at_ms: sample.decision_time_ms + 1,
                    source: "official-bars".to_owned(),
                    source_revision: "v1".to_owned(),
                },
                value_scaled: 70_000,
                value_scale: 1,
                quality_flags: vec![],
            })
            .collect::<Vec<_>>();
        MlDatasetManifestCreateRequest {
            manifest_id: "manifest-v1".to_owned(),
            audit_id: "audit-ml-v1".to_owned(),
            dataset_id: "dataset-ml-v1".to_owned(),
            asset: contract(),
            samples,
            features,
            split: TimeSplit {
                train_end_ms: 220,
                validation_start_ms: 250,
                validation_end_ms: 420,
                test_start_ms: 450,
            },
            audit: ForecastDatasetAuditInput {
                expected_feature_ids: vec!["price.close".to_owned()],
                corporate_action_coverage_confirmed: true,
                trading_session_coverage_confirmed: true,
                listing_history_checked: true,
            },
        }
    }

    fn seed_audit(bridge: &PersistenceBridge, request: &MlDatasetManifestCreateRequest) {
        save_dataset_audit(
            bridge,
            ForecastDatasetAuditRequest {
                audit_id: request.audit_id.clone(),
                dataset_id: request.dataset_id.clone(),
                asset: request.asset.clone(),
                samples: request.samples.clone(),
                features: request.features.clone(),
                split: request.split.clone(),
                audit: request.audit.clone(),
            },
        )
        .expect("audit");
    }

    fn shard_manifest_request(suffix: &str, decisions: [u64; 3]) -> MlDatasetManifestCreateRequest {
        let mut request = manifest_request();
        request.manifest_id = format!("manifest-shard-{suffix}");
        request.audit_id = format!("audit-shard-{suffix}");
        request.dataset_id = format!("dataset-shard-{suffix}");
        request.samples = decisions
            .iter()
            .enumerate()
            .map(|(index, decision)| ForecastSample {
                sample_id: format!("sample-{suffix}-{index}"),
                decision_time_ms: *decision,
                target_observed_at_ms: decision + 20,
                target_class: u8::try_from(index).expect("class"),
            })
            .collect();
        request.features = request
            .samples
            .iter()
            .map(|sample| ForecastFeature {
                feature_id: "price.close".to_owned(),
                sample_id: sample.sample_id.clone(),
                source_record_id: format!("bar-{}", sample.sample_id),
                dataset_version: request.dataset_id.clone(),
                metadata: TemporalMetadata {
                    event_time_ms: sample.decision_time_ms - 10,
                    available_at_ms: sample.decision_time_ms,
                    ingested_at_ms: sample.decision_time_ms + 1,
                    source: "official-bars".to_owned(),
                    source_revision: "v1".to_owned(),
                },
                value_scaled: 70_000,
                value_scale: 1,
                quality_flags: vec![],
            })
            .collect();
        request
    }

    fn prepare_request() -> MlTrainingJobPrepareRequest {
        MlTrainingJobPrepareRequest {
            job_id: "job-v1".to_owned(),
            manifest_id: "manifest-v1".to_owned(),
            algorithm: MlAlgorithm::Xgboost,
            code_version: "git-deadbeef".to_owned(),
            random_seed: 42,
            horizon_ms: 86_400_000,
            timeout_seconds: 3_600,
            memory_limit_mb: 4_096,
            max_threads: 4,
            hyperparameters: BTreeMap::from([
                ("max_depth".to_owned(), Value::from(6)),
                ("eta".to_owned(), Value::from(0.05)),
            ]),
            requested_at_ms: 1_000,
        }
    }

    #[test]
    fn manifest_rejects_target_leakage_across_split() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let audit_request = manifest_request();
        seed_audit(&bridge, &audit_request);
        let mut request = manifest_request();
        request.samples[1].target_observed_at_ms = request.split.validation_start_ms;
        let error = create_dataset_manifest(&bridge, request).expect_err("leakage");
        assert!(error.contains("누수"));
    }

    #[test]
    fn manifest_and_training_job_are_immutable_and_hash_pinned() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = manifest_request();
        seed_audit(&bridge, &request);
        let manifest = create_dataset_manifest(&bridge, request.clone()).expect("manifest");
        let replay = create_dataset_manifest(&bridge, request).expect("manifest replay");
        assert_eq!(manifest.content_sha256, replay.content_sha256);
        assert!(is_sha256(&manifest.content_sha256));
        let job = prepare_training_job(&bridge, prepare_request()).expect("job");
        assert_eq!(job.dataset_content_sha256, manifest.content_sha256);
        assert!(is_sha256(&job.input_sha256));
        assert!(!job.live_order_allowed);
        let bundle = training_job_bundle(&bridge, &job.job_id).expect("worker bundle");
        assert_eq!(bundle.manifest.content_sha256, manifest.content_sha256);
        assert!(!bundle.live_order_allowed);
    }

    #[test]
    fn shard_set_is_immutable_ordered_and_xgboost_worker_ready() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let first = shard_manifest_request("a", [100, 300, 500]);
        let second = shard_manifest_request("b", [200, 400, 600]);
        for request in [&first, &second] {
            seed_audit(&bridge, request);
            create_dataset_manifest(&bridge, request.clone()).expect("manifest");
        }
        let request = MlDatasetShardSetCreateRequest {
            shard_set_id: "shard-set-v1".to_owned(),
            dataset_id: "logical-minute-history-v1".to_owned(),
            manifest_ids: vec![first.manifest_id.clone(), second.manifest_id.clone()],
        };
        let stored = shard_set_record(&bridge, request.clone()).expect("shard set");
        let replay = shard_set_record(&bridge, request).expect("idempotent replay");
        assert_eq!(
            stored.combined_content_sha256,
            replay.combined_content_sha256
        );
        assert_eq!(stored.sample_count, 6);
        assert_eq!(stored.feature_count, 6);
        assert!(!stored.worker_ready);
        assert!(!stored.live_order_allowed);
        assert_eq!(
            shard_set_detail(&bridge, "shard-set-v1")
                .expect("verified detail")
                .shard_count,
            2
        );
        assert_eq!(
            shard_set_history(&bridge, 10)
                .expect("verified history")
                .shard_sets
                .len(),
            1
        );
        let mut prepare = prepare_request();
        prepare.job_id = "job-shard-v1".to_owned();
        prepare.manifest_id = "shard-set-v1".to_owned();
        prepare.algorithm = MlAlgorithm::Xgboost;
        let job = prepare_training_job(&bridge, prepare).expect("shard XGBoost job");
        assert_eq!(job.dataset_source_kind, MlDatasetSourceKind::ShardSet);
        assert_eq!(job.contract_version, SHARD_WORKER_CONTRACT_VERSION);
        assert!(matches!(
            training_job_source(&bridge, &job.job_id).expect("shard source"),
            MlTrainingJobSource::ShardSet { .. }
        ));
        assert!(training_job_bundle(&bridge, &job.job_id)
            .expect_err("inline bundle blocked")
            .contains("staging"));
        {
            let connection = bridge.connection.lock().expect("connection");
            connection
                .execute(
                    "UPDATE ml_training_job_sources SET source_content_sha256=?1 WHERE job_id=?2",
                    params!["0".repeat(64), job.job_id],
                )
                .expect("tamper lineage");
        }
        assert!(training_job_source(&bridge, &job.job_id)
            .expect_err("tampered source lineage rejected")
            .contains("계보"));
        {
            let connection = bridge.connection.lock().expect("connection");
            connection
                .execute(
                    "UPDATE ml_training_job_sources SET source_content_sha256=?1 WHERE job_id=?2",
                    params![job.dataset_content_sha256, job.job_id],
                )
                .expect("restore lineage");
        }

        let mut lightgbm = prepare_request();
        lightgbm.job_id = "job-shard-lightgbm-v1".to_owned();
        lightgbm.manifest_id = "shard-set-v1".to_owned();
        lightgbm.algorithm = MlAlgorithm::Lightgbm;
        assert!(prepare_training_job(&bridge, lightgbm)
            .expect_err("LightGBM shard blocked")
            .contains("XGBoost"));

        {
            let connection = bridge.connection.lock().expect("connection");
            connection
                .execute(
                    "UPDATE ml_dataset_manifests SET payload_json=payload_json || ' ' WHERE manifest_id=?1",
                    params![first.manifest_id],
                )
                .expect("tamper child shard");
        }
        assert!(shard_set_detail(&bridge, "shard-set-v1")
            .expect_err("tampered child rejected")
            .contains("해시"));
    }

    #[test]
    fn shard_set_rejects_reversed_overlap_duplicate_and_tampering() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let first = shard_manifest_request("c", [100, 300, 500]);
        let second = shard_manifest_request("d", [200, 400, 600]);
        for request in [&first, &second] {
            seed_audit(&bridge, request);
            create_dataset_manifest(&bridge, request.clone()).expect("manifest");
        }
        assert!(shard_set_record(
            &bridge,
            MlDatasetShardSetCreateRequest {
                shard_set_id: "shard-set-reversed".to_owned(),
                dataset_id: "logical-reversed".to_owned(),
                manifest_ids: vec![second.manifest_id.clone(), first.manifest_id.clone()],
            },
        )
        .expect_err("reversed")
        .contains("역전"));
        assert!(shard_set_record(
            &bridge,
            MlDatasetShardSetCreateRequest {
                shard_set_id: "shard-set-duplicate".to_owned(),
                dataset_id: "logical-duplicate".to_owned(),
                manifest_ids: vec![first.manifest_id.clone(), first.manifest_id.clone()],
            },
        )
        .expect_err("duplicate")
        .contains("중복"));

        {
            let connection = bridge.connection.lock().expect("connection");
            connection
                .execute(
                    "UPDATE ml_dataset_manifests SET payload_json=payload_json || ' ' WHERE manifest_id=?1",
                    params![second.manifest_id],
                )
                .expect("tamper fixture");
        }
        assert!(shard_set_record(
            &bridge,
            MlDatasetShardSetCreateRequest {
                shard_set_id: "shard-set-tampered".to_owned(),
                dataset_id: "logical-tampered".to_owned(),
                manifest_ids: vec![first.manifest_id, second.manifest_id],
            },
        )
        .expect_err("tampered")
        .contains("해시"));
    }

    #[test]
    fn worker_bundle_rejects_tampered_dataset_payload() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = manifest_request();
        seed_audit(&bridge, &request);
        create_dataset_manifest(&bridge, request).expect("manifest");
        let job = prepare_training_job(&bridge, prepare_request()).expect("job");
        {
            let connection = bridge.connection.lock().expect("lock");
            connection
                .execute(
                    "UPDATE ml_dataset_manifests SET payload_json='{}' WHERE manifest_id='manifest-v1'",
                    [],
                )
                .expect("tamper fixture");
        }
        let error = training_job_bundle(&bridge, &job.job_id).expect_err("tamper rejected");
        assert!(error.contains("해시"));
    }

    #[test]
    fn worker_failure_never_registers_a_model() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = manifest_request();
        seed_audit(&bridge, &request);
        create_dataset_manifest(&bridge, request).expect("manifest");
        let job = prepare_training_job(&bridge, prepare_request()).expect("job");
        let completion_request = MlTrainingJobCompleteRequest {
            job_id: job.job_id.clone(),
            input_sha256: job.input_sha256,
            completed_at_ms: 2_000,
            succeeded: false,
            failure_code: Some("worker_timeout".to_owned()),
            model_id: None,
            model_version: None,
            artifact: None,
            metrics: None,
            predictions: None,
        };
        let completion =
            complete_training_job(&bridge, completion_request.clone()).expect("failure stored");
        assert_eq!(completion.job.status, "failed");
        assert!(completion.model.is_none());
        let replay = complete_training_job(&bridge, completion_request).expect("idempotent");
        assert_eq!(replay.job.status, "failed");
    }

    #[test]
    fn successful_worker_result_is_only_a_review_candidate() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = manifest_request();
        seed_audit(&bridge, &request);
        create_dataset_manifest(&bridge, request).expect("manifest");
        let job = prepare_training_job(&bridge, prepare_request()).expect("job");
        let predictions = vec![
            MlOosPrediction {
                sample_id: "oos-1".to_owned(),
                fold_index: 0,
                target_class: 0,
                probability_down_millionths: 700_000,
                probability_flat_millionths: 200_000,
                probability_up_millionths: 100_000,
            },
            MlOosPrediction {
                sample_id: "oos-2".to_owned(),
                fold_index: 0,
                target_class: 1,
                probability_down_millionths: 100_000,
                probability_flat_millionths: 700_000,
                probability_up_millionths: 200_000,
            },
            MlOosPrediction {
                sample_id: "oos-3".to_owned(),
                fold_index: 0,
                target_class: 2,
                probability_down_millionths: 100_000,
                probability_flat_millionths: 200_000,
                probability_up_millionths: 700_000,
            },
        ];
        let metrics = recompute_oos_metrics(&predictions, 1_900).expect("metrics");
        let completion_request = MlTrainingJobCompleteRequest {
            job_id: job.job_id.clone(),
            input_sha256: job.input_sha256,
            completed_at_ms: 2_000,
            succeeded: true,
            failure_code: None,
            model_id: Some("direction-model".to_owned()),
            model_version: Some("1.0.0".to_owned()),
            artifact: Some(MlArtifactDescriptor {
                file_name: "model.json".to_owned(),
                format: MlArtifactFormat::XgboostJson,
                sha256: "a".repeat(64),
                byte_size: 10_000,
            }),
            metrics: Some(metrics),
            predictions: Some(predictions),
        };
        let completion =
            complete_training_job(&bridge, completion_request.clone()).expect("completion");
        let model = completion.model.expect("candidate model");
        assert_eq!(model.status, "candidate_review");
        assert!(!model.live_order_allowed);
        let replay = complete_training_job(&bridge, completion_request).expect("idempotent");
        assert!(replay.model.is_some());
        let prepare_replay =
            prepare_training_job(&bridge, prepare_request()).expect("idempotent prepare");
        assert_eq!(prepare_replay.status, "completed");
        assert!(training_job_bundle(&bridge, &prepare_replay.job_id).is_err());
        let history = pipeline_history(&bridge, 10).expect("history");
        assert_eq!(history.models.len(), 1);
        assert_eq!(history.jobs[0].status, "completed");
        assert!(!history.live_order_allowed);
    }

    #[test]
    fn worker_metric_mismatch_is_rejected_before_model_registration() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = manifest_request();
        seed_audit(&bridge, &request);
        create_dataset_manifest(&bridge, request).expect("manifest");
        let job = prepare_training_job(&bridge, prepare_request()).expect("job");
        let predictions = vec![MlOosPrediction {
            sample_id: "oos-only".to_owned(),
            fold_index: 0,
            target_class: 2,
            probability_down_millionths: 100_000,
            probability_flat_millionths: 200_000,
            probability_up_millionths: 700_000,
        }];
        let mut metrics = recompute_oos_metrics(&predictions, 1_900).expect("metrics");
        metrics.balanced_accuracy_bps -= 1;
        let error = complete_training_job(
            &bridge,
            MlTrainingJobCompleteRequest {
                job_id: job.job_id,
                input_sha256: job.input_sha256,
                completed_at_ms: 2_000,
                succeeded: true,
                failure_code: None,
                model_id: Some("direction-model".to_owned()),
                model_version: Some("1.0.0".to_owned()),
                artifact: Some(MlArtifactDescriptor {
                    file_name: "model.json".to_owned(),
                    format: MlArtifactFormat::XgboostJson,
                    sha256: "c".repeat(64),
                    byte_size: 100,
                }),
                metrics: Some(metrics),
                predictions: Some(predictions),
            },
        )
        .expect_err("mismatch rejected");
        assert!(error.contains("Rust 재계산"));
        assert!(pipeline_history(&bridge, 10)
            .expect("history")
            .models
            .is_empty());
    }

    #[test]
    fn rust_and_python_share_the_same_oos_metric_fixture() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Fixture {
            predictions: Vec<MlOosPrediction>,
            metrics: MlOosMetrics,
        }
        let fixture: Fixture = serde_json::from_str(include_str!(
            "../../ml-worker/tests/fixtures/oos_predictions_v1.json"
        ))
        .expect("shared fixture");
        assert_eq!(
            recompute_oos_metrics(&fixture.predictions, fixture.metrics.evaluated_at_ms)
                .expect("recomputed metrics"),
            fixture.metrics
        );
    }

    #[test]
    fn oos_probability_overflow_input_is_rejected_without_panicking() {
        let predictions = vec![MlOosPrediction {
            sample_id: "overflow".to_owned(),
            fold_index: 0,
            target_class: 0,
            probability_down_millionths: u32::MAX,
            probability_flat_millionths: u32::MAX,
            probability_up_millionths: u32::MAX,
        }];
        assert!(recompute_oos_metrics(&predictions, 2_000).is_err());
    }

    #[test]
    fn artifact_rejects_path_traversal_and_format_mismatch() {
        let artifact = MlArtifactDescriptor {
            file_name: "../model.pkl".to_owned(),
            format: MlArtifactFormat::XgboostJson,
            sha256: "b".repeat(64),
            byte_size: 100,
        };
        assert!(validate_artifact(MlAlgorithm::Xgboost, &artifact).is_err());
    }
}
