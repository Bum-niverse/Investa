from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SOURCE = Path(__file__).resolve().parents[1] / "investa_ml_worker.py"
SHARED_METRICS_FIXTURE = Path(__file__).resolve().parent / "fixtures" / "oos_predictions_v1.json"
SPEC = importlib.util.spec_from_file_location("investa_ml_worker", SOURCE)
assert SPEC is not None and SPEC.loader is not None
worker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = worker
SPEC.loader.exec_module(worker)


def compact(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"))


def make_bundle(algorithm="lightgbm"):
    feature_ids = ["momentum", "volatility"]
    samples = []
    features = []
    for index in range(180):
        decision = (index + 1) * 100
        target = index % 3
        sample_id = f"sample-{index:03d}"
        samples.append({"sampleId": sample_id, "decisionTimeMs": decision, "targetObservedAtMs": decision + 10, "targetClass": target})
        values = {"momentum": (target - 1) * 100 + (index % 7), "volatility": (index % 11) * 10}
        for feature_id in feature_ids:
            features.append({
                "featureId": feature_id, "sampleId": sample_id,
                "sourceRecordId": f"source-{index}-{feature_id}", "datasetVersion": "dataset-v1",
                "metadata": {"eventTimeMs": decision - 2, "availableAtMs": decision - 1, "ingestedAtMs": decision},
                "valueScaled": values[feature_id], "valueScale": 100, "qualityFlags": [],
            })
    payload = {
        "datasetId": "dataset-v1",
        "asset": {"contractId": "kr-stock-test", "assetClass": "korea_stock", "exchange": "KRX", "symbol": "TEST", "currency": "KRW", "timezone": "Asia/Seoul", "adjustedPricePolicy": "point_in_time", "corporateActionPolicy": "effective_at", "contractMultiplier": None, "expiryPolicy": None, "rolloverPolicy": None, "priceBasis": None, "fundingPolicy": None, "leveragePolicy": None},
        "samples": samples, "features": features,
        "split": {"trainEndMs": 10000, "validationStartMs": 10100, "validationEndMs": 14000, "testStartMs": 14100},
        "expectedFeatureIds": feature_ids,
    }
    payload_json = compact(payload)
    dataset_hash = hashlib.sha256(payload_json.encode()).hexdigest()
    schema_hash = hashlib.sha256(compact(feature_ids).encode()).hexdigest()
    job = {
        "jobId": f"job-{algorithm}", "manifestId": "manifest-v1", "datasetSourceKind": "manifest", "algorithm": algorithm,
        "contractVersion": worker.CONTRACT_VERSION, "datasetContentSha256": dataset_hash,
        "featureSchemaSha256": schema_hash, "inputSha256": "", "codeVersion": "baseline-v1",
        "randomSeed": 42, "horizonMs": 86400000, "timeoutSeconds": 600,
        "memoryLimitMb": 1024, "maxThreads": 2,
        "hyperparameters": {"early_stopping_rounds": 5, "num_boost_round": 30},
        "status": "prepared", "createdAtMs": 1000, "updatedAtMs": 1000,
        "liveOrderAllowed": False,
    }
    job["inputSha256"] = hashlib.sha256(compact(job).encode()).hexdigest()
    manifest = {"manifestId": "manifest-v1", "contentSha256": dataset_hash, "featureSchemaSha256": schema_hash}
    return {"contractVersion": worker.CONTRACT_VERSION, "job": job, "manifest": manifest, "datasetPayloadJson": payload_json, "liveOrderAllowed": False}


def make_shard_bundle(root: Path):
    feature_ids = ["momentum", "volatility"]
    schema_hash = hashlib.sha256(compact(feature_ids).encode()).hexdigest()
    asset = make_bundle("xgboost")["datasetPayloadJson"]
    asset = json.loads(asset)["asset"]
    split = {"trainEndMs": 10000, "validationStartMs": 10100, "validationEndMs": 14000, "testStartMs": 14100}
    shard_dir = root / "shards"
    shard_dir.mkdir(exist_ok=True)
    descriptors = []
    files = []
    total_samples = 0
    total_features = 0
    for shard_index in range(3):
        samples = []
        features = []
        decisions = (
            list(range(100 + shard_index * 1000, 1100 + shard_index * 1000, 100))
            + list(range(10100 + shard_index * 1000, 11100 + shard_index * 1000, 100))
            + list(range(14100 + shard_index * 1000, 15100 + shard_index * 1000, 100))
        )
        dataset_id = f"dataset-shard-{shard_index}"
        for index, decision in enumerate(decisions):
            target = (index + shard_index) % 3
            sample_id = f"shard-{shard_index}-sample-{index:03d}"
            samples.append({"sampleId": sample_id, "decisionTimeMs": decision, "targetObservedAtMs": decision + 10, "targetClass": target})
            for feature_id, value in (("momentum", (target - 1) * 100 + index), ("volatility", (index % 7) * 10)):
                features.append({
                    "featureId": feature_id, "sampleId": sample_id,
                    "sourceRecordId": f"source-{shard_index}-{index}-{feature_id}", "datasetVersion": dataset_id,
                    "metadata": {"eventTimeMs": decision - 2, "availableAtMs": decision - 1, "ingestedAtMs": decision},
                    "valueScaled": value, "valueScale": 100, "qualityFlags": [],
                })
        payload = {"datasetId": dataset_id, "asset": asset, "samples": samples, "features": features, "split": split, "expectedFeatureIds": feature_ids}
        payload_json = compact(payload)
        content_hash = hashlib.sha256(payload_json.encode()).hexdigest()
        manifest_id = f"manifest-shard-{shard_index}"
        manifest = {
            "manifestId": manifest_id, "contentSha256": content_hash,
            "featureSchemaSha256": schema_hash, "sampleCount": len(samples),
            "featureCount": len(features),
        }
        def extent(selected):
            return {"sampleCount": len(selected), "firstDecisionTimeMs": selected[0]["decisionTimeMs"], "lastDecisionTimeMs": selected[-1]["decisionTimeMs"]}
        train = [sample for sample in samples if sample["decisionTimeMs"] <= split["trainEndMs"]]
        validation = [sample for sample in samples if split["validationStartMs"] <= sample["decisionTimeMs"] <= split["validationEndMs"]]
        test = [sample for sample in samples if sample["decisionTimeMs"] >= split["testStartMs"]]
        descriptors.append({
            "manifestId": manifest_id, "contentSha256": content_hash,
            "sampleCount": len(samples), "featureCount": len(features),
            "train": extent(train), "validation": extent(validation), "test": extent(test),
        })
        file_name = f"shard-{shard_index:04d}.json"
        path = shard_dir / file_name
        path.write_text(payload_json, encoding="utf-8")
        files.append({"manifest": manifest, "fileName": file_name, "byteSize": path.stat().st_size})
        total_samples += len(samples)
        total_features += len(features)
    combined = ["logical-v1", asset, split, schema_hash, descriptors]
    combined_hash = hashlib.sha256(compact(combined).encode()).hexdigest()
    shard_set = {
        "shardSetId": "shard-set-v1", "datasetId": "logical-v1", "asset": asset,
        "split": split, "featureSchemaSha256": schema_hash,
        "combinedContentSha256": combined_hash, "shardCount": len(descriptors),
        "sampleCount": total_samples, "featureCount": total_features,
        "shards": descriptors, "workerReady": False, "liveOrderAllowed": False,
        "createdAtMs": 1000,
    }
    job = {
        "jobId": "job-xgboost-sharded", "manifestId": "shard-set-v1", "datasetSourceKind": "shard_set",
        "algorithm": "xgboost", "contractVersion": worker.SHARD_CONTRACT_VERSION,
        "datasetContentSha256": combined_hash, "featureSchemaSha256": schema_hash,
        "inputSha256": "", "codeVersion": "baseline-v1", "randomSeed": 42,
        "horizonMs": 86400000, "timeoutSeconds": 600, "memoryLimitMb": 1024,
        "maxThreads": 2, "hyperparameters": {"early_stopping_rounds": 5, "num_boost_round": 30},
        "status": "prepared", "createdAtMs": 1000, "updatedAtMs": 1000,
        "liveOrderAllowed": False,
    }
    job["inputSha256"] = hashlib.sha256(compact(job).encode()).hexdigest()
    return {"contractVersion": worker.SHARD_CONTRACT_VERSION, "job": job, "shardSet": shard_set, "datasetShards": files, "liveOrderAllowed": False}


class ContractTests(unittest.TestCase):
    def test_rejects_live_order_capability(self):
        bundle = make_bundle()
        bundle["liveOrderAllowed"] = True
        with self.assertRaisesRegex(worker.ContractError, "실주문"):
            worker.validate_bundle(bundle)

    def test_rejects_tampered_dataset(self):
        bundle = make_bundle()
        bundle["datasetPayloadJson"] += " "
        with self.assertRaisesRegex(worker.ContractError, "SHA-256"):
            worker.validate_bundle(bundle)

    def test_rejects_future_feature(self):
        bundle = make_bundle()
        job, manifest, payload = worker.validate_bundle(bundle)
        payload["features"][0]["metadata"]["availableAtMs"] = payload["samples"][0]["decisionTimeMs"] + 1
        with self.assertRaisesRegex(worker.ContractError, "결정 시각 이후"):
            worker.prepare_dataset(payload)

    def test_rejects_unknown_hyperparameter(self):
        bundle = make_bundle()
        bundle["job"]["hyperparameters"]["shell_command"] = "calc.exe"
        with self.assertRaisesRegex(worker.ContractError, "허용되지 않은"):
            worker.normalized_hyperparameters(bundle["job"])

    def test_rejects_algorithm_specific_hyperparameter(self):
        bundle = make_bundle("lightgbm")
        bundle["job"]["hyperparameters"]["colsample_bytree"] = 0.8
        with self.assertRaisesRegex(worker.ContractError, "지원하지 않는"):
            worker.normalized_hyperparameters(bundle["job"])

    def test_windows_safe_artifact_name(self):
        bundle = make_bundle()
        bundle["job"]["jobId"] = "job:lightgbm"
        name, _ = worker.safe_artifact_name(bundle["job"])
        self.assertNotIn(":", name)


class BaselineIntegrationTests(unittest.TestCase):
    def test_trains_xgboost_from_file_backed_shards(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output"
            bundle = make_shard_bundle(root)
            result = worker.run(bundle, output, completed_at_ms=2000, input_dir=root)
            self.assertTrue(result["succeeded"])
            self.assertEqual(result["metrics"]["sampleCount"], 30)
            self.assertEqual(len(result["datasetDiagnostics"]["logicalDatasetSha256"]), 64)
            self.assertFalse((output / ".job-xgboost-sharded.xgb-cache").exists())

    def test_sharded_contract_rejects_path_and_content_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = make_shard_bundle(root)
            bundle["datasetShards"][0]["fileName"] = "../shard-0000.json"
            with self.assertRaises(worker.ContractError) as path_error:
                worker.validate_shard_bundle(bundle, root)
            self.assertEqual(path_error.exception.code, "invalid_shard_path")
            bundle = make_shard_bundle(root)
            (root / "shards" / "shard-0000.json").write_text("{}", encoding="utf-8")
            with self.assertRaises(worker.ContractError) as content_error:
                worker.validate_shard_bundle(bundle, root)
            self.assertIn(content_error.exception.code, {"invalid_dataset", "dataset_hash_mismatch"})

    def test_sharded_contract_rejects_descriptor_that_hides_actual_overlap(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = make_shard_bundle(root)
            bundle["shardSet"]["shards"][1]["train"]["firstDecisionTimeMs"] = 1000
            shard_set = bundle["shardSet"]
            combined = [
                shard_set["datasetId"], shard_set["asset"], shard_set["split"],
                shard_set["featureSchemaSha256"], shard_set["shards"],
            ]
            combined_hash = hashlib.sha256(compact(combined).encode()).hexdigest()
            shard_set["combinedContentSha256"] = combined_hash
            bundle["job"]["datasetContentSha256"] = combined_hash
            bundle["job"]["inputSha256"] = ""
            bundle["job"]["inputSha256"] = hashlib.sha256(compact(bundle["job"]).encode()).hexdigest()
            with self.assertRaises(worker.ContractError) as error:
                worker.validate_shard_bundle(bundle, root)
            self.assertIn(error.exception.code, {"invalid_shard_set", "invalid_shard_order"})

    def test_trains_both_safe_artifact_formats(self):
        for algorithm, extension in (("lightgbm", ".txt"), ("xgboost", ".json")):
            with self.subTest(algorithm=algorithm), tempfile.TemporaryDirectory() as directory:
                result = worker.run(make_bundle(algorithm), Path(directory), completed_at_ms=2000)
                self.assertTrue(result["succeeded"])
                self.assertFalse(make_bundle(algorithm)["job"]["liveOrderAllowed"])
                self.assertTrue(result["artifact"]["fileName"].endswith(extension))
                self.assertGreater(result["metrics"]["sampleCount"], 0)
                self.assertLessEqual(result["metrics"]["balancedAccuracyBps"], 10000)
                self.assertEqual(len(result["predictions"]), result["metrics"]["sampleCount"])
                self.assertTrue(all(
                    item["probabilityDownMillionths"] + item["probabilityFlatMillionths"] + item["probabilityUpMillionths"] == 1_000_000
                    for item in result["predictions"]
                ))
                self.assertTrue((Path(directory) / result["artifact"]["fileName"]).is_file())

    def test_cli_emits_completion_json_without_secrets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle_path = root / "bundle.json"
            output_dir = root / "output"
            bundle_path.write_text(compact(make_bundle("xgboost")), encoding="utf-8")
            process = subprocess.run(
                [sys.executable, str(SOURCE), "--input", str(bundle_path), "--output-dir", str(output_dir)],
                check=False, capture_output=True, text=True, timeout=30,
            )
            self.assertEqual(process.returncode, 0, process.stderr)
            result = json.loads(process.stdout)
            self.assertTrue(result["succeeded"])
            self.assertNotIn(str(root), process.stdout)
            self.assertTrue((output_dir / result["artifact"]["fileName"]).is_file())

    def test_quantized_predictions_recompute_stably(self):
        probabilities = [[0.3333333, 0.3333333, 0.3333334], [0.8, 0.1, 0.1]]
        predictions = worker.quantize_predictions(["sample-a", "sample-b"], [2, 0], probabilities)
        first = worker.compute_metrics(predictions, 2_000)
        second = worker.compute_metrics(copy.deepcopy(predictions), 2_000)
        self.assertEqual(first, second)
        self.assertEqual(first["balancedAccuracyBps"], 10_000)

    def test_shared_rust_python_metric_fixture(self):
        fixture = json.loads(SHARED_METRICS_FIXTURE.read_text(encoding="utf-8"))
        self.assertEqual(
            worker.compute_metrics(fixture["predictions"], fixture["metrics"]["evaluatedAtMs"]),
            fixture["metrics"],
        )


if __name__ == "__main__":
    unittest.main()
