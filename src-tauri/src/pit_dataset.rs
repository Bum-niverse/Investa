use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::{
    data_quality::{QualityFlag, TemporalMetadata},
    forecast::{
        audit_forecast_dataset, ForecastAssetClass, ForecastAssetContract,
        ForecastDatasetAuditInput, ForecastDatasetAuditReview, ForecastFeature, ForecastSample,
        TimeSplit,
    },
    forecast_runtime::{
        save_dataset_audit, ForecastDatasetAuditRequest, StoredForecastDatasetAudit,
    },
    ml_pipeline::{
        create_dataset_manifest, MlDatasetManifestCreateRequest, StoredMlDatasetManifest,
    },
    persistence::PersistenceBridge,
    pit_providers::{
        completed_collection_covers, load_stored_range, PitOfficialProvider, PitProviderInterval,
        PitStoredRangeRequest,
    },
};

const BUILDER_VERSION: &str = "pit-dataset-builder-v1";
const MAX_PRICE_OBSERVATIONS: usize = 20_001;
const MAX_FEATURE_OBSERVATIONS: usize = 200_000;
const MAX_BOUNDARY_EVENTS: usize = 20_000;
const MAX_COLLECTION_WINDOWS: usize = 10_000;
const DERIVED_PRICE_WARMUP_BARS: usize = 5;
const DERIVED_PRICE_FEATURE_IDS: [&str; 3] = ["pit_return_1", "pit_return_5", "pit_ma_gap_5"];

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitInterval {
    Minute1,
    Minute3,
    Minute5,
    Minute15,
    Minute30,
    Hour1,
    Hour4,
    Day1,
}

