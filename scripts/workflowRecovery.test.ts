import assert from "node:assert/strict";
import test from "node:test";
import { canRestartInterruptedWorkflow, recoveryStageLabel } from "../src/workflowRecovery.ts";

test("중단된 회의만 다른 회의가 없을 때 재시작한다", () => {
  const interrupted = { jobId: "job-1", topic: "분석", stage: "department-analysis", status: "interrupted" };
  assert.equal(canRestartInterruptedWorkflow(null, interrupted), true);
  assert.equal(canRestartInterruptedWorkflow("진행 중", interrupted), false);
  assert.equal(canRestartInterruptedWorkflow(null, { ...interrupted, status: "completed" }), false);
});

test("복구 단계는 사용자용 문구로 표시한다", () => {
  assert.equal(recoveryStageLabel("department-analysis"), "부서 분석");
  assert.equal(recoveryStageLabel("unknown-stage"), "unknown-stage");
});
