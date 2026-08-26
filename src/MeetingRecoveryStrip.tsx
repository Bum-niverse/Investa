import { canRestartInterruptedWorkflow, recoveryStageLabel, type RecoveryJobSummary } from "./workflowRecovery";

type Props<T extends RecoveryJobSummary> = {
  error: string | null;
  jobs: T[];
  activeMeetingTopic: string | null;
  onRetryOperations: () => void;
  onRestart: (job: T) => void;
  onDismiss: (jobId: string) => void;
};

export function MeetingRecoveryStrip<T extends RecoveryJobSummary>({ error, jobs, activeMeetingTopic, onRetryOperations, onRestart, onDismiss }: Props<T>) {
  if (!error && jobs.length === 0) return null;
  return <section className="recovery-strip" aria-live="polite" aria-label="운영 복구">
    {error && <div role="alert"><strong>운영 엔진 확인 필요</strong><p>{error}</p><button type="button" onClick={onRetryOperations}>다시 확인</button></div>}
    {jobs.map((job) => <div key={job.jobId}><strong>중단된 회의 · {recoveryStageLabel(job.stage)}</strong><p>{job.topic}</p><span>
      <button type="button" disabled={!canRestartInterruptedWorkflow(activeMeetingTopic, job)} onClick={() => onRestart(job)}>처음부터 다시 실행</button>
      <button type="button" onClick={() => onDismiss(job.jobId)}>기록 닫기</button>
    </span></div>)}
  </section>;
}
