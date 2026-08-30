import assert from "node:assert/strict";
import test from "node:test";

import { summarizeConnectionRefresh } from "../src/connectionRefresh.ts";

test("전체 연결 조회 결과를 상태별로 빠짐없이 집계한다", () => {
  const summary = summarizeConnectionRefresh([
    { id: "toss", label: "토스증권", state: "connected" },
    { id: "kis", label: "KIS 모의투자", state: "attention" },
    { id: "upbit", label: "Upbit", state: "disconnected" },
    { id: "sec", label: "SEC", state: "failed" },
  ], 1234);

  assert.deepEqual(summary, {
    total: 4,
    connected: 1,
    attention: 1,
    disconnected: 1,
    failed: 1,
    completedAtMs: 1234,
  });
});

test("빈 결과도 완료 시각을 보존한다", () => {
  assert.deepEqual(summarizeConnectionRefresh([], 99), {
    total: 0,
    connected: 0,
    attention: 0,
    disconnected: 0,
    failed: 0,
    completedAtMs: 99,
  });
});
