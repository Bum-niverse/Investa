"""공식 공개 Binance 이력으로 Investa shard ML 경로를 재현 검증한다.

계좌·API Key·주문 권한을 사용하지 않으며 결과는 연구용 OOS 진단일 뿐이다.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import sys
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass
from decimal import Decimal, ROUND_HALF_UP
from pathlib import Path
from typing import Any


SPOT_ORIGIN = "https://api.binance.com"
FUTURES_ORIGIN = "https://fapi.binance.com"
ALLOWED_SYMBOLS = {"BTCUSDT", "ETHUSDT"}
ALLOWED_MARKETS = {"spot", "usdm"}
INTERVAL = "1h"
INTERVAL_MS = 3_600_000
FEATURE_SCALE = 100_000_000
HORIZON_BARS = 4
CLASS_THRESHOLD_BPS = 35
SHARD_COUNT = 3
MAX_RESPONSE_BYTES = 8 * 1024 * 1024
MAX_DAYS = 730
DEFAULT_SPOT_TAKER_FEE_BPS = Decimal("10")
DEFAULT_USDM_TAKER_FEE_BPS = Decimal("5")
DEFAULT_SLIPPAGE_BPS = Decimal("2")
COST_STRESS_MULTIPLIERS = (Decimal("1"), Decimal("1.5"), Decimal("2"))
TEMPERATURE_GRID_LOG_MIN = math.log(0.25)
TEMPERATURE_GRID_LOG_MAX = math.log(4.0)
TEMPERATURE_GRID_STEPS = 241
WORKER_PATH = Path(__file__).resolve().parents[1] / "ml-worker" / "investa_ml_worker.py"


@dataclass(frozen=True)
class IntervalSpec:
    interval: str
    interval_ms: int
    horizon_bars: int
    class_threshold_bps: int

    @property
    def horizon_ms(self) -> int:
        return self.interval_ms * self.horizon_bars


INTERVAL_SPECS = {
    "1h": IntervalSpec("1h", 3_600_000, 4, 35),
    "4h": IntervalSpec("4h", 14_400_000, 1, 35),
    "1d": IntervalSpec("1d", 86_400_000, 1, 100),
}


class ValidationError(ValueError):
    pass


@dataclass(frozen=True)
class Candle:
    open_time_ms: int
    close_time_ms: int
    open: Decimal
    high: Decimal
    low: Decimal
    close: Decimal
    volume: Decimal


def canonical_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def sha256_text(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def load_worker() -> Any:
    spec = importlib.util.spec_from_file_location("investa_real_validation_worker", WORKER_PATH)
    if spec is None or spec.loader is None:
        raise ValidationError("ML worker를 불러올 수 없습니다.")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class OfficialFetcher:
    def __init__(self) -> None:
        self.request_count = 0
        self.opener = urllib.request.build_opener(_RejectRedirects())

    def json(self, origin: str, path: str, params: dict[str, Any]) -> Any:
        if origin not in {SPOT_ORIGIN, FUTURES_ORIGIN} or not path.startswith("/"):
            raise ValidationError("공식 공급자 allowlist 밖 요청입니다.")
        url = f"{origin}{path}?{urllib.parse.urlencode(params)}"
        request = urllib.request.Request(url, headers={"Accept": "application/json", "User-Agent": "Investa-Real-ML-Validation/1.0"})
        try:
            with self.opener.open(request, timeout=20) as response:
                if response.status != 200:
                    raise ValidationError("공식 공급자가 성공 응답을 반환하지 않았습니다.")
                content_type = response.headers.get_content_type()
                if content_type not in {"application/json", "text/json"}:
                    raise ValidationError("공식 공급자 응답의 Content-Type이 JSON이 아닙니다.")
                length = response.headers.get("Content-Length")
                if length and int(length) > MAX_RESPONSE_BYTES:
                    raise ValidationError("공급자 응답이 크기 상한을 초과했습니다.")
                raw = response.read(MAX_RESPONSE_BYTES + 1)
        except OSError as error:
            raise ValidationError(f"공식 공급자 요청 실패: {type(error).__name__}") from error
        if len(raw) > MAX_RESPONSE_BYTES:
            raise ValidationError("공급자 응답이 크기 상한을 초과했습니다.")
        self.request_count += 1
        try:
            return json.loads(raw)
        except json.JSONDecodeError as error:
            raise ValidationError("공급자 JSON 응답이 올바르지 않습니다.") from error


class _RejectRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request: Any, file_pointer: Any, code: int, message: str, headers: Any, new_url: str) -> None:
        return None


def parse_candle(raw: Any, interval_ms: int = INTERVAL_MS) -> Candle:
    if not isinstance(raw, list) or len(raw) < 7:
        raise ValidationError("Binance kline 행 형식이 올바르지 않습니다.")
    try:
        candle = Candle(
            open_time_ms=int(raw[0]), close_time_ms=int(raw[6]) + 1,
            open=Decimal(str(raw[1])), high=Decimal(str(raw[2])),
            low=Decimal(str(raw[3])), close=Decimal(str(raw[4])),
            volume=Decimal(str(raw[5])),
        )
    except (ValueError, ArithmeticError) as error:
        raise ValidationError("Binance kline 숫자를 해석할 수 없습니다.") from error
    if (
        candle.open_time_ms <= 0 or candle.close_time_ms - candle.open_time_ms != interval_ms
        or min(candle.open, candle.high, candle.low, candle.close) <= 0
        or candle.volume < 0 or candle.low > min(candle.open, candle.close)
        or candle.high < max(candle.open, candle.close)
    ):
        raise ValidationError("Binance kline 가격·시간 범위가 올바르지 않습니다.")
    return candle


def fetch_klines(
    fetcher: OfficialFetcher, market: str, symbol: str, start_ms: int, end_ms: int,
    kind: str = "trade", interval_spec: IntervalSpec = INTERVAL_SPECS[INTERVAL],
) -> list[Candle]:
    if market == "spot" and kind == "trade":
        origin, path, symbol_key = SPOT_ORIGIN, "/api/v3/klines", "symbol"
    elif market == "usdm" and kind == "trade":
        origin, path, symbol_key = FUTURES_ORIGIN, "/fapi/v1/klines", "symbol"
    elif market == "usdm" and kind == "mark":
        origin, path, symbol_key = FUTURES_ORIGIN, "/fapi/v1/markPriceKlines", "symbol"
    elif market == "usdm" and kind == "index":
        origin, path, symbol_key = FUTURES_ORIGIN, "/fapi/v1/indexPriceKlines", "pair"
    else:
        raise ValidationError("지원하지 않는 시장·가격 기준입니다.")
    cursor = start_ms
    candles: list[Candle] = []
    while cursor < end_ms:
        payload = fetcher.json(origin, path, {symbol_key: symbol, "interval": interval_spec.interval, "startTime": cursor, "endTime": end_ms - 1, "limit": 1000})
        if not isinstance(payload, list):
            raise ValidationError("Binance kline 목록이 필요합니다.")
        page = [parse_candle(item, interval_spec.interval_ms) for item in payload]
        page = [item for item in page if start_ms <= item.open_time_ms and item.close_time_ms <= end_ms]
        if not page:
            break
        if any(right.open_time_ms - left.open_time_ms != interval_spec.interval_ms for left, right in zip(page, page[1:])):
            raise ValidationError("Binance kline 페이지 내부에 gap 또는 중복이 있습니다.")
        if candles and page[0].open_time_ms <= candles[-1].open_time_ms:
            raise ValidationError("Binance pagination이 역행하거나 중복됐습니다.")
        candles.extend(page)
        cursor = page[-1].open_time_ms + interval_spec.interval_ms
        if len(page) < 1000:
            break
        time.sleep(0.05)
    if len(candles) < 500:
        raise ValidationError("실제 검증에 필요한 완료 봉이 부족합니다.")
    if any(right.open_time_ms - left.open_time_ms != interval_spec.interval_ms for left, right in zip(candles, candles[1:])):
        raise ValidationError("수집 범위에 gap 또는 중복이 있습니다.")
    return candles


def fetch_funding(fetcher: OfficialFetcher, symbol: str, start_ms: int, end_ms: int) -> list[tuple[int, Decimal]]:
    cursor = start_ms
    rows: list[tuple[int, Decimal]] = []
    while cursor < end_ms:
        payload = fetcher.json(FUTURES_ORIGIN, "/fapi/v1/fundingRate", {"symbol": symbol, "startTime": cursor, "endTime": end_ms - 1, "limit": 1000})
        if not isinstance(payload, list):
            raise ValidationError("Binance funding 목록이 필요합니다.")
        page = [(int(item["fundingTime"]), Decimal(str(item["fundingRate"]))) for item in payload]
        if not page:
            break
        if rows and page[0][0] <= rows[-1][0]:
            raise ValidationError("funding pagination이 역행하거나 중복됐습니다.")
        rows.extend(page)
        cursor = page[-1][0] + 1
        if len(page) < 1000:
            break
        time.sleep(0.05)
    if not rows or any(right[0] <= left[0] for left, right in zip(rows, rows[1:])):
        raise ValidationError("funding 이력이 없거나 시간 순서가 올바르지 않습니다.")
    return rows


def scaled(value: Decimal) -> int:
    if not value.is_finite():
        raise ValidationError("유한하지 않은 파생 피처입니다.")
    result = int((value * FEATURE_SCALE).to_integral_value(rounding=ROUND_HALF_UP))
    if not -(2**63) <= result < 2**63:
        raise ValidationError("파생 피처 고정소수점 범위를 초과했습니다.")
    return result


def chunks(values: list[dict[str, Any]], count: int) -> list[list[dict[str, Any]]]:
    return [values[(len(values) * index) // count:(len(values) * (index + 1)) // count] for index in range(count)]


def build_samples(
    market: str, symbol: str, trade: list[Candle], mark: list[Candle] | None,
    index: list[Candle] | None, funding: list[tuple[int, Decimal]] | None,
    interval_spec: IntervalSpec = INTERVAL_SPECS[INTERVAL],
) -> tuple[list[dict[str, Any]], list[str], dict[str, Any]]:
    price = mark if market == "usdm" else trade
    if price is None or len(price) != len(trade):
        raise ValidationError("가격 기준 행 수가 일치하지 않습니다.")
    if market == "usdm":
        if index is None or funding is None or len(index) != len(trade):
            raise ValidationError("선물 mark·index·funding 이력이 필요합니다.")
        if any(a.open_time_ms != b.open_time_ms or a.open_time_ms != c.open_time_ms for a, b, c in zip(trade, price, index)):
            raise ValidationError("선물 trade·mark·index 시간이 일치하지 않습니다.")
    feature_ids = ["range_ratio", "realized_volatility_20", "return_1", "return_4", "sma_20_gap", "volume_ratio_20"]
    if market == "usdm":
        feature_ids += ["funding_rate", "mark_index_basis", "mark_trade_gap"]
    feature_ids.sort()
    returns = [Decimal(0)] + [price[i].close / price[i - 1].close - 1 for i in range(1, len(price))]
    rows: list[dict[str, Any]] = []
    funding_index = 0
    latest_funding = funding[0][1] if funding else Decimal(0)
    if funding and funding[0][0] > price[20].close_time_ms:
        raise ValidationError("첫 결정 시각 이전 funding 관측이 없습니다.")
    for position in range(20, len(price) - interval_spec.horizon_bars):
        decision = price[position].close_time_ms
        while funding and funding_index + 1 < len(funding) and funding[funding_index + 1][0] <= decision:
            funding_index += 1
            latest_funding = funding[funding_index][1]
        history = price[position - 19:position + 1]
        return_history = returns[position - 19:position + 1]
        mean_return = sum(return_history, Decimal(0)) / len(return_history)
        variance = sum((value - mean_return) ** 2 for value in return_history) / len(return_history)
        mean_close = sum((item.close for item in history), Decimal(0)) / len(history)
        mean_volume = sum((item.volume for item in trade[position - 19:position + 1]), Decimal(0)) / 20
        values = {
            "range_ratio": (trade[position].high - trade[position].low) / trade[position].close,
            "realized_volatility_20": variance.sqrt(),
            "return_1": returns[position],
            "return_4": price[position].close / price[position - 4].close - 1,
            "sma_20_gap": price[position].close / mean_close - 1,
            "volume_ratio_20": trade[position].volume / mean_volume - 1 if mean_volume else Decimal(0),
        }
        if market == "usdm":
            values.update({
                "funding_rate": latest_funding,
                "mark_index_basis": price[position].close / index[position].close - 1,
                "mark_trade_gap": price[position].close / trade[position].close - 1,
            })
        future_return = price[position + interval_spec.horizon_bars].close / price[position].close - 1
        threshold = Decimal(interval_spec.class_threshold_bps) / Decimal(10_000)
        target = 2 if future_return >= threshold else 0 if future_return <= -threshold else 1
        sample_id = f"{market}-{symbol}-{interval_spec.interval}-{decision}"
        rows.append({
            "sample": {"sampleId": sample_id, "decisionTimeMs": decision, "targetObservedAtMs": price[position + interval_spec.horizon_bars].close_time_ms, "targetClass": target},
            "features": [{
                "featureId": feature_id, "sampleId": sample_id,
                "sourceRecordId": f"binance-{market}-{symbol}-{interval_spec.interval}-{decision}-{feature_id}",
                "metadata": {"eventTimeMs": trade[position].open_time_ms, "availableAtMs": decision, "ingestedAtMs": int(time.time() * 1000)},
                "valueScaled": scaled(values[feature_id]), "valueScale": FEATURE_SCALE,
                "qualityFlags": [],
            } for feature_id in feature_ids],
        })
    asset = {
        "contractId": f"binance-{market}-{symbol.lower()}",
        "assetClass": "crypto_perpetual" if market == "usdm" else "crypto_spot",
        "exchange": "BINANCE", "symbol": symbol, "currency": "USDT", "timezone": "UTC",
        "adjustedPricePolicy": "not_applicable", "corporateActionPolicy": "not_applicable",
        "contractMultiplier": "1" if market == "usdm" else None,
        "expiryPolicy": "perpetual" if market == "usdm" else None,
        "rolloverPolicy": "not_applicable", "priceBasis": "mark" if market == "usdm" else "close",
        "fundingPolicy": "price_return_only" if market == "usdm" else None,
        "leveragePolicy": "research_only_no_leverage" if market == "usdm" else None,
    }
    return rows, feature_ids, asset


def build_bundle(
    worker: Any, market: str, symbol: str, rows: list[dict[str, Any]],
    feature_ids: list[str], asset: dict[str, Any], run_id: str,
    train_cut: int | None = None, validation_cut: int | None = None,
    test_end: int | None = None,
    interval_spec: IntervalSpec = INTERVAL_SPECS[INTERVAL],
) -> tuple[dict[str, Any], dict[str, int], list[tuple[dict[str, Any], str]]]:
    test_end = test_end or len(rows)
    working_rows = rows[:test_end]
    train_cut = train_cut or int(len(working_rows) * 0.60)
    validation_cut = validation_cut or int(len(working_rows) * 0.80)
    if not 0 < train_cut < validation_cut < len(working_rows):
        raise ValidationError("walk-forward split 경계가 올바르지 않습니다.")
    validation_start = working_rows[train_cut]["sample"]["decisionTimeMs"]
    test_start = working_rows[validation_cut]["sample"]["decisionTimeMs"]
    train_rows = [item for item in working_rows[:train_cut] if item["sample"]["targetObservedAtMs"] < validation_start]
    validation_rows = [item for item in working_rows[train_cut:validation_cut] if item["sample"]["targetObservedAtMs"] < test_start]
    test_rows = working_rows[validation_cut:]
    if min(map(len, (train_rows, validation_rows, test_rows))) < SHARD_COUNT * 3:
        raise ValidationError("시간순 split 또는 shard별 표본이 부족합니다.")
    split = {
        "trainEndMs": train_rows[-1]["sample"]["decisionTimeMs"],
        "validationStartMs": validation_rows[0]["sample"]["decisionTimeMs"],
        "validationEndMs": validation_rows[-1]["sample"]["decisionTimeMs"],
        "testStartMs": test_rows[0]["sample"]["decisionTimeMs"],
    }
    schema_hash = sha256_text(canonical_json(feature_ids))
    descriptors, files = [], []
    shard_root = []
    for shard_index, split_parts in enumerate(zip(chunks(train_rows, SHARD_COUNT), chunks(validation_rows, SHARD_COUNT), chunks(test_rows, SHARD_COUNT))):
        shard_rows = sorted([item for part in split_parts for item in part], key=lambda item: item["sample"]["decisionTimeMs"])
        dataset_id = f"real-{run_id}-{market}-{symbol.lower()}-shard-{shard_index}"
        samples = [item["sample"] for item in shard_rows]
        features = [feature for item in shard_rows for feature in item["features"]]
        for feature in features:
            feature["datasetVersion"] = dataset_id
        payload = {"datasetId": dataset_id, "asset": asset, "samples": samples, "features": features, "split": split, "expectedFeatureIds": feature_ids}
        payload_json = canonical_json(payload)
        content_hash = sha256_text(payload_json)
        manifest_id = f"manifest-{run_id}-{market}-{symbol.lower()}-{shard_index}"
        manifest = {
            "manifestId": manifest_id, "auditId": f"audit-{run_id}-{market}-{symbol.lower()}-{shard_index}",
            "datasetId": dataset_id, "asset": asset, "contentSha256": content_hash,
            "featureSchemaSha256": schema_hash, "sampleCount": len(samples), "featureCount": len(features),
            "firstDecisionTimeMs": samples[0]["decisionTimeMs"], "lastDecisionTimeMs": samples[-1]["decisionTimeMs"],
            "split": split, "createdAtMs": int(time.time() * 1000),
        }
        def extent(part: list[dict[str, Any]]) -> dict[str, int]:
            return {"sampleCount": len(part), "firstDecisionTimeMs": part[0]["sample"]["decisionTimeMs"], "lastDecisionTimeMs": part[-1]["sample"]["decisionTimeMs"]}
        descriptor = {
            "manifestId": manifest_id, "contentSha256": content_hash,
            "sampleCount": len(samples), "featureCount": len(features),
            "train": extent(split_parts[0]), "validation": extent(split_parts[1]), "test": extent(split_parts[2]),
        }
        descriptors.append(descriptor)
        files.append((manifest, payload_json))
        shard_root.append(payload_json)
    shard_set_id = f"shard-set-{run_id}-{market}-{symbol.lower()}"
    combined = [f"real-{run_id}-{market}-{symbol.lower()}", asset, split, schema_hash, descriptors]
    combined_hash = sha256_text(canonical_json(combined))
    shard_set = {
        "shardSetId": shard_set_id, "datasetId": combined[0], "asset": asset, "split": split,
        "featureSchemaSha256": schema_hash, "combinedContentSha256": combined_hash,
        "shardCount": SHARD_COUNT, "sampleCount": sum(item["sampleCount"] for item in descriptors),
        "featureCount": sum(item["featureCount"] for item in descriptors), "shards": descriptors,
        "createdAtMs": int(time.time() * 1000), "workerReady": True,
        "liveOrderAllowed": False,
    }
    job = {
        "jobId": f"real-job-{run_id}-{market}-{symbol.lower()}", "manifestId": shard_set_id,
        "datasetSourceKind": "shard_set", "algorithm": "xgboost",
        "contractVersion": worker.SHARD_CONTRACT_VERSION, "datasetContentSha256": combined_hash,
        "featureSchemaSha256": schema_hash, "inputSha256": "", "codeVersion": "real-validation-v1",
        "randomSeed": 42, "horizonMs": interval_spec.horizon_ms, "timeoutSeconds": 1800,
        "memoryLimitMb": 4096, "maxThreads": 4,
        "hyperparameters": {"early_stopping_rounds": 20, "num_boost_round": 300, "eta": 0.05, "max_depth": 6, "subsample": 0.8, "colsample_bytree": 0.8},
        "status": "prepared", "createdAtMs": int(time.time() * 1000), "updatedAtMs": int(time.time() * 1000),
        "liveOrderAllowed": False,
    }
    job["inputSha256"] = sha256_text(canonical_json(worker.canonical_job_for_hash(job)))
    bundle = {"contractVersion": worker.SHARD_CONTRACT_VERSION, "job": job, "shardSet": shard_set, "datasetShards": [], "liveOrderAllowed": False}
    return bundle, {"train": len(train_rows), "validation": len(validation_rows), "test": len(test_rows)}, files


def write_shards(bundle: dict[str, Any], files: list[tuple[dict[str, Any], str]], input_root: Path) -> None:
    shard_dir = input_root / "shards"
    shard_dir.mkdir(parents=True, exist_ok=False)
    for index, (manifest, payload_json) in enumerate(files):
        file_name = f"shard-{index:04d}.json"
        path = shard_dir / file_name
        path.write_text(payload_json, encoding="utf-8")
        bundle["datasetShards"].append({"manifest": manifest, "fileName": file_name, "byteSize": path.stat().st_size})


def build_direct_bundle(
    worker: Any, algorithm: str, market: str, symbol: str, rows: list[dict[str, Any]],
    feature_ids: list[str], asset: dict[str, Any], run_id: str, split: dict[str, int],
    test_end_ms: int, interval_spec: IntervalSpec = INTERVAL_SPECS[INTERVAL],
) -> dict[str, Any]:
    if algorithm != "lightgbm":
        raise ValidationError("실제 비교의 단일 manifest 경로는 LightGBM만 허용합니다.")
    selected_rows = [
        item for item in rows
        if (
            item["sample"]["decisionTimeMs"] <= split["trainEndMs"]
            or split["validationStartMs"] <= item["sample"]["decisionTimeMs"] <= split["validationEndMs"]
            or split["testStartMs"] <= item["sample"]["decisionTimeMs"] <= test_end_ms
        )
    ]
    dataset_id = f"real-{run_id}-{algorithm}-{market}-{symbol.lower()}"
    payload = {
        "datasetId": dataset_id,
        "asset": asset,
        "samples": [item["sample"] for item in selected_rows],
        "features": [
            {**feature, "datasetVersion": dataset_id}
            for item in selected_rows for feature in item["features"]
        ],
        "split": split,
        "expectedFeatureIds": feature_ids,
    }
    payload_json = canonical_json(payload)
    if len(payload_json.encode("utf-8")) > worker.MAX_INPUT_BYTES:
        raise ValidationError("LightGBM 단일 manifest 비교가 worker 입력 상한을 초과했습니다.")
    content_hash = sha256_text(payload_json)
    schema_hash = sha256_text(canonical_json(feature_ids))
    manifest_id = f"manifest-{run_id}-{algorithm}-{market}-{symbol.lower()}"
    job = {
        "jobId": f"real-job-{run_id}-{algorithm}-{market}-{symbol.lower()}",
        "manifestId": manifest_id,
        "datasetSourceKind": "manifest",
        "algorithm": algorithm,
        "contractVersion": worker.CONTRACT_VERSION,
        "datasetContentSha256": content_hash,
        "featureSchemaSha256": schema_hash,
        "inputSha256": "",
        "codeVersion": "real-model-comparison-v1",
        "randomSeed": 42,
        "horizonMs": interval_spec.horizon_ms,
        "timeoutSeconds": 1800,
        "memoryLimitMb": 4096,
        "maxThreads": 4,
        "hyperparameters": {
            "early_stopping_rounds": 20,
            "num_boost_round": 300,
            "learning_rate": 0.05,
            "max_depth": 6,
            "num_leaves": 31,
            "min_data_in_leaf": 20,
            "feature_fraction": 0.8,
            "bagging_fraction": 0.8,
        },
        "status": "prepared",
        "createdAtMs": int(time.time() * 1000),
        "updatedAtMs": int(time.time() * 1000),
        "liveOrderAllowed": False,
    }
    job["inputSha256"] = sha256_text(canonical_json(worker.canonical_job_for_hash(job)))
    return {
        "contractVersion": worker.CONTRACT_VERSION,
        "job": job,
        "manifest": {
            "manifestId": manifest_id,
            "contentSha256": content_hash,
            "featureSchemaSha256": schema_hash,
        },
        "datasetPayloadJson": payload_json,
        "liveOrderAllowed": False,
    }


def class_counts(
    rows: list[dict[str, Any]], start_ms: int = 0, end_ms: int | None = None,
    target_before_ms: int | None = None,
) -> list[int]:
    counts = [0, 0, 0]
    for item in rows:
        decision = item["sample"]["decisionTimeMs"]
        target_observed = item["sample"]["targetObservedAtMs"]
        if (
            decision >= start_ms
            and (end_ms is None or decision <= end_ms)
            and (target_before_ms is None or target_observed < target_before_ms)
        ):
            counts[item["sample"]["targetClass"]] += 1
    return counts


def fit_momentum_state_baseline(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if len(rows) < 30:
        raise ValidationError("단순 모멘텀 기준선을 만들 학습 표본이 부족합니다.")
    threshold = _nearest_rank(
        [abs(_feature_value_scaled(row, "return_4")) for row in rows], 500,
    )
    counts = {"down": [1, 1, 1], "flat": [1, 1, 1], "up": [1, 1, 1]}
    for row in rows:
        value = _feature_value_scaled(row, "return_4")
        state = "up" if value > threshold else "down" if value < -threshold else "flat"
        counts[state][int(row["sample"]["targetClass"])] += 1
    return {
        "contractVersion": "train-momentum-state-prior-v1",
        "absoluteReturn4MedianScaled": threshold,
        "featureScale": FEATURE_SCALE,
        "stateClassCountsWithLaplace": counts,
        "fitSampleCount": len(rows),
        "fitUsesTrainingRowsOnly": True,
        "liveOrderAllowed": False,
    }


def predict_momentum_state_baseline(
    worker: Any, rows: list[dict[str, Any]], baseline: dict[str, Any], fold_index: int,
) -> list[dict[str, Any]]:
    if (
        baseline.get("contractVersion") != "train-momentum-state-prior-v1"
        or baseline.get("fitUsesTrainingRowsOnly") is not True
    ):
        raise ValidationError("검증되지 않은 단순 모멘텀 기준선입니다.")
    threshold = int(baseline["absoluteReturn4MedianScaled"])
    counts = baseline["stateClassCountsWithLaplace"]
    probabilities = []
    for row in rows:
        value = _feature_value_scaled(row, "return_4")
        state = "up" if value > threshold else "down" if value < -threshold else "flat"
        state_counts = counts[state]
        total = sum(state_counts)
        probabilities.append([count / total for count in state_counts])
    predictions = worker.quantize_predictions(
        [item["sample"]["sampleId"] for item in rows],
        [item["sample"]["targetClass"] for item in rows],
        probabilities,
    )
    for prediction in predictions:
        prediction["foldIndex"] = fold_index
    return predictions


def _prediction_probabilities(prediction: dict[str, Any]) -> list[float]:
    probabilities = [
        int(prediction["probabilityDownMillionths"]),
        int(prediction["probabilityFlatMillionths"]),
        int(prediction["probabilityUpMillionths"]),
    ]
    if min(probabilities) <= 0 or sum(probabilities) != 1_000_000:
        raise ValidationError("온도 보정 입력 확률 계약이 올바르지 않습니다.")
    return [value / 1_000_000 for value in probabilities]


def _temperature_scaled(probabilities: list[float], temperature: float) -> list[float]:
    if len(probabilities) != 3 or not math.isfinite(temperature) or temperature <= 0:
        raise ValidationError("온도 보정 입력이 올바르지 않습니다.")
    logits = [math.log(max(value, 1e-12)) / temperature for value in probabilities]
    maximum = max(logits)
    exponentials = [math.exp(value - maximum) for value in logits]
    total = sum(exponentials)
    return [value / total for value in exponentials]


def _multiclass_log_loss(predictions: list[dict[str, Any]], temperature: float) -> float:
    if not predictions:
        raise ValidationError("온도 보정에 사용할 과거 OOS 예측이 없습니다.")
    loss = 0.0
    for prediction in predictions:
        target = int(prediction["targetClass"])
        if target not in range(3):
            raise ValidationError("온도 보정 정답 클래스가 올바르지 않습니다.")
        probabilities = _temperature_scaled(_prediction_probabilities(prediction), temperature)
        loss -= math.log(max(probabilities[target], 1e-12))
    return loss / len(predictions)


def fit_oos_temperature(
    predictions: list[dict[str, Any]], next_fold_index: int,
) -> dict[str, Any]:
    """현재 fold보다 앞선 OOS 확률만으로 단일 다중분류 temperature를 맞춘다."""
    if next_fold_index <= 0 or not predictions:
        raise ValidationError("온도 보정에는 한 개 이상의 과거 OOS fold가 필요합니다.")
    source_folds = sorted({int(item["foldIndex"]) for item in predictions})
    if source_folds != list(range(next_fold_index)):
        raise ValidationError("온도 보정 source fold가 과거 OOS 연속 구간이 아닙니다.")
    best_temperature = 1.0
    best_loss = _multiclass_log_loss(predictions, best_temperature)
    for index in range(TEMPERATURE_GRID_STEPS):
        ratio = index / (TEMPERATURE_GRID_STEPS - 1)
        temperature = math.exp(
            TEMPERATURE_GRID_LOG_MIN
            + (TEMPERATURE_GRID_LOG_MAX - TEMPERATURE_GRID_LOG_MIN) * ratio
        )
        loss = _multiclass_log_loss(predictions, temperature)
        if loss < best_loss - 1e-15 or (
            abs(loss - best_loss) <= 1e-15
            and abs(math.log(temperature)) < abs(math.log(best_temperature))
        ):
            best_temperature = temperature
            best_loss = loss
    return {
        "contractVersion": "rolling-oos-temperature-v1",
        "temperatureMillionths": round(best_temperature * 1_000_000),
        "sourceFoldIndexes": source_folds,
        "sourceSampleCount": len(predictions),
        "sourceLogLossBeforeMillionths": round(_multiclass_log_loss(predictions, 1.0) * 1_000_000),
        "sourceLogLossAfterMillionths": round(best_loss * 1_000_000),
        "fitUsesPriorOosOnly": True,
        "liveOrderAllowed": False,
    }


def apply_oos_temperature(
    worker: Any, predictions: list[dict[str, Any]], calibration: dict[str, Any],
    fold_index: int,
) -> list[dict[str, Any]]:
    if (
        calibration.get("contractVersion") != "rolling-oos-temperature-v1"
        or calibration.get("fitUsesPriorOosOnly") is not True
        or any(int(value) >= fold_index for value in calibration.get("sourceFoldIndexes", []))
    ):
        raise ValidationError("현재 fold보다 앞선 OOS 보정만 적용할 수 있습니다.")
    temperature = int(calibration["temperatureMillionths"]) / 1_000_000
    probabilities = [
        _temperature_scaled(_prediction_probabilities(prediction), temperature)
        for prediction in predictions
    ]
    calibrated = worker.quantize_predictions(
        [str(item["sampleId"]) for item in predictions],
        [int(item["targetClass"]) for item in predictions],
        probabilities,
    )
    for prediction in calibrated:
        prediction["foldIndex"] = fold_index
    return calibrated


def _reindex_folds(predictions: list[dict[str, Any]]) -> list[dict[str, Any]]:
    folds = sorted({int(item["foldIndex"]) for item in predictions})
    fold_map = {fold: index for index, fold in enumerate(folds)}
    return [{**item, "foldIndex": fold_map[int(item["foldIndex"])]} for item in predictions]


def _decimal_bps(value: Decimal) -> int:
    return int((value * Decimal(10_000)).to_integral_value(rounding=ROUND_HALF_UP))


def _feature_value_scaled(row: dict[str, Any], feature_id: str) -> int:
    matches = [feature for feature in row.get("features", []) if feature.get("featureId") == feature_id]
    if len(matches) != 1 or int(matches[0].get("valueScale", 0)) != FEATURE_SCALE:
        raise ValidationError(f"레짐 피처 {feature_id} 계약이 올바르지 않습니다.")
    return int(matches[0]["valueScaled"])


def _nearest_rank(values: list[int], quantile_milli: int) -> int:
    if not values or not 0 <= quantile_milli <= 1_000:
        raise ValidationError("레짐 분위수 입력이 올바르지 않습니다.")
    ordered = sorted(values)
    rank = max(1, math.ceil(len(ordered) * quantile_milli / 1_000))
    return ordered[rank - 1]


def fit_regime_thresholds(rows: list[dict[str, Any]]) -> dict[str, Any]:
    if len(rows) < 30:
        raise ValidationError("레짐 기준을 산출할 과거 학습 표본이 부족합니다.")
    trend_values = [abs(_feature_value_scaled(row, "return_4")) for row in rows]
    volatility_values = [_feature_value_scaled(row, "realized_volatility_20") for row in rows]
    return {
        "contractVersion": "observed-regime-v1",
        "fitSampleCount": len(rows),
        "trendAbsoluteMedianScaled": _nearest_rank(trend_values, 500),
        "highVolatilityP75Scaled": _nearest_rank(volatility_values, 750),
        "featureScale": FEATURE_SCALE,
        "fitUsesTrainingRowsOnly": True,
        "liveOrderAllowed": False,
    }


def classify_regime(row: dict[str, Any], thresholds: dict[str, Any]) -> str:
    if thresholds.get("contractVersion") != "observed-regime-v1" or not thresholds.get("fitUsesTrainingRowsOnly"):
        raise ValidationError("검증되지 않은 레짐 기준입니다.")
    trend = _feature_value_scaled(row, "return_4")
    volatility = _feature_value_scaled(row, "realized_volatility_20")
    trend_threshold = int(thresholds["trendAbsoluteMedianScaled"])
    volatility_threshold = int(thresholds["highVolatilityP75Scaled"])
    direction = "bull" if trend > trend_threshold else "bear" if trend < -trend_threshold else "range"
    volatility_state = "high_vol" if volatility >= volatility_threshold else "normal_vol"
    return f"{direction}_{volatility_state}"


def summarize_regime_returns(records: dict[str, list[tuple[Decimal, Decimal]]]) -> list[dict[str, Any]]:
    diagnostics = []
    for regime, returns in sorted(records.items()):
        gross_equity = net_equity = peak = Decimal(1)
        maximum_drawdown = Decimal(0)
        winning_count = 0
        for gross_return, net_return in returns:
            gross_equity *= 1 + gross_return
            net_equity *= 1 + net_return
            peak = max(peak, net_equity)
            maximum_drawdown = max(maximum_drawdown, (peak - net_equity) / peak)
            winning_count += int(net_return > 0)
        diagnostics.append({
            "regime": regime,
            "tradeCount": len(returns),
            "winningTradeCount": winning_count,
            "grossReturnBps": _decimal_bps(gross_equity - 1),
            "netReturnBps": _decimal_bps(net_equity - 1),
            "maximumDrawdownBps": _decimal_bps(maximum_drawdown),
            "winRateBps": round(winning_count / len(returns) * 10_000),
            "descriptiveOnly": True,
            "liveOrderAllowed": False,
        })
    return diagnostics


def evaluate_oos_strategy(
    predictions: list[dict[str, Any]], rows: list[dict[str, Any]], price: list[Candle],
    funding: list[tuple[int, Decimal]] | None, market: str, fee_bps: Decimal,
    slippage_bps: Decimal, cost_multiplier: Decimal,
    horizon_bars: int = HORIZON_BARS,
    regime_by_sample: dict[str, str] | None = None,
) -> dict[str, Any]:
    if market not in ALLOWED_MARKETS or fee_bps < 0 or slippage_bps < 0 or cost_multiplier <= 0 or horizon_bars <= 0:
        raise ValidationError("OOS 전략 비용 입력이 올바르지 않습니다.")
    row_by_sample = {item["sample"]["sampleId"]: item for item in rows}
    position_by_decision = {candle.close_time_ms: index for index, candle in enumerate(price)}
    prediction_ids = [str(item["sampleId"]) for item in predictions]
    if len(set(prediction_ids)) != len(prediction_ids) or any(sample_id not in row_by_sample for sample_id in prediction_ids):
        raise ValidationError("OOS 예측 sampleId가 중복되거나 원본에 없습니다.")
    ordered = sorted(
        predictions,
        key=lambda item: int(row_by_sample[str(item["sampleId"])]["sample"]["decisionTimeMs"]),
    )
    seen: set[str] = set()
    next_eligible_decision = 0
    gross_equity = Decimal(1)
    net_equity = Decimal(1)
    peak_equity = Decimal(1)
    maximum_drawdown = Decimal(0)
    total_fee_slippage = Decimal(0)
    total_funding_effect = Decimal(0)
    trade_count = long_count = short_count = winning_count = skipped_overlap = 0
    first_entry_ms = last_exit_ms = None
    held_duration_ms = 0
    regime_returns: dict[str, list[tuple[Decimal, Decimal]]] = {}
    per_side_cost = (fee_bps + slippage_bps) * cost_multiplier / Decimal(10_000)
    for prediction in ordered:
        sample_id = str(prediction["sampleId"])
        if sample_id in seen:
            raise ValidationError("OOS 예측 sampleId가 중복되거나 원본에 없습니다.")
        seen.add(sample_id)
        row = row_by_sample[sample_id]["sample"]
        if int(prediction["targetClass"]) != int(row["targetClass"]):
            raise ValidationError("OOS 예측 정답과 PIT 원본이 일치하지 않습니다.")
        probabilities = [
            int(prediction["probabilityDownMillionths"]),
            int(prediction["probabilityFlatMillionths"]),
            int(prediction["probabilityUpMillionths"]),
        ]
        if min(probabilities) <= 0 or sum(probabilities) != 1_000_000:
            raise ValidationError("OOS 예측 확률 계약이 올바르지 않습니다.")
        predicted_class = max(range(3), key=lambda index: (probabilities[index], -index))
        direction = 1 if predicted_class == 2 else -1 if predicted_class == 0 and market == "usdm" else 0
        if direction == 0:
            continue
        decision = int(row["decisionTimeMs"])
        if decision < next_eligible_decision:
            skipped_overlap += 1
            continue
        position = position_by_decision.get(decision)
        if position is None or position + horizon_bars >= len(price):
            raise ValidationError("OOS 거래의 진입·청산 가격을 찾을 수 없습니다.")
        entry = price[position + 1]
        exit_candle = price[position + horizon_bars]
        if entry.open_time_ms != decision or exit_candle.close_time_ms != int(row["targetObservedAtMs"]):
            raise ValidationError("OOS 거래 시각과 PIT target 관측 시각이 일치하지 않습니다.")
        gross_return = Decimal(direction) * (exit_candle.close / entry.open - 1)
        funding_effect = Decimal(0)
        if market == "usdm":
            if funding is None:
                raise ValidationError("선물 OOS 전략에는 funding 이력이 필요합니다.")
            paid_rates = sum(
                (rate for observed_at, rate in funding if entry.open_time_ms < observed_at <= exit_candle.close_time_ms),
                Decimal(0),
            )
            funding_effect = -Decimal(direction) * paid_rates
        fee_slippage = per_side_cost * 2
        net_return = gross_return - fee_slippage + funding_effect
        if net_return <= -1:
            raise ValidationError("OOS 거래 손실이 자본 전체를 초과했습니다.")
        gross_equity *= 1 + gross_return
        net_equity *= 1 + net_return
        peak_equity = max(peak_equity, net_equity)
        maximum_drawdown = max(maximum_drawdown, (peak_equity - net_equity) / peak_equity)
        total_fee_slippage += fee_slippage
        total_funding_effect += funding_effect
        if regime_by_sample is not None:
            regime = regime_by_sample.get(sample_id)
            if regime is None:
                raise ValidationError("OOS 거래 표본의 레짐 분류가 없습니다.")
            regime_returns.setdefault(regime, []).append((gross_return, net_return))
        trade_count += 1
        long_count += int(direction == 1)
        short_count += int(direction == -1)
        winning_count += int(net_return > 0)
        first_entry_ms = entry.open_time_ms if first_entry_ms is None else first_entry_ms
        last_exit_ms = exit_candle.close_time_ms
        held_duration_ms += exit_candle.close_time_ms - entry.open_time_ms
        next_eligible_decision = exit_candle.close_time_ms
    elapsed_ms = (last_exit_ms - first_entry_ms) if first_entry_ms is not None and last_exit_ms is not None else 0
    return {
        "costMultiplierMilli": int(cost_multiplier * 1_000),
        "feeBpsPerSideMilli": int(fee_bps * 1_000),
        "slippageBpsPerSideMilli": int(slippage_bps * 1_000),
        "tradeCount": trade_count, "longCount": long_count, "shortCount": short_count,
        "winningTradeCount": winning_count, "skippedOverlappingSignalCount": skipped_overlap,
        "grossReturnBps": _decimal_bps(gross_equity - 1),
        "netReturnBps": _decimal_bps(net_equity - 1),
        "maximumDrawdownBps": _decimal_bps(maximum_drawdown),
        "winRateBps": round(winning_count / trade_count * 10_000) if trade_count else 0,
        "feeAndSlippageBpsTotal": _decimal_bps(total_fee_slippage),
        "fundingEffectBpsTotal": _decimal_bps(total_funding_effect),
        "timeInMarketBps": min(10_000, round(held_duration_ms / elapsed_ms * 10_000)) if elapsed_ms else 0,
        "firstEntryTimeMs": first_entry_ms, "lastExitTimeMs": last_exit_ms,
        "regimeDiagnostics": summarize_regime_returns(regime_returns),
        "regimeDiagnosticsDescriptiveOnly": True,
        "nonOverlappingPositions": True, "candidateReviewOnly": True, "liveOrderAllowed": False,
    }


def evaluate_same_period_long_baseline(
    price: list[Candle], funding: list[tuple[int, Decimal]] | None, market: str,
    first_entry_ms: int | None, last_exit_ms: int | None, fee_bps: Decimal,
    slippage_bps: Decimal, cost_multiplier: Decimal,
) -> dict[str, Any]:
    if first_entry_ms is None or last_exit_ms is None:
        return {"available": False, "reason": "strategy_has_no_trade", "liveOrderAllowed": False}
    entry = next((candle for candle in price if candle.open_time_ms == first_entry_ms), None)
    exit_candle = next((candle for candle in price if candle.close_time_ms == last_exit_ms), None)
    if entry is None or exit_candle is None or entry.open_time_ms >= exit_candle.close_time_ms:
        raise ValidationError("동일 기간 장기보유 기준선 가격을 찾을 수 없습니다.")
    gross_return = exit_candle.close / entry.open - 1
    fee_slippage = (fee_bps + slippage_bps) * cost_multiplier * 2 / Decimal(10_000)
    funding_effect = Decimal(0)
    if market == "usdm":
        if funding is None:
            raise ValidationError("선물 장기보유 기준선에는 funding 이력이 필요합니다.")
        funding_effect = -sum(
            (rate for observed_at, rate in funding if entry.open_time_ms < observed_at <= exit_candle.close_time_ms),
            Decimal(0),
        )
    return {
        "available": True, "grossReturnBps": _decimal_bps(gross_return),
        "netReturnBps": _decimal_bps(gross_return - fee_slippage + funding_effect),
        "feeAndSlippageBpsTotal": _decimal_bps(fee_slippage),
        "fundingEffectBpsTotal": _decimal_bps(funding_effect),
        "firstEntryTimeMs": first_entry_ms, "lastExitTimeMs": last_exit_ms,
        "exposureMatched": False, "performanceGate": False, "liveOrderAllowed": False,
    }


def walk_forward_boundaries(row_count: int) -> list[tuple[int, int, int]]:
    boundaries = []
    for fold_index in range(4):
        train_cut = int(row_count * (0.50 + 0.10 * fold_index))
        validation_cut = int(row_count * (0.60 + 0.10 * fold_index))
        test_end = int(row_count * (0.70 + 0.10 * fold_index)) if fold_index < 3 else row_count
        if not 0 < train_cut < validation_cut < test_end <= row_count:
            raise ValidationError("walk-forward fold 경계가 올바르지 않습니다.")
        if boundaries and boundaries[-1][2] != validation_cut:
            raise ValidationError("walk-forward OOS 구간이 연속적이지 않습니다.")
        boundaries.append((train_cut, validation_cut, test_end))
    return boundaries


def parse_intervals(value: str) -> tuple[str, ...]:
    intervals = tuple(item.strip() for item in value.split(",") if item.strip())
    if not intervals or len(set(intervals)) != len(intervals):
        raise ValidationError("intervals는 중복 없는 한 개 이상의 주기여야 합니다.")
    unsupported = [item for item in intervals if item not in INTERVAL_SPECS]
    if unsupported:
        raise ValidationError(f"지원하지 않는 검증 주기입니다: {','.join(unsupported)}")
    return intervals


def run_validation(
    days: int, output_root: Path, intervals: tuple[str, ...] = (INTERVAL,),
    compare_models: bool = False,
) -> dict[str, Any]:
    if not 30 <= days <= MAX_DAYS:
        raise ValidationError(f"days는 30~{MAX_DAYS} 범위여야 합니다.")
    if not intervals or any(interval not in INTERVAL_SPECS for interval in intervals) or len(set(intervals)) != len(intervals):
        raise ValidationError("검증 주기 계약이 올바르지 않습니다.")
    worker = load_worker()
    fetcher = OfficialFetcher()
    run_id = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
    output_root.mkdir(parents=True, exist_ok=False)
    results = []
    observed_starts: list[int] = []
    observed_ends: list[int] = []
    for interval in intervals:
        interval_spec = INTERVAL_SPECS[interval]
        end_ms = (int(time.time() * 1000) // interval_spec.interval_ms) * interval_spec.interval_ms
        start_ms = end_ms - days * 86_400_000
        observed_starts.append(start_ms)
        observed_ends.append(end_ms)
        for market in ("spot", "usdm"):
            for symbol in sorted(ALLOWED_SYMBOLS):
                trade = fetch_klines(fetcher, market, symbol, start_ms, end_ms, "trade", interval_spec)
                mark = index = funding = None
                if market == "usdm":
                    mark = fetch_klines(fetcher, market, symbol, start_ms, end_ms, "mark", interval_spec)
                    index = fetch_klines(fetcher, market, symbol, start_ms, end_ms, "index", interval_spec)
                    funding = fetch_funding(fetcher, symbol, start_ms - 86_400_000, end_ms)
                rows, feature_ids, asset = build_samples(market, symbol, trade, mark, index, funding, interval_spec)
                fold_records = []
                all_predictions = []
                all_baseline_predictions = []
                all_lightgbm_predictions = []
                all_momentum_baseline_predictions = []
                calibrated_predictions = []
                calibration_comparable_raw_predictions = []
                observed_oos_sample_ids: set[str] = set()
                regime_by_sample: dict[str, str] = {}
                completed_at_ms = 0
                for fold_index, (train_cut, validation_cut, test_end) in enumerate(walk_forward_boundaries(len(rows))):
                    fold_id = f"{run_id}-{interval}-wf{fold_index}"
                    input_root = output_root / f"input-{interval}-{market}-{symbol.lower()}-wf{fold_index}"
                    model_root = output_root / f"model-{interval}-{market}-{symbol.lower()}-wf{fold_index}"
                    input_root.mkdir()
                    bundle, split_counts, files = build_bundle(
                        worker, market, symbol, rows, feature_ids, asset, fold_id,
                        train_cut=train_cut, validation_cut=validation_cut, test_end=test_end,
                        interval_spec=interval_spec,
                    )
                    write_shards(bundle, files, input_root)
                    result = worker.run(bundle, model_root, input_dir=input_root)
                    completed_at_ms = max(completed_at_ms, result["completedAtMs"])
                    split = bundle["shardSet"]["split"]
                    training_rows = [
                        item for item in rows[:test_end]
                        if item["sample"]["decisionTimeMs"] <= split["trainEndMs"]
                        and item["sample"]["targetObservedAtMs"] < split["validationStartMs"]
                    ]
                    regime_thresholds = fit_regime_thresholds(training_rows)
                    train_counts = class_counts(
                        rows, end_ms=split["trainEndMs"],
                        target_before_ms=split["validationStartMs"],
                    )
                    fold_test_rows = [
                        item for item in rows[:test_end]
                        if item["sample"]["decisionTimeMs"] >= split["testStartMs"]
                    ]
                    test_counts = class_counts(fold_test_rows)
                    prior = [(count + 1) / (sum(train_counts) + 3) for count in train_counts]
                    baseline_predictions = worker.quantize_predictions(
                        [item["sample"]["sampleId"] for item in fold_test_rows],
                        [item["sample"]["targetClass"] for item in fold_test_rows],
                        [prior for _ in fold_test_rows],
                    )
                    momentum_baseline = fit_momentum_state_baseline(training_rows)
                    momentum_predictions = predict_momentum_state_baseline(
                        worker, fold_test_rows, momentum_baseline, fold_index,
                    )
                    lightgbm_result = None
                    if compare_models:
                        lightgbm_root = output_root / f"model-{interval}-{market}-{symbol.lower()}-wf{fold_index}-lightgbm"
                        direct_bundle = build_direct_bundle(
                            worker, "lightgbm", market, symbol, rows, feature_ids, asset,
                            fold_id, split, fold_test_rows[-1]["sample"]["decisionTimeMs"],
                            interval_spec,
                        )
                        lightgbm_result = worker.run(direct_bundle, lightgbm_root)
                        if lightgbm_result["metrics"]["sampleCount"] != len(fold_test_rows):
                            raise ValidationError("LightGBM 예측 수와 OOS 표본 수가 일치하지 않습니다.")
                        expected_ids = [item["sample"]["sampleId"] for item in fold_test_rows]
                        actual_ids = [item["sampleId"] for item in lightgbm_result["predictions"]]
                        if actual_ids != expected_ids:
                            raise ValidationError("LightGBM과 XGBoost의 OOS 표본 순서가 다릅니다.")
                        for prediction in lightgbm_result["predictions"]:
                            prediction["foldIndex"] = fold_index
                        all_lightgbm_predictions.extend(lightgbm_result["predictions"])
                    fold_sample_ids = {item["sample"]["sampleId"] for item in fold_test_rows}
                    if len(fold_sample_ids) != len(fold_test_rows):
                        raise ValidationError("walk-forward fold 내부 OOS sampleId가 중복됩니다.")
                    if observed_oos_sample_ids.intersection(fold_sample_ids):
                        raise ValidationError("walk-forward fold 사이 OOS 표본이 겹칩니다.")
                    if result["metrics"]["sampleCount"] != len(fold_test_rows):
                        raise ValidationError("worker 예측 수와 OOS 표본 수가 일치하지 않습니다.")
                    observed_oos_sample_ids.update(fold_sample_ids)
                    for item in fold_test_rows:
                        sample_id = item["sample"]["sampleId"]
                        regime_by_sample[sample_id] = classify_regime(item, regime_thresholds)
                    for prediction in result["predictions"]:
                        prediction["foldIndex"] = fold_index
                    for prediction in baseline_predictions:
                        prediction["foldIndex"] = fold_index
                    calibration = None
                    if all_predictions:
                        calibration = fit_oos_temperature(all_predictions, fold_index)
                        calibrated_fold = apply_oos_temperature(
                            worker, result["predictions"], calibration, fold_index,
                        )
                        calibrated_predictions.extend(calibrated_fold)
                        calibration_comparable_raw_predictions.extend(
                            [{**item} for item in result["predictions"]]
                        )
                        calibration["currentFoldRawMetrics"] = worker.compute_metrics(
                            [{**item, "foldIndex": 0} for item in result["predictions"]],
                            result["completedAtMs"],
                        )
                        calibration["currentFoldCalibratedMetrics"] = worker.compute_metrics(
                            [{**item, "foldIndex": 0} for item in calibrated_fold],
                            result["completedAtMs"],
                        )
                    all_predictions.extend(result["predictions"])
                    all_baseline_predictions.extend(baseline_predictions)
                    all_momentum_baseline_predictions.extend(momentum_predictions)
                    fold_records.append({
                        "foldIndex": fold_index, "splitCounts": split_counts,
                        "split": split,
                        "testEndMs": fold_test_rows[-1]["sample"]["decisionTimeMs"],
                        "trainClassCounts": train_counts, "testClassCounts": test_counts,
                        "regimeThresholds": regime_thresholds,
                        "combinedContentSha256": bundle["shardSet"]["combinedContentSha256"],
                        "inputSha256": bundle["job"]["inputSha256"],
                        "artifact": result["artifact"], "metrics": result["metrics"],
                        "datasetDiagnostics": result["datasetDiagnostics"],
                        "rollingOosCalibration": calibration or {
                            "available": False,
                            "reason": "no_prior_oos_fold",
                            "fitUsesPriorOosOnly": True,
                            "liveOrderAllowed": False,
                        },
                        "modelComparison": {
                            "enabled": compare_models,
                            "sameOosSampleIds": True,
                            "momentumStateBaseline": momentum_baseline,
                            "momentumStateMetrics": worker.compute_metrics(
                                [{**item, "foldIndex": 0} for item in momentum_predictions],
                                result["completedAtMs"],
                            ),
                            "lightgbm": ({
                                "artifact": lightgbm_result["artifact"],
                                "metrics": lightgbm_result["metrics"],
                            } if lightgbm_result else None),
                            "liveOrderAllowed": False,
                        },
                    })
                aggregate_metrics = worker.compute_metrics(all_predictions, completed_at_ms)
                baseline_metrics = worker.compute_metrics(all_baseline_predictions, completed_at_ms)
                momentum_baseline_metrics = worker.compute_metrics(
                    all_momentum_baseline_predictions, completed_at_ms,
                )
                model_comparison = {
                    "contractVersion": "same-oos-model-comparison-v1",
                    "enabled": compare_models,
                    "sameFoldBoundaries": True,
                    "sameFeatureSchema": True,
                    "sameHorizon": True,
                    "sameOosSampleIds": True,
                    "xgboostMetrics": aggregate_metrics,
                    "trainMomentumStatePriorMetrics": momentum_baseline_metrics,
                    "empiricalClassPriorMetrics": baseline_metrics,
                    "lightgbmMetrics": (
                        worker.compute_metrics(all_lightgbm_predictions, completed_at_ms)
                        if compare_models else None
                    ),
                    "selectionPolicy": "descriptive_only_no_automatic_winner",
                    "candidateReviewOnly": True,
                    "liveOrderAllowed": False,
                }
                comparable = {
                    "xgboost": aggregate_metrics["logLossMillionths"],
                    "train_momentum_state_prior": momentum_baseline_metrics["logLossMillionths"],
                    "empirical_class_prior": baseline_metrics["logLossMillionths"],
                }
                if compare_models:
                    comparable["lightgbm"] = model_comparison["lightgbmMetrics"]["logLossMillionths"]
                model_comparison["lowestLogLossLabel"] = min(comparable, key=comparable.get)
                calibration_comparison = {
                    "contractVersion": "rolling-oos-temperature-v1",
                    "excludedColdStartFoldIndexes": [0],
                    "fitUsesPriorOosOnly": True,
                    "argmaxAndRankingPreserved": True,
                    "liveOrderAllowed": False,
                }
                if calibrated_predictions:
                    raw_comparable_metrics = worker.compute_metrics(
                        _reindex_folds(calibration_comparable_raw_predictions), completed_at_ms,
                    )
                    calibrated_metrics = worker.compute_metrics(
                        _reindex_folds(calibrated_predictions), completed_at_ms,
                    )
                    calibration_comparison.update({
                        "available": True,
                        "rawMetrics": raw_comparable_metrics,
                        "calibratedMetrics": calibrated_metrics,
                        "improvesLogLoss": calibrated_metrics["logLossMillionths"] < raw_comparable_metrics["logLossMillionths"],
                        "improvesExpectedCalibrationError": calibrated_metrics["expectedCalibrationErrorBps"] < raw_comparable_metrics["expectedCalibrationErrorBps"],
                    })
                else:
                    calibration_comparison.update({"available": False, "reason": "insufficient_prior_oos_folds"})
                price = mark if market == "usdm" else trade
                if price is None:
                    raise ValidationError("OOS 전략 가격 기준이 없습니다.")
                fee_bps = DEFAULT_USDM_TAKER_FEE_BPS if market == "usdm" else DEFAULT_SPOT_TAKER_FEE_BPS
                strategy_cost_stress = []
                for multiplier in COST_STRESS_MULTIPLIERS:
                    scenario = evaluate_oos_strategy(
                        all_predictions, rows, price, funding, market, fee_bps,
                        DEFAULT_SLIPPAGE_BPS, multiplier,
                        horizon_bars=interval_spec.horizon_bars,
                        regime_by_sample=regime_by_sample,
                    )
                    scenario["samePeriodLongBaseline"] = evaluate_same_period_long_baseline(
                        price, funding, market, scenario["firstEntryTimeMs"], scenario["lastExitTimeMs"],
                        fee_bps, DEFAULT_SLIPPAGE_BPS, multiplier,
                    )
                    strategy_cost_stress.append(scenario)
                results.append({
                    "market": market, "symbol": symbol, "provider": "Binance official public REST",
                    "interval": interval, "intervalMs": interval_spec.interval_ms,
                    "horizonBars": interval_spec.horizon_bars, "horizonMs": interval_spec.horizon_ms,
                    "classThresholdBps": interval_spec.class_threshold_bps, "rawCompletedBars": len(trade),
                    "featureIds": feature_ids, "walkForwardFoldCount": len(fold_records),
                    "walkForwardFolds": fold_records, "metrics": aggregate_metrics,
                    "empiricalPriorBaselineMetrics": baseline_metrics,
                    "beatsEmpiricalPriorLogLoss": aggregate_metrics["logLossMillionths"] < baseline_metrics["logLossMillionths"],
                    "rollingOosTemperatureCalibration": calibration_comparison,
                    "modelComparison": model_comparison,
                    "strategyEvaluation": {
                        "signalRule": "argmax; spot=up-only, usdm=up-long/down-short, flat=no-position",
                        "entryRule": "next_bar_open_after_decision",
                        "exitRule": "target_horizon_close",
                        "positionSizing": "full_equity_1x_unleveraged",
                        "regimeContract": "observed-regime-v1; fold training rows only",
                        "feeSource": "explicit_research_assumption_account_tier_not_queried",
                        "fundingSource": "official_point_in_time_history" if market == "usdm" else "not_applicable",
                        "costStress": strategy_cost_stress,
                        "noPerformancePassLine": True, "candidateReviewOnly": True,
                        "liveOrderAllowed": False,
                    },
                    "candidateReviewOnly": True, "liveOrderAllowed": False,
                })
    report = {
        "contractVersion": "investa-real-ml-validation-v2", "runId": run_id,
        "startedAtMs": min(observed_starts), "endedBeforeMs": max(observed_ends), "days": days,
        "intervals": list(intervals),
        "requestCount": fetcher.request_count, "results": results,
        "limitations": [
            "4개 expanding walk-forward fold이며 지원 주기는 1h·4h·1d로 제한된다.",
            "레짐은 각 fold의 과거 학습 표본에서 계산한 4봉 추세 절대값 중앙값과 20봉 변동성 75분위수의 설명용 분류다.",
            "전략 평가는 계정별 실제 수수료를 조회하지 않고 명시한 taker 수수료·슬리피지 연구 가정을 사용한다.",
            "비중첩 고정 horizon 보유 규칙만 검사하며 주문장 체결·시장 충격을 재현하지 않는다.",
            "온도 보정은 현재 fold 이전의 OOS 확률만 사용하며 첫 fold는 cold-start로 전후 비교에서 제외한다.",
            "온도 보정은 argmax와 순위를 바꾸지 않으며 성과 전략을 개선하는 매매 규칙이 아니다.",
            "XGBoost 기준선과 rolling OOS 온도 보정 진단은 모델 승격이나 주문 권한을 부여하지 않는다.",
            "모델 비교를 켠 경우 LightGBM·XGBoost·학습구간 모멘텀 상태 기준선은 같은 fold·피처·horizon·OOS 표본을 사용한다.",
            "최저 log loss 표시는 설명용이며 자동 모델 선택·승격·주문 권한을 부여하지 않는다.",
        ],
        "liveOrderAllowed": False,
    }
    (output_root / "validation-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    return report


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Investa 공식 공개 실제 데이터 ML 검증")
    parser.add_argument("--days", type=int, default=180)
    parser.add_argument("--intervals", type=parse_intervals, default=(INTERVAL,))
    parser.add_argument("--output-root", type=Path)
    parser.add_argument("--compare-models", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    base = Path(os.environ.get("LOCALAPPDATA", Path.home())) / "Investa" / "validation"
    output_root = args.output_root or base / f"real-ml-{time.strftime('%Y%m%d-%H%M%S')}"
    try:
        report = run_validation(args.days, output_root.resolve(), args.intervals, args.compare_models)
        print(canonical_json({"succeeded": True, "outputRoot": str(output_root.resolve()), "requestCount": report["requestCount"], "resultCount": len(report["results"]), "liveOrderAllowed": False}))
        return 0
    except ValidationError as error:
        print(canonical_json({"succeeded": False, "failureCode": "validation_failure", "message": str(error)}))
        return 2
    except Exception as error:
        print(canonical_json({"succeeded": False, "failureCode": "validation_internal_failure", "message": type(error).__name__}))
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
