import { invoke, isTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

type StreamCounters = { messages: number; reconnects: number; errors: number; transportTimeouts: number; marketGapEvents: number; lastMessageAtMs?: number | null };
type CloudSoakJob = {
  mode: "market" | "shadow-contract";
  jobName: string;
  executionName?: string | null;
  state: "unavailable" | "running" | "cancelled" | "failed" | "completed";
  startedAtMs?: number | null;
  completedAtMs?: number | null;
  elapsedMs?: number | null;
  latestHeartbeatAtMs?: number | null;
  heartbeat?: { streams: Record<string, StreamCounters>; eventCount: number; ledgerCount: number; failureCount: number; reconciliationPassed: boolean } | null;
  passed?: boolean | null;
  actualElapsed24hQualified: boolean;
  issues: string[];
  warnings: string[];
  collectionIssue?: string | null;
};
type CloudSoakReport = { collectedAtMs: number; status: "unavailable" | "running" | "warning" | "failed" | "completed"; liveOrderEnabled: false; jobs: CloudSoakJob[] };
type CloudSoakSnapshot = { available: boolean; report?: CloudSoakReport | null; issue?: string | null };

const dateTime = (value?: number | null) => value ? new Date(value).toLocaleString("ko-KR") : "관측 없음";
const duration = (value?: number | null) => {
  if (value == null) return "경과 확인 불가";
  const hours = Math.floor(value / 3_600_000);
  const minutes = Math.floor((value % 3_600_000) / 60_000);
  return `${hours}시간 ${minutes}분`;
};
const statusLabel = { unavailable: "수집 전", running: "검사 진행 중", warning: "경고 확인 필요", failed: "검사 실패", completed: "24시간 검사 통과" } as const;
const jobStateLabel = { unavailable: "수집 전", running: "진행 중", cancelled: "조기 종료", failed: "실패", completed: "완료" } as const;

export function CloudSoakStatusPanel() {
  const [snapshot, setSnapshot] = useState<CloudSoakSnapshot>();
  const [error, setError] = useState<string>();
  const [loading, setLoading] = useState(false);
  const load = useCallback(async () => {
    if (!isTauri()) {
      setSnapshot({ available: false, issue: "데스크톱 앱에서 저장된 Cloud Run 검사 결과를 확인할 수 있습니다." });
      return;
    }
    setLoading(true);
    try {
      setSnapshot(await invoke<CloudSoakSnapshot>("cloud_soak_report_snapshot"));
      setError(undefined);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);
  useEffect(() => { void load(); }, [load]);
  const report = snapshot?.report;
  return <article className={`cloud-soak-status is-${report?.status ?? "unavailable"}`}>
    <h4>Cloud Run 24시간 내구 검사</h4>
    <p>클라우드 권한과 원본 로그는 앱에 전달하지 않고, 읽기 전용 수집기가 만든 검증 캐시만 표시합니다.</p>
    <div className="readiness-actions"><button type="button" onClick={() => void load()} disabled={loading}>{loading ? "결과 읽는 중…" : "저장된 검사 결과 다시 읽기"}</button></div>
    <strong role="status">{statusLabel[report?.status ?? "unavailable"]} · 실전 주문 잠금</strong>
    {snapshot?.issue ? <small>{snapshot.issue}</small> : null}
    {error ? <small className="settings-error" role="alert">{error}</small> : null}
    {report?.jobs.map((job) => <section className="readiness-row" key={`${job.jobName}-${job.executionName ?? "none"}`}>
      <b>{job.mode === "market" ? "실시간 시세 스트림" : "내부 섀도우 원장"} · {jobStateLabel[job.state]}</b>
      <span>{duration(job.elapsedMs)} · 24시간 실측 {job.actualElapsed24hQualified ? "적격" : "미충족"}</span>
      <small>시작 {dateTime(job.startedAtMs)} · 최근 heartbeat {dateTime(job.latestHeartbeatAtMs)}</small>
      {job.heartbeat?.streams ? Object.entries(job.heartbeat.streams).map(([streamId, value]) => <small key={streamId}>{streamId}: 메시지 {value.messages} · 재연결 {value.reconnects} · 오류 {value.errors} · 전송 타임아웃 {value.transportTimeouts} · 시세 공백 {value.marketGapEvents}</small>) : null}
      {job.mode === "shadow-contract" && job.heartbeat ? <small>원장 사건 {job.heartbeat.eventCount} · DB 사건 {job.heartbeat.ledgerCount} · 오류 {job.heartbeat.failureCount} · 대사 {job.heartbeat.reconciliationPassed ? "통과" : "실패"}</small> : null}
      {job.issues.length || job.warnings.length || job.collectionIssue ? <small className="settings-error">{[...job.issues, ...job.warnings, job.collectionIssue].filter(Boolean).join(" · ")}</small> : null}
    </section>)}
    {report ? <small>캐시 수집 {dateTime(report.collectedAtMs)} · 앱의 새로 읽기는 클라우드 API를 호출하지 않습니다.</small> : null}
  </article>;
}
