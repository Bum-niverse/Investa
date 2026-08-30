from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from decimal import Decimal
from pathlib import Path


SOURCE = Path(__file__).resolve().parent / "run_real_ml_validation.py"
SPEC = importlib.util.spec_from_file_location("real_ml_validation", SOURCE)
assert SPEC is not None and SPEC.loader is not None
validation = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = validation
SPEC.loader.exec_module(validation)


class RealMlValidationTests(unittest.TestCase):
    @staticmethod
    def _strategy_fixture():
        candles = []
        for index in range(10):
            price = Decimal(100 + index)
            candles.append(validation.Candle(
                open_time_ms=index * validation.INTERVAL_MS,
                close_time_ms=(index + 1) * validation.INTERVAL_MS,
                open=price, high=price, low=price, close=price, volume=Decimal(1),
            ))
        rows = []
        predictions = []
        for position in (1, 2):
            decision = candles[position].close_time_ms
            sample_id = f"spot-BTCUSDT-{decision}"
            rows.append({"sample": {
                "sampleId": sample_id, "decisionTimeMs": decision,
                "targetObservedAtMs": candles[position + validation.HORIZON_BARS].close_time_ms,
                "targetClass": 2,
            }})
            predictions.append({
                "sampleId": sample_id, "foldIndex": 0, "targetClass": 2,
                "probabilityDownMillionths": 100_000,
                "probabilityFlatMillionths": 200_000,
                "probabilityUpMillionths": 700_000,
            })
        return candles, rows, predictions

    def test_oos_strategy_uses_next_open_skips_overlap_and_applies_costs(self):
        candles, rows, predictions = self._strategy_fixture()
        result = validation.evaluate_oos_strategy(
            predictions, rows, candles, None, "spot", Decimal(10), Decimal(0), Decimal(1),
        )
        self.assertEqual(result["tradeCount"], 1)
        self.assertEqual(result["skippedOverlappingSignalCount"], 1)
        self.assertEqual(result["feeAndSlippageBpsTotal"], 20)
        self.assertLess(result["netReturnBps"], result["grossReturnBps"])
        self.assertEqual(result["firstEntryTimeMs"], candles[2].open_time_ms)
        self.assertEqual(result["timeInMarketBps"], 10_000)
        stressed = validation.evaluate_oos_strategy(
            predictions, rows, candles, None, "spot", Decimal(10), Decimal(0), Decimal(2),
        )
        self.assertLess(stressed["netReturnBps"], result["netReturnBps"])
        baseline = validation.evaluate_same_period_long_baseline(
            candles, None, "spot", result["firstEntryTimeMs"], result["lastExitTimeMs"],
            Decimal(10), Decimal(0), Decimal(1),
        )
        self.assertTrue(baseline["available"])
        self.assertFalse(baseline["exposureMatched"])
        self.assertEqual(baseline["feeAndSlippageBpsTotal"], 20)

    def test_usdm_short_receives_positive_funding_and_never_enables_orders(self):
        candles, rows, predictions = self._strategy_fixture()
        rows = [rows[0]]
        rows[0]["sample"]["targetClass"] = 0
        predictions = [{
            **predictions[0], "targetClass": 0,
            "probabilityDownMillionths": 700_000,
            "probabilityFlatMillionths": 200_000,
            "probabilityUpMillionths": 100_000,
        }]
        funding_time = candles[4].close_time_ms
        result = validation.evaluate_oos_strategy(
            predictions, rows, candles, [(funding_time, Decimal("0.001"))],
            "usdm", Decimal(0), Decimal(0), Decimal(1),
        )
        self.assertEqual(result["shortCount"], 1)
        self.assertEqual(result["fundingEffectBpsTotal"], 10)
        self.assertFalse(result["liveOrderAllowed"])

    def test_walk_forward_boundaries_are_contiguous_and_non_overlapping(self):
        boundaries = validation.walk_forward_boundaries(4_299)
        self.assertEqual(len(boundaries), 4)
        for train_cut, validation_cut, test_end in boundaries:
            self.assertLess(train_cut, validation_cut)
            self.assertLess(validation_cut, test_end)
        for previous, current in zip(boundaries, boundaries[1:]):
            self.assertEqual(previous[2], current[1])

    def test_baseline_class_counts_exclude_targets_observed_after_split(self):
        rows = [
            {"sample": {"decisionTimeMs": 100, "targetObservedAtMs": 150, "targetClass": 0}},
            {"sample": {"decisionTimeMs": 200, "targetObservedAtMs": 350, "targetClass": 2}},
        ]
        counts = validation.class_counts(rows, end_ms=250, target_before_ms=300)
        self.assertEqual(counts, [1, 0, 0])

    def test_parses_only_complete_positive_hourly_candle(self):
        candle = validation.parse_candle([0 + 1, "100", "110", "90", "105", "2", validation.INTERVAL_MS])
        self.assertEqual(candle.close, Decimal("105"))
        with self.assertRaises(validation.ValidationError):
            validation.parse_candle([1, "100", "99", "90", "105", "2", validation.INTERVAL_MS])

    def test_multitimeframe_contract_rejects_duplicates_and_parses_four_hour_candle(self):
        self.assertEqual(validation.parse_intervals("1h,4h,1d"), ("1h", "4h", "1d"))
        with self.assertRaises(validation.ValidationError):
            validation.parse_intervals("1h,1h")
        with self.assertRaises(validation.ValidationError):
            validation.parse_intervals("15m")
        four_hour = validation.INTERVAL_SPECS["4h"]
        candle = validation.parse_candle(
            [1, "100", "110", "90", "105", "2", four_hour.interval_ms],
            four_hour.interval_ms,
        )
        self.assertEqual(candle.close_time_ms - candle.open_time_ms, four_hour.interval_ms)

    def test_regime_thresholds_use_only_supplied_training_rows(self):
        def row(index, trend, volatility):
            sample_id = f"sample-{index}"
            return {
                "sample": {"sampleId": sample_id, "decisionTimeMs": index, "targetObservedAtMs": index + 1, "targetClass": index % 3},
                "features": [
                    {"featureId": "return_4", "valueScaled": trend, "valueScale": validation.FEATURE_SCALE},
                    {"featureId": "realized_volatility_20", "valueScaled": volatility, "valueScale": validation.FEATURE_SCALE},
                ],
            }
        training = [row(index, index - 15, index + 1) for index in range(30)]
        thresholds = validation.fit_regime_thresholds(training)
        self.assertTrue(thresholds["fitUsesTrainingRowsOnly"])
        self.assertEqual(thresholds["fitSampleCount"], 30)
        future_outlier = row(999, 10_000_000, 10_000_000)
        self.assertEqual(thresholds, validation.fit_regime_thresholds(training))
        self.assertEqual(validation.classify_regime(future_outlier, thresholds), "bull_high_vol")

    def test_oos_regime_diagnostics_cover_only_executed_nonoverlapping_trades(self):
        candles, rows, predictions = self._strategy_fixture()
        regime_by_sample = {
            rows[0]["sample"]["sampleId"]: "bull_normal_vol",
            rows[1]["sample"]["sampleId"]: "bear_high_vol",
        }
        result = validation.evaluate_oos_strategy(
            predictions, rows, candles, None, "spot", Decimal(0), Decimal(0), Decimal(1),
            regime_by_sample=regime_by_sample,
        )
        self.assertEqual(result["tradeCount"], 1)
        self.assertEqual(sum(item["tradeCount"] for item in result["regimeDiagnostics"]), 1)
        self.assertEqual(result["regimeDiagnostics"][0]["regime"], "bull_normal_vol")
        self.assertTrue(result["regimeDiagnosticsDescriptiveOnly"])

    def test_generated_shards_pass_worker_contract_without_split_leakage(self):
        worker = validation.load_worker()
        feature_ids = ["return_1"]
        rows = []
        for index in range(300):
            decision = (index + 1) * 10_000
            sample_id = f"sample-{index:03d}"
            rows.append({
                "sample": {"sampleId": sample_id, "decisionTimeMs": decision, "targetObservedAtMs": decision + 1_000, "targetClass": index % 3},
                "features": [{
                    "featureId": "return_1", "sampleId": sample_id,
                    "sourceRecordId": f"source-{index}",
                    "metadata": {"eventTimeMs": decision - 2_000, "availableAtMs": decision - 1_000, "ingestedAtMs": decision},
                    "valueScaled": index, "valueScale": validation.FEATURE_SCALE, "qualityFlags": [],
                }],
            })
        asset = {
            "contractId": "binance-spot-btcusdt", "assetClass": "crypto_spot",
            "exchange": "BINANCE", "symbol": "BTCUSDT", "currency": "USDT", "timezone": "UTC",
            "adjustedPricePolicy": "not_applicable", "corporateActionPolicy": "not_applicable",
            "contractMultiplier": None, "expiryPolicy": None, "rolloverPolicy": "not_applicable",
            "priceBasis": "close", "fundingPolicy": None, "leveragePolicy": None,
        }
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            input_root = root / "input"
            input_root.mkdir()
            bundle, split_counts, files = validation.build_bundle(worker, "spot", "BTCUSDT", rows, feature_ids, asset, "test-v1")
            validation.write_shards(bundle, files, input_root)
            job, _, paths, summary = worker.validate_shard_bundle(bundle, input_root)
            self.assertEqual(job["datasetSourceKind"], "shard_set")
            self.assertEqual(len(paths), validation.SHARD_COUNT)
            self.assertEqual(len(summary["testSampleIds"]), split_counts["test"])
            for path in paths:
                payload = json.loads(path.read_text(encoding="utf-8"))
                for sample in payload["samples"]:
                    decision = sample["decisionTimeMs"]
                    if decision <= payload["split"]["trainEndMs"]:
                        self.assertLess(sample["targetObservedAtMs"], payload["split"]["validationStartMs"])
                    elif decision <= payload["split"]["validationEndMs"]:
                        self.assertLess(sample["targetObservedAtMs"], payload["split"]["testStartMs"])

    def test_temperature_calibration_uses_only_prior_oos_and_preserves_argmax(self):
        worker = validation.load_worker()
        source = []
        current = []
        targets = [0, 1, 2, 0, 1, 2]
        for index, target in enumerate(targets):
            probabilities = [25_000, 25_000, 25_000]
            probabilities[target] = 950_000
            if index % 2:
                wrong = (target + 1) % 3
                probabilities[target], probabilities[wrong] = probabilities[wrong], probabilities[target]
            source.append({
                "sampleId": f"source-{index}", "foldIndex": 0, "targetClass": target,
                "probabilityDownMillionths": probabilities[0],
                "probabilityFlatMillionths": probabilities[1],
                "probabilityUpMillionths": probabilities[2],
            })
            current.append({**source[-1], "sampleId": f"current-{index}", "foldIndex": 1})
        calibration = validation.fit_oos_temperature(source, 1)
        calibrated = validation.apply_oos_temperature(worker, current, calibration, 1)
        self.assertGreater(calibration["temperatureMillionths"], 1_000_000)
        self.assertTrue(calibration["fitUsesPriorOosOnly"])
        self.assertEqual(calibration["sourceFoldIndexes"], [0])
        self.assertTrue(all(sum(
            int(item[key]) for key in (
                "probabilityDownMillionths", "probabilityFlatMillionths", "probabilityUpMillionths",
            )
        ) == 1_000_000 for item in calibrated))
        for before, after in zip(current, calibrated):
            before_probs = validation._prediction_probabilities(before)
            after_probs = validation._prediction_probabilities(after)
            self.assertEqual(before_probs.index(max(before_probs)), after_probs.index(max(after_probs)))
        raw_loss = validation._multiclass_log_loss(current, 1.0)
        calibrated_loss = validation._multiclass_log_loss(calibrated, 1.0)
        self.assertLess(calibrated_loss, raw_loss)

    def test_temperature_calibration_rejects_current_or_future_fold_as_source(self):
        prediction = {
            "sampleId": "future", "foldIndex": 1, "targetClass": 2,
            "probabilityDownMillionths": 100_000,
            "probabilityFlatMillionths": 200_000,
            "probabilityUpMillionths": 700_000,
        }
        with self.assertRaises(validation.ValidationError):
            validation.fit_oos_temperature([prediction], 1)

    def test_momentum_baseline_uses_training_rows_only(self):
        def row(index, return_4, target):
            return {
                "sample": {
                    "sampleId": f"sample-{index}", "decisionTimeMs": index + 1,
                    "targetObservedAtMs": index + 2, "targetClass": target,
                },
                "features": [{
                    "featureId": "return_4", "valueScaled": return_4,
                    "valueScale": validation.FEATURE_SCALE,
                }],
            }
        training = [row(index, (index % 9 - 4) * 100, index % 3) for index in range(90)]
        baseline = validation.fit_momentum_state_baseline(training)
        self.assertTrue(baseline["fitUsesTrainingRowsOnly"])
        self.assertEqual(baseline["fitSampleCount"], 90)
        self.assertEqual(baseline, validation.fit_momentum_state_baseline(training))
        future = row(999, 10_000_000, 2)
        self.assertEqual(baseline, validation.fit_momentum_state_baseline(training))
        self.assertNotIn(future["sample"]["sampleId"], str(baseline))

    def test_lightgbm_direct_bundle_reuses_shard_split_and_test_ids(self):
        worker = validation.load_worker()
        feature_ids = ["return_1"]
        rows = []
        for index in range(300):
            decision = (index + 1) * 10_000
            sample_id = f"sample-{index:03d}"
            rows.append({
                "sample": {
                    "sampleId": sample_id, "decisionTimeMs": decision,
                    "targetObservedAtMs": decision + 1_000, "targetClass": index % 3,
                },
                "features": [{
                    "featureId": "return_1", "sampleId": sample_id,
                    "sourceRecordId": f"source-{index}",
                    "metadata": {
                        "eventTimeMs": decision - 2_000,
                        "availableAtMs": decision - 1_000,
                        "ingestedAtMs": decision,
                    },
                    "valueScaled": index, "valueScale": validation.FEATURE_SCALE,
                    "qualityFlags": [],
                }],
            })
        asset = {
            "contractId": "binance-spot-btcusdt", "assetClass": "crypto_spot",
            "exchange": "BINANCE", "symbol": "BTCUSDT", "currency": "USDT", "timezone": "UTC",
            "adjustedPricePolicy": "not_applicable", "corporateActionPolicy": "not_applicable",
            "contractMultiplier": None, "expiryPolicy": None, "rolloverPolicy": "not_applicable",
            "priceBasis": "close", "fundingPolicy": None, "leveragePolicy": None,
        }
        shard_bundle, _, _ = validation.build_bundle(
            worker, "spot", "BTCUSDT", rows, feature_ids, asset, "comparison-test",
        )
        split = shard_bundle["shardSet"]["split"]
        direct = validation.build_direct_bundle(
            worker, "lightgbm", "spot", "BTCUSDT", rows, feature_ids, asset,
            "comparison-test", split, rows[-1]["sample"]["decisionTimeMs"],
        )
        _, _, payload = worker.validate_bundle(direct)
        prepared = worker.prepare_dataset(payload)
        expected_test_ids = [
            item["sample"]["sampleId"] for item in rows
            if item["sample"]["decisionTimeMs"] >= split["testStartMs"]
        ]
        self.assertEqual(prepared.test_sample_ids, expected_test_ids)
        self.assertEqual(direct["job"]["horizonMs"], validation.INTERVAL_SPECS["1h"].horizon_ms)
        self.assertFalse(direct["liveOrderAllowed"])


if __name__ == "__main__":
    unittest.main()
