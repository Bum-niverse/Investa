import assert from "node:assert/strict";
import test from "node:test";
import { assessMarketStream, parseMarketStreamMessage, reconnectDelayMs, shouldAttemptAutomaticGapBackfill, toMarketAggregationInput, type MarketStreamSnapshot } from "../src/marketStreams.ts";

test("Upbit trade를 관측 시각·순번·체결수량이 있는 공개 시세로 정규화한다", () => {
  const sample = parseMarketStreamMessage("upbit_spot", JSON.stringify({ type: "trade", code: "KRW-BTC", trade_price: 123_456_789, trade_volume: 0.00001, trade_timestamp: 1_000, sequential_id: 77 }), 2_000);
  assert.equal(sample.price, 123_456_789);
  assert.equal(sample.eventAtMs, 1_000);
  assert.equal(sample.sequence, 77);
  assert.equal(toMarketAggregationInput(sample)?.quantity, "0.00001");
});

test("Binance 무기한선물 mark/index/funding을 서로 다른 필드로 보존한다", () => {
  const sample = parseMarketStreamMessage("binance_usdm", JSON.stringify({ stream: "btcusdt@markPrice@1s", data: { e: "markPriceUpdate", E: 2_000, s: "BTCUSDT", p: "62000.5", i: "61990.25", r: "0.0001", T: 9_000 } }), 2_100);
  assert.equal(sample.markPrice, 62_000.5);
  assert.equal(sample.indexPrice, 61_990.25);
  assert.equal(sample.fundingRate, 0.0001);
  assert.equal(sample.nextFundingAtMs, 9_000);
  assert.equal(toMarketAggregationInput(sample), undefined);

  const negative = parseMarketStreamMessage("binance_coinm", JSON.stringify({ E: 2_000, s: "BTCUSD_PERP", p: "62000.5", i: "61990.25", r: "-0.00025", T: 9_000 }), 2_100);
  assert.equal(negative.fundingRate, -0.00025);
});

test("Binance 선물 combined aggTrade만 공통 Tick 입력으로 보낸다", () => {
  const sample = parseMarketStreamMessage("binance_usdm", JSON.stringify({ stream: "btcusdt@aggTrade", data: { e: "aggTrade", E: 2_000, T: 1_990, s: "BTCUSDT", a: 81, p: "62000.50", q: "0.125" } }), 2_100);
  const input = toMarketAggregationInput(sample);
  assert.equal(input?.price, "62000.50");
  assert.equal(input?.quantity, "0.125");
  assert.equal(input?.sequence, 81);
  assert.equal(input?.assetClass, "crypto_futures");
});

test("Binance Spot aggTrade를 반복 ticker가 아닌 실제 체결 Tick으로 사용한다", () => {
  const sample = parseMarketStreamMessage("binance_spot", JSON.stringify({ e: "aggTrade", E: 2_000, T: 1_999, s: "BTCUSDT", a: 91, p: "62001.25", q: "0.01" }), 2_100);
  const input = toMarketAggregationInput(sample);
  assert.equal(input?.price, "62001.25");
  assert.equal(input?.quantity, "0.01");
  assert.equal(input?.sequence, 91);
  assert.equal(input?.assetClass, "crypto_spot");
});

test("다른 종목과 0·비정상 가격은 거부한다", () => {
  assert.throws(() => parseMarketStreamMessage("binance_spot", JSON.stringify({ e: "aggTrade", s: "ETHUSDT", p: "100", q: "1", E: 1 }), 2_000), /종목/);
  assert.throws(() => parseMarketStreamMessage("upbit_spot", JSON.stringify({ type: "trade", code: "KRW-BTC", trade_price: 0, trade_volume: 1 }), 2_000), /현재가/);
  assert.throws(() => parseMarketStreamMessage("binance_spot", JSON.stringify({ e: "aggTrade", s: "BTCUSDT", p: "100", q: "1", E: 1 }), 2_000), /순번/);
});

test("관측이 만료되면 stale로 전환하고 재연결 backoff를 제한한다", () => {
  const snapshot: MarketStreamSnapshot = { id: "binance_spot", label: "spot", phase: "live", attempt: 0, lastReceivedAtMs: 1_000 };
  assert.equal(assessMarketStream(snapshot, 16_001, 15_000).phase, "stale");
  assert.equal(reconnectDelayMs(0, 0), 1_000);
  assert.equal(reconnectDelayMs(10, 1), 36_000);
});

test("stale·오류·재연결에서 정상 복귀할 때만 공식 gap 자동 복구를 시도한다", () => {
  assert.equal(shouldAttemptAutomaticGapBackfill("reconnecting", "live"), true);
  assert.equal(shouldAttemptAutomaticGapBackfill("stale", "live"), true);
  assert.equal(shouldAttemptAutomaticGapBackfill("error", "live"), true);
  assert.equal(shouldAttemptAutomaticGapBackfill("connecting", "live"), false);
  assert.equal(shouldAttemptAutomaticGapBackfill("live", "live"), false);
});
