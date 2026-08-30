import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { appendShadowSoakSample, keepOnlyNewCandidateKey, parseShadowSoakSession, shadowSoakElapsedMs, shadowSoakReadyToFinalize, SHADOW_SOAK_SAMPLE_INTERVAL_MS, type ShadowSoakSample, type ShadowSoakSession } from "./shadowSoak";

const STORAGE_KEY = "investa.shadow-soak-session.v1";
const BOOT_ID = crypto.randomUUID();
type StoredAudit = { runId: string; sampleCount: number; actualElapsedQualified: boolean; audit: { durationMs: number; failClosed: boolean; warnings: string[]; maxObservationGapMs: number; restartReconciliationFailureCount: number } };

const elapsedLabel = (milliseconds: number) => {
  const totalMinutes = Math.floor(milliseconds / 60_000);
  return `${Math.floor(totalMinutes / 60)}시간 ${totalMinutes % 60}분`;
};

export function ShadowSoakPanel() {
  const [session, setSession] = useState<ShadowSoakSession | null>(() => parseShadowSoakSession(localStorage.getItem(STORAGE_KEY)));
  const [stored, setStored] = useState<StoredAudit | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const collecting = useRef(false);

  const persist = (next: ShadowSoakSession | null) => {
    if (next) localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
    else localStorage.removeItem(STORAGE_KEY);
    setSession(next);
  };

  const finalize = useCallback(async (target: ShadowSoakSession) => {
    const result = await invoke<StoredAudit>("shadow_soak_audit_save", { request: { runId: target.runId, samples: target.samples, simulatedTimeline: false } });
    setStored(result);
    persist(null);
    return result;
  }, []);

  const collect = useCallback(async (target: ShadowSoakSession) => {
    if (collecting.current) return;
    collecting.current = true;
    try {
      await invoke("operations_health_refresh");
      const rawSample = await invoke<ShadowSoakSample>("shadow_soak_sample", { request: { restarted: target.lastBootId !== BOOT_ID } });
      const sample = keepOnlyNewCandidateKey(target, rawSample);
      const next = appendShadowSoakSample(target, sample, BOOT_ID);
      persist(next);
      if (shadowSoakReadyToFinalize(next, Date.now())) await finalize(next);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      collecting.current = false;
    }
  }, [finalize]);

  useEffect(() => {
    if (!session || session.lastBootId !== BOOT_ID) return;
    const last = session.samples[session.samples.length - 1]?.observedAtMs ?? 0;
    if (Date.now() - last >= SHADOW_SOAK_SAMPLE_INTERVAL_MS) void collect(session);
    const timer = window.setInterval(() => void collect(parseShadowSoakSession(localStorage.getItem(STORAGE_KEY)) ?? session), SHADOW_SOAK_SAMPLE_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [collect, session?.runId]);

  const start = async () => {
    setBusy(true);
    try {
      const now = Date.now();
      const next: ShadowSoakSession = { schema: "investa.shadow-soak-session.v1", runId: `shadow-soak-${now}`, startedAtMs: now, lastBootId: BOOT_ID, samples: [] };
      persist(next);
      setStored(null);
      await collect(next);
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    if (!session || session.samples.length < 2) { setError("종료 결과를 저장하려면 표본이 두 개 이상 필요합니다."); return; }
    setBusy(true);
    try { await finalize(session); } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  };

  const reconcileAndContinue = async () => {
    if (!session) return;
    setBusy(true);
    try {
      await invoke("operations_runtime_reconcile");
      await collect(session);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };

  const elapsed = useMemo(() => session ? shadowSoakElapsedMs(session, Date.now()) : stored?.audit.durationMs ?? 0, [session, stored]);
  return <article className="shadow-soak-panel">
    <h4>24시간 내부 섀도우 내구 검사</h4>
    <p>실제 프로세스 메모리·SQLite 크기·활성 섀도우 작업자·내부 원장 상태를 1분마다 수집합니다. 앱 종료 공백과 재시작 대사 실패는 통과시키지 않습니다.</p>
    <dl><div><dt>상태</dt><dd>{session ? session.lastBootId !== BOOT_ID ? "재시작 대사 대기" : "검사 중" : stored ? stored.actualElapsedQualified && !stored.audit.failClosed ? "통과" : "완료·확인 필요" : "대기"}</dd></div><div><dt>실제 경과</dt><dd>{elapsedLabel(elapsed)}</dd></div><div><dt>표본</dt><dd>{session?.samples.length ?? stored?.sampleCount ?? 0}개</dd></div><div><dt>실주문</dt><dd>잠금</dd></div></dl>
    <div className="readiness-actions">{session ? <>{session.lastBootId !== BOOT_ID && <button type="button" disabled={busy} onClick={() => void reconcileAndContinue()}>재시작 원장 대사 후 계속</button>}<button type="button" disabled={busy} onClick={() => void stop()}>검사 종료·현재 결과 저장</button></> : <button type="button" disabled={busy} onClick={() => void start()}>24시간 검사 시작</button>}</div>
    {session && <small>실행 ID {session.runId} · 마지막 표본 {session.samples.length ? new Date(session.samples[session.samples.length - 1].observedAtMs).toLocaleString("ko-KR") : "수집 중"}</small>}
    {stored && <small>{stored.audit.warnings.join(" · ") || "내구 검사 경고 없음"}</small>}
    {error && <p className="ledger-error" role="alert">{error}</p>}
  </article>;
}
