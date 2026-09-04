import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PublicityReviewPanel } from "./PublicityReviewPanel";
import { OperationsDrillPanel } from "./OperationsDrillPanel";
import { MarketStreamStatusPanel } from "./MarketStreamStatusPanel";
import { ShadowSoakPanel } from "./ShadowSoakPanel";
import { CloudSoakStatusPanel } from "./CloudSoakStatusPanel";

type PortfolioMandate = "observation_only" | "focused" | "thematic" | "diversified" | "custom";
type WorkspacePreferences = { displayTimezone: "Asia/Seoul" | "America/New_York" | "UTC"; quietHoursStart: number; quietHoursEnd: number; staleAfterSeconds: number; notifyWarning: boolean; notifyCritical: boolean; portfolioMandate: PortfolioMandate; concentrationLimitsEnabled: boolean; maximumSymbolExposureBps: number; maximumSectorExposureBps: number; maximumMarketExposureBps: number };
type DashboardSnapshot = { observedAtMs: number; sourceTimestampsMs: Record<string, number>; counts: Record<string, number>; liveOrderEnabled: false; warnings: string[] };
type ProtectionDecision = { decisionId: number; createdAtMs: number; decision: { policyId: string; targetSymbol: string; evaluatedAtMs: number; canOpenNewPosition: boolean; globalLockUntilMs?: number | null; symbolLockUntilMs?: number | null; triggers: Array<{ code: string; reason?: string; observed?: string; lockedUntilMs: number }> } };
type PortfolioSnapshot = { snapshotId: string; createdAtMs: number; report: { currency: string; positionCount: number; historicalVar95Bps?: number | null; historicalCvar95Bps?: number | null; concentrationHhiBps: number; stressedPortfolioReturnBps?: number | null; warnings: string[] } };
type MarketCalendar = { market: "KR" | "US"; provider: string; fetchedAtMs: number; date: string; holiday: boolean; previousBusinessDay: string; nextBusinessDay: string; sessions: Array<{ name: string; startTime: string; endTime: string }> };
type MarketCalendars = { fetchedAtMs: number; calendars: MarketCalendar[] };

const dateTime = (value?: number | null) => value ? new Date(value).toLocaleString("ko-KR") : "관측 없음";
const percent = (value?: number | null) => value == null ? "계산 대기" : `${(value / 100).toFixed(2)}%`;

