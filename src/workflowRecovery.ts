export type RecoveryJobSummary = {
  jobId: string; topic: string; stage: string; status: string;
  selectedDepartmentIds?: string[]; reports?: Record<string, unknown>; updatedAtMs?: number;
};

export const canRestartInterruptedWorkflow = (activeMeetingTopic: string | null, job: RecoveryJobSummary) =>
  activeMeetingTopic === null && job.status === "interrupted";

export const recoveryStageLabel = (stage: string) => ({
  routing: "안건 분류", summoning: "부서장 소집", briefing: "안건 전달", dispatching: "부서 복귀",
  "department-analysis": "부서 분석", reconvening: "결과 보고 복귀", results: "최종 종합",
}[stage] ?? stage);

export const recoveryProgress = (job: RecoveryJobSummary) => {
  const total = job.selectedDepartmentIds?.length ?? 0;
  const completed = Object.keys(job.reports ?? {}).length;
  return { completed, total, remaining: Math.max(0, total - completed) };
};

export const recoveryActionLabel = (job: RecoveryJobSummary) => {
  const progress = recoveryProgress(job);
  return progress.completed > 0 ? `완료 보고 ${progress.completed}개 재사용 · 남은 부서 재개` : "체크포인트에서 안전 재개";
};
