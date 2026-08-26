import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type DrillScenario = "order_rejected" | "partial_fill" | "stale_market_data" | "broker_outage" | "loss_limit" | "reconciliation_mismatch";
type DrillResult = {
  drillId: string; scenario: DrillScenario; severity: string; observation: string;
  recommendedActions: string[]; killSwitchRequired: boolean; newEntriesAllowed: boolean;
  cancellationAllowed: boolean; executedAtMs: number; liveOrderAllowed: false;
};

const scenarios: Array<{ value: DrillScenario; label: string }> = [
  { value: "order_rejected", label: "주문 거부" },
  { value: "partial_fill", label: "부분 체결" },
  { value: "stale_market_data", label: "시세 지연" },
  { value: "broker_outage", label: "증권사 장애" },
  { value: "loss_limit", label: "손실 한도 초과" },
  { value: "reconciliation_mismatch", label: "원장 불일치" },
];

export function OperationsDrillPanel({ onMessage, onError }: { onMessage: (message: string) => void; onError: (message: string | null) => void }) {
  const [scenario, setScenario] = useState<DrillScenario>("stale_market_data");
  const [observation, setObservation] = useState("격리된 모의 운영 장애 훈련");
  const [history, setHistory] = useState<DrillResult[]>([]);
  const [busy, setBusy] = useState(false);

  const loadHistory = async () => {
    try { setHistory(await invoke<DrillResult[]>("operations_drill_history", { limit: 10 })); }
    catch (reason) { onError(String(reason)); }
  };
  useEffect(() => { void loadHistory(); }, []);

  const execute = async () => {
    setBusy(true);
    try {
      const now = Date.now();
      const result = await invoke<DrillResult>("operations_drill_execute", { request: { drillId: `ops-${scenario}-${now}`, scenario, observation, executedAtMs: now } });
      onMessage(`${result.killSwitchRequired ? "킬 스위치 필요" : "경고 대응"} · 신규 진입 ${result.newEntriesAllowed ? "허용" : "차단"} · 취소는 유지`);
      onError(null);
      await loadHistory();
    } catch (reason) { onError(String(reason)); }
    finally { setBusy(false); }
  };

  return <article className="readiness-drill">
    <h4>알림·킬 스위치 운영 훈련</h4>
    <p>실주문 없이 장애 시나리오를 판정하고, 신규 진입 차단·취소 허용·대응 절차를 감사 원장에 남깁니다.</p>
    <label>장애 시나리오<select value={scenario} onChange={(event) => setScenario(event.currentTarget.value as DrillScenario)}>{scenarios.map((item) => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
    <label>관측 내용<textarea rows={3} value={observation} onChange={(event) => setObservation(event.currentTarget.value)} /></label>
    <button type="button" disabled={busy || observation.trim().length < 3} onClick={() => void execute()}>{busy ? "훈련 기록 중" : "격리 훈련 실행"}</button>
    <div aria-live="polite">{history.length ? history.slice(0, 5).map((item) => <div className="readiness-row" key={item.drillId}><b>{scenarios.find((scenarioItem) => scenarioItem.value === item.scenario)?.label ?? item.scenario} · {item.severity}</b><span>{item.newEntriesAllowed ? "신규 진입 허용" : "신규 진입 차단"} · 취소 {item.cancellationAllowed ? "허용" : "차단"} · 실주문 미수행</span><small>{new Date(item.executedAtMs).toLocaleString("ko-KR")} · {item.recommendedActions.join(" → ")}</small></div>) : <p>저장된 운영 훈련이 없습니다.</p>}</div>
  </article>;
}
