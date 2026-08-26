use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::data_quality::{QualityFlag, TemporalMetadata};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastAssetClass {
    KoreaStock,
    UnitedStatesStock,
    EquityFuture,
    IndexFuture,
    CryptoSpot,
    CryptoPerpetual,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastAssetContract {
    pub contract_id: String,
    pub asset_class: ForecastAssetClass,
    pub exchange: String,
    pub symbol: String,
    pub currency: String,
    pub timezone: String,
    pub adjusted_price_policy: Option<String>,
    pub corporate_action_policy: Option<String>,
    pub contract_multiplier: Option<u64>,
    pub expiry_policy: Option<String>,
    pub rollover_policy: Option<String>,
    pub price_basis: Option<String>,
    pub funding_policy: Option<String>,
    pub leverage_policy: Option<String>,
}

pub fn validate_asset_contract(contract: &ForecastAssetContract) -> Result<(), String> {
    if contract.contract_id.trim().is_empty()
        || contract.exchange.trim().is_empty()
        || contract.symbol.trim().is_empty()
        || contract.currency.trim().is_empty()
        || contract.timezone.trim().is_empty()
    {
        return Err("자산 계약의 ID·거래소·종목·통화·시간대가 필요합니다.".to_owned());
    }
    match contract.asset_class {
        ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock => {
            if contract.adjusted_price_policy.is_none()
                || contract.corporate_action_policy.is_none()
            {
                return Err("주식은 수정주가와 기업행사 정책이 필요합니다.".to_owned());
            }
        }
        ForecastAssetClass::EquityFuture | ForecastAssetClass::IndexFuture => {
            if contract.contract_multiplier.is_none_or(|value| value == 0)
                || contract.expiry_policy.is_none()
                || contract.rollover_policy.is_none()
                || contract.price_basis.is_none()
            {
                return Err("증권 선물은 승수·만기·롤오버·베이시스 계약이 필요합니다.".to_owned());
            }
        }
        ForecastAssetClass::CryptoSpot => {
            if contract.price_basis.as_deref() != Some("last") {
                return Err("코인 현물 가격 기준은 last로 명시해야 합니다.".to_owned());
            }
        }
        ForecastAssetClass::CryptoPerpetual => {
            if contract.price_basis.is_none()
                || contract.funding_policy.is_none()
                || contract.leverage_policy.is_none()
            {
                return Err("코인 무기한 선물은 가격·펀딩·레버리지 계약이 필요합니다.".to_owned());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastFeature {
    pub feature_id: String,
    pub sample_id: String,
    pub source_record_id: String,
    pub dataset_version: String,
    pub metadata: TemporalMetadata,
    pub value_scaled: i64,
    pub value_scale: u64,
    pub quality_flags: Vec<QualityFlag>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastSample {
    pub sample_id: String,
    pub decision_time_ms: u64,
    pub target_observed_at_ms: u64,
    pub target_class: u8,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSplit {
    pub train_end_ms: u64,
    pub validation_start_ms: u64,
    pub validation_end_ms: u64,
    pub test_start_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastDatasetReview {
    pub valid: bool,
    pub sample_count: usize,
    pub feature_count: usize,
    pub missing_feature_count: usize,
    pub duplicate_feature_count: usize,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastDatasetAuditInput {
    pub expected_feature_ids: Vec<String>,
    pub corporate_action_coverage_confirmed: bool,
    pub trading_session_coverage_confirmed: bool,
    pub listing_history_checked: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastDatasetAuditReview {
    pub dataset: ForecastDatasetReview,
    pub missing_expected_feature_ids: Vec<String>,
    pub missing_expected_feature_count: usize,
    pub corporate_action_coverage_confirmed: bool,
    pub trading_session_coverage_confirmed: bool,
    pub listing_history_checked: bool,
    pub valid: bool,
    pub issues: Vec<String>,
}

pub fn review_forecast_dataset(
    asset: &ForecastAssetContract,
    dataset_id: &str,
    samples: &[ForecastSample],
    features: &[ForecastFeature],
    split: &TimeSplit,
) -> ForecastDatasetReview {
    let mut issues = Vec::new();
    if validate_asset_contract(asset).is_err() || dataset_id.trim().is_empty() {
        issues.push("자산 또는 데이터셋 계약이 올바르지 않습니다.".to_owned());
    }
    if !(split.train_end_ms < split.validation_start_ms
        && split.validation_start_ms <= split.validation_end_ms
        && split.validation_end_ms < split.test_start_ms)
    {
        issues.push("train·validation·test는 겹치지 않는 시간순 구간이어야 합니다.".to_owned());
    }
    let sample_ids = samples
        .iter()
        .map(|sample| sample.sample_id.as_str())
        .collect::<BTreeSet<_>>();
    if sample_ids.len() != samples.len()
        || samples.iter().any(|sample| {
            sample.sample_id.trim().is_empty()
                || sample.decision_time_ms >= sample.target_observed_at_ms
                || sample.target_class > 2
        })
    {
        issues.push("표본 ID·결정 시각·타깃 관측 시각 또는 클래스가 올바르지 않습니다.".to_owned());
    }
    let mut keys = BTreeSet::new();
    let mut duplicates = 0;
    for feature in features {
        if !sample_ids.contains(feature.sample_id.as_str())
            || feature.feature_id.trim().is_empty()
            || feature.source_record_id.trim().is_empty()
            || feature.dataset_version != dataset_id
            || feature.value_scale == 0
        {
            issues.push("피처의 표본·원천·데이터셋 버전·단위가 올바르지 않습니다.".to_owned());
            continue;
        }
        if !keys.insert((feature.sample_id.as_str(), feature.feature_id.as_str())) {
            duplicates += 1;
        }
        if let Some(sample) = samples
            .iter()
            .find(|sample| sample.sample_id == feature.sample_id)
        {
            if feature.metadata.available_at_ms > sample.decision_time_ms
                || feature.metadata.ingested_at_ms < feature.metadata.available_at_ms
                || feature.metadata.event_time_ms > feature.metadata.available_at_ms
            {
                issues.push(format!(
                    "{} 피처에 결정 시각 이후 정보가 포함되었습니다.",
                    feature.feature_id
                ));
            }
        }
    }
    if duplicates > 0 {
        issues.push("sample×feature N:N 중복을 발견했습니다.".to_owned());
    }
    let missing = samples
        .iter()
        .filter(|sample| {
            !features
                .iter()
                .any(|feature| feature.sample_id == sample.sample_id)
        })
        .count();
    if missing > 0 {
        issues.push("피처가 하나도 없는 표본이 있습니다.".to_owned());
    }
    ForecastDatasetReview {
        valid: issues.is_empty(),
        sample_count: samples.len(),
        feature_count: features.len(),
        missing_feature_count: missing,
        duplicate_feature_count: duplicates,
        issues,
    }
}

pub fn audit_forecast_dataset(
    asset: &ForecastAssetContract,
    dataset_id: &str,
    samples: &[ForecastSample],
    features: &[ForecastFeature],
    split: &TimeSplit,
    input: &ForecastDatasetAuditInput,
) -> ForecastDatasetAuditReview {
    let dataset = review_forecast_dataset(asset, dataset_id, samples, features, split);
    let available_features = features
        .iter()
        .map(|feature| feature.feature_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut missing_expected_feature_ids = Vec::new();
    let mut missing_expected_feature_count = 0;
    for sample in samples {
        for feature_id in &input.expected_feature_ids {
            if !features.iter().any(|feature| {
                feature.sample_id == sample.sample_id && feature.feature_id == *feature_id
            }) {
                missing_expected_feature_count += 1;
                missing_expected_feature_ids.push(feature_id.clone());
            }
        }
    }
    if samples.is_empty() {
        missing_expected_feature_ids.extend(
            input
                .expected_feature_ids
                .iter()
                .filter(|feature_id| !available_features.contains(feature_id.as_str()))
                .cloned(),
        );
    }
    missing_expected_feature_ids.sort();
    missing_expected_feature_ids.dedup();

    let mut issues = dataset.issues.clone();
    if input.expected_feature_ids.is_empty()
        || input
            .expected_feature_ids
            .iter()
            .any(|feature_id| feature_id.trim().is_empty())
    {
        issues.push("필수 피처 계약이 비어 있거나 올바르지 않습니다.".to_owned());
    }
    if !missing_expected_feature_ids.is_empty() {
        issues.push("필수 피처가 누락된 표본 또는 데이터셋입니다.".to_owned());
    }
    if matches!(
        asset.asset_class,
        ForecastAssetClass::KoreaStock | ForecastAssetClass::UnitedStatesStock
    ) && !input.corporate_action_coverage_confirmed
    {
        issues.push("액면분할·배당·병합 등 기업행사 반영을 확인하지 못했습니다.".to_owned());
    }
    if !input.trading_session_coverage_confirmed {
        issues.push("휴장·거래중단·세션 경계를 확인하지 못했습니다.".to_owned());
    }
    if !input.listing_history_checked {
        issues.push("상장 초기 또는 계약 개시 이전 데이터 여부를 확인하지 못했습니다.".to_owned());
    }

    ForecastDatasetAuditReview {
        dataset,
        missing_expected_feature_ids,
        missing_expected_feature_count,
        corporate_action_coverage_confirmed: input.corporate_action_coverage_confirmed,
        trading_session_coverage_confirmed: input.trading_session_coverage_confirmed,
        listing_history_checked: input.listing_history_checked,
        valid: issues.is_empty(),
        issues,
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbabilityForecast {
    pub forecast_id: String,
    pub model_id: String,
    pub model_version: String,
    pub dataset_id: String,
    pub asset_contract_id: String,
    pub horizon_ms: u64,
    pub generated_at_ms: u64,
    pub up_probability_bps: Option<u64>,
    pub down_probability_bps: Option<u64>,
    pub flat_probability_bps: Option<u64>,
    pub recommendation_confidence_bps: Option<u64>,
    pub model_reliability_bps: Option<u64>,
    pub unavailable_reason: Option<String>,
    pub price_only_fallback: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastEvidenceMode {
    FullFeatures,
    PriceOnlyFallback,
    Unavailable,
}

pub fn validate_forecast_evidence(
    forecast: &ProbabilityForecast,
    news_feature_available: bool,
    price_feature_available: bool,
) -> Result<ForecastEvidenceMode, String> {
    validate_probability_forecast(forecast)?;
    let probability_available = forecast.up_probability_bps.is_some();
    if !price_feature_available {
        if probability_available || forecast.price_only_fallback {
            return Err(
                "가격 피처가 없으면 확률을 산출하거나 가격 fallback으로 표시할 수 없습니다."
                    .to_owned(),
            );
        }
        return Ok(ForecastEvidenceMode::Unavailable);
    }
    if !news_feature_available {
        if probability_available && !forecast.price_only_fallback {
            return Err("뉴스 누락 시 가격 전용 fallback 여부를 명시해야 합니다.".to_owned());
        }
        if forecast.price_only_fallback {
            return Ok(ForecastEvidenceMode::PriceOnlyFallback);
        }
        return Ok(ForecastEvidenceMode::Unavailable);
    }
    if forecast.price_only_fallback {
        return Err(
            "전체 피처가 준비된 결과를 가격 전용 fallback으로 표시할 수 없습니다.".to_owned(),
        );
    }
    Ok(if probability_available {
        ForecastEvidenceMode::FullFeatures
    } else {
        ForecastEvidenceMode::Unavailable
    })
}

pub fn validate_probability_forecast(forecast: &ProbabilityForecast) -> Result<(), String> {
    if forecast.forecast_id.trim().is_empty()
        || forecast.model_id.trim().is_empty()
        || forecast.model_version.trim().is_empty()
        || forecast.dataset_id.trim().is_empty()
        || forecast.asset_contract_id.trim().is_empty()
        || forecast.horizon_ms == 0
        || forecast.generated_at_ms == 0
    {
        return Err("예측·모델·데이터셋·자산·horizon 식별자가 필요합니다.".to_owned());
    }
    let directions = [
        forecast.up_probability_bps,
        forecast.down_probability_bps,
        forecast.flat_probability_bps,
    ];
    if directions.iter().all(Option::is_none) {
        if forecast
            .unavailable_reason
            .as_deref()
            .is_none_or(str::is_empty)
            || forecast.recommendation_confidence_bps.is_some()
            || forecast.model_reliability_bps.is_some()
        {
            return Err("확률 산출 불가 시 숫자 대신 이유만 기록해야 합니다.".to_owned());
        }
        return Ok(());
    }
    if directions.iter().any(Option::is_none)
        || directions.iter().flatten().any(|value| *value > 10_000)
        || directions.iter().flatten().sum::<u64>() != 10_000
        || forecast
            .recommendation_confidence_bps
            .is_none_or(|value| value > 10_000)
        || forecast
            .model_reliability_bps
            .is_none_or(|value| value > 10_000)
        || forecast.unavailable_reason.is_some()
    {
        return Err("방향 확률 합계와 독립 확신도·신뢰도 범위가 올바르지 않습니다.".to_owned());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationObservation {
    pub predicted_up_bps: u64,
    pub actual_up: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationMetrics {
    pub sample_count: usize,
    pub brier_score_millionths: u64,
    pub log_loss_millionths: u64,
    pub expected_calibration_error_bps: u64,
    pub populated_bin_count: usize,
}

pub fn calibration_metrics(
    observations: &[CalibrationObservation],
) -> Result<CalibrationMetrics, String> {
    if observations.is_empty()
        || observations
            .iter()
            .any(|item| item.predicted_up_bps > 10_000)
    {
        return Err("0~100% 범위의 확률 관측값이 필요합니다.".to_owned());
    }
    let mut brier = 0f64;
    let mut log_loss = 0f64;
    let mut bins = [(0usize, 0u64, 0usize); 10];
    for item in observations {
        let probability = item.predicted_up_bps as f64 / 10_000.0;
        let actual = f64::from(item.actual_up);
        brier += (probability - actual).powi(2);
        let clipped = probability.clamp(1e-6, 1.0 - 1e-6);
        log_loss += -(actual * clipped.ln() + (1.0 - actual) * (1.0 - clipped).ln());
        let index = usize::try_from(item.predicted_up_bps.min(9_999) / 1_000)
            .expect("calibration bin is always within 0..=9");
        bins[index].0 += 1;
        bins[index].1 += item.predicted_up_bps;
        bins[index].2 += usize::from(item.actual_up);
    }
    let mut ece = 0f64;
    let mut populated = 0;
    for (count, predicted_sum, actual_sum) in bins {
        if count == 0 {
            continue;
        }
        populated += 1;
        let average_probability = predicted_sum as f64 / count as f64 / 10_000.0;
        let observed_frequency = actual_sum as f64 / count as f64;
        ece += (average_probability - observed_frequency).abs()
            * (count as f64 / observations.len() as f64);
    }
    Ok(CalibrationMetrics {
        sample_count: observations.len(),
        brier_score_millionths: (brier / observations.len() as f64 * 1_000_000.0).round() as u64,
        log_loss_millionths: (log_loss / observations.len() as f64 * 1_000_000.0).round() as u64,
        expected_calibration_error_bps: (ece * 10_000.0).round() as u64,
        populated_bin_count: populated,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastPromotionInput {
    pub dataset_valid: bool,
    pub contract_valid: bool,
    pub data_fresh: bool,
    pub drift_detected: bool,
    pub oos_sample_count: usize,
    pub calibration: CalibrationMetrics,
    pub maximum_brier_score_millionths: u64,
    pub maximum_log_loss_millionths: u64,
    pub maximum_calibration_error_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForecastPromotionDecision {
    pub eligible_for_shadow: bool,
    pub eligible_for_internal_paper: bool,
    pub live_order_permission: bool,
    pub observe_only: bool,
    pub blockers: Vec<String>,
}

pub fn evaluate_forecast_promotion(input: &ForecastPromotionInput) -> ForecastPromotionDecision {
    let mut blockers = Vec::new();
    if !input.dataset_valid || !input.contract_valid {
        blockers.push("데이터셋 또는 자산 계약 검증 실패".to_owned());
    }
    if !input.data_fresh {
        blockers.push("데이터 지연".to_owned());
    }
    if input.drift_detected {
        blockers.push("모델 drift 감지".to_owned());
    }
    if input.oos_sample_count < 30 {
        blockers.push("OOS 표본 부족".to_owned());
    }
    if input.calibration.brier_score_millionths > input.maximum_brier_score_millionths
        || input.calibration.log_loss_millionths > input.maximum_log_loss_millionths
        || input.calibration.expected_calibration_error_bps > input.maximum_calibration_error_bps
    {
        blockers.push("확률 보정 기준 미달".to_owned());
    }
    let eligible = blockers.is_empty();
    ForecastPromotionDecision {
        eligible_for_shadow: eligible,
        eligible_for_internal_paper: false,
        live_order_permission: false,
        observe_only: !eligible,
        blockers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn dataset_rejects_features_that_were_unavailable_at_decision_time() {
        let samples = vec![ForecastSample {
            sample_id: "sample-1".to_owned(),
            decision_time_ms: 100,
            target_observed_at_ms: 200,
            target_class: 1,
        }];
        let features = vec![ForecastFeature {
            feature_id: "news.sentiment".to_owned(),
            sample_id: "sample-1".to_owned(),
            source_record_id: "news-1".to_owned(),
            dataset_version: "dataset-v1".to_owned(),
            metadata: TemporalMetadata {
                event_time_ms: 90,
                available_at_ms: 101,
                ingested_at_ms: 102,
                source: "news".to_owned(),
                source_revision: "v1".to_owned(),
            },
            value_scaled: 1,
            value_scale: 1,
            quality_flags: vec![],
        }];
        let review = review_forecast_dataset(
            &stock_contract(),
            "dataset-v1",
            &samples,
            &features,
            &TimeSplit {
                train_end_ms: 10,
                validation_start_ms: 11,
                validation_end_ms: 20,
                test_start_ms: 21,
            },
        );
        assert!(!review.valid);
        assert!(review
            .issues
            .iter()
            .any(|issue| issue.contains("이후 정보")));
    }

    #[test]
    fn probability_contract_separates_direction_confidence_and_reliability() {
        let forecast = ProbabilityForecast {
            forecast_id: "forecast-1".to_owned(),
            model_id: "model-1".to_owned(),
            model_version: "v1".to_owned(),
            dataset_id: "dataset-1".to_owned(),
            asset_contract_id: "kr-stock-v1".to_owned(),
            horizon_ms: 86_400_000,
            generated_at_ms: 1,
            up_probability_bps: Some(5_000),
            down_probability_bps: Some(3_000),
            flat_probability_bps: Some(2_000),
            recommendation_confidence_bps: Some(4_000),
            model_reliability_bps: Some(6_000),
            unavailable_reason: None,
            price_only_fallback: false,
        };
        validate_probability_forecast(&forecast).expect("forecast");
    }

    #[test]
    fn dataset_audit_requires_market_calendar_corporate_actions_and_expected_features() {
        let samples = vec![ForecastSample {
            sample_id: "sample-1".to_owned(),
            decision_time_ms: 100,
            target_observed_at_ms: 200,
            target_class: 1,
        }];
        let features = vec![ForecastFeature {
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
        }];
        let review = audit_forecast_dataset(
            &stock_contract(),
            "dataset-v1",
            &samples,
            &features,
            &TimeSplit {
                train_end_ms: 10,
                validation_start_ms: 11,
                validation_end_ms: 20,
                test_start_ms: 21,
            },
            &ForecastDatasetAuditInput {
                expected_feature_ids: vec!["price.close".to_owned(), "news.sentiment".to_owned()],
                corporate_action_coverage_confirmed: false,
                trading_session_coverage_confirmed: false,
                listing_history_checked: false,
            },
        );
        assert!(!review.valid);
        assert_eq!(review.missing_expected_feature_ids, vec!["news.sentiment"]);
        assert!(review.issues.iter().any(|issue| issue.contains("기업행사")));
        assert!(review.issues.iter().any(|issue| issue.contains("휴장")));
        assert!(review
            .issues
            .iter()
            .any(|issue| issue.contains("상장 초기")));
    }

    #[test]
    fn news_gap_requires_explicit_price_only_fallback() {
        let mut forecast = ProbabilityForecast {
            forecast_id: "forecast-1".to_owned(),
            model_id: "model-1".to_owned(),
            model_version: "v1".to_owned(),
            dataset_id: "dataset-1".to_owned(),
            asset_contract_id: "kr-stock-v1".to_owned(),
            horizon_ms: 86_400_000,
            generated_at_ms: 1,
            up_probability_bps: Some(5_000),
            down_probability_bps: Some(3_000),
            flat_probability_bps: Some(2_000),
            recommendation_confidence_bps: Some(4_000),
            model_reliability_bps: Some(6_000),
            unavailable_reason: None,
            price_only_fallback: false,
        };
        assert!(validate_forecast_evidence(&forecast, false, true).is_err());
        forecast.price_only_fallback = true;
        assert_eq!(
            validate_forecast_evidence(&forecast, false, true).expect("fallback"),
            ForecastEvidenceMode::PriceOnlyFallback
        );
        assert!(validate_forecast_evidence(&forecast, true, true).is_err());
    }

    #[test]
    fn promotion_never_grants_live_or_internal_paper_permission() {
        let calibration = calibration_metrics(
            &[CalibrationObservation {
                predicted_up_bps: 8_000,
                actual_up: true,
            }; 30],
        )
        .expect("calibration");
        let decision = evaluate_forecast_promotion(&ForecastPromotionInput {
            dataset_valid: true,
            contract_valid: true,
            data_fresh: true,
            drift_detected: false,
            oos_sample_count: 30,
            calibration,
            maximum_brier_score_millionths: 100_000,
            maximum_log_loss_millionths: 500_000,
            maximum_calibration_error_bps: 3_000,
        });
        assert!(decision.eligible_for_shadow);
        assert!(!decision.eligible_for_internal_paper);
        assert!(!decision.live_order_permission);
    }
}
