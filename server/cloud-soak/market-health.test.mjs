import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyUpbitPayload,
  createMarketStreamState,
  evaluateMarketStreams,
  isTransportTimedOut,
  observeMarketGap,
  recordMarketMessage,
  recordTransportHeartbeat,
} from "./market-health.mjs";

test("Upbit UP 응답은 시장 체결과 분리한다", () => {
  assert.equal(classifyUpbitPayload('{"status":"UP"}'), "heartbeat");
  assert.equal(classifyUpbitPayload('{"type":"trade"}'), "market");
});

test("시장 공백은 한 구간당 한 번만 기록하고 다음 체결 뒤 다시 연다", () => {
  const state = createMarketStreamState();
  recordMarketMessage(state, 1_000);
  assert.equal(observeMarketGap(state, 22_000, 0), true);
  assert.equal(observeMarketGap(state, 27_000, 0), false);
  recordMarketMessage(state, 28_000);
  assert.equal(observeMarketGap(state, 49_000, 0), true);
  assert.equal(state.marketGapEvents, 2);
});

test("전송 heartbeat는 체결 수를 늘리지 않고 생존 시각만 갱신한다", () => {
  const state = createMarketStreamState();
  recordTransportHeartbeat(state, 30_000);
  assert.equal(state.messages, 0);
  assert.equal(state.transportHeartbeats, 1);
  assert.equal(isTransportTimedOut(state, 60_000, 0, 45_000), false);
  assert.equal(isTransportTimedOut(state, 80_001, 0, 45_000), true);
});

test("이벤트 기반 Upbit 시장 공백은 경고, 전송 timeout은 실패다", () => {
  const definition = {
    id: "upbit_spot",
    eventDriven: true,
    marketGapThresholdMs: 20_000,
  };
  const state = createMarketStreamState();
  recordMarketMessage(state, 1_000);
  recordMarketMessage(state, 25_000);
  let result = evaluateMarketStreams([definition], new Map([[definition.id, state]]));
  assert.deepEqual(result.issues, []);
  assert.equal(result.warnings.length, 1);

  state.transportTimeouts = 1;
  result = evaluateMarketStreams([definition], new Map([[definition.id, state]]));
  assert.equal(result.issues.length, 1);
});

test("정기 갱신 스트림의 20초 초과 공백은 실패다", () => {
  const definition = {
    id: "binance_usdm",
    eventDriven: false,
    marketGapThresholdMs: 20_000,
  };
  const state = createMarketStreamState();
  recordMarketMessage(state, 1_000);
  recordMarketMessage(state, 22_000);
  const result = evaluateMarketStreams([definition], new Map([[definition.id, state]]));
  assert.equal(result.issues.length, 1);
  assert.deepEqual(result.warnings, []);
});
