import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { OperationalReadinessPanel } from "./OperationalReadinessPanel";
import type { AnalysisMarket } from "./analysisMarket";

type Account = { account: { currency: string; cashMinor: number; realizedPnlMinor: number; positions: Record<string, { quantity: number; costBasisMinor: number }>; eventCount: number } };
type LedgerEvent = Record<string, Record<string, unknown>>;
type Analysis = { recordId: string; kind: string; market: AnalysisMarket; totalReturnBps?: number | null; maxDrawdownBps?: number | null; completedTradeCount?: number | null };
type ManualOrder = { orderId: string; market: string; symbol: string; side: string; quantity: number; quantityScale: number; status: string; updatedAtMs: number };
type Candidate = { candidateId: string; symbol: string; side: string; quantity: number; status: string; source: string; updatedAtMs: number };
type ReplayEntry = { experimentId: string; classification: "system_check" | "research_experiment" | "promotion_candidate"; title: string; symbol: string; currency: string; side: string; occurredAtMs: number; referencePriceMinor: number; executionPriceMinor: number; quantity: number; feeMinor: number; taxMinor: number };
type ReplayRun = { experimentId: string; classification: ReplayEntry["classification"]; title: string; symbol: string; currency: string; initialCashMinor: number; finalCashMinor: number; finalEquityMinor: number; realizedPnlMinor: number; totalReturnBps: number; openPositionQuantity: number };
type ReplayHistory = { runs: ReplayRun[]; entries: ReplayEntry[] };
type EngineRunSummary = { runId: string; status: "completed" | "blocked" | "cancelled" | "interrupted"; symbol: string; market: string; candidateReady: boolean; updatedAtMs: number };
type EngineRunReport = { runId: string; analysisIds: string[]; status: EngineRunSummary["status"]; symbol: string; market: string; candidateReady: boolean; blockers: string[]; createdAtMs: number };
type EngineOverview = { totalRuns: number; candidateReadyRuns: number; blockedRuns: number; interruptedRuns: number; latestRun?: EngineRunSummary | null; liveOrderEnabled: false };
type EngineCandidate = { candidateId: string; runId: string; symbol: string; market: string; currency: string; side: string; quantity: number; status: string; updatedAtMs: number };
type MeetingPaperHandoff = { handoffId: string; workflowJobId: string; analysisRecordId: string; symbol: string; strategy: string; experimentId?: string | null; paperCandidateId?: string | null; engineRunId?: string | null; status: string; blocker?: string | null; updatedAtMs: number; liveOrderEnabled: false };
type OperationalAlert = { alertId: string; severity: "info" | "warning" | "critical"; message: string; occurrenceCount: number; acknowledgedAtMs?: number | null; lastSeenAtMs: number; response?: string | null };
type HealthReport = { automatedTradingReady: boolean; components: Array<{ componentId: string; healthy: boolean; critical: boolean; detail: string }> };
type BackupInspection = { fileName: string; integrityOk: boolean; schemaVersion: number; supportedSchemaVersion: number; restoreReady: boolean; blockers: string[]; auditEventCount: number; paperLedgerEventCount: number; researchReportCount: number };
type ReconciliationState = { status: "needs_reconciliation" | "ready"; requiredSinceMs?: number | null; completedAtMs?: number | null; mismatchCount: number; detail: string; candidateActionsLocked: boolean };
type RecoveryRehearsalReceipt = { sourceFileName: string; safetyBackupFileName: string; schemaVersion: number; paperLedgerEventCount: number; krwLedgerReplayed: boolean; usdLedgerReplayed: boolean; isolatedCopyRemoved: boolean; liveOrderEnabled: false };
type BackupInventoryEntry = { fileName: string; createdAtMs: number; sizeBytes: number; inspection: BackupInspection };
type ExperimentBiasAudit = { experimentId: string; datasetId: string; localStrategyTrialCount: number; walkForwardValidationCount: number; oosFoldCount: number; oosTradeCount: number; dataSnoopingStatus: "needs_review"; survivorshipBiasStatus: "needs_review"; catalogCompleteness: "local_only"; universeMembershipEvidence: "missing"; details: string[] };

const money = (minor: number, currency: string) => new Intl.NumberFormat("ko-KR", { style: "currency", currency, maximumFractionDigits: currency === "KRW" ? 0 : 2 }).format(minor / (currency === "KRW" ? 1 : 100));
const eventParts = (event: LedgerEvent) => { const [type, payload] = Object.entries(event)[0] ?? ["unknown", {}]; return { type, payload }; };
const marketLabel: Record<AnalysisMarket, string> = { kr: "국장", us: "미장", coin: "코인", securities_futures: "증권 선물", crypto_futures: "코인 선물", mixed: "복합" };

