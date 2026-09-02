import assert from "node:assert/strict";
import test from "node:test";
import { evaluateReport, summarizeExecution } from "./cloud_soak_report.mjs";

const definition = { mode: "market", jobName: "investa-market-soak-24h-v2" };
const execution = {
  metadata: { name: "execution-1", creationTimestamp: "2026-09-02T00:00:00Z" },
  status: { startTime: "2026-09-02T00:00:01Z", conditions: [{ type: "Completed", status: "Unknown" }] },
};

test("구조화된 최신 heartbeat만 안전한 운영 요약으로 축약한다", () => {
  const result = summarizeExecution(definition, execution, [
    { timestamp: "2026-09-02T00:01:00Z", jsonPayload: { schema: "other", mode: "market", event: "heartbeat" } },
    { timestamp: "2026-09-02T00:02:00Z", jsonPayload: { schema: "investa.cloud-soak.v2", mode: "market", event: "heartbeat", elapsedMs: 119_000, streams: { upbit_spot: { messages: 10, errors: 0, reconnects: 1, token: "never-copy" } } } },
  ], Date.parse("2026-09-02T00:02:01Z"));
  assert.equal(result.state, "running");
  assert.equal(result.elapsedMs, 119_000);
  assert.deepEqual(result.heartbeat?.streams?.upbit_spot, { messages: 10, reconnects: 1, errors: 0, transportTimeouts: 0, marketGapEvents: 0, lastMessageAtMs: undefined });
  assert.equal(JSON.stringify(result).includes("never-copy"), false);
});

test("24시간 실측과 성공 종료를 모두 충족해야 완료 판정한다", () => {
  const base = { ...summarizeExecution(definition, execution, []), state: "completed", issues: [], warnings: [] };
  assert.equal(evaluateReport([{ ...base, passed: true, actualElapsed24hQualified: false }]), "running");
  assert.equal(evaluateReport([{ ...base, passed: true, actualElapsed24hQualified: true }]), "completed");
  assert.equal(evaluateReport([{ ...base, passed: false, actualElapsed24hQualified: true }]), "failed");
});

test("수집 불가와 경고를 성공으로 오인하지 않는다", () => {
  const unavailable = { ...summarizeExecution(definition, {}, []), collectionIssue: "없음" };
  assert.equal(evaluateReport([unavailable]), "unavailable");
  assert.equal(evaluateReport([{ ...unavailable, state: "running", warnings: ["관찰 필요"] }]), "warning");
});

test("격리 섀도우 heartbeat의 실제 SQLite snake_case 필드를 보존한다", () => {
  const result = summarizeExecution(
    { mode: "shadow-contract", jobName: "investa-shadow-contract-soak-24h" },
    execution,
    [{ timestamp: "2026-09-02T00:02:00Z", jsonPayload: { schema: "investa.cloud-soak.v2", mode: "shadow-contract", event: "heartbeat", elapsedMs: 119_000, event_count: 2, ledger_count: 2, failures: 0, reconciliationPassed: true } }],
  );
  assert.deepEqual(result.heartbeat, { eventCount: 2, ledgerCount: 2, failureCount: 0, reconciliationPassed: true });
});
