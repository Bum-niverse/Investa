export type RecoveryJobSummary = { jobId: string; topic: string; stage: string; status: string };

export const canRestartInterruptedWorkflow = (activeMeetingTopic: string | null, job: RecoveryJobSummary) =>
  activeMeetingTopic === null && job.status === "interrupted";

export const recoveryStageLabel = (stage: string) => ({
  routing: "안건 분류", summoning: "부서장 소집", briefing: "안건 전달", dispatching: "부서 복귀",
  "department-analysis": "부서 분석", reconvening: "결과 보고 복귀", results: "최종 종합",
}[stage] ?? stage);