export function OperationalReadinessPanel() {
  const [preferences, setPreferences] = useState<WorkspacePreferences | null>(null);
  const [dashboard, setDashboard] = useState<DashboardSnapshot | null>(null);
  const [protection, setProtection] = useState<ProtectionDecision[]>([]);
  const [portfolio, setPortfolio] = useState<PortfolioSnapshot[]>([]);
  const [calendars, setCalendars] = useState<MarketCalendars | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = async () => {
    try {
      const [nextPreferences, nextDashboard, nextProtection, nextPortfolio, nextCalendars] = await Promise.all([
        invoke<WorkspacePreferences>("workspace_preferences_get"), invoke<DashboardSnapshot>("operations_dashboard_snapshot"),
        invoke<ProtectionDecision[]>("strategy_protection_history", { limit: 20 }), invoke<PortfolioSnapshot[]>("portfolio_risk_snapshot_history", { limit: 20 }),
        invoke<MarketCalendars>("toss_market_calendars").catch(() => null),
      ]);
      setPreferences(nextPreferences); setDashboard(nextDashboard); setProtection(nextProtection); setPortfolio(nextPortfolio); setCalendars(nextCalendars); setError(null);
    } catch (reason) { setError(String(reason)); }
  };
  useEffect(() => { void load(); }, []);
  const staleSources = useMemo(() => !dashboard || !preferences ? [] : Object.entries(dashboard.sourceTimestampsMs).filter(([, observed]) => !observed || dashboard.observedAtMs - observed > preferences.staleAfterSeconds * 1_000).map(([source]) => source), [dashboard, preferences]);
  const savePreferences = async () => { if (!preferences) return; try { setPreferences(await invoke("workspace_preferences_save", { preferences })); setMessage("운영 설정과 포트폴리오 운용 원칙을 저장했습니다."); setError(null); } catch (reason) { setError(String(reason)); } };
  const composePortfolioRisk = async (currency: "KRW" | "USD") => { try { await invoke("portfolio_risk_from_ledger", { request: { snapshotId: `ledger-${currency.toLowerCase()}-${Date.now()}`, currency, stressShocksBps: {} } }); setMessage(`${currency} 내부 원장과 저장 가격 데이터로 위험 스냅샷을 만들었습니다.`); await load(); } catch (reason) { setError(String(reason)); } };

  return <section className="operational-readiness" aria-labelledby="operational-readiness-title">
    <header><div><span>LOCAL OPERATIONS CONTROL</span><h3 id="operational-readiness-title">운영 준비·근거·복구</h3><p>각 수치의 관측 시각을 함께 표시하며 실전 주문은 항상 잠겨 있습니다.</p></div><button type="button" onClick={() => void load()}>상태 다시 읽기</button></header>
    {error && <p className="ledger-error" role="alert">{error}</p>}{message && <p role="status">{message}</p>}
    <div className="operational-readiness-grid">
      <article><h4>실시간 운영 개요</h4><strong>{dashboard?.liveOrderEnabled ? "실주문 허용" : "SHADOW ONLY"}</strong><dl>{Object.entries(dashboard?.counts ?? {}).map(([key, value]) => <div key={key}><dt>{key}</dt><dd>{value}</dd></div>)}</dl><small>집계 {dateTime(dashboard?.observedAtMs)} · 만료 출처 {staleSources.length ? staleSources.join(", ") : "없음"}</small></article>
      <article><h4>전략 보호 판정</h4>{protection.length ? protection.slice(0, 5).map((item) => <div className="readiness-row" key={item.decisionId}><b>{item.decision.targetSymbol} · {item.decision.canOpenNewPosition ? "신규 진입 허용" : "잠금"}</b><span>{item.decision.triggers.map((trigger) => trigger.reason ?? trigger.observed ?? trigger.code).join(" · ") || "활성 트리거 없음"}</span><small>판정 {dateTime(item.decision.evaluatedAtMs)} · 만료 {dateTime(item.decision.symbolLockUntilMs ?? item.decision.globalLockUntilMs)}</small></div>) : <p>저장된 보호 판정이 없습니다.</p>}</article>
      <article><h4>통화별 포트폴리오 위험</h4><div className="readiness-actions"><button type="button" onClick={() => void composePortfolioRisk("KRW")}>KRW 원장에서 구성</button><button type="button" onClick={() => void composePortfolioRisk("USD")}>USD 원장에서 구성</button></div>{portfolio.length ? portfolio.slice(0, 5).map((item) => <div className="readiness-row" key={item.snapshotId}><b>{item.report.currency} · {item.report.positionCount}종목</b><span>VaR 95 {percent(item.report.historicalVar95Bps)} · CVaR {percent(item.report.historicalCvar95Bps)} · 집중도 {percent(item.report.concentrationHhiBps)}</span><small>{dateTime(item.createdAtMs)} · 통화 혼합 합산 안 함</small></div>) : <p>저장된 위험 스냅샷이 없습니다. 원장 포지션별 시점 정합 수익률을 구성한 뒤 저장하세요.</p>}</article>
      <article><h4>시장·시간·알림 설정</h4>{preferences && <div className="readiness-form"><label>표시 시간대<select value={preferences.displayTimezone} onChange={(event) => setPreferences({ ...preferences, displayTimezone: event.currentTarget.value as WorkspacePreferences["displayTimezone"] })}><option value="Asia/Seoul">KST · 서울</option><option value="America/New_York">ET · 뉴욕</option><option value="UTC">UTC</option></select></label><label>데이터 만료(초)<input type="number" min="30" max="86400" value={preferences.staleAfterSeconds} onChange={(event) => setPreferences({ ...preferences, staleAfterSeconds: Number(event.currentTarget.value) })} /></label><label><input type="checkbox" checked={preferences.notifyWarning} onChange={(event) => setPreferences({ ...preferences, notifyWarning: event.currentTarget.checked })} /> 경고 알림</label><label><input type="checkbox" checked={preferences.notifyCritical} onChange={(event) => setPreferences({ ...preferences, notifyCritical: event.currentTarget.checked })} /> 치명 알림</label><label>포트폴리오 운용 원칙<select value={preferences.portfolioMandate} onChange={(event) => setPreferences({ ...preferences, portfolioMandate: event.currentTarget.value as PortfolioMandate })}><option value="observation_only">관측 전용 · 앱 판단 없음</option><option value="focused">집중 투자</option><option value="thematic">테마 투자</option><option value="diversified">분산 투자</option><option value="custom">사용자 정의</option></select></label><label><input type="checkbox" checked={preferences.concentrationLimitsEnabled} onChange={(event) => setPreferences({ ...preferences, concentrationLimitsEnabled: event.currentTarget.checked })} /> 집중 한도를 판정 기준으로 사용</label>{preferences.concentrationLimitsEnabled && <><label>종목 최대 비중(%)<input type="number" min="0.01" max="100" step="0.01" value={preferences.maximumSymbolExposureBps / 100} onChange={(event) => setPreferences({ ...preferences, maximumSymbolExposureBps: Math.round(Number(event.currentTarget.value) * 100) })} /></label><label>섹터 최대 비중(%)<input type="number" min="0.01" max="100" step="0.01" value={preferences.maximumSectorExposureBps / 100} onChange={(event) => setPreferences({ ...preferences, maximumSectorExposureBps: Math.round(Number(event.currentTarget.value) * 100) })} /></label><label>시장 최대 비중(%)<input type="number" min="0.01" max="100" step="0.01" value={preferences.maximumMarketExposureBps / 100} onChange={(event) => setPreferences({ ...preferences, maximumMarketExposureBps: Math.round(Number(event.currentTarget.value) * 100) })} /></label></>}<small>비중은 항상 관측할 수 있지만, 이 스위치를 켜기 전에는 집중을 위반·감축·매도 사유로 사용하지 않습니다.</small><button type="button" onClick={() => void savePreferences()}>운영 설정 저장</button></div>}</article>
      <article><h4>토스 공식 장 캘린더</h4>{calendars ? calendars.calendars.map((calendar) => {
        const now = Date.now(); const active = calendar.sessions.find((session) => Date.parse(session.startTime) <= now && now < Date.parse(session.endTime));
        return <div className="readiness-row" key={calendar.market}><b>{calendar.market} · {calendar.holiday ? "휴장" : active ? `${active.name} 운영 중` : "장외 시간"}</b><span>{calendar.sessions.map((session) => `${session.name} ${new Date(session.startTime).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}–${new Date(session.endTime).toLocaleTimeString("ko-KR", { hour: "2-digit", minute: "2-digit" })}`).join(" · ") || "오늘 운영 세션 없음"}</span><small>{calendar.date} · 다음 영업일 {calendar.nextBusinessDay} · {calendar.provider}</small></div>;
      }) : <p>토스증권 연결 후 공식 KR·US 장 운영 시간을 확인합니다. 고정 시간표를 추정하지 않습니다.</p>}</article>
      <MarketStreamStatusPanel />
      <CloudSoakStatusPanel />
      <ShadowSoakPanel />
      <OperationsDrillPanel onMessage={setMessage} onError={setError} />
      <PublicityReviewPanel onMessage={setMessage} onError={setError} />
    </div>
  </section>;
}
