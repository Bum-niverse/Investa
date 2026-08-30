#!/usr/bin/env python3
"""Investa의 격리형, 주문 권한 없는 기준 ML worker."""

from __future__ import annotations

import argparse
import gc
import hashlib
import json
import math
import os
import re
import shutil
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


CONTRACT_VERSION = "investa-ml-worker-v1"
SHARD_CONTRACT_VERSION = "investa-ml-worker-sharded-v1"
MAX_INPUT_BYTES = 64 * 1024 * 1024
MAX_SAMPLES = 20_000
MAX_FEATURE_ROWS = 200_000
MAX_DATASET_SHARDS = 64
MAX_SHARDED_SAMPLES = 1_000_000
MAX_SHARDED_FEATURE_ROWS = 10_000_000
CLASS_COUNT = 3
PROBABILITY_SCALE = 1_000_000
SAFE_IDENTIFIER = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
ALLOWED_HYPERPARAMETERS = {
    "num_boost_round",
    "early_stopping_rounds",
    "eta",
    "learning_rate",
    "max_depth",
    "num_leaves",
    "min_data_in_leaf",
    "min_child_weight",
    "subsample",
    "colsample_bytree",
    "feature_fraction",
    "bagging_fraction",
    "lambda_l1",
    "lambda_l2",
    "reg_alpha",
    "reg_lambda",
}
ALGORITHM_HYPERPARAMETERS = {
    "lightgbm": {
        "learning_rate", "max_depth", "num_leaves", "min_data_in_leaf",
        "feature_fraction", "bagging_fraction", "lambda_l1", "lambda_l2",
    },
    "xgboost": {
        "learning_rate", "eta", "max_depth", "min_child_weight", "subsample",
        "colsample_bytree", "reg_alpha", "reg_lambda",
    },
}


class ContractError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


@dataclass(frozen=True)
class PreparedDataset:
    feature_ids: list[str]
    train_x: Any
    train_y: Any
    validation_x: Any
    validation_y: Any
    test_x: Any
    test_y: Any
    test_sample_ids: list[str]