impl PitInterval {
    fn duration_ms(self) -> u64 {
        match self {
            Self::Minute1 => 60_000,
            Self::Minute3 => 180_000,
            Self::Minute5 => 300_000,
            Self::Minute15 => 900_000,
            Self::Minute30 => 1_800_000,
            Self::Hour1 => 3_600_000,
            Self::Hour4 => 14_400_000,
            Self::Day1 => 86_400_000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionPlanRequest {
    pub asset: ForecastAssetContract,
    pub interval: PitInterval,
    pub start_ms: u64,
    pub end_exclusive_ms: u64,
    pub maximum_rows_per_window: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionWindow {
    pub window_index: usize,
    pub start_ms: u64,
    pub end_exclusive_ms: u64,
    pub maximum_rows: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitCollectionPlan {
    pub builder_version: &'static str,
    pub asset_contract_id: String,
    pub interval: PitInterval,
    pub interval_ms: u64,
    pub windows: Vec<PitCollectionWindow>,
    pub provider_credentials_required: bool,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitPriceBasis {
    Close,
    AdjustedClose,
    Settlement,
    Mark,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitBoundaryHandling {
    ExcludeCrossing,
    PriceReturnOnly,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitLabelPolicy {
    pub horizon_bars: u16,
    pub up_threshold_bps: u32,
    pub down_threshold_bps: u32,
    pub price_basis: PitPriceBasis,
    pub corporate_action_handling: PitBoundaryHandling,
    pub expiry_handling: PitBoundaryHandling,
    pub rollover_handling: PitBoundaryHandling,
    pub funding_handling: PitBoundaryHandling,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitPriceObservation {
    pub record_id: String,
    pub bar_end_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub source: String,
    pub source_revision: String,
    pub close_scaled: i64,
    pub price_scale: u64,
    pub final_bar: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitFeatureObservation {
    pub feature_id: String,
    pub source_record_id: String,
    pub metadata: TemporalMetadata,
    pub value_scaled: i64,
    pub value_scale: u64,
    pub quality_flags: Vec<QualityFlag>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PitBoundaryKind {
    CorporateAction,
    ContractExpiry,
    ContractRoll,
    FundingSettlement,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitBoundaryEvent {
    pub event_id: String,
    pub kind: PitBoundaryKind,
    pub effective_at_ms: u64,
    pub announced_at_ms: u64,
    pub available_at_ms: u64,
    pub ingested_at_ms: u64,
    pub source: String,
    pub source_revision: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitDatasetBuildRequest {
    pub audit_id: String,
    pub manifest_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub interval: PitInterval,
    pub label_policy: PitLabelPolicy,
    pub prices: Vec<PitPriceObservation>,
    pub feature_observations: Vec<PitFeatureObservation>,
    pub boundary_events: Vec<PitBoundaryEvent>,
    pub split: TimeSplit,
    pub audit: ForecastDatasetAuditInput,
    pub futures_lifecycle_coverage_confirmed: bool,
    pub funding_history_coverage_confirmed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PitStoredDatasetBuildRequest {
    pub audit_id: String,
    pub manifest_id: String,
    pub dataset_id: String,
    pub asset: ForecastAssetContract,
    pub provider: PitOfficialProvider,
    pub symbol: String,
    pub interval: PitProviderInterval,
    pub start_ms: u64,
    pub end_exclusive_ms: u64,
    pub maximum_rows: u16,
    pub label_policy: PitLabelPolicy,
    pub boundary_events: Vec<PitBoundaryEvent>,
    pub split: TimeSplit,
    pub corporate_action_coverage_confirmed: bool,
    pub trading_session_coverage_confirmed: bool,
    pub listing_history_checked: bool,
    pub futures_lifecycle_coverage_confirmed: bool,
    pub funding_history_coverage_confirmed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitStoredDatasetBuildSummary {
    pub provider: PitOfficialProvider,
    pub symbol: String,
    pub loaded_price_count: usize,
    pub warmup_discarded_count: usize,
    pub derived_feature_ids: Vec<&'static str>,
    pub dataset: PitDatasetBuildSummary,
    pub live_order_allowed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitStoredDatasetCommitResult {
    pub provider: PitOfficialProvider,
    pub symbol: String,
    pub loaded_price_count: usize,
    pub warmup_discarded_count: usize,
    pub derived_feature_ids: Vec<&'static str>,
    pub result: PitDatasetCommitResult,
    pub live_order_allowed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitDatasetBuildSummary {
    pub builder_version: &'static str,
    pub dataset_id: String,
    pub asset_contract_id: String,
    pub interval: PitInterval,
    pub label_policy: PitLabelPolicy,
    pub source_price_count: usize,
    pub source_feature_observation_count: usize,
    pub generated_sample_count: usize,
    pub generated_feature_count: usize,
    pub excluded_boundary_crossing_count: usize,
    pub detected_gap_count: usize,
    pub review: ForecastDatasetAuditReview,
    pub live_order_allowed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PitDatasetCommitResult {
    pub summary: PitDatasetBuildSummary,
    pub audit: StoredForecastDatasetAudit,
    pub manifest: StoredMlDatasetManifest,
    pub live_order_allowed: bool,
}

#[derive(Debug)]
struct PreparedPitDataset {
    summary: PitDatasetBuildSummary,
    samples: Vec<ForecastSample>,
    features: Vec<ForecastFeature>,
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn collection_plan(request: PitCollectionPlanRequest) -> Result<PitCollectionPlan, String> {
    crate::forecast::validate_asset_contract(&request.asset)?;
    if request.start_ms == 0
        || request.start_ms >= request.end_exclusive_ms
        || request.maximum_rows_per_window == 0
        || request.maximum_rows_per_window > 100_000
    {
        return Err("수집 시작·종료 시각과 창당 행 수가 올바르지 않습니다.".to_owned());
    }
    let interval_ms = request.interval.duration_ms();
    let window_span = interval_ms
        .checked_mul(u64::from(request.maximum_rows_per_window))
        .ok_or_else(|| "수집 창 범위가 지원 범위를 초과했습니다.".to_owned())?;
    let mut windows = Vec::new();
    let mut cursor = request.start_ms;
    while cursor < request.end_exclusive_ms {
        if windows.len() >= MAX_COLLECTION_WINDOWS {
            return Err(
                "수집 창이 10,000개를 초과했습니다. 기간 또는 창 크기를 조정해 주세요.".to_owned(),
            );
        }
        let end = cursor
            .checked_add(window_span)
            .unwrap_or(request.end_exclusive_ms)
            .min(request.end_exclusive_ms);
        let span = end - cursor;
        let maximum_rows = span
            .saturating_add(interval_ms - 1)
            .checked_div(interval_ms)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "수집 창 행 수를 계산하지 못했습니다.".to_owned())?;
        windows.push(PitCollectionWindow {
            window_index: windows.len(),
            start_ms: cursor,
            end_exclusive_ms: end,
            maximum_rows,
        });
        cursor = end;
    }
    Ok(PitCollectionPlan {
        builder_version: BUILDER_VERSION,
        asset_contract_id: request.asset.contract_id,
        interval: request.interval,
        interval_ms,
        windows,
        provider_credentials_required: false,
        live_order_allowed: false,
    })
}

fn dataset_interval(interval: PitProviderInterval) -> PitInterval {
    match interval {
        PitProviderInterval::Minute1 => PitInterval::Minute1,
        PitProviderInterval::Minute3 => PitInterval::Minute3,
        PitProviderInterval::Minute5 => PitInterval::Minute5,
        PitProviderInterval::Minute15 => PitInterval::Minute15,
        PitProviderInterval::Minute30 => PitInterval::Minute30,
        PitProviderInterval::Hour1 => PitInterval::Hour1,
        PitProviderInterval::Hour4 => PitInterval::Hour4,
        PitProviderInterval::Day1 => PitInterval::Day1,
    }
}

fn validate_stored_provider_asset(
    provider: PitOfficialProvider,
    symbol: &str,
    asset: &ForecastAssetContract,
) -> Result<(), String> {
    let expected_asset_class = match provider {
        PitOfficialProvider::UpbitSpot | PitOfficialProvider::BinanceSpot => {
            ForecastAssetClass::CryptoSpot
        }
        PitOfficialProvider::BinanceUsdm | PitOfficialProvider::BinanceCoinm => {
            ForecastAssetClass::CryptoPerpetual
        }
    };
    let expected_exchange = match provider {
        PitOfficialProvider::UpbitSpot => "UPBIT",
        PitOfficialProvider::BinanceSpot
        | PitOfficialProvider::BinanceUsdm
        | PitOfficialProvider::BinanceCoinm => "BINANCE",
    };
    if asset.asset_class != expected_asset_class
        || !asset.symbol.eq_ignore_ascii_case(symbol)
        || !asset.exchange.eq_ignore_ascii_case(expected_exchange)
    {
        return Err(
            "저장 PIT 공급자·심볼·거래소와 데이터셋 자산 계약이 일치하지 않습니다.".to_owned(),
        );
    }
    Ok(())
}

fn ratio_millionths(numerator: i128, denominator: i128) -> Result<i64, String> {
    if denominator <= 0 {
        return Err("PIT 파생 피처 분모는 0보다 커야 합니다.".to_owned());
    }
    let value = numerator
        .checked_mul(1_000_000)
        .and_then(|scaled| scaled.checked_div(denominator))
        .ok_or_else(|| "PIT 파생 피처 값을 계산하지 못했습니다.".to_owned())?;
    i64::try_from(value).map_err(|_| "PIT 파생 피처 값이 범위를 초과했습니다.".to_owned())
}

fn derived_feature_revision(feature_id: &str, prices: &[PitPriceObservation]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"pit-derived-price-features-v1|");
    hasher.update(feature_id.as_bytes());
    for price in prices {
        hasher.update(b"|");
        hasher.update(price.source_revision.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn derived_source_record_id(feature_id: &str, current: &PitPriceObservation) -> String {
    let canonical = format!("{feature_id}|{}", current.record_id);
    let digest = format!("{:x}", Sha256::digest(canonical.as_bytes()));
    format!("pit-derived:{feature_id}:{}", &digest[..24])
}

fn derive_price_features(
    prices: &[PitPriceObservation],
) -> Result<(Vec<PitPriceObservation>, Vec<PitFeatureObservation>), String> {
    if prices.len() <= DERIVED_PRICE_WARMUP_BARS + 1 {
        return Err("PIT 가격 피처에는 최소 7개의 연속 완료 봉이 필요합니다.".to_owned());
    }
    let mut features = Vec::with_capacity(
        (prices.len() - DERIVED_PRICE_WARMUP_BARS) * DERIVED_PRICE_FEATURE_IDS.len(),
    );
    for index in DERIVED_PRICE_WARMUP_BARS..prices.len() {
        let current = &prices[index];
        let previous = &prices[index - 1];
        let five_back = &prices[index - 5];
        let ma_window = &prices[index - 4..=index];
        let ma_sum = ma_window
            .iter()
            .try_fold(0_i128, |total, price| {
                total.checked_add(i128::from(price.close_scaled)).ok_or(())
            })
            .map_err(|_| "PIT 이동평균 합계가 범위를 초과했습니다.".to_owned())?;
        let values = [
            ratio_millionths(
                i128::from(current.close_scaled) - i128::from(previous.close_scaled),
                i128::from(previous.close_scaled),
            )?,
            ratio_millionths(
                i128::from(current.close_scaled) - i128::from(five_back.close_scaled),
                i128::from(five_back.close_scaled),
            )?,
            ratio_millionths(i128::from(current.close_scaled) * 5 - ma_sum, ma_sum)?,
        ];
        let maximum_ingested_at_ms = prices[index - 5..=index]
            .iter()
            .map(|price| price.ingested_at_ms)
            .max()
            .ok_or_else(|| "PIT 파생 피처 수집 시각을 계산하지 못했습니다.".to_owned())?;
        for (feature_id, value_scaled) in DERIVED_PRICE_FEATURE_IDS.iter().zip(values) {
            let lineage = match *feature_id {
                "pit_return_1" => &prices[index - 1..=index],
                "pit_return_5" => &prices[index - 5..=index],
                "pit_ma_gap_5" => ma_window,
                _ => return Err("지원하지 않는 PIT 파생 피처입니다.".to_owned()),
            };
            features.push(PitFeatureObservation {
                feature_id: (*feature_id).to_owned(),
                source_record_id: derived_source_record_id(feature_id, current),
                metadata: TemporalMetadata {
                    event_time_ms: current.bar_end_ms,
                    available_at_ms: current.available_at_ms,
                    ingested_at_ms: maximum_ingested_at_ms.max(current.ingested_at_ms),
                    source: "INVESTA_PIT_DERIVED_V1".to_owned(),
                    source_revision: derived_feature_revision(feature_id, lineage),
                },
                value_scaled,
                value_scale: 1_000_000,
                quality_flags: Vec::new(),
            });
        }
    }
    Ok((prices[DERIVED_PRICE_WARMUP_BARS..].to_vec(), features))
}

fn stored_dataset_request(
    bridge: &PersistenceBridge,
    request: &PitStoredDatasetBuildRequest,
) -> Result<(PitDatasetBuildRequest, usize), String> {
    crate::forecast::validate_asset_contract(&request.asset)?;
    validate_stored_provider_asset(request.provider, request.symbol.trim(), &request.asset)?;
    let range_request = PitStoredRangeRequest {
        provider: request.provider,
        symbol: request.symbol.clone(),
        interval: request.interval,
        start_ms: request.start_ms,
        end_exclusive_ms: request.end_exclusive_ms,
        maximum_rows: request.maximum_rows,
    };
    if !completed_collection_covers(bridge, &range_request)? {
        return Err(
            "요청 범위를 포함하는 완료된 PIT 수집 작업이 없습니다. 수집을 먼저 완료해 주세요."
                .to_owned(),
        );
    }
    let stored = load_stored_range(bridge, range_request)?;
    if stored.truncated {
        return Err("저장 PIT 범위가 조회 상한에서 잘렸습니다. 범위를 나눠 주세요.".to_owned());
    }
    if stored.internal_gap_count > 0 {
        return Err("저장 PIT 범위에 내부 gap이 있어 데이터셋 조립을 중단했습니다.".to_owned());
    }
    let loaded_price_count = stored.observations.len();
    let (prices, feature_observations) = derive_price_features(&stored.observations)?;
    Ok((
        PitDatasetBuildRequest {
            audit_id: request.audit_id.clone(),
            manifest_id: request.manifest_id.clone(),
            dataset_id: request.dataset_id.clone(),
            asset: request.asset.clone(),
            interval: dataset_interval(request.interval),
            label_policy: request.label_policy.clone(),
            prices,
            feature_observations,
            boundary_events: request.boundary_events.clone(),
            split: request.split.clone(),
            audit: ForecastDatasetAuditInput {
                expected_feature_ids: DERIVED_PRICE_FEATURE_IDS
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
                corporate_action_coverage_confirmed: request.corporate_action_coverage_confirmed,
                trading_session_coverage_confirmed: request.trading_session_coverage_confirmed,
                listing_history_checked: request.listing_history_checked,
            },
            futures_lifecycle_coverage_confirmed: request.futures_lifecycle_coverage_confirmed,
            funding_history_coverage_confirmed: request.funding_history_coverage_confirmed,
        },
        loaded_price_count,
    ))
}

fn validate_label_policy(
    asset: &ForecastAssetContract,
    policy: &PitLabelPolicy,
    futures_coverage: bool,
    funding_coverage: bool,
) -> Result<(), String> {
    if policy.horizon_bars == 0
        || policy.horizon_bars > 1_000
        || policy.up_threshold_bps == 0
        || policy.up_threshold_bps > 5_000
        || policy.down_threshold_bps == 0
        || policy.down_threshold_bps > 5_000
    {
        return Err(
            "라벨 horizon은 1~1,000봉, 상승·하락 임계값은 1~5,000bp여야 합니다.".to_owned(),
        );
    }
    let basis_valid = matches!(
        (asset.asset_class, policy.price_basis),
        (
            ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock,
            PitPriceBasis::AdjustedClose
        ) | (ForecastAssetClass::CryptoSpot, PitPriceBasis::Close)
            | (
                ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture,
                PitPriceBasis::Settlement
            )
            | (ForecastAssetClass::CryptoPerpetual, PitPriceBasis::Mark)
    );
    if !basis_valid {
        return Err("자산군별 가격 기준은 주식 adjusted_close, 현물 close, 증권선물 settlement, 코인 무기한선물 mark로 고정됩니다.".to_owned());
    }
    let contract_basis_valid = match asset.asset_class {
        ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock => asset
            .adjusted_price_policy
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        ForecastAssetClass::CryptoSpot => asset.price_basis.as_deref() == Some("last"),
        ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture => {
            asset.price_basis.as_deref() == Some("settlement")
        }
        ForecastAssetClass::CryptoPerpetual => asset.price_basis.as_deref() == Some("mark"),
    };
    if !contract_basis_valid {
        return Err("자산 계약의 가격 기준과 PIT 라벨 가격 기준이 일치하지 않습니다.".to_owned());
    }
    if policy.corporate_action_handling != PitBoundaryHandling::ExcludeCrossing
        || policy.expiry_handling != PitBoundaryHandling::ExcludeCrossing
        || policy.rollover_handling != PitBoundaryHandling::ExcludeCrossing
    {
        return Err("기업행사·만기·롤오버를 가로지르는 라벨은 반드시 제외해야 합니다.".to_owned());
    }
    if matches!(
        asset.asset_class,
        ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture
    ) && !futures_coverage
    {
        return Err("증권 선물은 만기·롤오버 이력 범위 확인이 필요합니다.".to_owned());
    }
    if asset.asset_class == ForecastAssetClass::CryptoPerpetual && !funding_coverage {
        return Err("코인 무기한선물은 펀딩 이력 범위 확인이 필요합니다.".to_owned());
    }
    Ok(())
}

fn boundary_kind_allowed(asset_class: ForecastAssetClass, kind: PitBoundaryKind) -> bool {
    matches!(
        (asset_class, kind),
        (
            ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock,
            PitBoundaryKind::CorporateAction
        ) | (
            ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture,
            PitBoundaryKind::ContractExpiry | PitBoundaryKind::ContractRoll
        ) | (
            ForecastAssetClass::CryptoPerpetual,
            PitBoundaryKind::FundingSettlement
        )
    )
}

fn validate_boundary_events(
    asset_class: ForecastAssetClass,
    events: &[PitBoundaryEvent],
) -> Result<Vec<PitBoundaryEvent>, String> {
    if events.len() > MAX_BOUNDARY_EVENTS {
        return Err("시장 경계 사건이 20,000개 제한을 초과했습니다.".to_owned());
    }
    let mut seen = BTreeSet::new();
    let mut canonical = events.to_vec();
    for event in &canonical {
        if !valid_identifier(&event.event_id)
            || !boundary_kind_allowed(asset_class, event.kind)
            || event.effective_at_ms == 0
            || event.announced_at_ms > event.available_at_ms
            || event.available_at_ms > event.ingested_at_ms
            || event.source.trim().is_empty()
            || event.source_revision.trim().is_empty()
            || !seen.insert((event.event_id.as_str(), event.source_revision.as_str()))
        {
            return Err(
                "시장 경계 사건의 자산군·시각·출처·리비전 계약이 올바르지 않습니다.".to_owned(),
            );
        }
    }
    canonical.sort_by_key(|event| (event.effective_at_ms, event.event_id.clone()));
    Ok(canonical)
}

fn validate_prices(
    interval: PitInterval,
    asset_class: ForecastAssetClass,
    prices: &[PitPriceObservation],
) -> Result<(Vec<PitPriceObservation>, usize), String> {
    if prices.len() < 2 || prices.len() > MAX_PRICE_OBSERVATIONS {
        return Err("가격 관측값은 2~20,001개 범위여야 합니다.".to_owned());
    }
    let mut canonical = prices.to_vec();
    canonical.sort_by_key(|price| (price.bar_end_ms, price.record_id.clone()));
    let scale = canonical[0].price_scale;
    let mut bar_ends = BTreeSet::new();
    for price in &canonical {
        if !valid_identifier(&price.record_id)
            || price.bar_end_ms == 0
            || price.available_at_ms < price.bar_end_ms
            || price.ingested_at_ms < price.available_at_ms
            || price.source.trim().is_empty()
            || price.source_revision.trim().is_empty()
            || price.close_scaled <= 0
            || price.price_scale == 0
            || price.price_scale != scale
            || !price.final_bar
            || !bar_ends.insert(price.bar_end_ms)
        {
            return Err(
                "완료 가격봉의 ID·시각·가격 단위·출처 또는 중복 경계가 올바르지 않습니다."
                    .to_owned(),
            );
        }
    }
    if canonical
        .windows(2)
        .any(|window| window[0].available_at_ms >= window[1].available_at_ms)
    {
        return Err("완료 가격봉의 이용 가능 시각은 중복 없이 증가해야 합니다.".to_owned());
    }
    let interval_ms = interval.duration_ms();
    let detected_gap_count = canonical
        .windows(2)
        .filter(|window| window[1].bar_end_ms.saturating_sub(window[0].bar_end_ms) > interval_ms)
        .count();
    if matches!(
        asset_class,
        ForecastAssetClass::CryptoSpot | ForecastAssetClass::CryptoPerpetual
    ) && detected_gap_count > 0
    {
        return Err(
            "24시간 자산의 가격봉에 누락 구간이 있습니다. gap을 백필한 뒤 다시 시도해 주세요."
                .to_owned(),
        );
    }
    Ok((canonical, detected_gap_count))
}

fn validate_feature_observations(
    observations: &[PitFeatureObservation],
) -> Result<Vec<PitFeatureObservation>, String> {
    if observations.is_empty() || observations.len() > MAX_FEATURE_OBSERVATIONS {
        return Err("피처 관측값은 1~200,000개 범위여야 합니다.".to_owned());
    }
    let mut seen = BTreeSet::new();
    let mut canonical = observations.to_vec();
    for feature in &canonical {
        if !valid_identifier(&feature.feature_id)
            || !valid_identifier(&feature.source_record_id)
            || feature.metadata.source.trim().is_empty()
            || feature.metadata.source_revision.trim().is_empty()
            || feature.metadata.event_time_ms > feature.metadata.available_at_ms
            || feature.metadata.available_at_ms > feature.metadata.ingested_at_ms
            || feature.value_scale == 0
            || !seen.insert((
                feature.feature_id.as_str(),
                feature.source_record_id.as_str(),
                feature.metadata.source_revision.as_str(),
            ))
        {
            return Err(
                "피처 관측값의 ID·시각·단위·출처·리비전 계약이 올바르지 않습니다.".to_owned(),
            );
        }
    }
    canonical.sort_by(|left, right| {
        (
            &left.feature_id,
            left.metadata.available_at_ms,
            left.metadata.ingested_at_ms,
            &left.metadata.source_revision,
            &left.source_record_id,
        )
            .cmp(&(
                &right.feature_id,
                right.metadata.available_at_ms,
                right.metadata.ingested_at_ms,
                &right.metadata.source_revision,
                &right.source_record_id,
            ))
    });
    Ok(canonical)
}

fn boundary_excludes(
    policy: &PitLabelPolicy,
    event: &PitBoundaryEvent,
    current_bar_end_ms: u64,
    target_bar_end_ms: u64,
) -> bool {
    if event.effective_at_ms <= current_bar_end_ms || event.effective_at_ms > target_bar_end_ms {
        return false;
    }
    match event.kind {
        PitBoundaryKind::CorporateAction => {
            policy.corporate_action_handling == PitBoundaryHandling::ExcludeCrossing
        }
        PitBoundaryKind::ContractExpiry => {
            policy.expiry_handling == PitBoundaryHandling::ExcludeCrossing
        }
        PitBoundaryKind::ContractRoll => {
            policy.rollover_handling == PitBoundaryHandling::ExcludeCrossing
        }
        PitBoundaryKind::FundingSettlement => {
            policy.funding_handling == PitBoundaryHandling::ExcludeCrossing
        }
    }
}

fn target_class(current: i64, future: i64, policy: &PitLabelPolicy) -> Result<u8, String> {
    let current = i128::from(current);
    let change_bps = (i128::from(future) - current)
        .checked_mul(10_000)
        .and_then(|value| value.checked_div(current))
        .ok_or_else(|| "라벨 수익률을 계산하지 못했습니다.".to_owned())?;
    Ok(if change_bps >= i128::from(policy.up_threshold_bps) {
        2
    } else if change_bps <= -i128::from(policy.down_threshold_bps) {
        0
    } else {
        1
    })
}

fn prepare_dataset(request: &PitDatasetBuildRequest) -> Result<PreparedPitDataset, String> {
    for (value, label) in [
        (&request.audit_id, "감사"),
        (&request.manifest_id, "매니페스트"),
        (&request.dataset_id, "데이터셋"),
    ] {
        if !valid_identifier(value) {
            return Err(format!("{label} 식별자가 올바르지 않습니다."));
        }
    }
    crate::forecast::validate_asset_contract(&request.asset)?;
    validate_label_policy(
        &request.asset,
        &request.label_policy,
        request.futures_lifecycle_coverage_confirmed,
        request.funding_history_coverage_confirmed,
    )?;
    let events = validate_boundary_events(request.asset.asset_class, &request.boundary_events)?;
    let (prices, detected_gap_count) =
        validate_prices(request.interval, request.asset.asset_class, &request.prices)?;
    let observations = validate_feature_observations(&request.feature_observations)?;
    let horizon = usize::from(request.label_policy.horizon_bars);
    if prices.len() <= horizon {
        return Err("라벨 horizon보다 많은 완료 가격봉이 필요합니다.".to_owned());
    }

    let mut samples = Vec::new();
    let mut features = Vec::new();
    let mut excluded_boundary_crossing_count = 0;
    let candidate_count = prices.len() - horizon;
    let candidate_prices = &prices[..candidate_count];
    let mut eligible_at = vec![Vec::<&PitFeatureObservation>::new(); candidate_count];
    for observation in &observations {
        let event_index = candidate_prices
            .partition_point(|price| price.bar_end_ms < observation.metadata.event_time_ms);
        let availability_index = candidate_prices
            .partition_point(|price| price.available_at_ms < observation.metadata.available_at_ms);
        let first_eligible_index = event_index.max(availability_index);
        if first_eligible_index < candidate_count {
            eligible_at[first_eligible_index].push(observation);
        }
    }
    let mut latest_by_feature: BTreeMap<&str, &PitFeatureObservation> = BTreeMap::new();
    for index in 0..prices.len() - horizon {
        let current = &prices[index];
        let target = &prices[index + horizon];
        for observation in &eligible_at[index] {
            let replace = latest_by_feature
                .get(observation.feature_id.as_str())
                .is_none_or(|previous| {
                    (
                        observation.metadata.available_at_ms,
                        observation.metadata.ingested_at_ms,
                        observation.metadata.source_revision.as_str(),
                        observation.source_record_id.as_str(),
                    ) > (
                        previous.metadata.available_at_ms,
                        previous.metadata.ingested_at_ms,
                        previous.metadata.source_revision.as_str(),
                        previous.source_record_id.as_str(),
                    )
                });
            if replace {
                latest_by_feature.insert(observation.feature_id.as_str(), observation);
            }
        }
        if events.iter().any(|event| {
            boundary_excludes(
                &request.label_policy,
                event,
                current.bar_end_ms,
                target.bar_end_ms,
            )
        }) {
            excluded_boundary_crossing_count += 1;
            continue;
        }
        let sample_id = format!("{}:{}", request.dataset_id, current.bar_end_ms);
        let sample = ForecastSample {
            sample_id: sample_id.clone(),
            decision_time_ms: current.available_at_ms,
            target_observed_at_ms: target.available_at_ms,
            target_class: target_class(
                current.close_scaled,
                target.close_scaled,
                &request.label_policy,
            )?,
        };
        for observation in latest_by_feature.values() {
            if features.len() >= MAX_FEATURE_OBSERVATIONS {
                return Err("as-of 조인 결과가 피처 200,000개 제한을 초과했습니다.".to_owned());
            }
            features.push(ForecastFeature {
                feature_id: observation.feature_id.clone(),
                sample_id: sample_id.clone(),
                source_record_id: observation.source_record_id.clone(),
                dataset_version: request.dataset_id.clone(),
                metadata: observation.metadata.clone(),
                value_scaled: observation.value_scaled,
                value_scale: observation.value_scale,
                quality_flags: observation.quality_flags.clone(),
            });
        }
        samples.push(sample);
    }
    if samples.is_empty() {
        return Err("시장 경계와 horizon을 적용한 뒤 남은 표본이 없습니다.".to_owned());
    }
    let review = audit_forecast_dataset(
        &request.asset,
        &request.dataset_id,
        &samples,
        &features,
        &request.split,
        &request.audit,
    );
    Ok(PreparedPitDataset {
        summary: PitDatasetBuildSummary {
            builder_version: BUILDER_VERSION,
            dataset_id: request.dataset_id.clone(),
            asset_contract_id: request.asset.contract_id.clone(),
            interval: request.interval,
            label_policy: request.label_policy.clone(),
            source_price_count: prices.len(),
            source_feature_observation_count: observations.len(),
            generated_sample_count: samples.len(),
            generated_feature_count: features.len(),
            excluded_boundary_crossing_count,
            detected_gap_count,
            review,
            live_order_allowed: false,
        },
        samples,
        features,
    })
}

fn commit_dataset(
    bridge: &PersistenceBridge,
    request: PitDatasetBuildRequest,
) -> Result<PitDatasetCommitResult, String> {
    let prepared = prepare_dataset(&request)?;
    if !prepared.summary.review.valid {
        return Err(format!(
            "PIT 데이터 품질 감사를 통과하지 못했습니다: {}",
            prepared.summary.review.issues.join(" ")
        ));
    }
    let audit = save_dataset_audit(
        bridge,
        ForecastDatasetAuditRequest {
            audit_id: request.audit_id.clone(),
            dataset_id: request.dataset_id.clone(),
            asset: request.asset.clone(),
            samples: prepared.samples.clone(),
            features: prepared.features.clone(),
            split: request.split.clone(),
            audit: request.audit.clone(),
        },
    )?;
    let manifest = create_dataset_manifest(
        bridge,
        MlDatasetManifestCreateRequest {
            manifest_id: request.manifest_id,
            audit_id: request.audit_id,
            dataset_id: request.dataset_id,
            asset: request.asset,
            samples: prepared.samples,
            features: prepared.features,
            split: request.split,
            audit: request.audit,
        },
    )?;
    Ok(PitDatasetCommitResult {
        summary: prepared.summary,
        audit,
        manifest,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn pit_collection_plan_create(
    request: PitCollectionPlanRequest,
) -> Result<PitCollectionPlan, String> {
    collection_plan(request)
}

#[tauri::command]
pub fn pit_dataset_build_preview(
    request: PitDatasetBuildRequest,
) -> Result<PitDatasetBuildSummary, String> {
    Ok(prepare_dataset(&request)?.summary)
}

#[tauri::command]
pub fn pit_dataset_build_commit(
    state: State<'_, PersistenceBridge>,
    request: PitDatasetBuildRequest,
) -> Result<PitDatasetCommitResult, String> {
    commit_dataset(&state, request)
}

#[tauri::command]
pub fn pit_stored_dataset_build_preview(
    state: State<'_, PersistenceBridge>,
    request: PitStoredDatasetBuildRequest,
) -> Result<PitStoredDatasetBuildSummary, String> {
    let provider = request.provider;
    let symbol = request.symbol.trim().to_ascii_uppercase();
    let (build_request, loaded_price_count) = stored_dataset_request(&state, &request)?;
    let dataset = prepare_dataset(&build_request)?.summary;
    Ok(PitStoredDatasetBuildSummary {
        provider,
        symbol,
        loaded_price_count,
        warmup_discarded_count: DERIVED_PRICE_WARMUP_BARS,
        derived_feature_ids: DERIVED_PRICE_FEATURE_IDS.to_vec(),
        dataset,
        live_order_allowed: false,
    })
}

#[tauri::command]
pub fn pit_stored_dataset_build_commit(
    state: State<'_, PersistenceBridge>,
    request: PitStoredDatasetBuildRequest,
) -> Result<PitStoredDatasetCommitResult, String> {
    let provider = request.provider;
    let symbol = request.symbol.trim().to_ascii_uppercase();
    let (build_request, loaded_price_count) = stored_dataset_request(&state, &request)?;
    let result = commit_dataset(&state, build_request)?;
    Ok(PitStoredDatasetCommitResult {
        provider,
        symbol,
        loaded_price_count,
        warmup_discarded_count: DERIVED_PRICE_WARMUP_BARS,
        derived_feature_ids: DERIVED_PRICE_FEATURE_IDS.to_vec(),
        result,
        live_order_allowed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn asset(asset_class: ForecastAssetClass) -> ForecastAssetContract {
        ForecastAssetContract {
            contract_id: "asset-v1".to_owned(),
            asset_class,
            exchange: "TEST".to_owned(),
            symbol: "TEST".to_owned(),
            currency: "USD".to_owned(),
            timezone: "UTC".to_owned(),
            adjusted_price_policy: matches!(
                asset_class,
                ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock
            )
            .then(|| "pit_adjusted".to_owned()),
            corporate_action_policy: matches!(
                asset_class,
                ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock
            )
            .then(|| "exclude_crossing".to_owned()),
            contract_multiplier: matches!(
                asset_class,
                ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture
            )
            .then_some(1),
            expiry_policy: matches!(
                asset_class,
                ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture
            )
            .then(|| "explicit".to_owned()),
            rollover_policy: matches!(
                asset_class,
                ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture
            )
            .then(|| "no_crossing".to_owned()),
            price_basis: Some(
                match asset_class {
                    ForecastAssetClass::CryptoSpot => "last",
                    ForecastAssetClass::CryptoPerpetual => "mark",
                    ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture => {
                        "settlement"
                    }
                    _ => "adjusted_close",
                }
                .to_owned(),
            ),
            funding_policy: (asset_class == ForecastAssetClass::CryptoPerpetual)
                .then(|| "price_return_only".to_owned()),
            leverage_policy: (asset_class == ForecastAssetClass::CryptoPerpetual)
                .then(|| "none_for_research".to_owned()),
        }
    }

    fn policy(asset_class: ForecastAssetClass) -> PitLabelPolicy {
        PitLabelPolicy {
            horizon_bars: 1,
            up_threshold_bps: 50,
            down_threshold_bps: 50,
            price_basis: match asset_class {
                ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock => {
                    PitPriceBasis::AdjustedClose
                }
                ForecastAssetClass::CryptoSpot => PitPriceBasis::Close,
                ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture => {
                    PitPriceBasis::Settlement
                }
                ForecastAssetClass::CryptoPerpetual => PitPriceBasis::Mark,
            },
            corporate_action_handling: PitBoundaryHandling::ExcludeCrossing,
            expiry_handling: PitBoundaryHandling::ExcludeCrossing,
            rollover_handling: PitBoundaryHandling::ExcludeCrossing,
            funding_handling: PitBoundaryHandling::PriceReturnOnly,
        }
    }

    fn prices(asset_class: ForecastAssetClass) -> Vec<PitPriceObservation> {
        let interval = if matches!(
            asset_class,
            ForecastAssetClass::CryptoSpot | ForecastAssetClass::CryptoPerpetual
        ) {
            60_000
        } else {
            86_400_000
        };
        (1..=7)
            .map(|index| PitPriceObservation {
                record_id: format!("price-{index}"),
                bar_end_ms: index * interval,
                available_at_ms: index * interval + 1,
                ingested_at_ms: index * interval + 2,
                source: "official-test".to_owned(),
                source_revision: "v1".to_owned(),
                close_scaled: 10_000 + i64::try_from(index).unwrap_or_default() * 100,
                price_scale: 100,
                final_bar: true,
            })
            .collect()
    }

    fn features(asset_class: ForecastAssetClass) -> Vec<PitFeatureObservation> {
        let interval = if matches!(
            asset_class,
            ForecastAssetClass::CryptoSpot | ForecastAssetClass::CryptoPerpetual
        ) {
            60_000
        } else {
            86_400_000
        };
        (1..=6)
            .map(|index| PitFeatureObservation {
                feature_id: "return_1".to_owned(),
                source_record_id: format!("feature-{index}"),
                metadata: TemporalMetadata {
                    event_time_ms: index * interval,
                    available_at_ms: index * interval + 1,
                    ingested_at_ms: index * interval + 2,
                    source: "official-test".to_owned(),
                    source_revision: "v1".to_owned(),
                },
                value_scaled: i64::try_from(index).unwrap_or_default(),
                value_scale: 100,
                quality_flags: Vec::new(),
            })
            .collect()
    }

    fn request(asset_class: ForecastAssetClass) -> PitDatasetBuildRequest {
        let interval = if matches!(
            asset_class,
            ForecastAssetClass::CryptoSpot | ForecastAssetClass::CryptoPerpetual
        ) {
            PitInterval::Minute1
        } else {
            PitInterval::Day1
        };
        let unit = interval.duration_ms();
        PitDatasetBuildRequest {
            audit_id: "audit-pit-v1".to_owned(),
            manifest_id: "manifest-pit-v1".to_owned(),
            dataset_id: "dataset-pit-v1".to_owned(),
            asset: asset(asset_class),
            interval,
            label_policy: policy(asset_class),
            prices: prices(asset_class),
            feature_observations: features(asset_class),
            boundary_events: Vec::new(),
            split: TimeSplit {
                train_end_ms: unit + 1,
                validation_start_ms: 3 * unit + 1,
                validation_end_ms: 4 * unit + 1,
                test_start_ms: 6 * unit + 1,
            },
            audit: ForecastDatasetAuditInput {
                expected_feature_ids: vec!["return_1".to_owned()],
                corporate_action_coverage_confirmed: matches!(
                    asset_class,
                    ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock
                ),
                trading_session_coverage_confirmed: true,
                listing_history_checked: true,
            },
            futures_lifecycle_coverage_confirmed: matches!(
                asset_class,
                ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture
            ),
            funding_history_coverage_confirmed: asset_class == ForecastAssetClass::CryptoPerpetual,
        }
    }

    fn seed_completed_binance_range(
        bridge: &PersistenceBridge,
        job_id: &str,
        missing_index: Option<u64>,
    ) {
        let start_ms = 1_700_000_000_000_u64;
        let end_exclusive_ms = start_ms + 13 * 60_000;
        let connection = bridge.connection.lock().expect("connection");
        connection
            .execute(
                "INSERT INTO pit_collection_jobs
                 (job_id, idempotency_key, request_hash, provider, symbol, interval,
                  requested_start_ms, requested_end_exclusive_ms, page_size, status,
                  cursor_start_ms, cursor_end_exclusive_ms, page_count, observation_count,
                  failure_count, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 'binance_spot', 'BTCUSDT', 'minute1', ?4, ?5,
                         1000, 'completed', ?5, ?5, 1, 12, 0, ?6, ?6)",
                rusqlite::params![
                    job_id,
                    format!("idem-{job_id}"),
                    "a".repeat(64),
                    start_ms,
                    end_exclusive_ms,
                    start_ms + 20 * 60_000,
                ],
            )
            .expect("job");
        for index in 1..=12_u64 {
            if missing_index == Some(index) {
                continue;
            }
            let bar_end_ms = start_ms + index * 60_000;
            let record_id = format!("BINANCE_SPOT_PUBLIC_KLINES:BTCUSDT:{bar_end_ms}");
            let source_revision = format!(
                "sha256:{:x}",
                Sha256::digest(format!("revision-{index}").as_bytes())
            );
            connection
                .execute(
                    "INSERT INTO pit_price_observations
                     (record_id, provider, symbol, interval, bar_end_ms, available_at_ms,
                      ingested_at_ms, source, source_revision, close_scaled, price_scale, final_bar)
                     VALUES (?1, 'binance_spot', 'BTCUSDT', 'minute1', ?2, ?2, ?3,
                             'BINANCE_SPOT_PUBLIC_KLINES', ?4, ?5, 100000000, 1)",
                    rusqlite::params![
                        record_id,
                        bar_end_ms,
                        start_ms + 20 * 60_000 + index,
                        source_revision,
                        10_000_000_000_i64 + i64::try_from(index).expect("index") * 20_000_000,
                    ],
                )
                .expect("observation");
        }
    }

    fn stored_request(job_suffix: &str) -> PitStoredDatasetBuildRequest {
        let start_ms = 1_700_000_000_000_u64;
        PitStoredDatasetBuildRequest {
            audit_id: format!("audit-stored-{job_suffix}"),
            manifest_id: format!("manifest-stored-{job_suffix}"),
            dataset_id: format!("dataset-stored-{job_suffix}"),
            asset: ForecastAssetContract {
                contract_id: "binance-btcusdt-spot".to_owned(),
                asset_class: ForecastAssetClass::CryptoSpot,
                exchange: "BINANCE".to_owned(),
                symbol: "BTCUSDT".to_owned(),
                currency: "USDT".to_owned(),
                timezone: "UTC".to_owned(),
                adjusted_price_policy: None,
                corporate_action_policy: None,
                contract_multiplier: None,
                expiry_policy: None,
                rollover_policy: None,
                price_basis: Some("last".to_owned()),
                funding_policy: None,
                leverage_policy: None,
            },
            provider: PitOfficialProvider::BinanceSpot,
            symbol: "BTCUSDT".to_owned(),
            interval: PitProviderInterval::Minute1,
            start_ms,
            end_exclusive_ms: start_ms + 13 * 60_000,
            maximum_rows: 20_001,
            label_policy: policy(ForecastAssetClass::CryptoSpot),
            boundary_events: Vec::new(),
            split: TimeSplit {
                train_end_ms: start_ms + 6 * 60_000,
                validation_start_ms: start_ms + 8 * 60_000,
                validation_end_ms: start_ms + 8 * 60_000,
                test_start_ms: start_ms + 10 * 60_000,
            },
            corporate_action_coverage_confirmed: false,
            trading_session_coverage_confirmed: true,
            listing_history_checked: true,
            futures_lifecycle_coverage_confirmed: false,
            funding_history_coverage_confirmed: false,
        }
    }

    #[test]
    fn collection_windows_are_contiguous_and_bounded() {
        let plan = collection_plan(PitCollectionPlanRequest {
            asset: asset(ForecastAssetClass::CryptoSpot),
            interval: PitInterval::Minute1,
            start_ms: 60_000,
            end_exclusive_ms: 660_000,
            maximum_rows_per_window: 4,
        })
        .expect("plan");
        assert_eq!(plan.windows.len(), 3);
        assert_eq!(plan.windows[0].end_exclusive_ms, plan.windows[1].start_ms);
        assert_eq!(plan.windows[2].maximum_rows, 2);
        assert!(!plan.live_order_allowed);
    }

    #[test]
    fn all_asset_families_build_with_explicit_policy() {
        for asset_class in [
            ForecastAssetClass::KoreaStock,
            ForecastAssetClass::UnitedStatesStock,
            ForecastAssetClass::EquityFuture,
            ForecastAssetClass::IndexFuture,
            ForecastAssetClass::CryptoSpot,
            ForecastAssetClass::CryptoPerpetual,
        ] {
            let prepared = prepare_dataset(&request(asset_class)).expect("prepared");
            assert!(
                prepared.summary.review.valid,
                "{:?}",
                prepared.summary.review.issues
            );
            assert_eq!(prepared.summary.generated_sample_count, 6);
        }
    }

    #[test]
    fn feature_revision_available_after_decision_is_not_joined() {
        let mut request = request(ForecastAssetClass::CryptoSpot);
        request.feature_observations[0].metadata.available_at_ms = 10 * 60_000;
        request.feature_observations[0].metadata.ingested_at_ms = 10 * 60_000 + 1;
        let prepared = prepare_dataset(&request).expect("preview");
        assert!(!prepared.summary.review.valid);
        assert!(prepared.summary.review.missing_expected_feature_count > 0);
    }

    #[test]
    fn stock_corporate_action_crossing_is_excluded() {
        let mut request = request(ForecastAssetClass::KoreaStock);
        request.boundary_events.push(PitBoundaryEvent {
            event_id: "split-v1".to_owned(),
            kind: PitBoundaryKind::CorporateAction,
            effective_at_ms: 3 * 86_400_000,
            announced_at_ms: 86_400_000,
            available_at_ms: 86_400_001,
            ingested_at_ms: 86_400_002,
            source: "official-test".to_owned(),
            source_revision: "v1".to_owned(),
        });
        let prepared = prepare_dataset(&request).expect("prepared");
        assert_eq!(prepared.summary.excluded_boundary_crossing_count, 1);
        assert_eq!(prepared.summary.generated_sample_count, 5);
    }

    #[test]
    fn futures_without_lifecycle_coverage_fail_closed() {
        let mut request = request(ForecastAssetClass::IndexFuture);
        request.futures_lifecycle_coverage_confirmed = false;
        assert!(prepare_dataset(&request)
            .expect_err("coverage")
            .contains("만기·롤오버"));
    }

    #[test]
    fn futures_expiry_crossing_is_excluded() {
        let mut request = request(ForecastAssetClass::IndexFuture);
        request.boundary_events.push(PitBoundaryEvent {
            event_id: "expiry-v1".to_owned(),
            kind: PitBoundaryKind::ContractExpiry,
            effective_at_ms: 4 * 86_400_000,
            announced_at_ms: 86_400_000,
            available_at_ms: 86_400_001,
            ingested_at_ms: 86_400_002,
            source: "official-test".to_owned(),
            source_revision: "v1".to_owned(),
        });
        let prepared = prepare_dataset(&request).expect("prepared");
        assert_eq!(prepared.summary.excluded_boundary_crossing_count, 1);
    }

    #[test]
    fn perpetual_funding_can_be_price_only_or_excluded() {
        let mut request = request(ForecastAssetClass::CryptoPerpetual);
        request.boundary_events.push(PitBoundaryEvent {
            event_id: "funding-v1".to_owned(),
            kind: PitBoundaryKind::FundingSettlement,
            effective_at_ms: 4 * 60_000,
            announced_at_ms: 3 * 60_000,
            available_at_ms: 3 * 60_000 + 1,
            ingested_at_ms: 3 * 60_000 + 2,
            source: "official-test".to_owned(),
            source_revision: "v1".to_owned(),
        });
        let price_only = prepare_dataset(&request).expect("price-only");
        assert_eq!(price_only.summary.excluded_boundary_crossing_count, 0);
        request.label_policy.funding_handling = PitBoundaryHandling::ExcludeCrossing;
        let excluded = prepare_dataset(&request).expect("excluded");
        assert_eq!(excluded.summary.excluded_boundary_crossing_count, 1);
    }

    #[test]
    fn crypto_gap_is_rejected() {
        let mut request = request(ForecastAssetClass::CryptoPerpetual);
        request.prices.remove(3);
        assert!(prepare_dataset(&request).expect_err("gap").contains("gap"));
    }

    #[test]
    fn stored_range_derives_causal_features_and_commits_idempotently() {
        let path = PathBuf::from(format!(
            "pit-stored-dataset-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let bridge = PersistenceBridge::open(&path).expect("bridge");
        seed_completed_binance_range(&bridge, "job-stored-valid", None);
        let (build_request, loaded_observation_count) =
            stored_dataset_request(&bridge, &stored_request("valid")).expect("stored");
        assert_eq!(loaded_observation_count, 12);
        assert_eq!(build_request.prices.len(), 7);
        assert_eq!(build_request.feature_observations.len(), 21);
        assert!(build_request
            .feature_observations
            .iter()
            .all(|feature| feature.metadata.available_at_ms <= feature.metadata.event_time_ms));
        assert!(build_request
            .feature_observations
            .iter()
            .all(|feature| feature.metadata.source_revision.starts_with("sha256:")));

        let first = commit_dataset(&bridge, build_request.clone()).expect("first commit");
        let second = commit_dataset(&bridge, build_request).expect("second commit");
        assert!(first.audit.review.valid);
        assert_eq!(
            first.manifest.content_sha256,
            second.manifest.content_sha256
        );
        assert!(!first.live_order_allowed);
        drop(bridge);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn stored_dataset_requires_completed_coverage_and_matching_asset() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        let request = stored_request("coverage");
        assert!(stored_dataset_request(&bridge, &request)
            .expect_err("missing collection")
            .contains("완료된"));

        seed_completed_binance_range(&bridge, "job-stored-coverage", None);
        let mut mismatched = request;
        mismatched.asset.exchange = "UPBIT".to_owned();
        assert!(stored_dataset_request(&bridge, &mismatched)
            .expect_err("asset mismatch")
            .contains("거래소"));
    }

    #[test]
    fn stored_dataset_rejects_truncated_or_gapped_ranges() {
        let bridge = PersistenceBridge::in_memory().expect("database");
        seed_completed_binance_range(&bridge, "job-stored-truncated", None);
        let mut truncated = stored_request("truncated");
        truncated.maximum_rows = 8;
        assert!(stored_dataset_request(&bridge, &truncated)
            .expect_err("truncated")
            .contains("잘렸"));

        let bridge = PersistenceBridge::in_memory().expect("gap database");
        seed_completed_binance_range(&bridge, "job-stored-gap", Some(6));
        assert!(stored_dataset_request(&bridge, &stored_request("gap"))
            .expect_err("gap")
            .contains("gap"));
    }

    #[test]
    fn valid_build_commits_existing_audit_and_manifest_contracts() {
        let path = PathBuf::from(format!("pit-dataset-{}.sqlite3", uuid::Uuid::new_v4()));
        let bridge = PersistenceBridge::open(&path).expect("bridge");
        let result =
            commit_dataset(&bridge, request(ForecastAssetClass::CryptoSpot)).expect("commit");
        assert!(result.audit.review.valid);
        assert_eq!(result.manifest.sample_count, 6);
        assert!(!result.live_order_allowed);
        drop(bridge);
        let _ = fs::remove_file(path);
    }
}
