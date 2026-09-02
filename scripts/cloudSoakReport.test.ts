import assert from "node:assert/strict";
import test from "node:test";
import { buildWindowsGcloudInvocation, classifyGcloudError, evaluateReport, summarizeExecution } from "./cloud_soak_report.mjs";

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

test("사용자가 조기 종료한 실행은 실제 실패가 아닌 경고로 분리한다", () => {
  const cancelled = summarizeExecution(
    definition,
    { ...execution, status: { cancelledCount: 1, conditions: [{ type: "Completed", status: "False", reason: "Cancelled" }] } },
    [{ timestamp: "2026-09-02T22:40:00Z", jsonPayload: { schema: "investa.cloud-soak.v2", mode: "market", event: "completed", elapsedMs: 81_600_000, passed: true, actualElapsed24hQualified: false, streams: {} } }],
  );
  assert.equal(cancelled.state, "cancelled");
  assert.equal(evaluateReport([cancelled]), "warning");
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

test("배포 전환 중 생성된 v1 완료 로그도 24시간 판정에 사용한다", () => {
  const result = summarizeExecution(
    { mode: "shadow-contract", jobName: "investa-shadow-contract-soak-24h" },
    { ...execution, status: { ...execution.status, completionTime: "2026-09-03T00:00:01Z", conditions: [{ type: "Completed", status: "True" }] } },
    [{ timestamp: "2026-09-03T00:00:00Z", jsonPayload: { schema: "investa.cloud-soak.v1", mode: "shadow-contract", event: "completed", elapsedMs: 86_400_000, passed: true, actualElapsed24hQualified: true, failures: 0, reconciliationPassed: true } }],
  );
  assert.equal(result.state, "completed");
  assert.equal(result.passed, true);
  assert.equal(result.actualElapsed24hQualified, true);
  assert.equal(result.heartbeat?.reconciliationPassed, true);
});

test("Windows에서는 확인된 gcloud.cmd를 고정 PowerShell 래퍼에 인자 배열로 전달한다", () => {
  const invocation = buildWindowsGcloudInvocation("C:\\Program Files\\Google\\Cloud SDK\\gcloud.cmd", [
    "run", "jobs", "executions", "list", "--format=json",
  ], { SystemRoot: "C:\\Windows" });
  assert.equal(invocation.file, "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
  assert.deepEqual(invocation.args.slice(-6), ["C:\\Program Files\\Google\\Cloud SDK\\gcloud.cmd", "run", "jobs", "executions", "list", "--format=json"]);
  assert.throws(() => buildWindowsGcloudInvocation("gcloud.cmd", ["--format=json"]), /절대 경로/);
  assert.throws(() => buildWindowsGcloudInvocation("C:\\gcloud.cmd", ["value\nnext"]), /허용되지 않은 문자/);
});

test("CLI 부재·인증·권한 오류를 성공이나 모호한 EINVAL로 표시하지 않는다", () => {
  assert.equal(classifyGcloudError(Object.assign(new Error("missing"), { code: "GCLOUD_NOT_FOUND" })), "Google Cloud CLI를 찾지 못했습니다.");
  assert.equal(classifyGcloudError(Object.assign(new Error("auth"), { stderr: "Please run gcloud auth login" })), "Google Cloud CLI 로그인이 필요합니다.");
  assert.equal(classifyGcloudError(Object.assign(new Error("iam"), { stderr: "Permission denied" })), "Cloud Run 또는 Logging 읽기 권한이 없습니다.");
});
