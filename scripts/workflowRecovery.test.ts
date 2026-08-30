import assert from "node:assert/strict";
import test from "node:test";
import { canRestartInterruptedWorkflow, recoveryActionLabel, recoveryProgress, recoveryStageLabel } from "../src/workflowRecovery.ts";

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

test("중단 회의는 완료된 부서 보고와 남은 범위를 설명한다", () => {
  const job = { jobId: "job-2", topic: "복합 분석", stage: "department-analysis", status: "interrupted", selectedDepartmentIds: ["research", "risk"], reports: { research: { summary: "완료" } } };
  assert.deepEqual(recoveryProgress(job), { completed: 1, total: 2, remaining: 1 });
  assert.match(recoveryActionLabel(job), /완료 보고 1개 재사용/);
  assert.match(recoveryActionLabel(job), /남은 부서 재개/);
});