def canonical_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def require_dict(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ContractError("invalid_contract", f"{label} 객체가 필요합니다.")
    return value


def require_int(value: Any, label: str, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        raise ContractError("invalid_contract", f"{label} 정수가 올바르지 않습니다.")
    return value


def require_identifier(value: Any, label: str) -> str:
    if not isinstance(value, str) or SAFE_IDENTIFIER.fullmatch(value) is None:
        raise ContractError("invalid_contract", f"{label} 식별자가 올바르지 않습니다.")
    return value


def canonical_job_for_hash(job: dict[str, Any]) -> dict[str, Any]:
    fields = [
        "jobId", "manifestId", "datasetSourceKind", "algorithm", "contractVersion",
        "datasetContentSha256", "featureSchemaSha256", "inputSha256",
        "codeVersion", "randomSeed", "horizonMs", "timeoutSeconds",
        "memoryLimitMb", "maxThreads", "hyperparameters", "status",
        "createdAtMs", "updatedAtMs", "liveOrderAllowed",
    ]
    if set(job) != set(fields):
        raise ContractError("invalid_contract", "학습 작업 필드가 worker 계약과 다릅니다.")
    ordered = {field: job[field] for field in fields}
    ordered["inputSha256"] = ""
    hyperparameters = require_dict(ordered["hyperparameters"], "hyperparameters")
    ordered["hyperparameters"] = {key: hyperparameters[key] for key in sorted(hyperparameters)}
    return ordered


def validate_payload(
    payload_json: str,
    manifest: dict[str, Any],
    expected_schema_hash: str,
) -> dict[str, Any]:
    if not payload_json or len(payload_json.encode("utf-8")) > MAX_INPUT_BYTES:
        raise ContractError("invalid_dataset", "데이터 payload 크기가 올바르지 않습니다.")
    content_hash = sha256_bytes(payload_json.encode("utf-8"))
    if content_hash != manifest.get("contentSha256"):
        raise ContractError("dataset_hash_mismatch", "데이터셋 SHA-256이 매니페스트와 다릅니다.")
    try:
        payload = require_dict(json.loads(payload_json), "dataset payload")
    except json.JSONDecodeError as error:
        raise ContractError("invalid_dataset", "데이터 payload JSON을 해석할 수 없습니다.") from error

    feature_ids = payload.get("expectedFeatureIds")
    if not isinstance(feature_ids, list) or not feature_ids or any(not isinstance(item, str) or not item for item in feature_ids):
        raise ContractError("invalid_feature_schema", "expectedFeatureIds가 필요합니다.")
    if feature_ids != sorted(set(feature_ids)):
        raise ContractError("invalid_feature_schema", "feature schema는 정렬된 고유 ID여야 합니다.")
    schema_hash = sha256_bytes(canonical_json(feature_ids).encode("utf-8"))
    if schema_hash != manifest.get("featureSchemaSha256") or schema_hash != expected_schema_hash:
        raise ContractError("feature_schema_hash_mismatch", "피처 스키마 SHA-256이 다릅니다.")
    return payload


def validate_bundle(bundle: dict[str, Any]) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
    if bundle.get("contractVersion") != CONTRACT_VERSION:
        raise ContractError("unsupported_contract", "지원하지 않는 worker 계약입니다.")
    job = require_dict(bundle.get("job"), "job")
    manifest = require_dict(bundle.get("manifest"), "manifest")
    if bundle.get("liveOrderAllowed") is not False or job.get("liveOrderAllowed") is not False:
        raise ContractError("live_order_forbidden", "ML worker에는 실주문 권한을 제공할 수 없습니다.")
    if job.get("contractVersion") != CONTRACT_VERSION or job.get("status") != "prepared":
        raise ContractError("invalid_job_state", "prepared 상태의 현재 계약 작업만 실행할 수 있습니다.")
    require_identifier(job.get("jobId"), "job")
    require_identifier(job.get("manifestId"), "manifest")
    require_identifier(job.get("codeVersion"), "code version")
    if job.get("manifestId") != manifest.get("manifestId"):
        raise ContractError("manifest_mismatch", "작업과 매니페스트 ID가 다릅니다.")
    if job.get("datasetSourceKind") != "manifest":
        raise ContractError("invalid_dataset_source", "단일 worker에는 manifest 원천만 허용합니다.")

    payload_json = bundle.get("datasetPayloadJson")
    if not isinstance(payload_json, str):
        raise ContractError("invalid_dataset", "데이터 payload 크기가 올바르지 않습니다.")
    payload = validate_payload(payload_json, manifest, str(job.get("featureSchemaSha256")))
    if manifest.get("contentSha256") != job.get("datasetContentSha256"):
        raise ContractError("dataset_hash_mismatch", "데이터셋 SHA-256이 매니페스트와 다릅니다.")

    expected_input_hash = job.get("inputSha256")
    actual_input_hash = sha256_bytes(canonical_json(canonical_job_for_hash(job)).encode("utf-8"))
    if expected_input_hash != actual_input_hash:
        raise ContractError("job_hash_mismatch", "학습 작업 SHA-256이 준비 시점과 다릅니다.")
    return job, manifest, payload


def validate_shard_bundle(
    bundle: dict[str, Any], input_dir: Path
) -> tuple[dict[str, Any], dict[str, Any], list[Path], dict[str, Any]]:
    if bundle.get("contractVersion") != SHARD_CONTRACT_VERSION:
        raise ContractError("unsupported_contract", "지원하지 않는 shard worker 계약입니다.")
    job = require_dict(bundle.get("job"), "job")
    shard_set = require_dict(bundle.get("shardSet"), "shard set")
    shard_files = bundle.get("datasetShards")
    if bundle.get("liveOrderAllowed") is not False or job.get("liveOrderAllowed") is not False:
        raise ContractError("live_order_forbidden", "ML worker에는 실주문 권한을 제공할 수 없습니다.")
    if (
        job.get("contractVersion") != SHARD_CONTRACT_VERSION
        or job.get("status") != "prepared"
        or job.get("datasetSourceKind") != "shard_set"
        or job.get("algorithm") != "xgboost"
    ):
        raise ContractError("invalid_job_state", "prepared 상태의 XGBoost shard 작업만 실행할 수 있습니다.")
    require_identifier(job.get("jobId"), "job")
    shard_set_id = require_identifier(shard_set.get("shardSetId"), "shard set")
    require_identifier(job.get("manifestId"), "shard set")
    require_identifier(job.get("codeVersion"), "code version")
    if job.get("manifestId") != shard_set_id:
        raise ContractError("manifest_mismatch", "작업과 shard set ID가 다릅니다.")
    if shard_set.get("liveOrderAllowed") is not False:
        raise ContractError("live_order_forbidden", "shard set에는 실주문 권한이 없어야 합니다.")
    descriptors = shard_set.get("shards")
    if (
        not isinstance(descriptors, list)
        or not 2 <= len(descriptors) <= MAX_DATASET_SHARDS
        or not isinstance(shard_files, list)
        or len(shard_files) != len(descriptors)
        or shard_set.get("shardCount") != len(descriptors)
    ):
        raise ContractError("invalid_shard_set", "shard 목록과 개수가 올바르지 않습니다.")
    combined_input = [
        shard_set.get("datasetId"),
        shard_set.get("asset"),
        shard_set.get("split"),
        shard_set.get("featureSchemaSha256"),
        descriptors,
    ]
    combined_hash = sha256_bytes(canonical_json(combined_input).encode("utf-8"))
    if (
        combined_hash != shard_set.get("combinedContentSha256")
        or combined_hash != job.get("datasetContentSha256")
        or shard_set.get("featureSchemaSha256") != job.get("featureSchemaSha256")
    ):
        raise ContractError("dataset_hash_mismatch", "shard set 결합 SHA-256이 다릅니다.")

    expected_input_hash = job.get("inputSha256")
    actual_input_hash = sha256_bytes(canonical_json(canonical_job_for_hash(job)).encode("utf-8"))
    if expected_input_hash != actual_input_hash:
        raise ContractError("job_hash_mismatch", "학습 작업 SHA-256이 준비 시점과 다릅니다.")

    shard_dir = (input_dir / "shards").resolve(strict=True)
    if not shard_dir.is_dir() or shard_dir.parent != input_dir.resolve(strict=True):
        raise ContractError("invalid_shard_path", "shard 입력 폴더 경계가 올바르지 않습니다.")
    paths: list[Path] = []
    seen_sample_ids: set[str] = set()
    train_classes: set[int] = set()
    test_sample_ids: list[str] = []
    test_targets: list[int] = []
    total_samples = 0
    total_features = 0
    logical_sample_xor = bytearray(32)
    logical_feature_xor = bytearray(32)
    previous_extents = {"train": -1, "validation": -1, "test": -1}
    for index, (descriptor_raw, file_raw) in enumerate(zip(descriptors, shard_files)):
        descriptor = require_dict(descriptor_raw, "shard descriptor")
        file_record = require_dict(file_raw, "shard file")
        manifest = require_dict(file_record.get("manifest"), "shard manifest")
        file_name = file_record.get("fileName")
        if not isinstance(file_name, str) or re.fullmatch(r"shard-[0-9]{4}\.json", file_name) is None:
            raise ContractError("invalid_shard_path", "shard 파일명 형식이 올바르지 않습니다.")
        if file_name != f"shard-{index:04}.json":
            raise ContractError("invalid_shard_order", "shard 파일 순서가 고정 목록과 다릅니다.")
        path = (shard_dir / file_name).resolve(strict=True)
        if path.parent != shard_dir or not path.is_file():
            raise ContractError("invalid_shard_path", "shard 파일이 허용 폴더 밖에 있습니다.")
        byte_size = require_int(file_record.get("byteSize"), "shard byteSize", 1)
        if path.stat().st_size != byte_size or byte_size > MAX_INPUT_BYTES:
            raise ContractError("invalid_dataset", "shard 파일 크기가 계약과 다릅니다.")
        if descriptor.get("manifestId") != manifest.get("manifestId"):
            raise ContractError("manifest_mismatch", "shard descriptor와 매니페스트 ID가 다릅니다.")
        payload_json = path.read_text(encoding="utf-8")
        payload = validate_payload(payload_json, manifest, str(job.get("featureSchemaSha256")))
        if manifest.get("contentSha256") != descriptor.get("contentSha256"):
            raise ContractError("dataset_hash_mismatch", "shard content SHA-256이 목록과 다릅니다.")
        if payload.get("asset") != shard_set.get("asset") or payload.get("split") != shard_set.get("split"):
            raise ContractError("invalid_shard_set", "shard 자산 또는 split 계약이 다릅니다.")
        prepared = prepare_dataset(payload, require_all_train_classes=False)
        samples = payload.get("samples")
        features = payload.get("features")
        if (
            descriptor.get("sampleCount") != len(samples)
            or descriptor.get("featureCount") != len(features)
            or manifest.get("sampleCount") != len(samples)
            or manifest.get("featureCount") != len(features)
        ):
            raise ContractError("invalid_shard_set", "shard 행 수가 매니페스트와 다릅니다.")
        for sample in samples:
            sample_id = require_identifier(sample.get("sampleId"), "sample")
            if sample_id in seen_sample_ids:
                raise ContractError("duplicate_sample", "shard 사이에 중복 표본 ID가 있습니다.")
            seen_sample_ids.add(sample_id)
            record_hash = hashlib.sha256(b"sample\0" + canonical_json(sample).encode("utf-8")).digest()
            for byte_index, value in enumerate(record_hash):
                logical_sample_xor[byte_index] ^= value
        for feature in sorted(features, key=lambda item: (item.get("sampleId", ""), item.get("featureId", ""))):
            record_hash = hashlib.sha256(b"feature\0" + canonical_json(feature).encode("utf-8")).digest()
            for byte_index, value in enumerate(record_hash):
                logical_feature_xor[byte_index] ^= value
        train_end = payload["split"]["trainEndMs"]
        validation_start = payload["split"]["validationStartMs"]
        validation_end = payload["split"]["validationEndMs"]
        test_start = payload["split"]["testStartMs"]
        split_arrays = {
            "train": (prepared.train_y, descriptor.get("train"), [item["decisionTimeMs"] for item in samples if item["decisionTimeMs"] <= train_end]),
            "validation": (prepared.validation_y, descriptor.get("validation"), [item["decisionTimeMs"] for item in samples if validation_start <= item["decisionTimeMs"] <= validation_end]),
            "test": (prepared.test_y, descriptor.get("test"), [item["decisionTimeMs"] for item in samples if item["decisionTimeMs"] >= test_start]),
        }
        for label, (targets, extent_raw, decisions) in split_arrays.items():
            extent = require_dict(extent_raw, f"{label} extent")
            first = require_int(extent.get("firstDecisionTimeMs"), "firstDecisionTimeMs", 1)
            last = require_int(extent.get("lastDecisionTimeMs"), "lastDecisionTimeMs", 1)
            if (
                extent.get("sampleCount") != len(targets)
                or not decisions
                or first != decisions[0]
                or last != decisions[-1]
            ):
                raise ContractError("invalid_shard_set", f"{label} shard 실제 시간 범위가 descriptor와 다릅니다.")
            if first > last or first <= previous_extents[label]:
                raise ContractError("invalid_shard_order", f"{label} shard 시간이 겹치거나 역전됐습니다.")
            previous_extents[label] = last
        train_classes.update(int(value) for value in prepared.train_y.tolist())
        test_sample_ids.extend(prepared.test_sample_ids)
        test_targets.extend(int(value) for value in prepared.test_y.tolist())
        total_samples += len(samples)
        total_features += len(features)
        paths.append(path)
    if train_classes != set(range(CLASS_COUNT)):
        raise ContractError("insufficient_class_coverage", "전체 학습 shard에 세 방향 클래스가 모두 필요합니다.")
    if (
        total_samples != shard_set.get("sampleCount")
        or total_features != shard_set.get("featureCount")
        or total_samples > MAX_SHARDED_SAMPLES
        or total_features > MAX_SHARDED_FEATURE_ROWS
    ):
        raise ContractError("invalid_shard_set", "shard set 전체 행 수가 계약과 다릅니다.")
    summary = {
        "testSampleIds": test_sample_ids,
        "testTargets": test_targets,
        "logicalDatasetSha256": sha256_bytes(canonical_json([
            total_samples, total_features, logical_sample_xor.hex(), logical_feature_xor.hex(),
        ]).encode("utf-8")),
    }
    return job, shard_set, paths, summary


def prepare_dataset(payload: dict[str, Any], *, require_all_train_classes: bool = True) -> PreparedDataset:
    import numpy as np

    samples = payload.get("samples")
    features = payload.get("features")
    split = require_dict(payload.get("split"), "split")
    feature_ids = payload.get("expectedFeatureIds")
    if not isinstance(samples, list) or not 3 <= len(samples) <= MAX_SAMPLES:
        raise ContractError("invalid_dataset", "표본 수가 허용 범위를 벗어났습니다.")
    if not isinstance(features, list) or not features or len(features) > MAX_FEATURE_ROWS:
        raise ContractError("invalid_dataset", "피처 수가 허용 범위를 벗어났습니다.")
    train_end = require_int(split.get("trainEndMs"), "trainEndMs", 1)
    validation_start = require_int(split.get("validationStartMs"), "validationStartMs", 1)
    validation_end = require_int(split.get("validationEndMs"), "validationEndMs", 1)
    test_start = require_int(split.get("testStartMs"), "testStartMs", 1)
    if not train_end < validation_start <= validation_end < test_start:
        raise ContractError("invalid_split", "train·validation·test 시간 구간이 겹칩니다.")

    sample_by_id: dict[str, tuple[int, int, int]] = {}
    previous_decision = -1
    ordered_ids: list[str] = []
    for raw in samples:
        sample = require_dict(raw, "sample")
        sample_id = require_identifier(sample.get("sampleId"), "sample")
        decision = require_int(sample.get("decisionTimeMs"), "decisionTimeMs", 1)
        observed = require_int(sample.get("targetObservedAtMs"), "targetObservedAtMs", 1)
        target = require_int(sample.get("targetClass"), "targetClass")
        if sample_id in sample_by_id or decision <= previous_decision or observed <= decision or target >= CLASS_COUNT:
            raise ContractError("invalid_sample", "표본 순서·시각·클래스가 올바르지 않습니다.")
        if (decision <= train_end and observed >= validation_start) or (
            validation_start <= decision <= validation_end and observed >= test_start
        ):
            raise ContractError("target_leakage", "타깃 관측 시각이 다음 구간으로 넘어갑니다.")
        sample_by_id[sample_id] = (decision, observed, target)
        ordered_ids.append(sample_id)
        previous_decision = decision

    feature_index = {name: index for index, name in enumerate(feature_ids)}
    matrix = np.empty((len(samples), len(feature_ids)), dtype=np.float64)
    matrix.fill(np.nan)
    row_index = {sample_id: index for index, sample_id in enumerate(ordered_ids)}
    seen: set[tuple[str, str]] = set()
    dataset_id = payload.get("datasetId")
    for raw in features:
        feature = require_dict(raw, "feature")
        sample_id = feature.get("sampleId")
        feature_id = feature.get("featureId")
        if sample_id not in row_index or feature_id not in feature_index:
            raise ContractError("unexpected_feature", "표본 또는 피처 ID가 스키마와 다릅니다.")
        key = (sample_id, feature_id)
        if key in seen or feature.get("datasetVersion") != dataset_id:
            raise ContractError("duplicate_feature", "중복 피처 또는 데이터셋 버전 불일치가 있습니다.")
        seen.add(key)
        metadata = require_dict(feature.get("metadata"), "feature metadata")
        event_time = require_int(metadata.get("eventTimeMs"), "eventTimeMs")
        available_at = require_int(metadata.get("availableAtMs"), "availableAtMs")
        ingested_at = require_int(metadata.get("ingestedAtMs"), "ingestedAtMs")
        decision = sample_by_id[sample_id][0]
        if event_time > available_at or available_at > decision or ingested_at < available_at:
            raise ContractError("feature_leakage", "결정 시각 이후 피처가 포함되었습니다.")
        scaled = require_int(feature.get("valueScaled"), "valueScaled", -2**63)
        scale = require_int(feature.get("valueScale"), "valueScale", 1)
        value = scaled / scale
        if not math.isfinite(value):
            raise ContractError("invalid_feature_value", "유한하지 않은 피처 값입니다.")
        matrix[row_index[sample_id], feature_index[feature_id]] = value
    if len(seen) != len(samples) * len(feature_ids) or np.isnan(matrix).any():
        raise ContractError("missing_feature", "모든 표본에 동일한 피처가 필요합니다.")

    decisions = np.array([sample_by_id[sample_id][0] for sample_id in ordered_ids], dtype=np.int64)
    targets = np.array([sample_by_id[sample_id][2] for sample_id in ordered_ids], dtype=np.int32)
    train_mask = decisions <= train_end
    validation_mask = (decisions >= validation_start) & (decisions <= validation_end)
    test_mask = decisions >= test_start
    if not train_mask.any() or not validation_mask.any() or not test_mask.any():
        raise ContractError("empty_split", "각 시간 구간에 최소 한 표본이 필요합니다.")
    if require_all_train_classes and set(targets[train_mask].tolist()) != set(range(CLASS_COUNT)):
        raise ContractError("insufficient_class_coverage", "학습 구간에 세 방향 클래스가 모두 필요합니다.")
    return PreparedDataset(
        feature_ids=list(feature_ids), train_x=matrix[train_mask], train_y=targets[train_mask],
        validation_x=matrix[validation_mask], validation_y=targets[validation_mask],
        test_x=matrix[test_mask], test_y=targets[test_mask],
        test_sample_ids=[sample_id for sample_id, selected in zip(ordered_ids, test_mask.tolist()) if selected],
    )


def normalized_hyperparameters(job: dict[str, Any]) -> dict[str, Any]:
    raw = require_dict(job.get("hyperparameters"), "hyperparameters")
    unknown = sorted(set(raw) - ALLOWED_HYPERPARAMETERS)
    if unknown:
        raise ContractError("unsupported_hyperparameter", f"허용되지 않은 하이퍼파라미터: {', '.join(unknown)}")
    values = dict(raw)
    algorithm = job.get("algorithm")
    if algorithm not in ALGORITHM_HYPERPARAMETERS:
        raise ContractError("unsupported_algorithm", "현재 worker는 LightGBM과 XGBoost만 실행합니다.")
    incompatible = sorted(set(values) - {"num_boost_round", "early_stopping_rounds"} - ALGORITHM_HYPERPARAMETERS[algorithm])
    if incompatible:
        raise ContractError("unsupported_hyperparameter", f"{algorithm}에서 지원하지 않는 하이퍼파라미터: {', '.join(incompatible)}")
    rounds_value = values.pop("num_boost_round", 100)
    early_value = values.pop("early_stopping_rounds", 15)
    if isinstance(rounds_value, bool) or not isinstance(rounds_value, int):
        raise ContractError("invalid_hyperparameter", "num_boost_round는 정수여야 합니다.")
    if isinstance(early_value, bool) or not isinstance(early_value, int):
        raise ContractError("invalid_hyperparameter", "early_stopping_rounds는 정수여야 합니다.")
    rounds = rounds_value
    early = early_value
    if not 10 <= rounds <= 2_000 or not 0 <= early <= min(200, rounds - 1):
        raise ContractError("invalid_hyperparameter", "boost round 또는 early stopping 범위가 올바르지 않습니다.")
    for name, value in values.items():
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(float(value)):
            raise ContractError("invalid_hyperparameter", f"{name}은 유한한 숫자여야 합니다.")
        if name in {"learning_rate", "eta"} and not 0.0001 <= float(value) <= 1.0:
            raise ContractError("invalid_hyperparameter", f"{name}는 0.0001~1 범위여야 합니다.")
        if name in {"subsample", "colsample_bytree", "feature_fraction", "bagging_fraction"} and not 0.1 <= float(value) <= 1.0:
            raise ContractError("invalid_hyperparameter", f"{name}은 0.1~1 범위여야 합니다.")
        if name in {"max_depth", "num_leaves", "min_data_in_leaf"} and not 1 <= int(value) <= 4096:
            raise ContractError("invalid_hyperparameter", f"{name} 범위가 올바르지 않습니다.")
    values["num_boost_round"] = rounds
    values["early_stopping_rounds"] = early
    return values


def train_model(job: dict[str, Any], dataset: PreparedDataset, artifact_path: Path) -> Any:
    algorithm = job.get("algorithm")
    values = normalized_hyperparameters(job)
    rounds = values.pop("num_boost_round")
    early = values.pop("early_stopping_rounds")
    seed = require_int(job.get("randomSeed"), "randomSeed")
    threads = require_int(job.get("maxThreads"), "maxThreads", 1)
    threads = min(threads, 16)
    os.environ["OMP_NUM_THREADS"] = str(threads)
    os.environ["OPENBLAS_NUM_THREADS"] = str(threads)
    os.environ["MKL_NUM_THREADS"] = str(threads)

    if algorithm == "lightgbm":
        import lightgbm as lgb

        params: dict[str, Any] = {
            "objective": "multiclass", "num_class": CLASS_COUNT, "metric": "multi_logloss",
            "verbosity": -1, "seed": seed, "feature_fraction_seed": seed,
            "bagging_seed": seed, "data_random_seed": seed, "deterministic": True,
            "force_col_wise": True, "num_threads": threads,
        }
        params.update(values)
        safe_feature_names = [f"feature_{index:04d}" for index in range(len(dataset.feature_ids))]
        train = lgb.Dataset(dataset.train_x, label=dataset.train_y, feature_name=safe_feature_names)
        validation = lgb.Dataset(dataset.validation_x, label=dataset.validation_y, reference=train)
        callbacks = [lgb.early_stopping(early, verbose=False)] if early else []
        model = lgb.train(params, train, num_boost_round=rounds, valid_sets=[validation], callbacks=callbacks)
        model.save_model(str(artifact_path))
        return model.predict(dataset.test_x)
    if algorithm == "xgboost":
        import xgboost as xgb

        params = {
            "objective": "multi:softprob", "num_class": CLASS_COUNT, "eval_metric": "mlogloss",
            "seed": seed, "nthread": threads, "tree_method": "hist",
        }
        params.update(values)
        safe_feature_names = [f"feature_{index:04d}" for index in range(len(dataset.feature_ids))]
        train = xgb.DMatrix(dataset.train_x, label=dataset.train_y, feature_names=safe_feature_names)
        validation = xgb.DMatrix(dataset.validation_x, label=dataset.validation_y, feature_names=safe_feature_names)
        test = xgb.DMatrix(dataset.test_x, feature_names=safe_feature_names)
        model = xgb.train(
            params, train, num_boost_round=rounds, evals=[(validation, "validation")],
            early_stopping_rounds=early or None, verbose_eval=False,
        )
        model.save_model(artifact_path)
        return model.predict(test)
    raise ContractError("unsupported_algorithm", "현재 worker는 LightGBM과 XGBoost만 실행합니다.")


class ShardDataIter:
    """XGBoost가 요청한 split만 한 shard씩 메모리에 올리는 반복자."""

    def __new__(cls, *args: Any, **kwargs: Any) -> Any:
        import xgboost as xgb

        class _Iterator(xgb.DataIter):
            def __init__(self, paths: list[Path], split_name: str, cache_prefix: Path) -> None:
                super().__init__(cache_prefix=str(cache_prefix), release_data=True, on_host=True)
                self.paths = paths
                self.split_name = split_name
                self.index = 0

            def reset(self) -> None:
                self.index = 0

            def next(self, input_data: Any) -> bool:
                if self.index >= len(self.paths):
                    return False
                payload = require_dict(json.loads(self.paths[self.index].read_text(encoding="utf-8")), "dataset payload")
                prepared = prepare_dataset(payload, require_all_train_classes=False)
                input_data(
                    data=getattr(prepared, f"{self.split_name}_x"),
                    label=getattr(prepared, f"{self.split_name}_y"),
                )
                self.index += 1
                return True

        return _Iterator(*args, **kwargs)


def train_sharded_xgboost(
    job: dict[str, Any], shard_paths: list[Path], artifact_path: Path, cache_dir: Path
) -> Any:
    import xgboost as xgb

    values = normalized_hyperparameters(job)
    rounds = values.pop("num_boost_round")
    early = values.pop("early_stopping_rounds")
    seed = require_int(job.get("randomSeed"), "randomSeed")
    threads = min(require_int(job.get("maxThreads"), "maxThreads", 1), 16)
    os.environ["OMP_NUM_THREADS"] = str(threads)
    os.environ["OPENBLAS_NUM_THREADS"] = str(threads)
    os.environ["MKL_NUM_THREADS"] = str(threads)
    cache_dir.mkdir(parents=False, exist_ok=False)
    params: dict[str, Any] = {
        "objective": "multi:softprob", "num_class": CLASS_COUNT,
        "eval_metric": "mlogloss", "seed": seed, "nthread": threads,
        "tree_method": "hist",
    }
    params.update(values)
    train = validation = test = model = None
    try:
        train = xgb.ExtMemQuantileDMatrix(
            ShardDataIter(shard_paths, "train", cache_dir / "train"), max_bin=256,
        )
        validation = xgb.ExtMemQuantileDMatrix(
            ShardDataIter(shard_paths, "validation", cache_dir / "validation"),
            max_bin=256, ref=train,
        )
        test = xgb.ExtMemQuantileDMatrix(
            ShardDataIter(shard_paths, "test", cache_dir / "test"),
            max_bin=256, ref=train,
        )
        model = xgb.train(
            params, train, num_boost_round=rounds,
            evals=[(validation, "validation")],
            early_stopping_rounds=early or None, verbose_eval=False,
        )
        model.save_model(artifact_path)
        return model.predict(test)
    finally:
        model = test = validation = train = None
        gc.collect()
        shutil.rmtree(cache_dir, ignore_errors=True)


def quantize_predictions(sample_ids: list[str], targets: Any, probabilities: Any) -> list[dict[str, int | str]]:
    import numpy as np

    probabilities = np.asarray(probabilities, dtype=np.float64)
    targets = np.asarray(targets, dtype=np.int32)
    if len(sample_ids) != len(targets) or probabilities.shape != (len(targets), CLASS_COUNT) or not np.isfinite(probabilities).all():
        raise ContractError("invalid_prediction", "모델 확률 출력 형식이 올바르지 않습니다.")
    row_sums = probabilities.sum(axis=1)
    if (probabilities < 0).any() or not np.allclose(row_sums, 1.0, atol=1e-6):
        raise ContractError("invalid_prediction", "확률 합계 또는 범위가 올바르지 않습니다.")
    records: list[dict[str, int | str]] = []
    distributable = PROBABILITY_SCALE - CLASS_COUNT
    for sample_id, target, row in zip(sample_ids, targets.tolist(), probabilities.tolist()):
        normalized = [value / sum(row) for value in row]
        raw = [value * distributable for value in normalized]
        scaled = [math.floor(value) + 1 for value in raw]
        remainder = PROBABILITY_SCALE - sum(scaled)
        order = sorted(range(CLASS_COUNT), key=lambda index: (raw[index] - math.floor(raw[index]), -index), reverse=True)
        for index in order[:remainder]:
            scaled[index] += 1
        records.append({
            "sampleId": sample_id, "foldIndex": 0, "targetClass": int(target),
            "probabilityDownMillionths": scaled[0], "probabilityFlatMillionths": scaled[1],
            "probabilityUpMillionths": scaled[2],
        })
    return records


def round_positive(value: float) -> int:
    return math.floor(value + 0.5)


def compute_metrics(predictions: list[dict[str, int | str]], evaluated_at_ms: int) -> dict[str, int]:
    if not predictions:
        raise ContractError("invalid_prediction", "OOS 원시 예측이 필요합니다.")
    class_total = [0, 0, 0]
    class_correct = [0, 0, 0]
    brier_numerator = 0
    log_loss = 0.0
    ece_bins = [[0, 0, 0] for _ in range(10)]
    folds: set[int] = set()
    for prediction in predictions:
        target = int(prediction["targetClass"])
        fold = int(prediction["foldIndex"])
        probabilities = [
            int(prediction["probabilityDownMillionths"]),
            int(prediction["probabilityFlatMillionths"]),
            int(prediction["probabilityUpMillionths"]),
        ]
        if target not in range(CLASS_COUNT) or sum(probabilities) != PROBABILITY_SCALE or min(probabilities) <= 0:
            raise ContractError("invalid_prediction", "양자화된 OOS 확률이 올바르지 않습니다.")
        folds.add(fold)
        log_loss -= math.log(probabilities[target] / PROBABILITY_SCALE)
        for class_index, probability in enumerate(probabilities):
            expected = PROBABILITY_SCALE if class_index == target else 0
            brier_numerator += (probability - expected) ** 2
        predicted = max(range(CLASS_COUNT), key=lambda index: (probabilities[index], -index))
        class_total[target] += 1
        correct = int(predicted == target)
        class_correct[target] += correct
        confidence = probabilities[predicted]
        bin_index = min(confidence // 100_000, 9)
        ece_bins[bin_index][0] += 1
        ece_bins[bin_index][1] += confidence
        ece_bins[bin_index][2] += correct
    if sorted(folds) != list(range(len(folds))):
        raise ContractError("invalid_prediction", "OOS fold index가 연속적이지 않습니다.")
    sample_count = len(predictions)
    recalls = [class_correct[index] / total for index, total in enumerate(class_total) if total]
    balanced_accuracy = sum(recalls) / len(recalls)
    brier_denominator = sample_count * CLASS_COUNT * PROBABILITY_SCALE
    ece_numerator = sum(abs(correct * PROBABILITY_SCALE - confidence_sum) for count, confidence_sum, correct in ece_bins if count)
    ece_denominator = sample_count * PROBABILITY_SCALE
    return {
        "sampleCount": sample_count, "foldCount": len(folds),
        "logLossMillionths": round_positive(log_loss / sample_count * PROBABILITY_SCALE),
        "brierScoreMillionths": (brier_numerator + brier_denominator // 2) // brier_denominator,
        "expectedCalibrationErrorBps": (ece_numerator * 10_000 + ece_denominator // 2) // ece_denominator,
        "balancedAccuracyBps": round_positive(balanced_accuracy * 10_000),
        "evaluatedAtMs": evaluated_at_ms,
    }


def safe_artifact_name(job: dict[str, Any]) -> tuple[str, str]:
    algorithm = job["algorithm"]
    extension, artifact_format = ("txt", "lightgbm_text") if algorithm == "lightgbm" else ("json", "xgboost_json")
    safe_job_id = re.sub(r"[^A-Za-z0-9_.-]", "_", job["jobId"])
    return f"{safe_job_id}.{extension}", artifact_format


def build_result(
    job: dict[str, Any], asset_class: str, artifact_name: str,
    artifact_format: str, artifact_bytes: bytes, predictions: list[dict[str, int | str]],
    completed_at_ms: int | None,
) -> dict[str, Any]:
    now_ms = completed_at_ms or int(time.time() * 1000)
    now_ms = max(now_ms, require_int(job.get("createdAtMs"), "createdAtMs", 1))
    return {
        "jobId": job["jobId"], "inputSha256": job["inputSha256"], "completedAtMs": now_ms,
        "succeeded": True, "failureCode": None,
        "modelId": f"{job['algorithm']}-{asset_class}",
        "modelVersion": f"{job['codeVersion']}-{job['inputSha256'][:12]}",
        "artifact": {"fileName": artifact_name, "format": artifact_format, "sha256": sha256_bytes(artifact_bytes), "byteSize": len(artifact_bytes)},
        "metrics": compute_metrics(predictions, now_ms), "predictions": predictions,
    }


def run(
    bundle: dict[str, Any], output_dir: Path, completed_at_ms: int | None = None,
    input_dir: Path | None = None,
) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    if not output_dir.is_dir():
        raise ContractError("invalid_output_directory", "출력 디렉터리를 만들 수 없습니다.")
    if bundle.get("contractVersion") == SHARD_CONTRACT_VERSION:
        if input_dir is None:
            raise ContractError("invalid_shard_path", "shard worker 입력 폴더가 필요합니다.")
        job, shard_set, paths, summary = validate_shard_bundle(bundle, input_dir)
        artifact_name, artifact_format = safe_artifact_name(job)
        artifact_path = output_dir / artifact_name
        partial_path = output_dir / artifact_name.replace(".", ".partial.", 1)
        cache_dir = output_dir / f".{job['jobId']}.xgb-cache"
        try:
            probabilities = train_sharded_xgboost(job, paths, partial_path, cache_dir)
            os.replace(partial_path, artifact_path)
        finally:
            partial_path.unlink(missing_ok=True)
            shutil.rmtree(cache_dir, ignore_errors=True)
        artifact_bytes = artifact_path.read_bytes()
        if not artifact_bytes:
            raise ContractError("empty_artifact", "모델 아티팩트가 비어 있습니다.")
        predictions = quantize_predictions(summary["testSampleIds"], summary["testTargets"], probabilities)
        result = build_result(
            job, shard_set["asset"]["assetClass"], artifact_name,
            artifact_format, artifact_bytes, predictions, completed_at_ms,
        )
        result["datasetDiagnostics"] = {"logicalDatasetSha256": summary["logicalDatasetSha256"]}
        (output_dir / f"{job['jobId']}.result.json").write_text(canonical_json(result), encoding="utf-8")
        return result
    job, _, payload = validate_bundle(bundle)
    dataset = prepare_dataset(payload)
    artifact_name, artifact_format = safe_artifact_name(job)
    artifact_path = output_dir / artifact_name
    partial_name = artifact_name.replace(".", ".partial.", 1)
    partial_path = output_dir / partial_name
    try:
        probabilities = train_model(job, dataset, partial_path)
        os.replace(partial_path, artifact_path)
    finally:
        partial_path.unlink(missing_ok=True)
    artifact_bytes = artifact_path.read_bytes()
    if not artifact_bytes:
        raise ContractError("empty_artifact", "모델 아티팩트가 비어 있습니다.")
    predictions = quantize_predictions(dataset.test_sample_ids, dataset.test_y, probabilities)
    result = build_result(
        job, payload["asset"]["assetClass"], artifact_name, artifact_format,
        artifact_bytes, predictions, completed_at_ms,
    )
    (output_dir / f"{job['jobId']}.result.json").write_text(canonical_json(result), encoding="utf-8")
    return result


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Investa SHADOW ONLY ML worker")
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv or sys.argv[1:])
    try:
        input_path = args.input.resolve(strict=True)
        if input_path.stat().st_size > MAX_INPUT_BYTES:
            raise ContractError("input_too_large", "worker 입력이 64MiB를 초과했습니다.")
        bundle = require_dict(json.loads(input_path.read_text(encoding="utf-8")), "bundle")
        result = run(bundle, args.output_dir.resolve(), input_dir=input_path.parent)
        print(canonical_json(result))
        return 0
    except ContractError as error:
        print(canonical_json({"succeeded": False, "failureCode": error.code, "message": str(error)}), file=sys.stderr)
        return 2
    except (OSError, json.JSONDecodeError) as error:
        print(canonical_json({"succeeded": False, "failureCode": "worker_io_failure", "message": type(error).__name__}), file=sys.stderr)
        return 3
    except Exception as error:  # 민감한 입력·경로·스택을 외부 응답에 노출하지 않는다.
        print(canonical_json({"succeeded": False, "failureCode": "worker_internal_failure", "message": type(error).__name__}), file=sys.stderr)
        return 4


if __name__ == "__main__":
    raise SystemExit(main())