export function LedgerWorkspace() {
  const [tab, setTab] = useState<"ledger" | "performance">("ledger");
  const [ledgerSource, setLedgerSource] = useState<"internal" | "manual" | "shadow_engine" | "kis" | "backtest">("internal");
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [events, setEvents] = useState<Array<{ currency: string; event: LedgerEvent }>>([]);
  const [manualOrders, setManualOrders] = useState<ManualOrder[]>([]);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [replayEntries, setReplayEntries] = useState<ReplayEntry[]>([]);
  const [replayRuns, setReplayRuns] = useState<ReplayRun[]>([]);
  const [selectedReplayId, setSelectedReplayId] = useState<string>("");
  const [analyses, setAnalyses] = useState<Analysis[]>([]);
  const [engineOverview, setEngineOverview] = useState<EngineOverview | null>(null);
  const [engineRuns, setEngineRuns] = useState<EngineRunSummary[]>([]);
  const [latestEngineReport, setLatestEngineReport] = useState<EngineRunReport | null>(null);
  const [engineCandidates, setEngineCandidates] = useState<EngineCandidate[]>([]);
  const [meetingHandoffs, setMeetingHandoffs] = useState<MeetingPaperHandoff[]>([]);
  const [operationalAlerts, setOperationalAlerts] = useState<OperationalAlert[]>([]);
  const [healthReport, setHealthReport] = useState<HealthReport | null>(null);
  const [reconciliationState, setReconciliationState] = useState<ReconciliationState | null>(null);
  const [operationMessage, setOperationMessage] = useState<string | null>(null);
  const [backupFileName, setBackupFileName] = useState("");
  const [backupInventory, setBackupInventory] = useState<BackupInventoryEntry[]>([]);
  const [biasAudit, setBiasAudit] = useState<ExperimentBiasAudit | null>(null);
  const [alertResponses, setAlertResponses] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [policy, setPolicy] = useState({
    policyId: `risk-${Date.now()}`,
    maxOrderNotionalMinor: "1000000",
    maxBacktestDrawdownBps: "2000",
    stopLossBps: "500",
    takeProfitBps: "1000",
    dailyLossLimitMinor: "500000",
    lookbackHours: "168",
    lockDurationMinutes: "60",
    cooldownMinutes: "30",
    maximumStopLossCount: "3",
    maximumConsecutiveLossCount: "3",
    protectionMaximumDrawdownBps: "1500",
    minimumSymbolTradeCount: "5",
    minimumSymbolNetPnlMinor: "0",
  });
  const [protectionEnabled, setProtectionEnabled] = useState(false);
  const [policyMessage, setPolicyMessage] = useState<string | null>(null);
  const [recommendedPolicyId, setRecommendedPolicyId] = useState<string | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      const refreshedHealth = await invoke<HealthReport>("operations_health_refresh").catch(() => null);
      const [accountResult, krwEvents, usdEvents, orders, candidateResult, analysisResult, replayResult, overviewResult, engineRunResult, engineCandidateResult, handoffResult, alertResult, reconciliationResult, backupResult] = await Promise.all([
        invoke<{ accounts: Account[] }>("paper_accounts_status"),
        invoke<LedgerEvent[]>("paper_ledger_history", { currency: "KRW", limit: 100 }),
        invoke<LedgerEvent[]>("paper_ledger_history", { currency: "USD", limit: 100 }),
        invoke<ManualOrder[]>("manual_paper_orders"), invoke<Candidate[]>("paper_order_candidates"),
        invoke<Analysis[]>("analysis_record_history", { limit: 100 }),
        invoke<ReplayHistory>("backtest_replay_history", { limit: 100 }),
        invoke<EngineOverview>("engine_runtime_overview"),
        invoke<EngineRunSummary[]>("engine_run_history", { limit: 20 }),
        invoke<EngineCandidate[]>("engine_order_candidates"),
        invoke<MeetingPaperHandoff[]>("meeting_paper_handoff_history", { limit: 100 }),
        invoke<OperationalAlert[]>("operational_alerts"),
        invoke<ReconciliationState>("runtime_reconciliation_status"),
        invoke<BackupInventoryEntry[]>("local_backup_inventory"),
      ]);
      setAccounts(accountResult.accounts); setEvents([...krwEvents.map((event) => ({ currency: "KRW", event })), ...usdEvents.map((event) => ({ currency: "USD", event }))]);
      setManualOrders(orders); setCandidates(candidateResult); setAnalyses(analysisResult); setReplayEntries(replayResult.entries); setReplayRuns(replayResult.runs); setEngineOverview(overviewResult); setEngineRuns(engineRunResult); setSelectedReplayId((current) => replayResult.runs.some((run) => run.experimentId === current) ? current : replayResult.runs[0]?.experimentId ?? "");
      setEngineCandidates(engineCandidateResult); setMeetingHandoffs(handoffResult); setOperationalAlerts(alertResult);
      setHealthReport(refreshedHealth);
      setReconciliationState(reconciliationResult);
      setBackupInventory(backupResult);
      setLatestEngineReport(engineRunResult[0] ? await invoke<EngineRunReport>("engine_run_detail", { runId: engineRunResult[0].runId }) : null); setError(null);
    } catch (reason) { setError(String(reason)); } finally { setLoading(false); }
  };
  useEffect(() => { void load(); }, []);
  useEffect(() => {
    if (!selectedReplayId) { setBiasAudit(null); return; }
    void invoke<ExperimentBiasAudit>("backtest_experiment_bias_audit", { experimentId: selectedReplayId })
      .then(setBiasAudit)
      .catch((reason) => { setBiasAudit(null); setError(String(reason)); });
  }, [selectedReplayId]);
  const selectedReplay = replayRuns.find((run) => run.experimentId === selectedReplayId) ?? null;
  const selectedReplayEntries = replayEntries.filter((entry) => entry.experimentId === selectedReplayId);
  const performance = useMemo(() => (["kr", "us", "coin", "securities_futures", "crypto_futures"] as const).map((market) => {
    const runs = analyses.filter((item) => item.kind === "strategy" && item.market === market && item.totalReturnBps != null);
    const average = runs.length ? Math.round(runs.reduce((sum, item) => sum + (item.totalReturnBps ?? 0), 0) / runs.length) : null;
    const worstDrawdown = runs.length ? Math.max(...runs.map((item) => item.maxDrawdownBps ?? 0)) : null;
    return { market, count: runs.length, average, worstDrawdown, trades: runs.reduce((sum, item) => sum + (item.completedTradeCount ?? 0), 0) };
  }), [analyses]);
  const policyRequest = () => ({
    policy: {
      policyId: policy.policyId,
      maxOrderNotionalMinor: Number(policy.maxOrderNotionalMinor),
      maxBacktestDrawdownBps: Number(policy.maxBacktestDrawdownBps),
      stopLossBps: Number(policy.stopLossBps),
      takeProfitBps: Number(policy.takeProfitBps),
      dailyLossLimitMinor: Number(policy.dailyLossLimitMinor),
      protection: protectionEnabled ? {
        policyId: `${policy.policyId}-protection`,
        lookbackMs: Number(policy.lookbackHours) * 3_600_000,
        lockDurationMs: Number(policy.lockDurationMinutes) * 60_000,
        cooldownMs: Number(policy.cooldownMinutes) * 60_000,
        maximumStopLossCount: Number(policy.maximumStopLossCount),
        maximumConsecutiveLossCount: Number(policy.maximumConsecutiveLossCount),
        maximumDrawdownBps: Number(policy.protectionMaximumDrawdownBps),
        minimumSymbolTradeCount: Number(policy.minimumSymbolTradeCount),
        minimumSymbolNetPnlMinor: Number(policy.minimumSymbolNetPnlMinor),
      } : null,
    },
    experimentIds: analyses.filter((item) => item.kind === "strategy").map((item) => item.recordId),
  });
  const evaluatePolicy = async () => { try { const result = await invoke<{ experiments: Array<{ passed: boolean }>; unsupportedChecks: string[] }>("risk_policy_evaluate", { request: policyRequest() }); setPolicyMessage(`비교 ${result.experiments.length}건 · MDD 통과 ${result.experiments.filter((item) => item.passed).length}건. ${result.unsupportedChecks.join(" ")}`); } catch (reason) { setPolicyMessage(String(reason)); } };
  const savePolicy = async () => { try { await invoke("risk_policy_save_recommendation", { request: policyRequest() }); setRecommendedPolicyId(policy.policyId); setPolicyMessage("연구 추천안을 저장했습니다. 아직 활성 정책이 아니며 별도 승인이 필요합니다."); } catch (reason) { setPolicyMessage(String(reason)); } };
  const approvePolicy = async () => { if (!recommendedPolicyId) return; try { await invoke("risk_policy_approve", { policyId: recommendedPolicyId }); setPolicyMessage(`사용자 승인으로 주문 금액·MDD·손절·익절·일일손실${protectionEnabled ? "과 신규 진입 보호" : ""} 게이트를 활성화했습니다.`); setRecommendedPolicyId(null); } catch (reason) { setPolicyMessage(String(reason)); } };
  const cancelEngineRun = async (runId: string) => { try { await invoke("engine_run_cancel", { runId }); await load(); } catch (reason) { setError(String(reason)); } };
  const createEngineCandidate = async (runId: string) => { if (reconciliationState?.candidateActionsLocked !== false) { setError("재시작 원장 대사를 완료한 뒤 모의 후보를 생성할 수 있습니다."); return; } try { await invoke("engine_order_candidate_create", { request: { runId } }); setOperationMessage("엔진 결과를 내부 모의주문 후보로 만들었습니다. 체결에는 사용자 승인이 필요합니다."); await load(); } catch (reason) { setError(String(reason)); } };
  const approveEngineCandidate = async (candidateId: string) => { if (reconciliationState?.candidateActionsLocked !== false) { setError("재시작 원장 대사를 완료한 뒤 내부 모의체결을 승인할 수 있습니다."); return; } try { await invoke("engine_order_candidate_approve", { request: { candidateId } }); setOperationMessage("내부 모의원장에만 체결했습니다. 실전 주문은 잠겨 있습니다."); await load(); } catch (reason) { setError(String(reason)); } };
  const rejectEngineCandidate = async (candidateId: string) => { try { await invoke("engine_order_candidate_reject", { request: { candidateId } }); setOperationMessage("주문 후보를 기각했습니다."); await load(); } catch (reason) { setError(String(reason)); } };
  const reconcileRuntime = async () => { try { const result = await invoke<{ checkedCount: number; repairedCount: number; mismatchCount: number }>("operations_runtime_reconcile"); setOperationMessage(`대사 ${result.checkedCount}건 · 복구 ${result.repairedCount}건 · 불일치 ${result.mismatchCount}건`); await load(); } catch (reason) { setError(String(reason)); } };
  const createBackup = async () => { try { const result = await invoke<{ fileName: string }>("local_backup_create"); setBackupFileName(result.fileName); setOperationMessage(`무결성 검사를 통과한 로컬 백업을 만들었습니다: ${result.fileName}`); await load(); } catch (reason) { setError(String(reason)); } };
  const exportAudit = async () => { try { const result = await invoke<{ fileName: string; eventCount: number; sha256: string }>("audit_event_export"); setOperationMessage(`감사 사건 ${result.eventCount}건을 ${result.fileName}로 내보냈습니다. SHA-256 ${result.sha256.slice(0, 12)}…`); } catch (reason) { setError(String(reason)); } };
  const inspectBackup = async () => { try { const result = await invoke<BackupInspection>("local_backup_inspect", { request: { fileName: backupFileName } }); setOperationMessage(result.restoreReady ? `복원 사전검사 통과 · DB v${result.schemaVersion} · 감사 ${result.auditEventCount}건 · 원장 ${result.paperLedgerEventCount}건 · 연구 ${result.researchReportCount}건` : `복원 보류: ${result.blockers.join(" · ")}`); } catch (reason) { setError(String(reason)); } };
  const rehearseBackup = async () => { try { const result = await invoke<RecoveryRehearsalReceipt>("local_backup_rehearse", { request: { fileName: backupFileName } }); setOperationMessage(`격리 복구 훈련 통과 · DB v${result.schemaVersion} · KRW/USD 원장 재생 · 임시본 삭제 · 안전 백업 ${result.safetyBackupFileName}`); await load(); } catch (reason) { setError(String(reason)); } };
  const exportRecoveryEvidence = async () => { try { const result = await invoke<{ fileName: string; sha256: string }>("local_recovery_evidence_export", { request: { fileName: backupFileName } }); setOperationMessage(`복구 증거를 ${result.fileName}로 내보냈습니다. SHA-256 ${result.sha256.slice(0, 12)}…`); } catch (reason) { setError(String(reason)); } };
  const acknowledgeAlert = async (alert: OperationalAlert) => { const response = alertResponses[alert.alertId]?.trim() ?? ""; if (alert.severity === "critical" && !response) return; try { await invoke("operational_alert_acknowledge", { request: { alertId: alert.alertId, response: response || "운영 화면에서 확인" } }); setAlertResponses((current) => ({ ...current, [alert.alertId]: "" })); await load(); } catch (reason) { setError(String(reason)); } };

  return <main className="ledger-workspace">
    <header className="ledger-header"><div><p className="eyebrow">APPEND-ONLY PAPER LEDGER</p><h2>모의 원장·성과</h2><p>내부 체결 사건과 검증된 백테스트 성과만 표시합니다.</p></div><button type="button" onClick={() => void load()} disabled={loading}>{loading ? "검사 중" : "새로고침"}</button></header>
    <div className="ledger-tabs" role="tablist"><button role="tab" aria-selected={tab === "ledger"} className={tab === "ledger" ? "is-active" : ""} onClick={() => setTab("ledger")}>원장</button><button role="tab" aria-selected={tab === "performance"} className={tab === "performance" ? "is-active" : ""} onClick={() => setTab("performance")}>성과 분석</button></div>
    {error && <p className="ledger-error" role="alert">{error}</p>}
    {tab === "performance" && meetingHandoffs.length > 0 && <section className="ledger-panels" aria-label="회의 분석 인계 계보"><article><h3>회의 → 백테스트 → 섀도우 후보 계보</h3><p>회의 종합 보고는 주문으로 바로 전송되지 않습니다. 동일 분석 기록을 참조한 백테스트만 연결하며, 현재 신호가 생겨도 사용자 승인 전 내부 모의주문 후보로만 남습니다.</p>{meetingHandoffs.slice(0, 5).map((handoff) => <div className="ledger-order-row" key={handoff.handoffId}><div><strong>{handoff.symbol} · {handoff.status}</strong><span>{handoff.strategy}</span></div><small>{handoff.analysisRecordId} → {handoff.experimentId ?? "백테스트 대기"} → {handoff.paperCandidateId ?? handoff.engineRunId ?? "신호 감시"}{handoff.blocker ? ` · ${handoff.blocker}` : ""}</small></div>)}</article></section>}
    {tab === "performance" && reconciliationState?.candidateActionsLocked !== false && <div className="ledger-error" role="status"><strong>재시작 원장 대사 전 주문 후보 생성·승인이 잠겨 있습니다.</strong> <button type="button" onClick={() => void reconcileRuntime()}>지금 내부 원장 대사</button></div>}
    {tab === "ledger" ? <>
      <div className="ledger-tabs" aria-label="원장 출처">
        {([['internal', '내부 체결'], ['manual', '수동 주문'], ['shadow_engine', '섀도우 자동매매'], ['kis', 'KIS 모의'], ['backtest', '백테스트 재생']] as const).map(([id, label]) => <button key={id} type="button" aria-pressed={ledgerSource === id} className={ledgerSource === id ? "is-active" : ""} onClick={() => setLedgerSource(id)}>{label}</button>)}
      </div>
      {ledgerSource === "backtest" ? selectedReplay ? <><section className="ledger-account-grid"><article><span>{selectedReplay.currency} BACKTEST · INITIAL</span><strong>{money(selectedReplay.initialCashMinor, selectedReplay.currency)}</strong><small>{selectedReplay.symbol} · {selectedReplay.title}</small></article><article><span>{selectedReplay.currency} BACKTEST · FINAL EQUITY</span><strong>{money(selectedReplay.finalEquityMinor, selectedReplay.currency)}</strong><small>최종 현금 {money(selectedReplay.finalCashMinor, selectedReplay.currency)} · 미청산 수량 {selectedReplay.openPositionQuantity}</small></article><article><span>BACKTEST PERFORMANCE</span><strong className={selectedReplay.totalReturnBps >= 0 ? "is-positive" : "is-negative"}>{selectedReplay.totalReturnBps >= 0 ? "+" : ""}{(selectedReplay.totalReturnBps / 100).toFixed(2)}%</strong><small>실현손익 {money(selectedReplay.realizedPnlMinor, selectedReplay.currency)} · 내부 모의계좌 미반영</small></article></section><section className="ledger-panels"><article><div className="ledger-replay-heading"><div><h3>백테스트 재생 체결</h3><p>과거 데이터 재생 기록이며 내부 모의 잔고에는 반영되지 않습니다.</p></div><label>실험 선택<select value={selectedReplayId} onChange={(event) => setSelectedReplayId(event.currentTarget.value)}>{replayRuns.map((run) => <option key={run.experimentId} value={run.experimentId}>{run.symbol} · {run.title} · {(run.totalReturnBps / 100).toFixed(2)}%</option>)}</select></label></div><table><thead><tr><th>분류</th><th>종목·방향</th><th>체결가</th><th>비용</th><th>시각</th></tr></thead><tbody>{selectedReplayEntries.map((entry, index) => <tr key={`${entry.experimentId}-${index}`}><td>{entry.classification}</td><td>{entry.symbol} · {entry.side}</td><td>{money(entry.executionPriceMinor, entry.currency)}</td><td>{money(entry.feeMinor + entry.taxMinor, entry.currency)}</td><td>{new Date(entry.occurredAtMs).toLocaleString("ko-KR")}</td></tr>)}</tbody></table></article></section></> : <section className="ledger-panels"><article><h3>백테스트 재생 기록 없음</h3><p>저장된 백테스트를 실행하면 초기자금과 최종 평가자산을 이곳에 표시합니다.</p></article></section>
      : <section className="ledger-account-grid">{accounts.map(({ account }) => <article key={account.currency}><span>{account.currency} INTERNAL</span><strong>{money(account.cashMinor, account.currency)}</strong><small>실현손익 {money(account.realizedPnlMinor, account.currency)} · 사건 {account.eventCount}건 · 보유 {Object.keys(account.positions).length}종목</small></article>)}</section>}
      {ledgerSource === "backtest" && selectedReplay && biasAudit && <section className="ledger-panels" aria-label="저장 실험 편향 감사"><article><h3>저장 실험 편향 감사</h3><dl className="analysis-meta"><div><dt>동일 데이터 로컬 시도</dt><dd>{biasAudit.localStrategyTrialCount}회</dd></div><div><dt>OOS 검증</dt><dd>{biasAudit.walkForwardValidationCount}회 · {biasAudit.oosFoldCount}구간</dd></div><div><dt>OOS 거래</dt><dd>{biasAudit.oosTradeCount}건</dd></div></dl><div className="ledger-order-row"><strong>데이터 스누핑 · 검토 필요</strong><span>로컬 카탈로그만 집계됨</span></div><div className="ledger-order-row"><strong>생존편향 · 검토 필요</strong><span>과거 종목군 편입·상장폐지 근거 없음</span></div><small>{biasAudit.details.join(" ")}</small></article></section>}
      {ledgerSource === "backtest" ? null
      : ledgerSource === "kis" ? <section className="ledger-panels"><article><h3>KIS 모의 원장</h3><p>KIS 모의계좌 연결 보류 상태입니다. 연결 후 원격 주문·취소·대사 기록만 이 보기에서 표시합니다.</p></article></section>
      : <section className="ledger-panels"><article><h3>{ledgerSource === "internal" ? "불변 체결 원장" : ledgerSource === "manual" ? "수동 주문 기록" : "섀도우 자동매매 기록"}</h3>{ledgerSource === "internal" ? <table><thead><tr><th>통화</th><th>사건</th><th>종목·주문</th><th>시각</th></tr></thead><tbody>{events.map(({ currency, event }, index) => { const { type, payload } = eventParts(event); return <tr key={`${currency}-${index}`}><td>{currency}</td><td>{type}</td><td>{String(payload.symbol ?? payload.orderId ?? payload.accountId ?? "-")}</td><td>{payload.occurredAtMs ? new Date(Number(payload.occurredAtMs)).toLocaleString("ko-KR") : "-"}</td></tr>; })}</tbody></table> : <p>주문 출처별 상태를 오른쪽에서 확인합니다. 체결 사건은 변조 방지를 위해 통합 불변 원장에 보존됩니다.</p>}</article>
      <article><h3>주문 상태</h3>{[...manualOrders.map((order) => ({ id: order.orderId, symbol: order.symbol, side: order.side, quantity: order.quantity, status: order.status, source: "manual" })), ...candidates.map((candidate) => ({ id: candidate.candidateId, symbol: candidate.symbol, side: candidate.side, quantity: candidate.quantity, status: candidate.status, source: candidate.source }))].filter((order) => ledgerSource === "internal" || order.source === ledgerSource).map((order) => <div className="ledger-order-row" key={order.id}><strong>{order.symbol} · {order.side} {order.quantity}</strong><span>{order.source} · {order.status}</span></div>)}</article></section>}
    </> : <><section className="performance-grid">{performance.map((item) => <article key={item.market}><span>{marketLabel[item.market]} BACKTEST</span><strong>{item.average == null ? "기록 없음" : `평균 ${(item.average / 100).toFixed(2)}%`}</strong><dl><div><dt>검증 수</dt><dd>{item.count}</dd></div><div><dt>최대 MDD</dt><dd>{item.worstDrawdown == null ? "-" : `${(item.worstDrawdown / 100).toFixed(2)}%`}</dd></div><div><dt>완료 거래</dt><dd>{item.trades}</dd></div></dl><small>서로 다른 기간·전략의 단순 평균이며 미래 성과 예측값이 아닙니다.</small></article>)}</section><section className="ledger-panels"><article><div className="ledger-replay-heading"><div><h3>통합 분석 엔진</h3><p>시점 정합 데이터 → 스크리닝 → 상·하방 토론 → 위험 심의 → 내부 모의 후보의 상태입니다.</p></div><strong>{engineOverview?.liveOrderEnabled ? "실주문 허용" : "실주문 잠금"}</strong></div><dl className="analysis-meta"><div><dt>전체 실행</dt><dd>{engineOverview?.totalRuns ?? 0}</dd></div><div><dt>후보 준비</dt><dd>{engineOverview?.candidateReadyRuns ?? 0}</dd></div><div><dt>차단·중단</dt><dd>{(engineOverview?.blockedRuns ?? 0) + (engineOverview?.interruptedRuns ?? 0)}</dd></div></dl>{latestEngineReport && <div className="ledger-order-row"><strong>최근 {latestEngineReport.symbol} · {latestEngineReport.candidateReady ? "내부 모의 후보 준비" : latestEngineReport.status}</strong><span>{latestEngineReport.blockers.length ? latestEngineReport.blockers.join(" · ") : "모든 결정론적 게이트 통과"}</span></div>}<table><thead><tr><th>종목</th><th>시장</th><th>상태</th><th>갱신 시각</th><th>작업</th></tr></thead><tbody>{engineRuns.length ? engineRuns.map((run) => <tr key={run.runId}><td>{run.symbol}</td><td>{run.market}</td><td>{run.candidateReady ? "후보 준비" : run.status}</td><td>{new Date(run.updatedAtMs).toLocaleString("ko-KR")}</td><td>{run.status === "blocked" || run.status === "interrupted" ? <button type="button" onClick={() => void cancelEngineRun(run.runId)}>실행 취소</button> : run.candidateReady && !engineCandidates.some((candidate) => candidate.runId === run.runId) ? <button type="button" disabled={reconciliationState?.candidateActionsLocked !== false} title={reconciliationState?.candidateActionsLocked !== false ? "재시작 원장 대사 후 사용할 수 있습니다." : undefined} onClick={() => void createEngineCandidate(run.runId)}>모의 후보 생성</button> : "-"}</td></tr>) : <tr><td colSpan={5}>아직 통합 엔진 실행 기록이 없습니다.</td></tr>}</tbody></table><small>확률·신뢰도는 분석 화면의 표본 기준을 충족한 경우에만 표시하며, 후보 준비는 주문 체결을 뜻하지 않습니다.</small></article><article><h3>엔진 모의주문 후보</h3>{engineCandidates.length ? engineCandidates.map((candidate) => <div className="ledger-order-row" key={candidate.candidateId}><div><strong>{candidate.symbol} · {candidate.side} {candidate.quantity}</strong><span>{candidate.currency} · {candidate.status}</span></div>{candidate.status === "safety_approved" && <div><button type="button" disabled={reconciliationState?.candidateActionsLocked !== false} title={reconciliationState?.candidateActionsLocked !== false ? "재시작 원장 대사 후 사용할 수 있습니다." : undefined} onClick={() => void approveEngineCandidate(candidate.candidateId)}>내부 모의체결 승인</button><button type="button" onClick={() => void rejectEngineCandidate(candidate.candidateId)}>기각</button></div>}</div>) : <p>생성된 엔진 주문 후보가 없습니다.</p>}</article></section><section className="ledger-panels"><article><div className="ledger-replay-heading"><div><h3>운영 안전 상태</h3><p>원장 대사, 공급자 상태 만료, 중복 제거 알림과 로컬 백업을 관리합니다.</p></div><strong>{healthReport?.automatedTradingReady ? "자동운용 준비" : "확인 필요"}</strong></div><div className="risk-policy-actions"><button type="button" onClick={() => void reconcileRuntime()}>내부 원장 대사</button><button type="button" onClick={() => void createBackup()}>검증 백업 생성</button></div>{operationMessage && <p role="status">{operationMessage}</p>}{healthReport?.components.map((component) => <div className="ledger-order-row" key={component.componentId}><strong>{component.componentId}</strong><span>{component.healthy ? "정상" : component.detail}</span></div>)}</article><article><h3>운영 알림</h3>{operationalAlerts.length ? operationalAlerts.map((alert) => <div className="ledger-order-row" key={alert.alertId}><strong>{alert.severity.toUpperCase()} · {alert.message}</strong><span>{alert.occurrenceCount}회 · {alert.acknowledgedAtMs ? "확인 완료" : "미확인"}</span></div>) : <p>현재 저장된 운영 알림이 없습니다.</p>}</article></section><section className="risk-policy-panel"><header><div><span>RESEARCH → REVIEW → APPROVE</span><h3>위험 정책 비교</h3></div><strong>활성화 전 별도 승인</strong></header><div className="risk-policy-fields">{Object.entries({ maxOrderNotionalMinor: "주문금액 한도", maxBacktestDrawdownBps: "백테스트 MDD(bp)", stopLossBps: "손절(bp)", takeProfitBps: "익절(bp)", dailyLossLimitMinor: "일일손실 한도" }).map(([key, label]) => <label key={key}>{label}<input type="number" min="1" value={policy[key as keyof typeof policy]} onChange={(event) => setPolicy((current) => ({ ...current, [key]: event.currentTarget.value }))} /></label>)}</div><div className="risk-policy-actions"><button type="button" onClick={() => void evaluatePolicy()}>저장 백테스트와 비교</button><button type="button" onClick={() => void savePolicy()}>연구 추천안 저장</button><button type="button" className="is-approve" disabled={!recommendedPolicyId} onClick={() => void approvePolicy()}>추천안 승인·활성화</button></div>{policyMessage && <p role="status">{policyMessage}</p>}<small>현재 활성 게이트는 주문 금액과 백테스트 MDD만 강제합니다. 손절·익절·일일손실은 엔진이 실제 계산하기 전까지 저장만 되며 적용됐다고 표시하지 않습니다.</small></section></>}
    {tab === "performance" && <section className="risk-policy-panel" aria-labelledby="strategy-protection-title">
      <header><div><span>OPTIONAL · NEW ENTRY ONLY</span><h3 id="strategy-protection-title">전략 보호정책</h3></div><strong>{protectionEnabled ? "추천안에 포함" : "사용 안 함"}</strong></header>
      <label><input type="checkbox" checked={protectionEnabled} onChange={(event) => setProtectionEnabled(event.currentTarget.checked)} /> 쿨다운·연속손실·낙폭 보호를 위험정책 추천안에 포함</label>
      {protectionEnabled && <div className="risk-policy-fields">
        {([
          ["lookbackHours", "관측 기간(시간)"],
          ["lockDurationMinutes", "잠금 기간(분)"],
          ["cooldownMinutes", "종목 쿨다운(분)"],
          ["maximumStopLossCount", "반복 손절 횟수"],
          ["maximumConsecutiveLossCount", "연속 손실 횟수"],
          ["protectionMaximumDrawdownBps", "보호 최대낙폭(bp)"],
          ["minimumSymbolTradeCount", "종목 최소 거래 수"],
          ["minimumSymbolNetPnlMinor", "종목 최소 순손익"],
        ] as const).map(([key, label]) => <label key={key}>{label}<input type="number" min={key === "minimumSymbolNetPnlMinor" ? undefined : "1"} value={policy[key]} onChange={(event) => setPolicy((current) => ({ ...current, [key]: event.currentTarget.value }))} /></label>)}
      </div>}
      <small>활성화하면 저장 백테스트에서 트리거를 비교하고, 승인 후에는 내부 KRW·USD 모의원장의 종료 거래를 기준으로 신규 매수 후보 생성과 승인 시 다시 검사합니다. 매도와 실전 주문은 각각 위험 축소 허용·전송 금지입니다.</small>
    </section>}
    {tab === "performance" && <section className="ledger-panels" aria-label="운영 복구 도구"><article><h3>감사·백업 복구 도구</h3><p>감사 원본은 변경하지 않고 앱 데이터 폴더로 내보냅니다. 복구 훈련은 운영 DB를 교체하지 않고 격리 복사본의 마이그레이션·무결성·KRW/USD 원장 재생을 검사합니다.</p><div className="risk-policy-actions"><button type="button" onClick={() => void exportAudit()}>감사 JSON 내보내기</button><button type="button" onClick={() => void createBackup()}>새 검증 백업</button></div><label>검사할 백업 파일명<input value={backupFileName} onChange={(event) => setBackupFileName(event.currentTarget.value)} placeholder="investa-123456.sqlite3" /></label><div className="risk-policy-actions"><button type="button" disabled={!backupFileName.trim()} onClick={() => void inspectBackup()}>복원 사전검사</button><button type="button" disabled={!backupFileName.trim()} onClick={() => void rehearseBackup()}>격리 복구 훈련</button></div><div className="ledger-order-row"><strong>재시작 주문 잠금</strong><span>{reconciliationState?.candidateActionsLocked ? `잠김 · ${reconciliationState.detail}` : `대사 완료 · ${reconciliationState?.detail ?? "상태 확인 중"}`}</span></div></article><article><h3>미확인 운영 알림 대응</h3>{operationalAlerts.filter((alert) => !alert.acknowledgedAtMs).length ? operationalAlerts.filter((alert) => !alert.acknowledgedAtMs).map((alert) => <div className="ledger-order-row" key={`response-${alert.alertId}`}><div><strong>{alert.severity.toUpperCase()} · {alert.message}</strong><span>{alert.occurrenceCount}회 발생</span></div><label>대응 내용<input value={alertResponses[alert.alertId] ?? ""} onChange={(event) => setAlertResponses((current) => ({ ...current, [alert.alertId]: event.currentTarget.value }))} placeholder={alert.severity === "critical" ? "치명 알림은 대응 내용 필수" : "선택 입력"} /></label><button type="button" disabled={alert.severity === "critical" && !(alertResponses[alert.alertId]?.trim())} onClick={() => void acknowledgeAlert(alert)}>확인 처리</button></div>) : <p>미확인 운영 알림이 없습니다.</p>}</article></section>}
    {tab === "performance" && <section className="ledger-panels" aria-label="백업 목록과 복구 증거"><article><h3>검증 백업 목록</h3><p>앱 전용 백업 폴더의 최근 100개만 읽기 전용 사전검사와 함께 표시합니다.</p>{backupInventory.length ? backupInventory.map((entry) => <button key={entry.fileName} type="button" className="ledger-order-row" aria-pressed={backupFileName === entry.fileName} onClick={() => setBackupFileName(entry.fileName)}><strong>{entry.fileName}</strong><span>{entry.inspection.restoreReady ? "복원 준비" : "복원 보류"} · {(entry.sizeBytes / 1024 / 1024).toFixed(1)} MB · {new Date(entry.createdAtMs).toLocaleString("ko-KR")}</span></button>) : <p>아직 생성된 검증 백업이 없습니다.</p>}<div className="risk-policy-actions"><button type="button" disabled={!backupFileName.trim()} onClick={() => void exportRecoveryEvidence()}>선택 백업 복구 증거 내보내기</button></div></article></section>}
    {tab === "performance" && <OperationalReadinessPanel />}
  </main>;
}
